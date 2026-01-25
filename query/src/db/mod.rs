use std::any::Any;
use std::sync::RwLock;
use std::sync::atomic::AtomicUsize;

use dashmap::DashMap;
use petgraph::graph::DiGraph;
use petgraph::graph::NodeIndex;

use crate::AnyOutput;
use crate::Output;
use crate::QueryKey;
use crate::to_hash::Hash;

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Color {
    Green,
    Red,
}

#[derive(Default)]
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

#[derive(Default)]
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

    pub fn dependencies(&self, key: &QueryKey) -> Option<Vec<QueryKey>> {
        self.indices.get(key).map(|key| {
            let graph = self.graph.read().unwrap();
            graph
                .neighbors_directed(*key, petgraph::Direction::Incoming)
                .map(|i| graph[i].clone())
                .collect::<Vec<_>>()
        })
    }
}

#[derive(Default)]
pub struct Database {
    pub colors: ColorMap,
    pub revision: AtomicUsize,

    /// ENSURES: intern.get(hash).to_hash() == hash
    /// This does also require that hashes are unique _per-type_, which is only possible since we
    /// control the hash function strategy.
    pub interned: DashMap<Hash, AnyOutput>,
    /// ENSURES: intern.get(cache.get(key)).type_id() == <QueryKey as Producer>::Output.type_id()
    pub cached: DashMap<QueryKey, Hash>,
}

impl Database {
    pub fn intern<T: Output>(&self, value: T) -> Hash {
        let hash = value.to_hash();
        let ty = value.type_id();
        if self
            .interned
            .insert(hash, AnyOutput::new(value))
            .is_some_and(|old| old.type_id() != ty)
        {
            panic!("found hash collision at {hash:?}");
        }

        hash
    }

    pub fn get_interned(&self, hash: &Hash) -> dashmap::mapref::one::Ref<'_, Hash, AnyOutput> {
        self.interned.get(hash).expect("hash should always exist")
    }
}
