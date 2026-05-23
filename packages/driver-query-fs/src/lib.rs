mod list_directory;
mod read_file;
pub use list_directory::ListDirectory;
pub use read_file::ReadFile;

pub struct Output {
    prev: Option<WriteOutputs>,
    curr: WriteOutputs,
}

pub async fn run<'a>(
    rt: Arc<Executor>,
    file: PathBuf,
    args: impl IntoIterator<Item = &'a str>,
) -> crate::Result<Output> {
    let key = RunFile {
        file,
        arg: query::js::parse_args(args),
    };
    // SAFETY: we are the one place this function is allowed to be called.
    let prev = match unsafe { rt.db.get_value(key.clone()).await } {
        None => None,
        Some(Ok(v)) => Some(v.outputs),
        Some(Err(e)) => return Err(e),
    };

    let output = rt
        .query(key.into(), None)
        .await
        .downcast::<<RunFile as Producer>::Output>()
        .expect("invalid type");
    let output = (*output)?;
    Ok(Output {
        prev,
        curr: output.outputs,
    })
}

#[derive(Default)]
pub struct WriteOptions {
    /// If this is specified, we only write new files, never delete old ones.
    pub no_delete_missing: bool,
}

impl Output {
    pub async fn write(self, rt: &Executor, options: &WriteOptions) -> crate::Result<()> {
        let root = &rt.options.output_path;
        match self.prev {
            None => {
                // Ignore errors removing directory; it's just a safety measure
                async_fs::remove_dir_all(root).await.unwrap_or_default();
                write(rt, root, self.curr.iter()).await
            }
            Some(prev) => {
                let ((), ()) = (
                    write(
                        rt,
                        root,
                        self.curr.iter().filter(|(path, object)| {
                            prev.get(*path)
                                .is_none_or(|prev_object| &prev_object != object)
                        }),
                    ),
                    remove(
                        root,
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
    rt: &Executor,
    root: &Path,
    iter: impl Iterator<Item = (&PathBuf, &Object)>,
) -> crate::Result<()> {
    let mut futs = Vec::new();
    for (path, object) in iter {
        let full_path = root.join(path);
        futs.push(async move {
            async_fs::create_dir_all(full_path.parent().unwrap()).await?;
            rt.db.objects.copy(object, &full_path).await?;
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
