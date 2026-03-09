use std::any::Any;
use std::any::TypeId;
use std::fmt::Debug;
use std::fmt::Display;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use dyn_clone::DynClone;
use tracing::debug;
use tracing::trace;

use crate::db::Color;
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

#[derive(Debug)]
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

    #[tracing::instrument(level = "trace", skip(self, entry), fields(key=%key))]
    async fn query_entry<'a>(&self, key: QueryKey, entry: &mut Entry<'a>) -> AnyOutput {
        trace!("starting query");
        if let Some(parent) = &self.parent {
            tracing::info!("adding edge {parent} -> {key}");
            self.db.add_dependency(parent.clone(), key.clone()).await;
            trace!("added");
        }

        let revision = self.db.revision.load(Ordering::SeqCst);

        let Some((_, rev)) = entry.color() else {
            debug!("not found in colors db");
            return self.update_value(revision, key, entry).await;
        };
        if key.is_input() && rev < revision {
            debug!("key is input and revision outdated");
            return self.update_value(revision, key, entry).await;
        }

        match self.try_mark_green(revision, key.clone(), entry).await {
            Ok(value) => {
                debug!("marked green after trying");
                value
            }
            Err(()) => {
                debug!("marked red after trying");
                self.update_value(revision, key, entry).await
            }
        }
    }

    #[tracing::instrument(level = "trace", skip(self, entry), fields(key=%key))]
    async fn update_value<'a>(
        &self,
        revision: usize,
        key: QueryKey,
        entry: &mut Entry<'a>,
    ) -> AnyOutput {
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

    /// Ok == Green, Err == Red
    async fn try_mark_green<'a>(
        &self,
        revision: usize,
        key: QueryKey,
        entry: &mut Entry<'a>,
    ) -> Result<AnyOutput, ()> {
        // If we have no dependencies in the graph, assume we need to run the query.
        let Some(deps) = self.db.dependencies(&key).await else {
            debug!("no dependencies found");
            return Err(());
        };
        for dep in deps {
            match self.db.get_color(&dep).await {
                // Dependency is up-to-date in this revision, is ok
                Some((Color::Green, rev)) if revision == rev => {
                    debug!("dependency {dep} green the first time");
                    continue;
                }
                // Out-of-date dependency, we must also be out-of-date
                Some((Color::Red, _)) => {
                    debug!("dependency {dep} was outdated");
                    return Err(());
                }
                _ => {
                    trace!("locking {dep}");
                    self.db
                        .with_entry(dep.clone(), async |mut dep_entry| {
                            trace!("locked {dep}");
                            let dep_entry = &mut dep_entry;
                            let needs_recalculation = if dep.is_input() {
                                // Inputs always need to be recalculated.
                                true
                            } else {
                                // Dependencies that themselves have out-of-date dependencies need to be
                                // recalculated.
                                Box::pin(self.try_mark_green(revision, dep.clone(), dep_entry))
                                    .await
                                    .is_err()
                            };
                            if needs_recalculation {
                                let _ = Box::pin(self.query_entry(dep.clone(), dep_entry)).await;
                                // Because we just ran the query, we can be sure the revision is
                                // up-to-date.
                                match dep_entry.color() {
                                    Some((Color::Green, _)) => {
                                        debug!("dependency {dep} green the second time");
                                        Ok(())
                                    }
                                    Some((Color::Red, _)) => {
                                        debug!("dependency {dep} was still outdated");
                                        Err(())
                                    }
                                    None => unreachable!("we just ran the query"),
                                }
                            } else {
                                debug!("successfully marked dependency {dep} green");
                                Ok(())
                            }
                        })
                        .await?
                }
            }
        }

        // If we marked all dependencies as green, mark this node green too.
        entry.mark_green(revision);
        entry.value().cloned().ok_or(())
    }

    pub async fn save(&self) -> crate::Result<()> {
        let cache_dir = OPTIONS.read().unwrap().cache_dir.clone();
        crate::db::save_to_directory(&cache_dir, &self.db).await
    }

    async fn restore(rt: Arc<tokio::runtime::Runtime>) -> crate::Result<Self> {
        let cache_dir = OPTIONS.read().unwrap().cache_dir.clone();
        let db = crate::db::restore_from_directory(&cache_dir).await?;
        Ok(Self {
            rt,
            parent: None,
            db: Arc::new(db),
        })
    }

    pub async fn restore_or_default(rt: Arc<tokio::runtime::Runtime>) -> Self {
        match Self::restore(rt.clone()).await {
            Ok(ctx) => ctx,
            Err(e) => {
                tracing::warn!("error restoring context: {e}");
                Self {
                    rt,
                    parent: None,
                    db: Default::default(),
                }
            }
        }
    }
}
