use std::any::Any;
use std::any::TypeId;
use std::fmt::Debug;
use std::fmt::Display;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use dyn_clone::DynClone;
use tracing::{info, trace};

use crate::db::Database;
use crate::db::Entry;
use crate::options::OPTIONS;
use crate::query::key::QueryKey;
use crate::to_hash::ToHash;

/// NOTE: a newtype is needed to get around some associated type jank.
#[derive(Clone, Debug)]
pub struct AnyOutput(pub Box<dyn Output>);
pub trait Output: ToHash + DynClone + Any + Debug + Send + Sync {}
dyn_clone::clone_trait_object!(Output);

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

impl PartialEq for AnyOutput {
    fn eq(&self, other: &Self) -> bool {
        self.to_hash() == other.to_hash()
    }
}

pub trait Producer {
    // NOTE: in order to make the lifetimes work out, we really really want it such that the output
    // is easily clone-able. This will eventually require string interning somewhere, not quite
    // sure where yet.
    type Output: Output + Sized + 'static;
    fn produce(&self, ctx: &QueryContext) -> impl Future<Output = Self::Output> + Send;
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

#[derive(Debug, Clone)]
pub struct QueryContext {
    pub(crate) rt: Arc<tokio::runtime::Runtime>,
    parent: Option<QueryKey>,
    pub(crate) db: Arc<Database>,
}

impl QueryContext {
    pub fn new_revision(&self) {
        self.db.revision.fetch_add(1, Ordering::SeqCst);
    }

    pub fn display_dep_graph(&self) -> impl Display + '_ {
        self.db.display_dep_graph()
    }

    #[tracing::instrument(level = "debug", skip(self), fields(key=%key))]
    pub(crate) async fn query(&self, key: QueryKey) -> AnyOutput {
        trace!("locking db entry");
        self.db
            .with_entry(key.clone(), async |mut entry| {
                trace!("locked");
                let entry = &mut entry;
                self.query_entry(key, entry).await
            })
            .await
    }

    #[tracing::instrument(level = "debug", skip(self, entry), fields(key=%key))]
    async fn query_entry<'a>(&self, key: QueryKey, entry: &mut Entry<'a>) -> AnyOutput {
        trace!("starting query");
        if let Some(parent) = &self.parent {
            info!("adding edge {parent} -> {key}");
            self.db.add_dependency(parent.clone(), key.clone()).await;
            trace!("added");
        }

        let revision = self.db.revision.load(Ordering::SeqCst);
        let verified_at = entry.revision().map(|rev| rev.verified_at);

        let is_changed = match verified_at {
            // If we've never seen it before, it's always "changed"
            None => true,
            // If we have seen it before, check it again
            Some(verified_at) => {
                self.maybe_changed_after(verified_at, key.clone(), revision, entry)
                    .await
            }
        };
        if !is_changed {
            return entry
                .value()
                .unwrap_or_else(|| panic!("Verified query {key} missing value in cache"))
                .clone();
        }

        trace!("removing dependencies");
        // We're about to run the key again, so remove any dependencies it once had
        self.db.remove_all_dependencies(&key).await;
        trace!("removed");

        let rt = self.rt.clone();
        let db = self.db.clone();
        let value = tokio::spawn(async move {
            key.produce(&QueryContext {
                rt,
                parent: Some(key.clone()),
                db,
            })
            .await
        })
        .await
        .expect("joining task");
        trace!("produced value");

        entry.insert(revision, value.clone());
        trace!("inserted entry");

        value
    }

    #[tracing::instrument(level = "debug", skip(self, entry), fields(key=%key))]
    async fn maybe_changed_after<'a>(
        &self,
        verified_at: usize,
        key: QueryKey,
        current_revision: usize,
        entry: &mut Entry<'a>,
    ) -> bool {
        let Some(rev) = entry.revision() else {
            trace!("no revision, need to calculate");
            return true;
        };

        if key.is_input() || rev.verified_at >= current_revision {
            trace!("checking condition: {} > {}?", rev.changed_at, verified_at);
            return rev.changed_at > verified_at;
        }

        trace!("trying to get dependencies");
        let Some(deps) = self.db.dependencies(&key).await else {
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
            let dep_changed = self
                .db
                .with_entry(dep.clone(), async |mut dep_entry| {
                    trace!("locked {dep}");
                    Box::pin(self.maybe_changed_after(
                        verified_at,
                        dep,
                        current_revision,
                        &mut dep_entry,
                    ))
                    .await
                })
                .await;
            if dep_changed {
                return true;
            }
        }

        // If we marked all dependencies as green, mark this node green too.
        entry.mark_verified(current_revision);
        rev.changed_at > verified_at
    }

    pub async fn save(&self, rt: Arc<tokio::runtime::Runtime>) -> crate::Result<()> {
        let cache_path = OPTIONS.read().unwrap().cache_path.clone();
        Database::save_to_file(self.db.clone(), rt, &cache_path).await
    }

    pub async fn restore_or_default(rt: Arc<tokio::runtime::Runtime>) -> Self {
        let cache_path = OPTIONS.read().unwrap().cache_path.clone();
        let db = Database::restore_from_file(&cache_path)
            .await
            .unwrap_or_else(|err| {
                eprintln!("error restoring database: {err}");
                Default::default()
            });

        let out = Self {
            rt,
            parent: None,
            db: Arc::new(db),
        };
        // Bust cache immediately, since it was just read from disk
        out.new_revision();
        out
    }
}
