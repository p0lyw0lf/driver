use std::sync::RwLock;
use std::sync::atomic::AtomicUsize;

use dashmap::DashMap;
use petgraph::graph::DiGraph;
use petgraph::graph::NodeIndex;

use crate::QueryKey;
use crate::query_key::QueryCache;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Color {
    Green,
    Red,
}

#[derive(Default, Debug)]
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

#[derive(Default, Debug)]
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
}
