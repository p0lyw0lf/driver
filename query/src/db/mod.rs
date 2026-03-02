use std::collections::BTreeSet;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;

use scc::hash_map::HashMap;
use serde::Deserialize;
use serde::Serialize;
use tokio::sync::Mutex;

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
    /// This needs to be a Mutex because multiple children could be modifying the same parent at
    /// the same time.
    dependencies: SerializedMutex<BTreeSet<QueryKey>>,
}

#[derive(Default, Debug)]
pub struct Database {
    pub revision: AtomicUsize,

    pub cache: HashMap<QueryKey, Value>,

    pub objects: object::Objects,
    pub remotes: remote::RemoteObjects,
}

impl Database {
    pub async fn entry(&self, key: QueryKey) -> Entry<'_> {
        match self.cache.entry_async(key).await {
            scc::hash_map::Entry::Occupied(entry) => Entry {
                entry,
                has_value: true,
                has_color: false,
            },
            scc::hash_map::Entry::Vacant(entry) => Entry {
                entry: entry.insert_entry(Value {
                    // Leave as temporary value until we can actually insert
                    value: AnyOutput::new(()),
                    color: (Color::Red, 0),
                    dependencies: Default::default(),
                }),
                has_value: false,
                has_color: false,
            },
        }
    }

    pub fn get_color(&self, key: &QueryKey) -> Option<(Color, usize)> {
        self.cache.get_sync(key).map(|entry| entry.color)
    }
}

#[derive(Debug)]
pub struct Entry<'a> {
    entry: scc::hash_map::OccupiedEntry<'a, QueryKey, Value>,
    has_value: bool,
    has_color: bool,
}

impl<'a> Entry<'a> {
    pub async fn add_dependency(&self, child: QueryKey) {
        self.entry.dependencies.lock().await.insert(child);
    }

    pub async fn remove_all_dependencies(&self) {
        self.entry.dependencies.lock().await.clear();
    }

    pub async fn dependencies(&self) -> Option<Vec<QueryKey>> {
        if self.has_value {
            Some(
                self.entry
                    .dependencies
                    .lock()
                    .await
                    .iter()
                    .map(Clone::clone)
                    .collect(),
            )
        } else {
            None
        }
    }

    pub fn insert(&mut self, revision: usize, value: AnyOutput) {
        let hash = value.to_hash();
        let old = std::mem::replace(&mut self.entry.value, value);
        let is_fresh = if self.has_value {
            old.to_hash() == hash
        } else {
            // If there was no previous value, new one always fresh
            true
        };
        self.has_value = true;

        if is_fresh {
            self.entry.color = (Color::Green, revision);
        } else {
            self.entry.color = (Color::Red, revision);
        }
        self.has_color = true;
    }

    pub fn color(&self) -> Option<(Color, usize)> {
        if self.has_value {
            Some(self.entry.color)
        } else {
            None
        }
    }

    pub fn mark_green(&mut self, revision: usize) {
        self.entry.color = (Color::Green, revision);
        self.has_color = true;
    }

    pub fn value(&self) -> Option<&'_ AnyOutput> {
        if self.has_value {
            Some(&self.entry.value)
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
    fn as_serialized(&self) -> SerializedDatabase {
        todo!()
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
    {
        let cache_file = std::fs::File::create(cache_filename)?;
        postcard::to_io(&db.as_serialized(), cache_file)?;
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

    let cache_bytes = async_fs::read(cache_filename).await?;
    let cache: SerializedDatabase = postcard::from_bytes(&cache_bytes[..])?;

    let remote_bytes = async_fs::read(remote_filename).await?;
    let remotes: RemoteObjects = postcard::from_bytes(&remote_bytes[..])?;

    // Everything loaded from the disk is green to start with; this will be busted by input queries
    // changing for the next revision.
    let cache: scc::HashMap<QueryKey, Value> = cache
        .into_iter()
        .map(|(key, (value, dependencies))| {
            (
                key,
                Value {
                    value,
                    color: (Color::Green, 0),
                    dependencies: SerializedMutex(Mutex::new(dependencies)),
                },
            )
        })
        .collect();

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
        remotes,
        objects,
    })
}
