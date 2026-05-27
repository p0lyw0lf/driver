query_key!(MinifyHtml(pub Object););

impl Producer for MinifyHtml {
    type Output = crate::Result<Object>;

    #[tracing::instrument(level = "debug", skip_all)]
    async fn produce(&self, ctx: &QueryContext) -> Self::Output {
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
        let object = ctx.db().objects.store(output)?;
        Ok(object)
    }
}

/// Very silly things I have to do to get thread locals to reference properly...
struct ArboriumHighlighter(&'static std::thread::LocalKey<arborium::Highlighter>);
impl comrak::adapters::SyntaxHighlighterAdapter for ArboriumHighlighter {
    fn write_highlighted(
        &self,
        output: &mut dyn std::fmt::Write,
        lang: Option<&str>,
        code: &str,
    ) -> std::fmt::Result {
        match lang {
            None => comrak::html::escape(output, code),
            Some(lang) => {
                if lang.is_empty() {
                    comrak::html::escape(output, code)
                } else {
                    let mut highlighter = self.0.with(|h| h.fork());
                    match highlighter.highlight(lang, code).map_err(|e| {
                        eprintln!("error highlighting code: {e}");
                        std::fmt::Error
                    }) {
                        Ok(html) => output.write_str(&html),
                        Err(_) => comrak::html::escape(output, code),
                    }
                }
            }
        }
    }

    fn write_pre_tag(
        &self,
        output: &mut dyn std::fmt::Write,
        _attributes: std::collections::HashMap<&'static str, std::borrow::Cow<'_, str>>,
    ) -> std::fmt::Result {
        comrak::html::write_opening_tag(output, "pre", vec![("class", "syntax-highlighting")])
    }

    fn write_code_tag(
        &self,
        output: &mut dyn std::fmt::Write,
        attributes: std::collections::HashMap<&'static str, std::borrow::Cow<'_, str>>,
    ) -> std::fmt::Result {
        comrak::html::write_opening_tag(output, "code", attributes)
    }
}
