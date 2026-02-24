use std::any::Any;
use std::any::TypeId;
use std::fmt::Debug;
use std::fmt::Display;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use dyn_clone::DynClone;
use tracing::debug;

use crate::db::Color;
use crate::db::Database;
use crate::db::DepGraph;
use crate::options::OPTIONS;
use crate::query::key::QueryKey;
use crate::to_hash::ToHash;

/// NOTE: a newtype is needed to get around some associated type jank.
#[derive(Clone, Debug)]
pub struct AnyOutput(pub Box<dyn Output>);
pub trait Output: ToHash + DynClone + Any + Debug {}
dyn_clone::clone_trait_object!(Output);
impl<T> Output for T where T: ToHash + DynClone + Any + Debug {}
impl ToHash for AnyOutput {
    fn run_hash(&self, hasher: &mut sha2::Sha256) {
        // no prefix because we _do_ want this to be treated as the underlying value.
        self.0.run_hash(hasher);
    }
}
impl AnyOutput {
    pub fn new(t: impl Output) -> Self {
        if t.type_id() == TypeId::of::<AnyOutput>() {
            panic!("tried to put box inside of box");
        }
        Self(Box::new(t))
    }
    pub fn downcast<T: Output>(self) -> Option<Box<T>> {
        (self.0 as Box<dyn Any>).downcast().ok()
    }
}

pub trait Producer {
    // NOTE: in order to make the lifetimes work out, we really really want it such that the output
    // is easily clone-able. This will eventually require string interning somewhere, not quite
    // sure where yet.
    type Output: Output + Sized + 'static;
    async fn produce(&self, ctx: &QueryContext) -> Self::Output;
    async fn query(self, ctx: &QueryContext) -> Self::Output
    where
        Self: Sized,
        QueryKey: From<Self>,
    {
        let value = ctx.query(self.into()).await;
        *value
            .downcast()
            .expect("query produced wrong value somehow")
    }
}

#[derive(Default, Debug)]
pub struct QueryContext {
    parent: Option<QueryKey>,
    pub(crate) db: Arc<Database>,
    dep_graph: Arc<DepGraph>,
}

impl QueryContext {
    pub fn new_revision(&self) {
        self.db.revision.fetch_add(1, Ordering::SeqCst);
    }

    pub fn display_dep_graph(&self) -> &'_ impl Display {
        &self.dep_graph
    }

    #[tracing::instrument(level = "debug", skip(self))]
    pub(crate) async fn query(&self, key: QueryKey) -> AnyOutput {
        if let Some(parent) = &self.parent {
            self.dep_graph.add_dependency(parent.clone(), key.clone());
        }

        let revision = self.db.revision.load(Ordering::SeqCst);
        let update_value = |key: QueryKey| {
            // We're about to run the key again, so remove any dependencies it once had
            self.dep_graph.remove_all_dependencies(key.clone());

            let value = key.produce(&QueryContext {
                parent: Some(key.clone()),
                db: self.db.clone(),
                dep_graph: self.dep_graph.clone(),
            });

            if self.db.cache.insert(key.clone(), value.clone()) {
                debug!("marked green {key}");
                self.db.colors.mark_green(&key, revision);
            } else {
                debug!("marked red {key}");
                self.db.colors.mark_red(&key, revision);
            }

            value
        };

        let Some((_, rev)) = self.db.colors.get(&key) else {
            debug!("not found in colors db {key}");
            return update_value(key);
        };
        if key.is_input() && rev < revision {
            debug!("key is input and revision outdated {key}");
            return update_value(key);
        }

        match self.try_mark_green(key.clone()) {
            Color::Green => {
                debug!("marked green after trying {key}");
                self.db
                    .cache
                    .get(&key)
                    .unwrap_or_else(|| panic!("Green query {key} missing value in cache"))
            }
            Color::Red => {
                debug!("marked red after trying {key}");
                update_value(key)
            }
        }
    }

    fn try_mark_green(&self, key: QueryKey) -> Color {
        let revision = self.db.revision.load(Ordering::SeqCst);
        // If we have no dependencies in the graph, assume we need to run the query.
        let Some(deps) = self.dep_graph.dependencies(&key) else {
            debug!("no dependencies found {key}");
            return Color::Red;
        };
        for dep in deps {
            match self.db.colors.get(&dep) {
                // Dependency is up-to-date in this revision, is ok
                Some((Color::Green, rev)) if revision == rev => {
                    debug!("dependency {dep} green the first time for {key}");
                    continue;
                }
                // Out-of-date dependency, we must also be out-of-date
                Some((Color::Red, _)) => {
                    debug!("dependency {dep} was outdated for {key}");
                    return Color::Red;
                }
                _ => {
                    if dep.is_input() || self.try_mark_green(dep.clone()) != Color::Green {
                        let _ = self.query(dep.clone());
                        // Because we just ran the query, we can be sure the revision is
                        // up-to-date.
                        match self.db.colors.get(&dep) {
                            Some((Color::Green, _)) => {
                                debug!("dependency {dep} green the second time for {key}");
                                continue;
                            }
                            Some((Color::Red, _)) => {
                                debug!("dependency {dep} was still outdated for {key}");
                                return Color::Red;
                            }
                            None => unreachable!("we just ran the query"),
                        }
                    } else {
                        debug!("successfully marked dependency {dep} green for {key}");
                    }
                }
            }
        }

        // If we marked all dependencies as green, mark this node green too.
        self.db.colors.mark_green(&key, revision);
        Color::Green
    }

    pub fn save(&self) -> crate::Result<()> {
        let cache_dir = &OPTIONS.read().unwrap().cache_dir;
        crate::db::save_to_directory(cache_dir, &self.db, &self.dep_graph)
    }

    pub fn restore() -> crate::Result<Self> {
        let cache_dir = &OPTIONS.read().unwrap().cache_dir;
        let (db, dep_graph) = crate::db::restore_from_directory(cache_dir)?;
        Ok(Self {
            parent: None,
            db: Arc::new(db),
            dep_graph: Arc::new(dep_graph),
        })
    }
}
