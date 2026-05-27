use driver_db::Object;

driver_engine::key!(
    #[input=|_| false]
    struct MinifyHtml(pub Object);
);

driver_engine::producer!(MinifyHtml(self, ctx) -> driver_util::Result<Object> {
    let contents = self.0.contents_as_string(ctx)?;
    let cfg = minify_html::Cfg {
        keep_closing_tags: true,
        keep_comments: true,
        keep_html_and_head_opening_tags: true,
        minify_css: true,
        minify_js: true,
        ..Default::default()
    };
    let output = minify_html::minify(contents.as_bytes(), &cfg);
    let object = ctx.store(output)?;
    Ok(object)
});

impl std::fmt::Display for MinifyHtml {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "minify_html({})", self.0)
    }
}
