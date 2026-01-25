use std::any::Any;
use std::any::TypeId;
use std::fmt::Debug;
use std::path::PathBuf;

mod db;
mod files;
mod query_context;
mod query_key;
mod to_hash;

use query_context::QueryContext;
use query_key::QueryKey;
use sha2::Digest;

use crate::to_hash::Hash;
use crate::to_hash::ToHash;

/// NOTE: a newtype is needed to get around some associated type jank.
#[derive(Debug)]
struct AnyOutput(pub Box<dyn Output>);
trait Output: ToHash + Any + Debug {}
impl<T> Output for T where T: ToHash + Any + Debug {}
impl ToHash for AnyOutput {
    fn run_hash(&self, hasher: &mut sha2::Sha256) {
        // no prefix because we _do_ want this to be treated as the underlying value.
        self.0.run_hash(hasher);
    }
}
impl AnyOutput {
    fn new(t: impl Output) -> Self {
        if t.type_id() == TypeId::of::<AnyOutput>() {
            panic!("tried to put box inside of box");
        }
        Self(Box::new(t))
    }
    fn downcast_ref<T: Output>(v: &impl std::ops::Deref<Target = Self>) -> Option<&T> {
        v.0.downcast_ref()
    }
}
impl dyn Output {
    fn downcast_ref<T: Output>(&self) -> Option<&T> {
        let a = self as &dyn Any;
        a.downcast_ref()
    }
}

trait Producer {
    type Output: ToHash + Debug + Sized + 'static;
    fn produce(&self, ctx: &QueryContext) -> anyhow::Result<Self::Output>;
    fn downcast_ref(v: &impl std::ops::Deref<Target = AnyOutput>) -> &Self::Output {
        AnyOutput::downcast_ref(v).expect("used unsafely whoops")
    }
}

fn walk_impl(ctx: &QueryContext, dir: PathBuf) -> anyhow::Result<Hash> {
    let mut hasher = sha2::Sha256::new();
    let hash = ctx.query(files::ListDirectory(dir).into())?;
    let value = ctx.db.get_interned(&hash);
    let entries = files::ListDirectory::downcast_ref(&value);
    for entry in entries {
        let digest = if entry.is_dir() {
            walk_impl(ctx, entry.clone())?
        } else {
            ctx.query(files::ReadFile(entry.clone()).into())?
        };
        hasher.update(digest);
    }
    Ok(hasher.finalize())
}

pub fn walk(dir: PathBuf) -> anyhow::Result<Hash> {
    walk_impl(&QueryContext::default(), dir)
}
