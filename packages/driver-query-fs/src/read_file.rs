use std::path::PathBuf;

use driver_db::Object;

driver_engine::key!(
    #[input=|_| true]
    struct ReadFile(pub PathBuf);
);

driver_engine::producer!(ReadFile(self, ctx) -> driver_util::Result<Object> {
    let content = async_fs::read(&self.0).await?;
    let object = ctx.store(content)?;
    Ok(object)
});

impl std::fmt::Display for ReadFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "read_file(\"{}\")", self.0.display())
    }
}
