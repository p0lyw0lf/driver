use std::collections::{BTreeSet, HashMap, hash_map};
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::sync::{Arc, atomic::AtomicUsize};

use async_compression::tokio::{bufread::ZstdDecoder, write::ZstdEncoder};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::MutexGuard;

use crate::QueryKey;
use crate::db::object::Object;
use crate::db::remote::RemoteObjects;
use crate::query::context::AnyOutput;
use crate::serde::SerializedMutex;
use crate::to_hash::Hash;
use crate::to_hash::ToHash;

pub mod object;
mod remote;

#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Color {
    Green,
    Red,
}

/// Represents the current known state of a query. It is bundled together because it should all be
/// operated on at once.
#[derive(Debug, Serialize, Deserialize)]
pub struct Value {
    value: AnyOutput,
    color: (Color, usize),
}

#[derive(Default, Debug)]
pub struct Database {
    pub(crate) revision: AtomicUsize,

    cache: RwLock<HashMap<QueryKey, Arc<SerializedMutex<Value>>>>,
    dep_graph: scc::HashMap<QueryKey, BTreeSet<QueryKey>>,

    pub objects: object::Objects,
    pub remotes: remote::RemoteObjects,
}

impl Database {
    pub async fn get_color(&self, key: &QueryKey) -> Option<(Color, usize)> {
        let value = {
            // NOTE: we have to be very careful to NOT overlap the scope of the HashMap lock with
            // the scope of the bucket lock; we don't want "blocking on a bucket" to mean "blocking
            // on the table", that's the whole reason we do per-bucket things in the first place.
            let cache = self.cache.read().unwrap();
            // Cloning the value out is safe because once a bucket is filled, we only ever modify
            // the value at that bucket, never replace it. This means there is no TOCTTOU risk.
            cache.get(key).cloned()
        }?;
        let value = value.lock().await;
        Some(value.color)
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
            let mut cache = self.cache.write().unwrap();
            match cache.entry(key.clone()) {
                hash_map::Entry::Occupied(entry) => {
                    let value = entry.get().clone();
                    (value, true)
                }
                hash_map::Entry::Vacant(entry) => {
                    let placeholder_value = Value {
                        // PLACEHOLDER
                        value: AnyOutput::new(()),
                        color: (Color::Red, 0),
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
        if self.value.color.1 < revision {
            self.value.color = (if is_fresh { Color::Green } else { Color::Red }, revision);
        }
        self.has_color = true;
    }

    pub fn color(&self) -> Option<(Color, usize)> {
        if self.has_value {
            Some(self.value.color)
        } else {
            None
        }
    }

    pub fn mark_green(&mut self, revision: usize) {
        // Only mark things if we're moving the revision forward
        if self.value.color.1 < revision {
            self.value.color = (Color::Green, revision);
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

struct Filenames {
    cache_filename: PathBuf,
    remote_filename: PathBuf,
    objects_dirname: PathBuf,
}

fn get_filenames(dir: &Path) -> Filenames {
    Filenames {
        cache_filename: dir.join("cache.v2.pc"),
        remote_filename: dir.join("remote.v2.pc"),
        objects_dirname: dir.join("objects"),
    }
}

type SerializedDatabase =
    std::collections::HashMap<QueryKey, (AnyOutput, std::collections::BTreeSet<QueryKey>)>;

impl Database {
    // SAFETY: MUST be run when we have no contention on the `cache` field.
    pub(crate) async fn as_serialized(&self) -> SerializedDatabase {
        let cache = { self.cache.read().unwrap().clone() };
        let mut out = std::collections::HashMap::with_capacity(cache.len());

        for (key, value) in cache.iter() {
            let value = value.lock().await.value.clone();
            let dependencies = self
                .dep_graph
                .get_sync(key)
                .map(|e| e.get().clone())
                .unwrap_or_default();
            out.insert(key.clone(), (value, dependencies));
        }

        out
    }
}

pub async fn save_to_directory(dir: &Path, db: Arc<Database>) -> crate::Result<()> {
    let Filenames {
        cache_filename,
        remote_filename,
        objects_dirname,
    } = get_filenames(dir);

    tokio::fs::create_dir_all(dir).await?;

    let mut js = tokio::task::JoinSet::new();
    let _ = js.spawn({
        let db = db.clone();
        async move {
            let bytes = postcard::to_stdvec(&db.as_serialized().await)?;
            tokio::fs::write(cache_filename, bytes).await?;
            crate::Result::Ok(())
        }
    });
    let _ = js.spawn({
        let db = db.clone();
        async move {
            let bytes = postcard::to_stdvec(&db.remotes)?;
            tokio::fs::write(remote_filename, bytes).await?;
            crate::Result::Ok(())
        }
    });
    db.objects.for_each(|hash, contents| {
        let hash = hash.to_string();
        let (prefix, rest) = hash.split_at(2);
        let object_directory = objects_dirname.join(prefix);
        let object_filename = object_directory.join(rest);
        if std::fs::exists(&object_filename)? {
            // By the uniqueness of the hash, we're already done
            return Ok(());
        }

        let contents = contents.clone();
        let _ = js.spawn(async move {
            // Otherwise, we have to write the object
            tokio::fs::create_dir_all(object_directory).await?;
            // Compress with zstd so we don't have to read/write as much data to disk
            // NOTE: this needs to be sync for zstd
            let object_file = tokio::fs::File::create(&object_filename).await?;
            let mut encoder = ZstdEncoder::new(object_file);
            encoder.write_all(&contents).await?;
            encoder.shutdown().await?;

            Ok(())
        });

        crate::Result::Ok(())
    })?;

    js.join_all()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;

    Ok(())
}

pub async fn restore_from_directory(dir: &Path) -> crate::Result<Database> {
    let Filenames {
        cache_filename,
        remote_filename,
        objects_dirname,
    } = get_filenames(dir);

    let ((cache, dep_graph), remotes, objects) = tokio::try_join!(
        async {
            let cache_bytes = tokio::fs::read(cache_filename).await?;
            let serialized_database: SerializedDatabase = postcard::from_bytes(&cache_bytes[..])?;

            // Everything loaded from the disk is green to start with; this will be busted by input
            // queries changing for the next revision.
            let mut cache = HashMap::with_capacity(serialized_database.len());
            let dep_graph = scc::HashMap::with_capacity(serialized_database.len());

            for (key, (value, dependencies)) in serialized_database {
                let _ = cache.insert(
                    key.clone(),
                    Arc::new(SerializedMutex::new(Value {
                        value,
                        color: (Color::Green, 0),
                    })),
                );
                let _ = dep_graph.insert_async(key, dependencies).await;
            }

            crate::Result::Ok((cache, dep_graph))
        },
        async {
            let remote_bytes = tokio::fs::read(remote_filename).await?;
            let remotes: RemoteObjects = postcard::from_bytes(&remote_bytes[..])?;
            crate::Result::Ok(remotes)
        },
        async {
            let objects = Arc::new(object::Objects::default());
            let mut prefix_dir = tokio::fs::read_dir(objects_dirname).await?;

            // Use 1 layer of concurrency, spawning a task for each prefix
            let mut js = tokio::task::JoinSet::new();

            while let Some(prefix_entry) = prefix_dir.next_entry().await? {
                let prefix = prefix_entry
                    .file_name()
                    .into_string()
                    .map_err(|_| crate::Error::new("couldn't convert object prefix to string"))?;
                let objects = objects.clone();
                let _ = js.spawn(async move {
                    let mut object_dir = tokio::fs::read_dir(prefix_entry.path()).await?;
                    while let Some(object_entry) = object_dir.next_entry().await? {
                        let rest = object_entry.file_name().into_string().map_err(|_| {
                            crate::Error::new("couldn't convert object suffix to string")
                        })?;

                        let hash = format!("{}{}", prefix, rest);
                        let hash_bytes = hex::decode(&hash)?;
                        // TODO: remove this once sha2 updates off generic-array@0.14.9
                        #[allow(deprecated)]
                        let hash = Hash::from_exact_iter(hash_bytes).ok_or_else(|| {
                            crate::Error::new("couldn't convert object filename to hash")
                        })?;
                        // SAFETY: we are restoring from disk here
                        let object = unsafe { Object::from_hash(hash) };

                        let file = tokio::fs::File::open(object_entry.path()).await?;
                        let file = tokio::io::BufReader::new(file);
                        let mut decoder = ZstdDecoder::new(file);
                        let contents = {
                            let mut contents = Vec::<u8>::new();
                            decoder.read_to_end(&mut contents).await?;
                            contents
                        };

                        // SAFETY: the data on disk should be trustworthy, no need to re-do hash
                        unsafe {
                            objects.store_raw(object, contents);
                        }
                    }
                    crate::Result::Ok(())
                });
            }

            js.join_all()
                .await
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?;

            crate::Result::Ok(
                Arc::into_inner(objects).expect("all other holders should be finished"),
            )
        },
    )?;

    Ok(Database {
        // Bust cache immediately
        revision: AtomicUsize::new(1),
        cache: RwLock::new(cache),
        dep_graph,
        remotes,
        objects,
    })
}

impl Database {
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
