use std::any::Any;
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
struct AnyOutput(pub Box<dyn Output>);
trait Output: ToHash + Any {}
impl<T> Output for T where T: ToHash + Any {}
impl ToHash for AnyOutput {
    fn run_hash(&self, hasher: &mut sha2::Sha256) {
        // no prefix because we _do_ want this to be treated as the underlying value.
        self.0.run_hash(hasher);
    }
}
impl AnyOutput {
    fn new(t: impl Output) -> Self {
        Self(Box::new(t))
    }
}

trait Producer {
    type Output: Sized + ToHash + 'static;
    fn produce(&self, ctx: &QueryContext) -> anyhow::Result<Self::Output>;
}

fn walk_impl(ctx: &QueryContext, dir: PathBuf) -> anyhow::Result<Hash> {
    let mut hasher = sha2::Sha256::new();
    let hash = ctx.query(files::ListDirectory(dir).into())?;
    let entries = ctx.db.get_interned(&hash);
    for entry in [] {
        let digest = if entry.is_dir() {
            walk_impl(ctx, entry.clone())?
        } else {
            files::ReadFile(entry.clone()).query(ctx)?.to_hash()
        };
        hasher.update(digest);
    }
    Ok(hasher.finalize())
}

pub fn walk(dir: PathBuf) -> anyhow::Result<Hash> {
    walk_impl(&QueryContext::default(), dir)
}
