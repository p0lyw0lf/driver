use std::path::{Path, PathBuf};

use driver_engine::{Object, query};
use driver_query_ssg::boa::{RunJs, WriteOutputs, parse_args};
use driver_query_ssg::{QueryContext, QueryOutput};
use futures_concurrency::future::TryJoin as _;

pub struct RunOutput {
    prev: Option<WriteOutputs>,
    curr: WriteOutputs,
}

pub async fn run<'a>(
    root: &QueryContext,
    file: PathBuf,
    args: impl IntoIterator<Item = &'a str>,
) -> driver_util::Result<RunOutput> {
    let key = RunJs {
        file,
        arg: parse_args(args),
    };
    // SAFETY: we are the one place this function is allowed to be called.
    let prev = match root.db().get_value(&key.clone().into()) {
        None => None,
        Some(QueryOutput::RunJs(Ok(v))) => Some(v.writes),
        Some(QueryOutput::RunJs(Err(e))) => return Err(e),
        Some(other) => {
            return Err(driver_util::Error::new(&format!(
                "expected RunJs, got {other:?}"
            )));
        }
    };

    let output = query(root, key).await?;
    Ok(RunOutput {
        prev,
        curr: output.writes,
    })
}

#[derive(Default)]
pub struct WriteOptions {
    /// The directory we are going to write to.
    pub output_path: PathBuf,
    /// If this is specified, we only write new files, never delete old ones.
    pub no_delete_missing: bool,
}

impl RunOutput {
    pub async fn write(
        self,
        root: &QueryContext,
        options: &WriteOptions,
    ) -> driver_util::Result<()> {
        let base = &options.output_path;
        match self.prev {
            None => {
                // Ignore errors removing directory; it's just a safety measure
                std::fs::remove_dir_all(base).unwrap_or_default();
                write(root, base, self.curr.iter()).await
            }
            Some(prev) => {
                let ((), ()) = (
                    write(
                        root,
                        base,
                        self.curr.iter().filter(|(path, object)| {
                            prev.get(*path)
                                .is_none_or(|prev_object| &prev_object != object)
                        }),
                    ),
                    remove(
                        base,
                        prev.iter().filter_map(|(path, _)| {
                            if options.no_delete_missing || self.curr.contains_key(path) {
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
    root: &QueryContext,
    base: &Path,
    iter: impl Iterator<Item = (&PathBuf, &Object)>,
) -> driver_util::Result<()> {
    let mut futs = Vec::new();
    for (path, object) in iter {
        let full_path = base.join(path);
        futs.push(async move {
            std::fs::create_dir_all(full_path.parent().unwrap())?;
            root.db().objects.copy(root.options(), object, &full_path)?;
            driver_util::Result::Ok(())
        });
    }
    let _ = futs.try_join().await?;
    Ok(())
}

async fn remove(base: &Path, iter: impl Iterator<Item = &PathBuf>) -> driver_util::Result<()> {
    let mut futs = Vec::new();
    for path in iter {
        let full_path = base.join(path);
        // TODO: should we be removing empty directories too? How?
        futs.push(async move {
            std::fs::remove_file(&full_path)?;
            driver_util::Result::Ok(())
        });
    }
    let _ = futs.try_join().await?;
    Ok(())
}
