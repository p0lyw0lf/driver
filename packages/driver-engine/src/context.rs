use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::{fmt::Debug, hash::Hash};

use tracing::{info, trace};

use async_tpc_executor::Executor;
use driver_db::{Database, Entry, Options};

use crate::Producer;

#[derive(Debug)]
struct State<Key: Hash + Ord + Eq, Output> {
    db: Database<Key, Output>,
    executor: Executor,
}

#[derive(Debug, Clone)]
pub struct Context<Key: Hash + Ord + Eq, Output> {
    pub(crate) parent: Option<Key>,
    state: Arc<State<Key, Output>>,
}

impl<Key, Output> Context<Key, Output> {
    /// Get the database associated with the context.
    pub fn db(&self) -> &Database<Key, Key::Output> {
        &self.state.db
    }

    /// Get the executor associated with the context.
    pub fn executor(&self) -> &Executor {
        &self.state.executor
    }
}

impl<Key: Producer<Key>> Context<Key, Key::Output> {
    /// Starts a new root context. Users SHOULD call `.destroy_root()` before dropping it. MUST be
    /// called outside of any async context.
    pub fn create_root(options: &Options) -> Self {
        let db = Database::restore(options);

        // Bust cache immediately
        // TODO: should only bust the input queries here; right now this busts "everything" which
        // is wrong; see thunderseethe's email response.
        db.revision.fetch_add(1, Ordering::Relaxed);

        let executor = Executor::start();

        Self {
            parent: None,
            state: Arc::new(State { db, executor }),
        }
    }

    /// Stops a root context. MUST only be called:
    /// - on contexts directly created by `Context::create_root()`
    /// - with the same `options` as that `create_root()` call.
    /// - outside of any async context.
    ///
    /// TODO: I should probably find a more type-safe way to enforce this API...
    pub fn destroy_root(self, options: &Options) -> driver_util::Result<()> {
        let Self { parent: _, state } = self;
        let state = Arc::into_inner(state).expect("was still running");
        state.executor.stop();
        state.db.save(options)
    }

    /// Creates a root context with an empty database and a single-threaded executor. Only meant
    /// for testing, you probably want to use `Context::create_root()` instead.
    pub fn create_empty_root_for_testing_only() -> Self {
        let db = Database::empty();
        let executor = Executor::start_n_threads(1);

        Self {
            parent: None,
            state: Arc::new(State { db, executor }),
        }
    }

    /// NOTE: most code that runs inside a query itself should use the `key.query(ctx)` form
    /// instead. This function is meant to be used by the executor itself.
    #[tracing::instrument(level = "debug", skip(self), fields(key=%key))]
    pub(crate) async fn query_internal(self, key: Key) -> Key::Output {
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
    async fn query_entry(&self, key: Key, entry: &mut Entry<Key::Output>) -> Key::Output {
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
            .produce(&Context {
                parent: Some(key.clone()),
                state: self.state.clone(),
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
        key: Key,
        current_revision: usize,
        entry: &mut Entry<Key::Output>,
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
                .with_entry(dep.clone(), async |dep_entry| {
                    trace!("locked {dep}");
                    let dep_maybe_changed = Box::pin(self.maybe_changed_after(
                        verified_at,
                        dep.clone(),
                        current_revision,
                        dep_entry,
                    ))
                    .await;
                    if !dep_maybe_changed {
                        trace!("dep {dep} definitely hasn't changed");
                        return false;
                    }

                    trace!("pre-querying dep {dep}");
                    let _ = Box::pin(self.query_entry(dep, dep_entry)).await;

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
