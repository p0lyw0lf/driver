use std::any::Any;
use std::any::TypeId;
use std::fmt::Debug;
use std::path::PathBuf;

mod db;
mod files;
mod query_context;
mod query_key;
mod to_hash;

use dyn_clone::DynClone;
pub use query_context::QueryContext;
use query_key::QueryKey;
use sha2::Digest;

use crate::to_hash::Hash;
use crate::to_hash::ToHash;

/// NOTE: a newtype is needed to get around some associated type jank.
#[derive(Clone, Debug)]
struct AnyOutput(pub Box<dyn Output>);
trait Output: ToHash + DynClone + Any + Debug {}
dyn_clone::clone_trait_object!(Output);
impl<T> Output for T where T: ToHash + DynClone + Any + Debug {}
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
    fn downcast<T: Output>(self) -> Option<Box<T>> {
        (self.0 as Box<dyn Any>).downcast().ok()
    }
}

trait Producer {
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

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
struct HashDirectory(PathBuf);

impl Producer for HashDirectory {
    type Output = Hash;
    fn produce(&self, ctx: &QueryContext) -> anyhow::Result<Self::Output> {
        println!("hashing {}", self.0.display());
        let mut hasher = sha2::Sha256::new();
        let entries = files::ListDirectory(self.0.clone()).query(ctx)?;
        for entry in entries {
            let digest = if entry.is_dir() {
                HashDirectory(entry.clone()).query(ctx)?
            } else {
                HashFile(entry.clone()).query(ctx)?
            };
            hasher.update(digest);
        }
        Ok(hasher.finalize())
    }
}

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
struct HashFile(PathBuf);

impl Producer for HashFile {
    type Output = Hash;
    fn produce(&self, ctx: &QueryContext) -> anyhow::Result<Self::Output> {
        println!("hashing {}", self.0.display());
        let mut hasher = sha2::Sha256::new();
        let contents = files::ReadFile(self.0.clone()).query(ctx)?;
        hasher.update(contents.as_bytes());
        Ok(hasher.finalize())
    }
}

pub fn walk(dir: PathBuf, ctx: &QueryContext) -> anyhow::Result<Hash> {
    HashDirectory(dir).query(ctx)
}
