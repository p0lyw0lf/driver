query_key!(ReadFile(pub PathBuf););

impl Producer for ReadFile {
    type Output = crate::Result<Object>;
    #[tracing::instrument(level = "debug", skip_all)]
    async fn produce(&self, ctx: &QueryContext) -> Self::Output {
        let content = async_fs::read(&self.0).await?;
        let object = ctx.db().objects.store(content)?;
        Ok(object)
    }
}
