use std::fmt::Display;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;

use petgraph::graph::DiGraph;
use petgraph::graph::NodeIndex;
use scc::hash_map::HashMap;
use serde::Deserialize;
use serde::Serialize;
use tokio::sync::RwLock;

use crate::QueryKey;
use crate::db::object::Object;
use crate::db::remote::RemoteObjects;
use crate::query::key::QueryCache;
use crate::to_hash::Hash;

pub mod object;
mod remote;

#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Color {
    Green,
    Red,
}

#[derive(Default, Debug)]
pub struct ColorMap(HashMap<QueryKey, (Color, usize)>);

impl ColorMap {
    pub async fn mark_green(&self, key: &QueryKey, revision: usize) {
        self.mark_color(key, Color::Green, revision).await;
    }

    pub async fn mark_red(&self, key: &QueryKey, revision: usize) {
        self.mark_color(key, Color::Red, revision).await;
    }

    /// This function makes sure that, once marked for a revision, only future revisions can update
    /// the color.
    async fn mark_color(&self, key: &QueryKey, color: Color, revision: usize) {
        match self.0.entry_async(key.clone()).await {
            scc::hash_map::Entry::Vacant(vacant_entry) => {
                vacant_entry.insert_entry((color, revision));
            }
            scc::hash_map::Entry::Occupied(mut occupied_entry) => {
                let (_, old_revision) = occupied_entry.get();
                if *old_revision < revision {
                    occupied_entry.insert((color, revision));
                } else {
                    // Keep current color around
                }
            }
        };
    }

    pub fn get(&self, key: &QueryKey) -> Option<(Color, usize)> {
        self.0
            .get_sync(key)
            .as_deref()
            .map(|(color, rev)| (*color, *rev))
    }
}

#[derive(Default, Debug)]
pub struct DepGraph {
    graph: RwLock<DiGraph<QueryKey, ()>>,
    indices: HashMap<QueryKey, NodeIndex>,
}

impl DepGraph {
    async fn node_for(&self, key: QueryKey) -> NodeIndex {
        match self.indices.entry_async(key.clone()).await {
            scc::hash_map::Entry::Occupied(entry) => *entry.get(),
            scc::hash_map::Entry::Vacant(entry) => {
                let idx = self.graph.write().await.add_node(key);
                *entry.insert_entry(idx)
            }
        }
    }

    pub async fn add_dependency(&self, from: QueryKey, to: QueryKey) {
        let from = self.node_for(from).await;
        let to = self.node_for(to).await;
        self.graph.write().await.update_edge(from, to, ());
    }

    pub async fn remove_all_dependencies(&self, from: QueryKey) {
        let from = self.node_for(from).await;
        self.graph.write().await.retain_edges(|this, edge_index| {
            let (edge_from, _) = &this.edge_endpoints(edge_index).unwrap();
            *edge_from != from
        });
    }

    pub async fn dependencies(&self, key: &QueryKey) -> Option<Vec<QueryKey>> {
        let key = self.indices.get_async(key).await?;
        let graph = self.graph.read().await;
        let mut deps = graph
            .neighbors_directed(*key, petgraph::Direction::Outgoing)
            .map(|i| graph[i].clone())
            .collect::<Vec<_>>();
        deps.sort();
        Some(deps)
    }

    fn blocking_dependencies(&self, key: &QueryKey) -> Option<Vec<QueryKey>> {
        let key = self.indices.get_sync(key)?;
        let graph = self.graph.blocking_read();
        let mut deps = graph
            .neighbors_directed(*key, petgraph::Direction::Outgoing)
            .map(|i| graph[i].clone())
            .collect::<Vec<_>>();
        deps.sort();
        Some(deps)
    }
}

impl Display for DepGraph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let graph = self.graph.blocking_read();
        let mut nodes = graph.node_weights().collect::<Vec<_>>();
        nodes.sort();
        for node in nodes.into_iter() {
            write!(f, "{}: ", node)?;
            if let Some(mut deps) = self.blocking_dependencies(node)
                && !deps.is_empty()
            {
                deps.sort();
                writeln!(f, "[")?;
                for dep in deps.into_iter() {
                    writeln!(f, "\t{},", dep)?;
                }
                writeln!(f, "]")?;
            } else {
                writeln!(f, "None")?;
            }
        }
        Ok(())
    }
}

impl Serialize for DepGraph {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        todo!()
    }
}

impl<'de> Deserialize<'de> for DepGraph {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        todo!()
    }
}

#[derive(Default, Debug)]
pub struct Database {
    pub colors: ColorMap,
    pub revision: AtomicUsize,

    pub cache: QueryCache,

    pub objects: object::Objects,
    pub remotes: remote::RemoteObjects,
}

struct Filenames {
    cache_filename: PathBuf,
    depgraph_filename: PathBuf,
    remote_filename: PathBuf,
    objects_dirname: PathBuf,
}

fn get_filenames(dir: &Path) -> Filenames {
    Filenames {
        cache_filename: dir.join("cache.v1.pc"),
        depgraph_filename: dir.join("depgraph.v1.pc"),
        remote_filename: dir.join("remote.v1.pc"),
        objects_dirname: dir.join("objects"),
    }
}

pub async fn save_to_directory(dir: &Path, db: &Database, deps: &DepGraph) -> crate::Result<()> {
    let Filenames {
        cache_filename,
        depgraph_filename,
        remote_filename,
        objects_dirname,
    } = get_filenames(dir);

    async_fs::create_dir_all(dir).await?;

    // TODO: make these writes async & concurrent
    {
        let cache_file = std::fs::File::create(cache_filename)?;
        postcard::to_io(&db.cache, cache_file)?;
    }
    {
        let depgraph_file = std::fs::File::create(depgraph_filename)?;
        postcard::to_io(deps, depgraph_file)?;
    }
    {
        let remote_file = std::fs::File::create(remote_filename)?;
        postcard::to_io(&db.remotes, remote_file)?;
    }

    db.objects
        .for_each(|hash, contents| -> crate::Result<_> {
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
        })
        .await?;

    Ok(())
}

pub async fn restore_from_directory(dir: &Path) -> crate::Result<(Database, DepGraph)> {
    let Filenames {
        cache_filename,
        depgraph_filename,
        remote_filename,
        objects_dirname,
    } = get_filenames(dir);

    let cache_bytes = std::fs::read(cache_filename)?;
    let cache: QueryCache = postcard::from_bytes(&cache_bytes[..])?;

    let depgraph_bytes = std::fs::read(depgraph_filename)?;
    let depgraph: DepGraph = postcard::from_bytes(&depgraph_bytes[..])?;

    let remote_bytes = std::fs::read(remote_filename)?;
    let remotes: RemoteObjects = postcard::from_bytes(&remote_bytes[..])?;

    // Everything loaded from the disk is green to start with; this will be busted by input queries
    // changing for the next revision.
    let colors = ColorMap::default();
    cache
        .for_each_key(|key| {
            // Wow this sucks can't wait for explicit captures
            let colors = &colors;
            async move { colors.mark_green(&key, 0).await }
        })
        .await;

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
        remotes,
        objects,
    };

    Ok((db, depgraph))
}
