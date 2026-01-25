use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::Producer;
use crate::db::Color;
use crate::db::Database;
use crate::db::DepGraph;
use crate::query_key::QueryKey;
use crate::to_hash::Hash;
use crate::to_hash::ToHash;

#[derive(Default, Debug)]
pub struct QueryContext {
    parent: Option<QueryKey>,
    pub db: Arc<Database>,
    dep_graph: Arc<DepGraph>,
}

impl QueryContext {
    pub fn query(&self, key: QueryKey) -> anyhow::Result<Hash> {
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
            let hash = value.to_hash();

            let old_hash = self.db.cached.insert(key.clone(), hash);
            if old_hash.is_some_and(|old| old == hash) {
                self.db.colors.mark_green(&key, revision);
            } else {
                self.db.colors.mark_red(&key, revision);
            }

            Ok(self.db.intern(value))
        };

        let Some((_, rev)) = self.db.colors.get(&key) else {
            return update_value(key);
        };
        if rev < revision {
            return update_value(key);
        }

        match self.try_mark_green(key.clone())? {
            Color::Green => {
                let hash = self
                    .db
                    .cached
                    .get(&key)
                    .unwrap_or_else(|| panic!("Green query {:?} missing value in cache", key));
                Ok(*hash)
            }
            Color::Red => update_value(key),
        }
    }

    fn try_mark_green(&self, key: QueryKey) -> anyhow::Result<Color> {
        let revision = self.db.revision.load(Ordering::SeqCst);
        // If we have no dependencies in the graph, assume we need to run the query.
        let Some(deps) = self.dep_graph.dependencies(&key) else {
            return Ok(Color::Red);
        };
        for dep in deps {
            match self.db.colors.get(&dep) {
                // Dependency is up-to-date in this revision, is ok
                Some((Color::Green, rev)) if revision == rev => continue,
                // Out-of-date dependency, we must also be out-of-date
                Some((Color::Red, _)) => return Ok(Color::Red),
                _ => {
                    if self.try_mark_green(dep.clone())? != Color::Green {
                        let _ = self.query(dep.clone())?;
                        // Because we just ran the query, we can be sure the revision is
                        // up-to-date.
                        match self.db.colors.get(&dep) {
                            Some((Color::Green, _)) => continue,
                            Some((Color::Red, _)) => return Ok(Color::Red),
                            None => unreachable!("we just ran the query"),
                        }
                    }
                }
            }
        }

        // If we marked all dependencies as green, mark this node green too.
        self.db.colors.mark_green(&key, revision);
        Ok(Color::Green)
    }
}
