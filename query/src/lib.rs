use std::path::{Path, PathBuf};

mod engine;
mod error;
mod options;
mod query;
mod serde;
mod to_hash;

use crate::engine::db::Object;
use crate::engine::{Producer, Queryable};
use crate::query::{RunFile, js::WriteOutputs};

pub use engine::QueryContext;
pub use error::Error;
use futures_concurrency::future::TryJoin;

pub type Result<T> = std::result::Result<T, Error>;

pub struct Output {
    prev: Option<WriteOutputs>,
    curr: WriteOutputs,
}

pub async fn run(file: PathBuf, ctx: &QueryContext) -> crate::Result<Output> {
    let key = RunFile { file, arg: None };
    // SAFETY: we are the one place this function is allowed to be called.
    let prev = match unsafe { ctx.db().get_value(key.clone()).await } {
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
        let root = &ctx.options().output_path;
        match self.prev {
            None => {
                // Ignore errors removing directory; it's just a safety measure
                async_fs::remove_dir_all(root).await.unwrap_or_default();
                write(ctx, root, self.curr.iter()).await
            }
            Some(prev) => {
                let ((), ()) = (
                    write(
                        ctx,
                        root,
                        self.curr.iter().filter(|(path, object)| {
                            prev.get(*path)
                                .is_none_or(|prev_object| &prev_object != object)
                        }),
                    ),
                    remove(
                        root,
                        prev.iter().filter_map(|(path, _)| {
                            if self.curr.contains_key(path) {
                                None
                            } else {
                                Some(path)
                            }
                        }),
                    ),
                )
                    .try_join()
                    .await?;
                Ok(())
            }
        }
    }
}

async fn write(
    ctx: &QueryContext,
    root: &Path,
    iter: impl Iterator<Item = (&PathBuf, &Object)>,
) -> crate::Result<()> {
    let mut futs = Vec::new();
    for (path, object) in iter {
        let full_path = root.join(path);
        // TODO: is it even worth to clone here? Feels like the concurrency gains might not be
        // worth it in general... Should benchmark eventually
        let contents = ctx
            .db()
            .objects
            .with(object, |obj| Vec::from(obj.expect("missing object")));
        // TODO: should we run these on separate threads instead of just concurrently?
        futs.push(async move {
            async_fs::create_dir_all(full_path.parent().unwrap()).await?;
            async_fs::write(full_path, contents).await?;
            crate::Result::Ok(())
        });
    }
    let _ = futs.try_join().await?;
    Ok(())
}

async fn remove(root: &Path, iter: impl Iterator<Item = &PathBuf>) -> crate::Result<()> {
    let mut futs = Vec::new();
    for path in iter {
        let full_path = root.join(path);
        // TODO: should we be removing empty directories too? How?
        futs.push(async move {
            async_fs::remove_file(&full_path).await?;
            crate::Result::Ok(())
        });
    }
    let _ = futs.try_join().await?;
    Ok(())
}
