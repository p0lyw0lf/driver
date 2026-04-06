use std::fmt::Debug;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use tracing::{info, trace};

use super::any_output::{AnyOutput, Output};
use super::db::{Database, Entry};
use super::executor::Executor;
use super::key::QueryKey;

/// The main trait for
pub trait Producer {
    type Output: Output + Sized + 'static;
    fn produce(&self, ctx: &QueryContext) -> impl Future<Output = Self::Output>;
}

pub trait Queryable: Producer + Into<QueryKey> + Sized {
    async fn query(self, ctx: &QueryContext) -> Self::Output;
}

impl<T: Producer + Into<QueryKey> + Sized> Queryable for T {
    async fn query(self, ctx: &QueryContext) -> Self::Output {
        let value = ctx
            .executor
            .clone()
            .query(self.into(), ctx.parent.clone())
            .await;
        *value
            .downcast()
            .expect("query produced wrong value somehow")
    }
}

#[derive(Debug, Clone)]
pub struct QueryContext {
    pub(crate) parent: Option<QueryKey>,
    pub(crate) executor: Arc<Executor>,
}

impl QueryContext {
    /// Get the database associated with the context.
    pub fn db(&self) -> &Database {
        &self.executor.db
    }

    /// NOTE: most code that runs inside a query itself should use the `key.query(ctx)` form
    /// instead. This function is meant to be used by the executor itself.
    #[tracing::instrument(level = "debug", skip(self), fields(key=%key))]
    pub(crate) async fn query_internal(&self, key: QueryKey) -> AnyOutput {
        trace!("locking db entry");
        self.db()
            .with_entry(key.clone(), async |mut entry| {
                trace!("locked");
                let entry = &mut entry;
                self.query_entry(key, entry).await
            })
            .await
    }

    #[tracing::instrument(level = "debug", skip(self, entry), fields(key=%key))]
    async fn query_entry(&self, key: QueryKey, entry: &mut Entry) -> AnyOutput {
        trace!("starting query");
        if let Some(parent) = &self.parent {
            info!("adding edge {parent} -> {key}");
            self.db().add_dependency(parent.clone(), key.clone()).await;
            trace!("added");
        }

        let revision = self.db().revision.load(Ordering::SeqCst);
        let verified_at = entry.revision().map(|rev| rev.verified_at);

        let maybe_changed = match verified_at {
            // If we've never seen it before, it's always "changed"
            None => {
                trace!("never seen this key in my life");
                true
            }
            // If we have seen it before, check it again
            Some(verified_at) => {
                self.maybe_changed_after(verified_at, key.clone(), revision, entry)
                    .await
            }
        };
        if !maybe_changed {
            return entry
                .value()
                .unwrap_or_else(|| panic!("Verified query {key} missing value in cache"));
        }

        trace!("removing dependencies");
        // We're about to run the key again, so remove any dependencies it once had
        self.db().remove_all_dependencies(&key).await;
        trace!("removed");

        let value = key
            .produce(&QueryContext {
                parent: Some(key.clone()),
                executor: self.executor.clone(),
            })
            .await;
        trace!("produced value");

        entry.insert(revision, value.clone());
        trace!("inserted entry");

        value
    }

    #[tracing::instrument(level = "debug", skip(self, entry), fields(key=%key))]
    async fn maybe_changed_after(
        &self,
        verified_at: usize,
        key: QueryKey,
        current_revision: usize,
        entry: &mut Entry,
    ) -> bool {
        let Some(rev) = entry.revision() else {
            trace!("no revision, need to calculate");
            return true;
        };

        if key.is_input() {
            trace!(
                "checking input: ({} > {}) || ({} > {})?",
                current_revision, rev.verified_at, rev.changed_at, verified_at
            );
            return current_revision > rev.verified_at || rev.changed_at > verified_at;
        }

        if rev.verified_at >= current_revision {
            trace!("checking condition: {} > {}?", rev.changed_at, verified_at);
            return rev.changed_at > verified_at;
        }

        trace!("trying to get dependencies");
        let Some(deps) = self.db().dependencies(&key).await else {
            trace!("no dependencies");
            // Input queries should be handled the above case; these sorts of queries with no
            // dependencies are deterministic ones entirely determined by their key, so we can mark
            // them verified early
            entry.mark_verified(current_revision);
            return false;
        };

        trace!("got dependencies");
        for dep in deps {
            trace!("locking {dep}");
            if self
                .db()
                .with_entry(dep.clone(), async |mut dep_entry| {
                    trace!("locked {dep}");
                    let dep_maybe_changed = Box::pin(self.maybe_changed_after(
                        verified_at,
                        dep.clone(),
                        current_revision,
                        &mut dep_entry,
                    ))
                    .await;
                    if !dep_maybe_changed {
                        trace!("dep {dep} definitely hasn't changed");
                        return false;
                    }

                    trace!("pre-querying dep {dep}");
                    let _ = Box::pin(self.query_entry(dep, &mut dep_entry)).await;

                    let dep_rev = dep_entry
                        .revision()
                        .expect("revision must be set after query");
                    trace!(
                        "checking dep condition: {} > {}?",
                        dep_rev.changed_at, verified_at
                    );
                    dep_rev.changed_at > verified_at
                })
                .await
            {
                return true;
            }
        }

        // If we marked all dependencies as green, mark this node green too.
        entry.mark_verified(current_revision);
        rev.changed_at > verified_at
    }
}
