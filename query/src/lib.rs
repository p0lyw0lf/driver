use std::fmt::Debug;
use std::path::PathBuf;

use sha2::Digest;

mod db;
mod error;
mod js;
mod options;
mod query;
mod to_hash;

pub use error::Error;
use query::context::Producer;
pub use query::context::QueryContext;
use query::key::QueryKey;
use to_hash::Hash;

use crate::options::OPTIONS;
use crate::to_hash::ToHash;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
struct HashDirectory(PathBuf);

impl Producer for HashDirectory {
    type Output = Result<Hash>;
    fn produce(&self, ctx: &QueryContext) -> Self::Output {
        println!("hashing {}", self.0.display());
        let mut hasher = sha2::Sha256::new();
        let entries = query::files::ListDirectory(self.0.clone()).query(ctx)?;
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
        let object = query::files::ReadFile(self.0.clone()).query(ctx)?;
        Ok(object.to_hash())
    }
}

pub fn walk(dir: PathBuf, ctx: &QueryContext) -> crate::Result<Hash> {
    HashDirectory(dir).query(ctx)
}

pub fn run(file: PathBuf, ctx: &QueryContext) -> crate::Result<()> {
    let outputs = js::RunFile { file, args: None }.query(ctx)?;
    // TODO: eventually I'd like to have some sort of diffing algorithm to make this more
    // efficient. But for now a "wipe and re-write" is probably good enough.
    let root = &OPTIONS.read().unwrap().output_dir;
    std::fs::remove_dir_all(root)?;
    for output in outputs {
        let full_path = root.join(output.path);
        std::fs::create_dir_all(full_path.parent().unwrap())?;
        let content = ctx.db.objects.get(&output.object).expect("missing object");
        std::fs::write(full_path, content.as_ref())?;
    }
    Ok(())
}
