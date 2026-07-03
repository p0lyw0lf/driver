use std::path::PathBuf;

use driver_engine::Blob;

driver_engine::key!(
    #[input=|_| true]
    struct ReadFile(pub PathBuf);
);
driver_engine::no_blobs!(ReadFile);

driver_engine::producer!(ReadFile(self, ctx) -> driver_util::Result<Blob> {
    let content = async_fs::read(&self.0).await?;
    let blob = ctx.store(content)?;
    Ok(blob)
});

impl std::fmt::Display for ReadFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "read_file(\"{}\")", self.0.display())
    }
}
