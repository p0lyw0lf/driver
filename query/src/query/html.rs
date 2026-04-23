use serde::{Deserialize, Serialize};

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

        thread_local! {
            static OPTIONS: comrak::Options<'static> = comrak::Options {
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
                    .block_directive(true)
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
            };
            static HIGHLIGHTER: comrak::plugins::syntect::SyntectAdapter = comrak::plugins::syntect::SyntectAdapterBuilder::new()
                .css()
                .build();

        }

        /// Very silly things I have to do to get thread locals to reference properly...
        struct LocalHighlighter(
            &'static std::thread::LocalKey<comrak::plugins::syntect::SyntectAdapter>,
        );
        impl comrak::adapters::SyntaxHighlighterAdapter for LocalHighlighter {
            fn write_highlighted(
                &self,
                output: &mut dyn std::fmt::Write,
                lang: Option<&str>,
                code: &str,
            ) -> std::fmt::Result {
                self.0
                    .with(|inner| inner.write_highlighted(output, lang, code))
            }

            fn write_pre_tag(
                &self,
                output: &mut dyn std::fmt::Write,
                attributes: std::collections::HashMap<&'static str, std::borrow::Cow<'_, str>>,
            ) -> std::fmt::Result {
                self.0.with(|inner| inner.write_pre_tag(output, attributes))
            }

            fn write_code_tag(
                &self,
                output: &mut dyn std::fmt::Write,
                attributes: std::collections::HashMap<&'static str, std::borrow::Cow<'_, str>>,
            ) -> std::fmt::Result {
                self.0
                    .with(|inner| inner.write_code_tag(output, attributes))
            }
        }

        thread_local! {
            static PLUGINS: comrak::options::Plugins<'static> = comrak::options::Plugins::builder()
                .render(comrak::options::RenderPlugins {
                    codefence_renderers: Default::default(),
                    codefence_syntax_highlighter: Some(
                        &LocalHighlighter(&HIGHLIGHTER),
                    ),
                    heading_adapter: None,
                })
                .build();
        }

        let output = OPTIONS.with(|options| {
            PLUGINS
                .with(|plugins| comrak::markdown_to_html_with_plugins(&contents, options, plugins))
        });

        let object = ctx.db().objects.store(output.into_bytes())?;
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
        let object = ctx.db().objects.store(output)?;
        Ok(object)
    }
}
