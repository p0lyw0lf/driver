use std::collections::BTreeSet;
use std::fmt::Display;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use scc::hash_map::HashMap;
use serde::Deserialize;
use serde::Serialize;
use tokio::sync::MutexGuard;
use tracing::trace;

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

    cache: HashMap<QueryKey, Arc<SerializedMutex<Value>>>,
    dep_graph: HashMap<QueryKey, BTreeSet<QueryKey>>,

    pub objects: object::Objects,
    pub remotes: remote::RemoteObjects,
}

impl Database {
    pub async fn get_color(&self, key: &QueryKey) -> Option<(Color, usize)> {
        let value = self.cache.get_async(key).await?;
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
        let (value, occupied) = match self.cache.entry_async(key.clone()).await {
            scc::hash_map::Entry::Occupied(entry) => {
                let value = entry.get().clone();
                (value, true)
            }
            scc::hash_map::Entry::Vacant(entry) => {
                let placeholder_value = Value {
                    // PLACEHOLDER
                    value: AnyOutput::new(()),
                    color: (Color::Red, 0),
                };
                let entry = entry.insert_entry(Arc::new(SerializedMutex::new(placeholder_value)));
                let value = entry.get().clone();
                (value, false)
            }
        };

        let value = value.lock().await;
        f(Entry {
            value,
            has_value: occupied,
            has_color: false,
        })
        .await
    }
}

pub struct Entry<'a> {
    value: MutexGuard<'a, Value>,
    has_value: bool,
    has_color: bool,
}

impl<'a> Drop for Entry<'a> {
    fn drop(&mut self) {
        if !self.has_value {
            panic!("dropped entry without inserting value");
        }
        if !self.has_color {
            panic!("dropped entry without updating color");
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

        if is_fresh {
            self.value.color = (Color::Green, revision);
        } else {
            self.value.color = (Color::Red, revision);
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
        self.value.color = (Color::Green, revision);
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
    pub(crate) async fn as_serialized(&self) -> SerializedDatabase {
        let mut out = std::collections::HashMap::with_capacity(self.cache.len());

        let mut entry = self.cache.begin_sync();
        while let Some(e) = entry {
            let key = e.key();
            let value = e.get().lock().await.value.clone();
            let dependencies = self
                .dep_graph
                .get_sync(key)
                .map(|e| e.get().clone())
                .unwrap_or_default();
            out.insert(key.clone(), (value, dependencies));
            entry = e.next_sync();
        }

        out
    }
}

pub async fn save_to_directory(dir: &Path, db: &Database) -> crate::Result<()> {
    let Filenames {
        cache_filename,
        remote_filename,
        objects_dirname,
    } = get_filenames(dir);

    async_fs::create_dir_all(dir).await?;

    // TODO: make these writes async & concurrent
    // {
    //     let cache_file = std::fs::File::create(cache_filename)?;
    //     postcard::to_io(&db.as_serialized().await, cache_file)?;
    // }
    {
        let bytes = postcard::to_stdvec(&db.as_serialized().await)?;
        std::fs::write(cache_filename, bytes)?;
    }
    {
        let remote_file = std::fs::File::create(remote_filename)?;
        postcard::to_io(&db.remotes, remote_file)?;
    }

    db.objects.for_each(|hash, contents| -> crate::Result<_> {
        let hash = hash.to_string();
        let (prefix, rest) = hash.split_at(2);
        let object_directory = objects_dirname.join(prefix);
        let object_filename = object_directory.join(rest);
        if std::fs::exists(&object_filename)? {
            // By the uniqueness of the hash, we're already done
            return Ok(());
        }

        // Otherwise, we have to write the object
        std::fs::create_dir_all(object_directory)?;
        // Compress with zstd so we don't have to read/write as much data to disk
        // Launch in worker so we can do other stuff in the meantime.
        let object_file = std::fs::File::create(&object_filename)?;
        let mut encoder = zstd::stream::Encoder::new(object_file, 0)?.auto_finish();
        encoder.write_all(contents)?;
        encoder.flush()?;
        Ok(())
    })?;

    Ok(())
}

pub async fn restore_from_directory(dir: &Path) -> crate::Result<Database> {
    let Filenames {
        cache_filename,
        remote_filename,
        objects_dirname,
    } = get_filenames(dir);

    // TODO: make these reads async & concurrent

    trace!("deserializing cache {}", cache_filename.display());
    let cache_bytes = async_fs::read(cache_filename).await?;
    trace!("read cache file");
    let serialized_cache: SerializedDatabase = postcard::from_bytes(&cache_bytes[..])?;

    // Everything loaded from the disk is green to start with; this will be busted by input queries
    // changing for the next revision.
    let cache = HashMap::with_capacity(serialized_cache.len());
    let dep_graph = HashMap::with_capacity(serialized_cache.len());

    trace!("reading cache into maps");
    for (key, (value, dependencies)) in serialized_cache {
        let _ = cache
            .insert_async(
                key.clone(),
                Arc::new(SerializedMutex::new(Value {
                    value,
                    color: (Color::Green, 0),
                })),
            )
            .await;
        let _ = dep_graph.insert_async(key, dependencies).await;
    }

    trace!("deserializing remotes");
    let remote_bytes = async_fs::read(remote_filename).await?;
    let remotes: RemoteObjects = postcard::from_bytes(&remote_bytes[..])?;

    trace!("deserializing objects");
    let objects = object::Objects::default();
    for prefix_entry in std::fs::read_dir(objects_dirname)? {
        let prefix_entry = prefix_entry?;
        let prefix = prefix_entry
            .file_name()
            .into_string()
            .map_err(|_| crate::Error::new("couldn't convert object prefix to string"))?;
        for object_entry in std::fs::read_dir(prefix_entry.path())? {
            let object_entry = object_entry?;
            let rest = object_entry
                .file_name()
                .into_string()
                .map_err(|_| crate::Error::new("couldn't convert object suffix to string"))?;

            let hash = format!("{}{}", prefix, rest);
            let hash_bytes = hex::decode(&hash)?;
            // TODO: remove this once sha2 updates off generic-array@0.14.9
            #[allow(deprecated)]
            let hash = Hash::from_exact_iter(hash_bytes)
                .ok_or_else(|| crate::Error::new("couldn't convert object filename to hash"))?;
            // SAFETY: we are restoring from disk here
            let object = unsafe { Object::from_hash(hash) };

            let file = std::fs::File::open(object_entry.path())?;
            let mut decoder = zstd::stream::Decoder::new(file)?;
            let contents = {
                let mut contents = Vec::<u8>::new();
                decoder.read_to_end(&mut contents)?;
                contents
            };

            // SAFETY: the data on disk should be trustworthy, no need to re-do hash
            unsafe {
                objects.store_raw(object, contents);
            }
        }
    }

    Ok(Database {
        // Bust cache immediately
        revision: AtomicUsize::new(1),
        cache,
        dep_graph,
        remotes,
        objects,
    })
}

impl Database {
    pub(crate) fn display_dep_graph(&self) -> impl Display + '_ {
        struct GraphDisplayer<'a>(&'a HashMap<QueryKey, BTreeSet<QueryKey>>);

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
                    writeln!(f, "{}: ", key)?;

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
