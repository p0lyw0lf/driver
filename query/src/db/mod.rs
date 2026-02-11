use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::RwLock;
use std::sync::atomic::AtomicUsize;

use dashmap::DashMap;
use petgraph::graph::DiGraph;
use petgraph::graph::NodeIndex;
use serde::Deserialize;
use serde::Serialize;

use crate::QueryKey;
use crate::db::object::Object;
use crate::query::key::QueryCache;
use crate::to_hash::Hash;

pub mod object;

#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Color {
    Green,
    Red,
}

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct ColorMap(DashMap<QueryKey, (Color, usize)>);

impl ColorMap {
    pub fn mark_green(&self, key: &QueryKey, revision: usize) {
        self.0.insert(key.clone(), (Color::Green, revision));
    }

    pub fn mark_red(&self, key: &QueryKey, revision: usize) {
        self.0.insert(key.clone(), (Color::Red, revision));
    }

    pub fn get(&self, key: &QueryKey) -> Option<(Color, usize)> {
        self.0
            .get(key)
            .as_deref()
            .map(|(color, rev)| (*color, *rev))
    }
}

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct DepGraph {
    graph: RwLock<DiGraph<QueryKey, ()>>,
    indices: DashMap<QueryKey, NodeIndex>,
}

impl DepGraph {
    fn node_for(&self, key: QueryKey) -> NodeIndex {
        match self.indices.entry(key.clone()) {
            dashmap::Entry::Occupied(entry) => *entry.get(),
            dashmap::Entry::Vacant(entry) => {
                let idx = self.graph.write().unwrap().add_node(key);
                *entry.insert(idx)
            }
        }
    }

    pub fn add_dependency(&self, from: QueryKey, to: QueryKey) {
        let from = self.node_for(from);
        let to = self.node_for(to);
        self.graph.write().unwrap().update_edge(from, to, ());
    }

    pub fn remove_all_dependencies(&self, from: QueryKey) {
        let from = self.node_for(from);
        self.graph
            .write()
            .unwrap()
            .retain_edges(|this, edge_index| {
                let (edge_from, _) = &this.edge_endpoints(edge_index).unwrap();
                *edge_from != from
            });
    }

    pub fn dependencies(&self, key: &QueryKey) -> Option<Vec<QueryKey>> {
        self.indices.get(key).map(|key| {
            let graph = self.graph.read().unwrap();
            graph
                .neighbors_directed(*key, petgraph::Direction::Outgoing)
                .map(|i| graph[i].clone())
                .collect::<Vec<_>>()
        })
    }
}

#[derive(Default, Debug)]
pub struct Database {
    pub colors: ColorMap,
    pub revision: AtomicUsize,

    pub cache: QueryCache,

    pub objects: object::Objects,
}

struct Filenames {
    cache_filename: PathBuf,
    depgraph_filename: PathBuf,
    objects_dirname: PathBuf,
}

fn get_filenames(dir: &Path) -> Filenames {
    Filenames {
        cache_filename: dir.join("cache.v1.pc"),
        depgraph_filename: dir.join("depgraph.v1.pc"),
        objects_dirname: dir.join("objects"),
    }
}

pub fn save_to_directory(dir: &Path, db: &Database, deps: &DepGraph) -> crate::Result<()> {
    let Filenames {
        cache_filename,
        depgraph_filename,
        objects_dirname,
    } = get_filenames(dir);

    std::fs::create_dir_all(dir)?;

    {
        let cache_file = std::fs::File::create(cache_filename)?;
        postcard::to_io(&db.cache, cache_file)?;
    }
    {
        let depgraph_file = std::fs::File::create(depgraph_filename)?;
        postcard::to_io(deps, depgraph_file)?;
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
        let object_file = std::fs::File::create(&object_filename)?;
        let mut encoder = zstd::stream::Encoder::new(object_file, 0)?.auto_finish();
        encoder.write_all(contents)?;
        encoder.flush()?;
        Ok(())
    })?;

    Ok(())
}

pub fn restore_from_directory(dir: &Path) -> crate::Result<(Database, DepGraph)> {
    let Filenames {
        cache_filename,
        depgraph_filename,
        objects_dirname,
    } = get_filenames(dir);

    let cache_bytes = std::fs::read(cache_filename)?;
    let cache: QueryCache = postcard::from_bytes(&cache_bytes[..])?;

    let depgraph_bytes = std::fs::read(depgraph_filename)?;
    let depgraph: DepGraph = postcard::from_bytes(&depgraph_bytes[..])?;

    // Everything loaded from the disk is green to start with; this will be busted by input queries
    // changing for the next revision.
    let colors = ColorMap::default();
    for key in cache.iter_keys() {
        colors.mark_green(&key, 0);
    }

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

    let db = Database {
        colors,
        // Bust cache immediately
        revision: AtomicUsize::new(1),
        cache,
        objects,
    };

    Ok((db, depgraph))
}
