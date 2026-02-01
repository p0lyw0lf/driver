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
use koto::Koto;
pub use query_context::QueryContext;
use query_key::QueryKey;
use sha2::Digest;

use crate::to_hash::Hash;
use crate::to_hash::ToHash;
use crate::to_hash::koto::HashedKFunction;

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

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
struct MemoizeKotoFunction(String);

impl Producer for MemoizeKotoFunction {
    type Output = ();
    fn produce(&self, ctx: &QueryContext) -> anyhow::Result<Self::Output> {
        let mut koto = Koto::default();
        let value = koto.compile_and_run(&self.0)?;
        let f = match value {
            koto::runtime::KValue::Function(f) => f,
            _ => anyhow::bail!("not a function"),
        };

        let h = HashedKFunction::Function(f);
        RunKotoFunction(h).query(ctx)?;

        Ok(())
    }
}

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
struct RunKotoFunction(HashedKFunction);

impl Producer for RunKotoFunction {
    type Output = ();
    fn produce(&self, _ctx: &QueryContext) -> anyhow::Result<Self::Output> {
        match &self.0 {
            HashedKFunction::Function(kfunction) => {
                let mut koto = Koto::default();
                let value =
                    koto.call_function(koto::runtime::KValue::Function(kfunction.clone()), &[])?;
                println!("value {value:?}");
                Ok(())
            }
            HashedKFunction::Hash(_) => {
                anyhow::bail!("cannot run hashed function")
            }
        }
    }
}

pub fn koto(source: &str, ctx: &QueryContext) {
    MemoizeKotoFunction(source.to_string())
        .query(ctx)
        .expect("failed to do the thing")
}
