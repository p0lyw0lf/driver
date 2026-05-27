query_key!(MarkdownToHtml(pub Object););

struct Options {
    comrak_options: comrak::Options<'static>,
    comrak_plugins: comrak::options::Plugins<'static>,
    katex_ctx: katex::KatexContext,
    katex_settings: katex::Settings,
}

impl Default for Options {
    fn default() -> Self {
        thread_local! {
            static HIGHLIGHTER: arborium::Highlighter = arborium::Highlighter::with_config(arborium::Config {
                html_format: arborium::HtmlFormat::ClassNames,
                ..Default::default()
            });
        }

        Self {
            comrak_options: comrak::Options {
                extension: comrak::options::Extension::builder()
                    .strikethrough(true)
                    .table(true)
                    .autolink(false)
                    .tasklist(true)
                    .header_id_prefix("heading-".to_string())
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
            },
            comrak_plugins: comrak::options::Plugins::builder()
                .render(comrak::options::RenderPlugins {
                    codefence_renderers: Default::default(),
                    codefence_syntax_highlighter: Some(&ArboriumHighlighter(&HIGHLIGHTER)),
                    heading_adapter: None,
                })
                .build(),
            katex_ctx: katex::KatexContext::default(),
            katex_settings: katex::Settings::default(),
        }
    }
}

impl Producer for MarkdownToHtml {
    type Output = crate::Result<Object>;

    #[tracing::instrument(level = "debug", skip_all)]
    async fn produce(&self, ctx: &QueryContext) -> Self::Output {
        let contents = self.0.contents_as_string(ctx)?;

        thread_local! {
            static OPTIONS: Options = Options::default();
        }

        let output = OPTIONS.with(|options| -> crate::Result<_> {
            let arena = comrak::Arena::new();
            let root = comrak::parse_document(&arena, &contents, &options.comrak_options);

            for node in root.descendants() {
                let node_value = &mut node.data_mut().value;
                if let comrak::nodes::NodeValue::Math(node_math) = node_value {
                    *node_value =
                        comrak::nodes::NodeValue::HtmlBlock(comrak::nodes::NodeHtmlBlock {
                            // TODO: I have no idea what this is supposed to be, hope it doesn't
                            // matter lol
                            block_type: 0,
                            literal: katex::render_to_string(
                                &options.katex_ctx,
                                &node_math.literal,
                                &options.katex_settings,
                            )?,
                        });
                }
            }

            let mut out = String::new();
            comrak::html::format_document_with_plugins(
                root,
                &options.comrak_options,
                &mut out,
                &options.comrak_plugins,
            )?;

            // Doing this here instead of in javascript for a slight bit of extra perf :)
            out = out
                .replace("<table>", "<div class=table-wrapper><table>")
                .replace("</table>", "</table></div>");

            Ok(out)
        })?;

        let object = ctx.db().objects.store(output.into_bytes())?;
        Ok(object)
    }
}
