use driver_engine::{Blob, Uri};

driver_engine::key!(
    #[input=|_| true]
    struct GetUrl(pub Uri);
);
driver_engine::no_blobs!(GetUrl);

driver_engine::producer!(GetUrl(self, ctx) -> driver_util::Result<Blob> {
    ctx.fetch(self.0.clone()).await
});

impl std::fmt::Display for GetUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "get_url(\"{}\")", self.0)
    }
}
