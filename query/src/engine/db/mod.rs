use std::collections::BTreeSet;
use std::fmt::Display;
use std::io::Read;
use std::ops::Deref;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};

use crate::Options;
use crate::engine::{AnyOutput, QueryKey, Queryable};
use crate::serde::SerializedMap;
use crate::to_hash::ToHash;

mod http_client;
pub mod object;
pub mod remote;

#[cfg(test)]
mod test;

pub use object::Object;

/// Tracks the range [changed_at, verified_at], to confirm the value is corresponds to is the same
/// for that entire range of revisions.
#[derive(Copy, Clone, Default, Debug, Serialize, Deserialize)]
pub struct Revision {
    /// The revision at which we've executed a query and noticed that the value has changed.
    pub(crate) changed_at: usize,
    /// The revision at which we've verified a value has not changed since changed_at.
    pub(crate) verified_at: usize,
}

/// Represents the current known state of a query. It is bundled together because it should all be
/// operated on at once.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Value {
    value: AnyOutput,
    #[serde(skip)]
    #[serde(default)]
    revision: Revision,
}

/// Represents a value as it's being computed by the system. Allows for multiple logcial queries
/// for the same key to be in-flight at the same time, while only doing one actual computation.
#[derive(Debug, Serialize, Deserialize)]
enum LogicalValue {
    Materialized(Value),
    /// This is just a oneshot because each entry notifies just the next one waiting that its it's
    /// turn. I think this is slightly less efficient than using a condition variable + mutex to
    /// gate tasks one-at-a-time, but it's more correct than async_broadcast which is my closest
    /// alternative.
    /// NOTE: acutally, I'm not so sure about this! The receiver could be in a thread
    /// that's currently doing a lot of other work, and there could be lots of other things waiting
    /// on it that need to complete as well. Serializing things this way doesn't seem ideal, but
    /// getting a "real" "hey whoever can take this next, it's up for grabs" seems a bit harder.
    #[serde(skip)]
    Computing(ThreadsafeReceiver<Value>),
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Core {
    #[serde(skip)]
    pub(crate) revision: AtomicUsize,

    /// Used to check that, when a `LogicalValue::Computing` is inserted/taken out, we get the same
    /// one back. We do this because we can't compare the `oneshot::Receiver`s directly.
    #[serde(skip)]
    with_entry_nonce: AtomicUsize,
    cache: SerializedMap<QueryKey, LogicalValue>,
    dep_graph: SerializedMap<QueryKey, BTreeSet<QueryKey>>,

    /// TODO: save these separately
    pub remotes: remote::RemoteObjects,
}

#[derive(Debug)]
pub struct Database {
    core: Core,
    pub objects: object::Objects,
}

impl Deref for Database {
    type Target = Core;

    fn deref(&self) -> &Self::Target {
        &self.core
    }
}

impl Core {
    pub(crate) async fn add_dependency(&self, parent: QueryKey, child: QueryKey) {
        let entry = self.dep_graph.entry_async(parent).await;
        let mut child = BTreeSet::from([child]);
        entry
            .and_modify(|deps| {
                deps.append(&mut child);
            })
            .or_insert(child);
    }

    pub(crate) async fn remove_all_dependencies(&self, parent: &QueryKey) {
        self.dep_graph.remove_async(parent).await;
    }

    pub(crate) async fn dependencies(&self, parent: &QueryKey) -> Option<Vec<QueryKey>> {
        let deps = self.dep_graph.get_async(parent).await?;
        Some(deps.get().iter().map(Clone::clone).collect())
    }

    /// Running this acquires a lock on the given entry, meaning the current task will suspend
    /// until the entry is unlocked by the task that currently has it acquired. This is necessary
    /// for us to run each query exactly once per revision, otherwise we could be running the same
    /// query concurrently (does too much work).
    pub(crate) async fn with_entry<T>(
        &self,
        key: QueryKey,
        f: impl for<'a> AsyncFnOnce(&'a mut Entry) -> T,
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
        enum Case {
            Present(Value),
            Missing,
            Contended(ThreadsafeReceiver<Value>),
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
        match self.cache.entry_sync(key) {
            scc::hash_map::Entry::Vacant(_) => {
                panic!("got None after computing value; expected LogicalValue::Computing")
            }
            scc::hash_map::Entry::Occupied(mut entry) => {
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
            }
        };

        out
    }

    /// Gets the value associated with an entry. MUST ONLY be used to compute diffs between past
    /// known values and queried values; MUST NOT be relied on as an accurate "this is up to date".
    pub(crate) async unsafe fn get_value<K>(&self, key: K) -> Option<K::Output>
    where
        K: Queryable,
    {
        let key = key.into();
        let value = match self.cache.get_sync(&key)?.get() {
            LogicalValue::Materialized(value) => value.value.clone(),
            LogicalValue::Computing(_) => {
                panic!("should not be computing {key}")
            }
        };
        value.downcast().map(|x| *x)
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

impl Value {
    fn mark_changed(&mut self, revision: usize) {
        // Only move the revision forward
        self.revision.changed_at = std::cmp::max(self.revision.changed_at, revision);
    }
    fn mark_verified(&mut self, revision: usize) {
        // Only ever move revision forward.
        self.revision.verified_at = std::cmp::max(self.revision.verified_at, revision);
    }
}

pub(crate) struct Entry {
    value: Option<Value>,
}

impl Entry {
    pub fn insert(&mut self, revision: usize, value: AnyOutput) {
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
                let hash = value.to_hash();
                let old = std::mem::replace(&mut this.value, value);

                let did_change = old.to_hash() != hash;

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

    pub fn value(&self) -> Option<AnyOutput> {
        self.value.as_ref().map(|value| value.value.clone())
    }
}

impl Database {
    pub(crate) fn save(self, options: &Options) -> crate::Result<()> {
        let file = std::fs::File::create(&options.cache_path)?;
        let file = zstd::Encoder::new(file, 1)?;
        let file = postcard::to_io(&self.cache, file)?;
        file.finish()?;
        // self.objects are already saved as part of normal operation
        Ok(())
    }

    pub(crate) fn restore(options: &Options) -> crate::Result<Self> {
        std::fs::create_dir_all(
            options
                .cache_path
                .parent()
                .ok_or_else(|| crate::Error::new("invalid cache path"))?,
        )?;
        std::fs::create_dir_all(&options.objects_path)?;

        let file = std::fs::File::open(&options.cache_path)?;
        let mut file = zstd::Decoder::new(file)?;
        let mut bytes = Vec::<u8>::new();
        file.read_to_end(&mut bytes)?;
        let core: Core = postcard::from_bytes(&bytes)?;
        let objects = object::Objects::new(options.objects_path.clone());
        Ok(Self { core, objects })
    }

    pub(crate) fn new(options: &Options) -> Self {
        let core = Core::default();
        let objects = object::Objects::new(options.objects_path.clone());
        Self { core, objects }
    }

    pub(crate) fn display_dep_graph(&self) -> impl Display + '_ {
        struct GraphDisplayer<'a>(&'a scc::HashMap<QueryKey, BTreeSet<QueryKey>>);

        impl<'a> Display for GraphDisplayer<'a> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                let mut keys = Vec::<QueryKey>::with_capacity(self.0.len());
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
}
