use serde::Deserialize;
use serde::Serialize;

use crate::{
    engine::{Producer, QueryContext, db::Object},
    query_key,
};

query_key!(MarkdownToHtml(pub Object););

impl Producer for MarkdownToHtml {
    type Output = crate::Result<Object>;

    #[tracing::instrument(level = "debug", skip_all)]
    async fn produce(&self, ctx: &QueryContext) -> Self::Output {
        let contents = self.0.contents_as_string(ctx)?;

        let output = comrak::markdown_to_html_with_plugins(
            &contents,
            &comrak::Options {
                extension: comrak::options::Extension::builder()
                    .strikethrough(true)
                    .table(true)
                    .autolink(false)
                    .tasklist(true)
                    .superscript(false)
                    .subscript(false)
                    .footnotes(true)
                    .math_dollars(true)
                    .shortcodes(false)
                    .underline(false)
                    .spoiler(true)
                    .subtext(true)
                    .highlight(true)
                    .build(),
                parse: comrak::options::Parse::builder()
                    .smart(false)
                    .tasklist_in_table(true)
                    .ignore_setext(true)
                    .build(),
                render: comrak::options::Render::builder()
                    .hardbreaks(false)
                    .r#unsafe(true)
                    .escape(false)
                    .tasklist_classes(true)
                    .build(),
            },
            &comrak::options::Plugins::builder()
                .render(comrak::options::RenderPlugins {
                    codefence_syntax_highlighter: Some(
                        &comrak::plugins::syntect::SyntectAdapterBuilder::new().build(),
                    ),
                    heading_adapter: None,
                })
                .build(),
        );

        let object = ctx.db().objects.store(output.into_bytes());
        Ok(object)
    }
}

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
        let object = ctx.db().objects.store(output);
        Ok(object)
    }
}
