use std::fmt::Debug;
use std::path::PathBuf;

use sha2::Digest;

mod db;
mod error;
mod files;
mod js;
mod query;
mod to_hash;

pub use error::Error;
use query::context::Producer;
pub use query::context::QueryContext;
use query::key::QueryKey;
use to_hash::Hash;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
struct HashDirectory(PathBuf);

impl Producer for HashDirectory {
    type Output = Result<Hash>;
    fn produce(&self, ctx: &QueryContext) -> Self::Output {
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
    type Output = Result<Hash>;
    fn produce(&self, ctx: &QueryContext) -> Self::Output {
        println!("hashing {}", self.0.display());
        let mut hasher = sha2::Sha256::new();
        let contents = files::ReadFile(self.0.clone()).query(ctx)?;
        hasher.update(&contents[..]);
        Ok(hasher.finalize())
    }
}

pub fn walk(dir: PathBuf, ctx: &QueryContext) -> crate::Result<Hash> {
    HashDirectory(dir).query(ctx)
}

pub fn run(file: PathBuf, ctx: &QueryContext) -> crate::Result<()> {
    let v = js::RunFile { file, args: None }.query(ctx);
    println!("{v:?}");
    Ok(())
}
