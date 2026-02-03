use std::fmt::Debug;
use std::path::PathBuf;

mod db;
mod files;
mod query;
mod to_hash;

pub use query::context::QueryContext;
use query::key::QueryKey;
use sha2::Digest;

use crate::query::context::Producer;
use crate::to_hash::Hash;

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
