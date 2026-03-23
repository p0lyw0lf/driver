use std::path::{Path, PathBuf};

mod db;
mod error;
mod js;
mod options;
mod query;
mod serde;
mod to_hash;

use options::OPTIONS;
use query::context::Producer;
use query::key::QueryKey;

pub use error::Error;
pub use query::context::QueryContext;

use crate::db::object::Object;
use crate::js::WriteOutputs;

pub type Result<T> = std::result::Result<T, Error>;

pub struct Output {
    prev: Option<WriteOutputs>,
    curr: WriteOutputs,
}

pub async fn run(file: PathBuf, ctx: &QueryContext) -> crate::Result<Output> {
    let key = js::RunFile { file, arg: None };
    // SAFETY: we are the one place this is supposed to be used
    let prev = match unsafe { ctx.db.get_value(key.clone()).await } {
        None => None,
        Some(Ok(v)) => Some(v.outputs),
        Some(Err(e)) => return Err(e),
    };
    let output = key.query(ctx).await?;
    Ok(Output {
        prev,
        curr: output.outputs,
    })
}

impl Output {
    pub async fn write(self, ctx: &QueryContext) -> crate::Result<()> {
        let root = &OPTIONS.read().unwrap().output_path.clone();
        match self.prev {
            None => write(ctx, root, self.curr.into_iter()).await,
            Some(prev) => {
                write(
                    ctx,
                    root,
                    self.curr.into_iter().filter(|(path, object)| {
                        prev.get(path)
                            .is_none_or(|prev_object| prev_object != object)
                    }),
                )
                .await
            }
        }
    }
}

async fn write(
    ctx: &QueryContext,
    root: &Path,
    iter: impl Iterator<Item = (PathBuf, Object)>,
) -> crate::Result<()> {
    let mut js = tokio::task::JoinSet::new();
    for (path, object) in iter {
        let full_path = root.join(path);
        // TODO: is it even worth to clone here? Feels like the concurrency gains might not be
        // worth it in general... Should benchmark eventually
        let contents = ctx
            .db
            .objects
            .with(&object, |obj| Vec::from(obj.expect("missing object")));
        js.spawn(async move {
            tokio::fs::create_dir_all(full_path.parent().unwrap()).await?;
            tokio::fs::write(full_path, contents).await?;
            Ok(())
        });
    }
    js.join_all()
        .await
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    Ok(())
}
