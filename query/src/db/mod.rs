use std::collections::BTreeSet;
use std::fmt::Display;
use std::path::Path;
use std::sync::{Arc, atomic::AtomicUsize};

use async_compression::tokio::{bufread::ZstdDecoder, write::ZstdEncoder};
use scc::hash_map;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::MutexGuard;

use crate::QueryKey;
use crate::query::context::AnyOutput;
use crate::serde::{SerializedMap, SerializedMutex};
use crate::to_hash::ToHash;

pub mod object;
pub mod remote;

#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Color {
    Green,
    Red,
}

impl Color {
    /// neede for serde because it sucks
    fn green() -> Self {
        Color::Green
    }
}

/// Represents the current known state of a query. It is bundled together because it should all be
/// operated on at once.
#[cfg(not(test))]
#[derive(Debug, Serialize, Deserialize)]
pub struct Value {
    value: AnyOutput,
    #[serde(skip)]
    #[serde(default = "Color::green")]
    color: Color,
    #[serde(skip)]
    #[serde(default)]
    revision: usize,
}

// TODO: I should really use pub_if...
#[cfg(test)]
#[derive(Debug, Serialize, Deserialize)]
pub struct Value {
    pub value: AnyOutput,
    #[serde(skip)]
    #[serde(default = "Color::green")]
    pub color: Color,
    #[serde(skip)]
    #[serde(default)]
    pub revision: usize,
}

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct Database {
    #[serde(skip)]
    pub(crate) revision: AtomicUsize,

    #[cfg(not(test))]
    cache: SerializedMap<QueryKey, Arc<SerializedMutex<Value>>>,
    #[cfg(not(test))]
    dep_graph: SerializedMap<QueryKey, BTreeSet<QueryKey>>,
    #[cfg(test)]
    pub cache: SerializedMap<QueryKey, Arc<SerializedMutex<Value>>>,
    #[cfg(test)]
    pub dep_graph: SerializedMap<QueryKey, BTreeSet<QueryKey>>,

    pub objects: object::Objects,
    pub remotes: remote::RemoteObjects,
}

impl Database {
    pub async fn get_color(&self, key: &QueryKey) -> Option<(Color, usize)> {
        let value = {
            // NOTE: we have to be very careful to NOT overlap the scope of the HashMap lock with
            // the scope of the bucket lock; we don't want "blocking on a bucket" to mean "blocking
            // on the table", that's the whole reason we do per-bucket things in the first place.
            // Cloning the value out is safe because once a bucket is filled, we only ever modify
            // the value at that bucket, never replace it. This means there is no TOCTTOU risk.
            self.cache.get_sync(key).map(|entry| entry.get().clone())
        }?;
        let value = value.lock().await;
        Some((value.color, value.revision))
    }

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
                    let placeholder_value = Value {
                        // PLACEHOLDER
                        value: AnyOutput::new(()),
                        color: Color::Red,
                        revision: 0,
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
            has_color: false,
        })
        .await
    }
}

pub struct Entry<'a> {
    key: QueryKey,
    value: MutexGuard<'a, Value>,
    has_value: bool,
    has_color: bool,
}

impl<'a> Drop for Entry<'a> {
    fn drop(&mut self) {
        if !self.has_value {
            panic!("dropped entry for {} without inserting value", self.key);
        }
        if !self.has_color {
            panic!("dropped entry for {} without updating color", self.key);
        }
    }
}

impl<'a> Entry<'a> {
    pub fn insert(&mut self, revision: usize, value: AnyOutput) {
        let hash = value.to_hash();
        let old = std::mem::replace(&mut self.value.value, value);
        let is_fresh = if self.has_value {
            old.to_hash() == hash
        } else {
            // If there was no previous value, new one always fresh
            true
        };
        self.has_value = true;

        // Only mark things if we're moving the revision forward
        if self.value.revision < revision {
            self.value.color = if is_fresh { Color::Green } else { Color::Red };
            self.value.revision = revision;
        }
        self.has_color = true;
    }

    pub fn color(&self) -> Option<(Color, usize)> {
        if self.has_value {
            Some((self.value.color, self.value.revision))
        } else {
            None
        }
    }

    pub fn mark_green(&mut self, revision: usize) {
        // Only mark things if we're moving the revision forward
        if self.value.revision < revision {
            self.value.color = Color::Green;
            self.value.revision = revision;
        }
        self.has_color = true;
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
    pub async fn save_to_file(
        db: Arc<Database>,
        rt: Arc<tokio::runtime::Runtime>,
        file: &Path,
    ) -> crate::Result<()> {
        let bytes = rt
            .spawn_blocking(move || postcard::to_stdvec(&db))
            .await??;
        let file = tokio::fs::File::create(file).await?;
        let mut encoder = ZstdEncoder::new(file);
        encoder.write_all(&bytes).await?;
        encoder.shutdown().await?;

        Ok(())
    }

    pub async fn restore_from_file(file: &Path) -> crate::Result<Database> {
        let file = tokio::fs::File::open(file).await?;
        let file = tokio::io::BufReader::new(file);
        let mut decoder = ZstdDecoder::new(file);

        let bytes = {
            let mut bytes = Vec::<u8>::new();
            decoder.read_to_end(&mut bytes).await?;
            bytes
        };
        let mut db: Database = postcard::from_bytes(&bytes)?;

        // Bust cache immediately. This works because the revisions always get restored as 0.
        db.revision = AtomicUsize::new(1);
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
