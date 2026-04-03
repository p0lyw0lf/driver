use std::collections::BTreeSet;
use std::fmt::Display;
use std::io::Read;
use std::path::Path;
use std::sync::{Arc, atomic::AtomicUsize};

use pub_if::pub_if;
use scc::hash_map;
use serde::{Deserialize, Serialize};

use crate::engine::{AnyOutput, QueryKey, Queryable};
use crate::serde::{SerializedMap, SerializedMutex};
use crate::to_hash::ToHash;

pub mod object;
pub mod remote;

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
#[pub_if(test)]
#[derive(Debug, Serialize, Deserialize)]
pub struct Value {
    value: AnyOutput,
    #[serde(skip)]
    #[serde(default)]
    revision: Revision,
}

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct Database {
    #[serde(skip)]
    pub(crate) revision: AtomicUsize,

    cache: SerializedMap<QueryKey, Arc<SerializedMutex<Value>>>,
    dep_graph: SerializedMap<QueryKey, BTreeSet<QueryKey>>,

    pub objects: object::Objects,
    pub remotes: remote::RemoteObjects,
}

impl Database {
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
        f: impl for<'a> AsyncFnOnce(Entry<'a>) -> T,
    ) -> T {
        let (value, occupied) = {
            match self.cache.entry_sync(key.clone()) {
                hash_map::Entry::Occupied(entry) => {
                    let value = entry.get().clone();
                    (value, true)
                }
                hash_map::Entry::Vacant(entry) => {
                    // PLACEHOLDER, MEANS NOTHING, MUST NOT BE USED
                    let placeholder_value = Value {
                        value: AnyOutput::new(()),
                        revision: Revision::default(),
                    };
                    let entry =
                        entry.insert_entry(Arc::new(SerializedMutex::new(placeholder_value)));
                    let value = entry.get().clone();
                    (value, false)
                }
            }
        };

        let value = value.lock().await;
        f(Entry {
            key,
            value,
            has_value: occupied,
        })
        .await
    }

    /// Gets the value associated with an entry. MUST ONLY be used to compute diffs between past
    /// known values and queried values; MUST NOT be relied on as an accurate "this is up to date".
    pub(crate) async unsafe fn get_value<K>(&self, key: K) -> Option<K::Output>
    where
        K: Queryable,
    {
        let value = { self.cache.get_sync(&key.into())?.get().clone() };
        let value = {
            let value = value.lock().await;
            value.value.clone()
        };
        value.downcast().map(|x| *x)
    }
}

pub struct Entry<'a> {
    key: QueryKey,
    value: MutexGuard<'a, Value>,
    has_value: bool,
}

impl<'a> Drop for Entry<'a> {
    fn drop(&mut self) {
        if !self.has_value {
            panic!("dropped entry for {} without inserting value", self.key);
        }
    }
}

impl<'a> Entry<'a> {
    pub fn insert(&mut self, revision: usize, value: AnyOutput) {
        let hash = value.to_hash();
        let old = std::mem::replace(&mut self.value.value, value);
        self.has_value = true;

        let did_change = if self.has_value {
            old.to_hash() != hash
        } else {
            // If there was no previous value, it's always a change
            true
        };

        self.mark_verified(revision);
        if did_change {
            // Only move the revision forward
            self.value.revision.changed_at =
                std::cmp::max(self.value.revision.changed_at, revision);
        }
    }

    pub fn revision(&self) -> Option<Revision> {
        if self.has_value {
            Some(self.value.revision)
        } else {
            None
        }
    }

    pub fn mark_verified(&mut self, revision: usize) {
        // Only move the revision forward
        self.value.revision.verified_at = std::cmp::max(self.value.revision.verified_at, revision);
    }

    pub fn value(&self) -> Option<&'_ AnyOutput> {
        if self.has_value {
            Some(&self.value.value)
        } else {
            None
        }
    }
}

impl Database {
    pub(crate) fn save_to_file(db: Database, file: &Path) -> crate::Result<()> {
        let file = std::fs::File::create(file)?;
        let file = zstd::Encoder::new(file, 1)?;
        let file = postcard::to_io(&db, file)?;
        file.finish()?;
        Ok(())
    }

    pub(crate) fn restore_from_file(file: &Path) -> crate::Result<Database> {
        let file = std::fs::File::open(file)?;
        let mut file = zstd::Decoder::new(file)?;
        let mut bytes = Vec::<u8>::new();
        file.read_to_end(&mut bytes)?;
        let db: Database = postcard::from_bytes(&bytes)?;
        Ok(db)
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
