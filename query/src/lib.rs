use std::path::PathBuf;

mod db;
mod error;
mod js;
mod options;
mod query;
mod to_hash;

use options::OPTIONS;
use query::context::Producer;
use query::key::QueryKey;

pub use error::Error;
pub use query::context::QueryContext;

pub type Result<T> = std::result::Result<T, Error>;

pub fn run(file: PathBuf, ctx: &QueryContext) -> crate::Result<()> {
    let output = js::RunFile { file, args: None }.query(ctx)?;
    // TODO: eventually I'd like to have some sort of diffing algorithm to make this more
    // efficient. But for now a "wipe and re-write" is probably good enough.
    let root = &OPTIONS.read().unwrap().output_dir;
    std::fs::remove_dir_all(root)?;
    for output in output.outputs {
        let full_path = root.join(output.path);
        std::fs::create_dir_all(full_path.parent().unwrap())?;
        let content = ctx.db.objects.get(&output.object).expect("missing object");
        std::fs::write(full_path, content.as_ref())?;
    }
    Ok(())
}
