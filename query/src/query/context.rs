use std::any::Any;
use std::any::TypeId;
use std::fmt::Debug;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use dyn_clone::DynClone;

use crate::db::Color;
use crate::db::Database;
use crate::db::DepGraph;
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
    fn produce(&self, ctx: &QueryContext) -> anyhow::Result<Self::Output>;
    fn query(self, ctx: &QueryContext) -> anyhow::Result<Self::Output>
    where
        Self: Sized,
        QueryKey: From<Self>,
    {
        let value = ctx.query(self.into())?;
        Ok(*value
            .downcast()
            .expect("query produced wrong value somehow"))
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

    pub(crate) fn query(&self, key: QueryKey) -> anyhow::Result<AnyOutput> {
        let revision = self.db.revision.load(Ordering::SeqCst);
        let update_value = |key: QueryKey| -> anyhow::Result<_> {
            if let Some(parent) = &self.parent {
                self.dep_graph.add_dependency(parent.clone(), key.clone());
            }

            let value = key.produce(&QueryContext {
                parent: Some(key.clone()),
                db: self.db.clone(),
                dep_graph: self.dep_graph.clone(),
            })?;

            if self.db.cache.insert(key.clone(), value.clone()) {
                // println!("marked green {key:?}");
                self.db.colors.mark_green(&key, revision);
            } else {
                // println!("marked red {key:?}");
                self.db.colors.mark_red(&key, revision);
            }

            Ok(value)
        };

        let Some((_, rev)) = self.db.colors.get(&key) else {
            // println!("not found in colors db {key:?}");
            return update_value(key);
        };
        if key.is_input() && rev < revision {
            // println!("key is input and revision outdated {key:?}");
            return update_value(key);
        }

        match self.try_mark_green(key.clone())? {
            Color::Green => {
                // println!("marked green after trying {key:?}");
                Ok(self
                    .db
                    .cache
                    .get(&key)
                    .unwrap_or_else(|| panic!("Green query {:?} missing value in cache", key)))
            }
            Color::Red => {
                // println!("marked red after trying {key:?}");
                update_value(key)
            }
        }
    }

    fn try_mark_green(&self, key: QueryKey) -> anyhow::Result<Color> {
        let revision = self.db.revision.load(Ordering::SeqCst);
        // If we have no dependencies in the graph, assume we need to run the query.
        let Some(deps) = self.dep_graph.dependencies(&key) else {
            // println!("no dependencies found {key:?}");
            return Ok(Color::Red);
        };
        // println!("dependencies {deps:?} for key {key:?}");
        for dep in deps {
            match self.db.colors.get(&dep) {
                // Dependency is up-to-date in this revision, is ok
                Some((Color::Green, rev)) if revision == rev => {
                    // println!("dependency {dep:?} green the first time for {key:?}");
                    continue;
                }
                // Out-of-date dependency, we must also be out-of-date
                Some((Color::Red, _)) => {
                    // println!("dependency {dep:?} was outdated for {key:?}");
                    return Ok(Color::Red);
                }
                _ => {
                    if dep.is_input() || self.try_mark_green(dep.clone())? != Color::Green {
                        let _ = self.query(dep.clone())?;
                        // Because we just ran the query, we can be sure the revision is
                        // up-to-date.
                        match self.db.colors.get(&dep) {
                            Some((Color::Green, _)) => {
                                // println!("dependency {dep:?} green the second time for {key:?}");
                                continue;
                            }
                            Some((Color::Red, _)) => {
                                // println!("dependency {dep:?} was still outdated for {key:?}");
                                return Ok(Color::Red);
                            }
                            None => unreachable!("we just ran the query"),
                        }
                    } else {
                        // println!("successfully marked dependency {dep:?} green for {key:?}");
                    }
                }
            }
        }

        // If we marked all dependencies as green, mark this node green too.
        self.db.colors.mark_green(&key, revision);
        Ok(Color::Green)
    }
}
