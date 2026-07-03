use driver_engine::Blob;

driver_engine::key!(
    #[input=|_| false]
    struct MinifyHtml(pub Blob);
);
driver_engine::blob_trace!(MinifyHtml => (0));

driver_engine::producer!(MinifyHtml(self, ctx) -> driver_util::Result<Blob> {
    let contents = ctx.load_string(&self.0)?;
    let cfg = minify_html::Cfg {
        keep_closing_tags: true,
        keep_comments: true,
        keep_html_and_head_opening_tags: true,
        minify_css: true,
        minify_js: true,
        ..Default::default()
    };
    let output = minify_html::minify(contents.as_bytes(), &cfg);
    let blob = ctx.store(output)?;
    Ok(blob)
});

impl std::fmt::Display for MinifyHtml {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "minify_html({})", self.0)
    }
}
