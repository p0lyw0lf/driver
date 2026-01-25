use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::AnyOutput;
use crate::Producer;
use crate::db::Color;
use crate::db::Database;
use crate::db::DepGraph;
use crate::query_key::QueryKey;

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
