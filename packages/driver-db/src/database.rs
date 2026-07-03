use std::collections::{BTreeSet, HashSet};
use std::fmt::Display;
use std::hash::Hash;
use std::io::Read;
use std::ops::Deref;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};

use crate::{Object, Objects, Options, RemoteObjects};
use driver_util::SerializedMap;

/// Tracks the range [changed_at, verified_at], to confirm the value is corresponds to is the same
/// for that entire range of revisions.
#[derive(Copy, Clone, Default, Debug, Serialize, Deserialize)]
pub struct Revision {
    /// The revision at which we've executed a query and noticed that the value has changed.
    pub changed_at: usize,
    /// The revision at which we've verified a value has not changed since changed_at.
    pub verified_at: usize,
}

/// Represents the current known state of a query. It is bundled together because it should all be
/// operated on at once.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Value<Output> {
    value: Output,
    #[serde(skip)]
    #[serde(default)]
    revision: Revision,
}

/// Represents a value as it's being computed by the system. Allows for multiple logcial queries
/// for the same key to be in-flight at the same time, while only doing one actual computation.
#[derive(Debug, Serialize, Deserialize)]
enum LogicalValue<Output> {
    Materialized(Value<Output>),
    /// This is just a oneshot because each entry notifies just the next one waiting that its it's
    /// turn. I think this is slightly less efficient than using a condition variable + mutex to
    /// gate tasks one-at-a-time, but it's more correct than async_broadcast which is my closest
    /// alternative.
    /// NOTE: acutally, I'm not so sure about this! The receiver could be in a thread
    /// that's currently doing a lot of other work, and there could be lots of other things waiting
    /// on it that need to complete as well. Serializing things this way doesn't seem ideal, but
    /// getting a "real" "hey whoever can take this next, it's up for grabs" seems a bit harder.
    #[serde(skip)]
    Computing(ThreadsafeReceiver<Value<Output>>),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Core<Key: Hash + Ord + Eq, Output> {
    #[serde(skip)]
    pub revision: AtomicUsize,

    /// Used to check that, when a `LogicalValue::Computing` is inserted/taken out, we get the same
    /// one back. We do this because we can't compare the `oneshot::Receiver`s directly.
    #[serde(skip)]
    with_entry_nonce: AtomicUsize,
    cache: SerializedMap<Key, LogicalValue<Output>>,
    dep_graph: SerializedMap<Key, BTreeSet<Key>>,
}

/// Manual impl to avoid extraneous bounds on `Output`.
impl<Key: Hash + Ord + Eq, Output> Default for Core<Key, Output> {
    fn default() -> Self {
        Self {
            revision: Default::default(),
            with_entry_nonce: Default::default(),
            cache: Default::default(),
            dep_graph: Default::default(),
        }
    }
}

#[derive(Debug)]
pub struct Database<Key: Hash + Ord + Eq, Output> {
    core: Core<Key, Output>,
    pub objects: Objects,
    pub remotes: RemoteObjects,
}

impl<Key: Hash + Ord + Eq, Output> Deref for Database<Key, Output> {
    type Target = Core<Key, Output>;

    fn deref(&self) -> &Self::Target {
        &self.core
    }
}

impl<Key: driver_util::Key, Output: driver_util::Output> Core<Key, Output> {
    pub fn add_dependency(&self, parent: Key, child: Key) {
        let entry = self.dep_graph.entry_sync(parent);
        let mut child = BTreeSet::from([child]);
        entry
            .and_modify(|deps| {
                deps.append(&mut child);
            })
            .or_insert(child);
    }

    pub fn remove_all_dependencies(&self, parent: &Key) {
        self.dep_graph.remove_sync(parent);
    }

    pub fn dependencies<T: FromIterator<Key>>(&self, parent: &Key) -> Option<T> {
        let deps = self.dep_graph.get_sync(parent)?;
        Some(deps.get().iter().cloned().collect())
    }

    /// Running this acquires a lock on the given entry, meaning the current task will suspend
    /// until the entry is unlocked by the task that currently has it acquired. This is necessary
    /// for us to run each query exactly once per revision, otherwise we could be running the same
    /// query concurrently (does too much work).
    pub async fn with_entry<T>(
        &self,
        key: Key,
        f: impl for<'a> AsyncFnOnce(&'a mut Entry<Output>) -> T,
    ) -> T {
        /// Here, we effectively queue the waiters in FIFO order, based on the time they swap in
        /// their local oneshot channel into the map. There are other possible algorithms we can do
        /// here, but they have more overhead and I'm not 100% convinced they are globally better,
        /// so let's just do this for now.
        ///
        /// (Specifically, I think trying to get earlier waiters processed "faster" is fine, even
        /// if we could be doing more concurrent stuff in-flight, because overall I think there
        /// won't be _too_ many thread-blocking tasks going on, and even if there are, they'll gum
        /// up stuff either way, or something like that).
        enum Case<Output> {
            Present(Value<Output>),
            Missing,
            Contended(ThreadsafeReceiver<Value<Output>>),
        }

        let (send, recv) = oneshot::channel();
        let nonce = self.with_entry_nonce.fetch_add(1, Ordering::Relaxed);
        let recv = ThreadsafeReceiver { recv, nonce };

        let case = match self
            .cache
            .upsert_sync(key.clone(), LogicalValue::Computing(recv))
        {
            None => Case::Missing,
            Some(LogicalValue::Materialized(value)) => Case::Present(value),
            Some(LogicalValue::Computing(recv)) => Case::Contended(recv),
        };

        // This is split out from the above so that we don't hold the lock on the map for too long.
        let value = match case {
            Case::Present(value) => Some(value),
            Case::Missing => None,
            Case::Contended(recv) => Some(recv.await.expect("value receive error")),
        };

        let mut entry = Entry { value };
        let out = f(&mut entry).await;

        let value = entry
            .value
            .unwrap_or_else(|| panic!("operated on entry {} without inserting value", key));

        // If there are no waiters (that is, if no one has swapped out our
        // LogicalValue::Computing), then let's just immediately swap back in a
        // LogicalValue::Materialized. Otherwise, we have to send the value to the next waiter.
        self.cache.get_sync(&key).map(|mut entry| {
            let old_nonce = match entry.get() {
                LogicalValue::Materialized(_) => panic!(
                    "got LogicalValue::Materialized after computing value; expected LogicalValue::Computing because we're holding a lock"
                ),
                LogicalValue::Computing(recv) => recv.nonce,
            };
            if old_nonce == nonce {
                let _ = entry.insert(LogicalValue::Materialized(value));
            } else {
                send.send(value).expect("value send error");
            }
        }).unwrap_or_else(|| panic!("got None after computing value; expected LogicalValue::Computing"));

        out
    }

    /// Gets the value associated with an entry. MUST ONLY be used to compute diffs between past
    /// known values and queried values; MUST NOT be relied on as an accurate "this is up to date".
    pub fn get_value(&self, key: &Key) -> Option<Output> {
        match self.cache.get_sync(key)?.get() {
            LogicalValue::Materialized(value) => Some(value.value.clone()),
            LogicalValue::Computing(_) => {
                panic!("should not be computing {key}")
            }
        }
    }
}

/// We need a threadsafe version of `oneshot::Receiver` in order to store values computed by other
/// threads in our map, without acquiring a lock on the entry. We make it threadsafe by limiting
/// the allowed operations to "never" take a shared reference.
#[derive(Debug)]
struct ThreadsafeReceiver<T> {
    recv: oneshot::Receiver<T>,
    nonce: usize,
}
unsafe impl<T> Sync for ThreadsafeReceiver<T> where T: Send {}

impl<T> IntoFuture for ThreadsafeReceiver<T> {
    type Output = Result<T, oneshot::RecvError>;
    type IntoFuture = oneshot::AsyncReceiver<T>;

    fn into_future(self) -> Self::IntoFuture {
        self.recv.into_future()
    }
}

impl<Output> Value<Output> {
    fn mark_changed(&mut self, revision: usize) {
        // Only move the revision forward
        self.revision.changed_at = std::cmp::max(self.revision.changed_at, revision);
    }
    fn mark_verified(&mut self, revision: usize) {
        // Only ever move revision forward.
        self.revision.verified_at = std::cmp::max(self.revision.verified_at, revision);
    }
}

pub struct Entry<Output> {
    value: Option<Value<Output>>,
}

impl<Output: driver_util::Output> Entry<Output> {
    pub fn insert(&mut self, revision: usize, value: Output) {
        match self.value {
            None => {
                self.value = Some(Value {
                    value,
                    revision: Revision {
                        changed_at: revision,
                        verified_at: revision,
                    },
                });
            }
            Some(ref mut this) => {
                let did_change = this.value != value;
                this.value = value;

                this.mark_verified(revision);
                if did_change {
                    this.mark_changed(revision);
                }
            }
        }
    }

    pub fn revision(&self) -> Option<Revision> {
        self.value.as_ref().map(|value| value.revision)
    }

    /// MUST only be called when the value is known to be present.
    pub fn mark_verified(&mut self, revision: usize) {
        let this = self
            .value
            .as_mut()
            .unwrap_or_else(|| panic!("tried to mark value as verified before inserting"));
        this.mark_verified(revision);
    }

    pub fn value(&self) -> Option<Output> {
        self.value.as_ref().map(|value| value.value.clone())
    }
}

impl<Key: driver_util::Key, Output: driver_util::Output> Database<Key, Output> {
    pub fn save(self, options: &Options) -> driver_util::Result<()> {
        std::fs::create_dir_all(
            options
                .cache_path
                .parent()
                .ok_or_else(|| driver_util::Error::new("invalid cache path"))?,
        )?;
        let file = std::fs::File::create(&options.cache_path)?;
        let file = zstd::Encoder::new(file, 1)?;
        let file = postcard::to_io(&self.core, file)?;
        file.finish()?;

        // TODO: allow saving two files concurrently with async
        std::fs::create_dir_all(
            options
                .remotes_path
                .parent()
                .ok_or_else(|| driver_util::Error::new("invalid remotes path"))?,
        )?;
        let file = std::fs::File::create(&options.remotes_path)?;
        let file = zstd::Encoder::new(file, 1)?;
        let file = postcard::to_io(&self.remotes, file)?;
        file.finish()?;

        // self.objects are already saved as part of normal operation
        Ok(())
    }

    pub fn restore(options: &Options) -> Self {
        std::fs::create_dir_all(&options.objects_path)
            .expect("could not create/read object directory");
        let objects = Objects::new();

        let core = (|| {
            let file = std::fs::File::open(&options.cache_path)?;
            let mut file = zstd::Decoder::new(file)?;
            let mut bytes = Vec::<u8>::new();
            file.read_to_end(&mut bytes)?;
            let core: Core<Key, Output> = postcard::from_bytes(&bytes)?;
            driver_util::Result::Ok(core)
        })()
        .unwrap_or_else(|err| {
            eprintln!("error restoring {}: {}", options.cache_path.display(), err);
            Default::default()
        });

        // TODO: allow restoring from both files concurrently
        let remotes = (|| {
            let file = std::fs::File::open(&options.remotes_path)?;
            let mut file = zstd::Decoder::new(file)?;
            let mut bytes = Vec::<u8>::new();
            file.read_to_end(&mut bytes)?;
            let remotes: RemoteObjects = postcard::from_bytes(&bytes)?;
            driver_util::Result::Ok(remotes)
        })()
        .unwrap_or_else(|err| {
            eprintln!(
                "error restoring {}: {}",
                options.remotes_path.display(),
                err
            );
            Default::default()
        });

        Self {
            core,
            remotes,
            objects,
        }
    }
}

/// Implementation of functions that MUST be run outside an async context, with effectively an
/// exclusive reference. Sorry for not enforcing this in the types better...
impl<Key: driver_util::Key, Output: driver_util::Output> Database<Key, Output> {
    pub fn clear(&self) {
        self.cache.clear_sync();
        self.dep_graph.clear_sync();
    }

    pub fn remove_keys_matching_prefixes(&self, prefixes: &[&String]) {
        let mut keys_to_remove = vec![];

        let mut entry = self.cache.begin_sync();
        while let Some(e) = entry {
            let key = e.key();
            let key_str = key.to_string();
            if prefixes.iter().any(|prefix| key_str.starts_with(*prefix)) {
                keys_to_remove.push(key.clone());
            }
            entry = e.next_sync();
        }

        self.cache
            .retain_sync(|key, _| !keys_to_remove.contains(key));
        self.dep_graph
            .retain_sync(|key, _| !keys_to_remove.contains(key));
    }

    pub fn clear_remote(&self) {
        self.remotes.cache.clear_sync();
    }

    /// Removes all keys that don't have a parent; this corresponds to the "roots" of the graph that
    /// could possibly used to produce output.
    pub fn remove_root_keys(&self) {
        let mut keys_to_keep = HashSet::new();

        let mut entry = self.dep_graph.begin_sync();
        while let Some(e) = entry {
            let deps = e.get();
            keys_to_keep.extend(deps.iter().cloned());
            entry = e.next_sync();
        }

        self.cache.retain_sync(|key, _| keys_to_keep.contains(key));
        self.dep_graph
            .retain_sync(|key, _| keys_to_keep.contains(key));
    }

    /// Removes all [`Object`]s that aren't referenced from the local or remote caches.
    pub fn garbage_collect(&self, options: &Options) -> driver_util::Result<()> {
        let objects = self.collect_objects();
        self.objects
            .retain(options, |object| objects.contains(object))
    }

    /// Finds all [`Objects`]s that are referenced in the local and remote caches.
    fn collect_objects(&self) -> HashSet<Object> {
        let mut objects = HashSet::new();

        self.cache.iter_sync(|key, value| {
            objects.extend(key.trace().cloned());
            objects.extend(
                match value {
                    LogicalValue::Materialized(value) => &value.value,
                    LogicalValue::Computing(_) => {
                        panic!("should not be computing {key}")
                    }
                }
                .trace()
                .cloned(),
            );
            true
        });

        self.remotes.cache.iter_sync(|_key, value| {
            objects.insert(value.object.clone());
            true
        });

        objects
    }

    pub fn display_dep_graph(&self) -> impl Display + '_ {
        struct GraphDisplayer<'a, Key: Hash + Eq>(&'a scc::HashMap<Key, BTreeSet<Key>>);

        impl<'a, Key: driver_util::Key> Display for GraphDisplayer<'a, Key> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                let mut keys = Vec::<Key>::with_capacity(self.0.len());
                let mut entry = self.0.begin_sync();
                while let Some(e) = entry {
                    keys.push(e.key().clone());
                    entry = e.next_sync();
                }

                keys.sort();

                for key in keys {
                    write!(f, "{}: ", key)?;

                    if let Some(deps) = self.0.get_sync(&key)
                        && !deps.is_empty()
                    {
                        writeln!(f, "[")?;
                        for dep in deps.iter() {
                            writeln!(f, "\t{},", dep)?;
                        }
                        writeln!(f, "]")?;
                    } else {
                        writeln!(f, "None")?;
                    };
                }

                Ok(())
            }
        }

        GraphDisplayer(&self.dep_graph)
    }

    pub fn display_dep_graph_with_outputs(&self) -> impl Display + '_ {
        struct GraphAndOutputDisplayer<'a, Key: Hash + Ord + Eq, Output>(&'a Core<Key, Output>);

        impl<'a, Key: driver_util::Key, Output: driver_util::Output> Display
            for GraphAndOutputDisplayer<'a, Key, Output>
        {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                let mut keys = Vec::<Key>::with_capacity(self.0.dep_graph.len());
                let mut entry = self.0.dep_graph.begin_sync();
                while let Some(e) = entry {
                    keys.push(e.key().clone());
                    entry = e.next_sync();
                }

                keys.sort();

                for key in keys {
                    write!(f, "{} -> {:?}: ", key, self.0.get_value(&key))?;

                    if let Some(deps) = self.0.dep_graph.get_sync(&key)
                        && !deps.is_empty()
                    {
                        writeln!(f, "[")?;
                        for dep in deps.iter() {
                            writeln!(f, "\t{} -> {:?},", dep, self.0.get_value(dep))?;
                        }
                        writeln!(f, "]")?;
                    } else {
                        writeln!(f, "None")?;
                    };
                }

                Ok(())
            }
        }

        GraphAndOutputDisplayer(self)
    }
}

/// Testing utility functions
impl<Key: driver_util::Key, Output: driver_util::Output> Database<Key, Output> {
    /// Intentionally creates an empty database. Meant for testing, you should probably be using
    /// [`Database::restore()`] instead.
    pub fn empty() -> Self {
        Self {
            core: Default::default(),
            remotes: Default::default(),
            objects: Objects::new(),
        }
    }
}
