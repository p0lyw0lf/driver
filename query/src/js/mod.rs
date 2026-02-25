use std::{cell::RefCell, path::PathBuf, sync::Arc};

use rquickjs::{
    Ctx, FromJs, IntoJs, Value,
    loader::{BuiltinResolver, FileResolver, ModuleLoader},
};
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;

use crate::{
    db::object::Object,
    js::store_object::StoreObject,
    js::value::RustValue,
    query::{
        context::{Producer, QueryContext},
        files::ReadFile,
    },
    query_key,
    to_hash::ToHash,
};

mod store_object;
mod value;

struct ContextFrame {
    curr: *const QueryContext,
    output_queue: Vec<WriteOutput>,
}

tokio::task_local! {
    static QUERY_CONTEXT: RefCell<ContextFrame>;
}

/// Runs a closure with a QueryContext pushed onto the stack. All calls to `get_context()` that run
/// as a result of that closure will access this ctx object. Therefore, all pointer accesses from
/// `get_context()` have safety ensured as a result of running in this function.
async fn with_query_context<T, F: Future<Output = crate::Result<T>>>(
    ctx: &QueryContext,
    f: impl FnOnce() -> F,
) -> crate::Result<(T, Vec<WriteOutput>)> {
    let curr = ctx as *const _;
    let new_frame = ContextFrame {
        curr,
        output_queue: vec![],
    };
    let fut = QUERY_CONTEXT.scope(RefCell::new(new_frame), async { f().await });
    tokio::pin!(fut);

    let out = (&mut fut).await?;
    let popped_frame = fut
        .take_value()
        .expect("no context frame to pop")
        .into_inner();
    Ok((out, popped_frame.output_queue))
}

fn error_message(
    message: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>,
) -> rquickjs::Error {
    rquickjs::Error::Io(std::io::Error::other(message))
}

/// Only safe to dereference the returned pointer if running inside a call to `with_context()`.
fn get_context() -> rquickjs::Result<*const QueryContext> {
    QUERY_CONTEXT
        .try_with(|ctx| ctx.borrow().curr)
        .map_err(error_message)
}

// SHOULD be called from a Javascript callback
fn get_current_file(js_ctx: Ctx<'_>) -> rquickjs::Result<PathBuf> {
    Ok(PathBuf::from(
        js_ctx
            .script_or_module_name(0)
            .ok_or(error_message("not running in a module"))?
            .to_string()?,
    ))
}

/// SAFETY: only safe to call when running inside `with_query_context()`
async unsafe fn run_task(file: PathBuf, args: Option<RustValue>) -> rquickjs::Result<RustValue> {
    let ctx = unsafe { &*get_context()? };
    let task = RunFile {
        file: file.clone(),
        args,
    };
    let FileOutput { value, outputs } = task.query(ctx).await.map_err(|e| {
        rquickjs::Error::new_loading_message(file.display().to_string(), e.to_string())
    })?;

    // Limit the amount of time we borrow QUERY_CONTEXT so that the RunFile can re-borrow during.
    // SAFETY: by precondition
    unsafe { push_outputs(outputs) }?;

    Ok(value)
}

/// SAFETY: only safe to call when running inside `with_query_context()`
unsafe fn push_outputs(outputs: impl IntoIterator<Item = WriteOutput>) -> rquickjs::Result<()> {
    QUERY_CONTEXT.with(|ctx| -> rquickjs::Result<_> {
        ctx.try_borrow_mut()
            .map_err(error_message)?
            .output_queue
            .extend(outputs);
        Ok(())
    })
}

#[rquickjs::module]
mod driver {
    use std::path::{Component, PathBuf};

    use either::Either;
    use relative_path::RelativePath;
    use relative_path::RelativePathBuf;
    use rquickjs::Ctx;
    use rquickjs::prelude::Promised;

    use super::WriteOutput;
    use super::error_message;
    use super::get_context;

    use crate::js::MarkdownToHtml;
    use crate::js::MinifyHtml;
    use crate::js::{get_current_file, store_object::StoreObject, value::RustValue};
    use crate::{
        query::context::Producer,
        query::files::{ListDirectory, ReadFile},
    };

    /// Helper function that formats a path relative to the current file
    /// NOTE: this is only safe to call **BEFORE** the promise starts to execute. This is why we
    /// have to do such crazy type signature things below.
    fn to_relative_path(js_ctx: Ctx<'_>, path: String) -> rquickjs::Result<PathBuf> {
        let base_file = get_current_file(js_ctx)?;
        let base_directory = base_file
            .parent()
            .ok_or(error_message("no parent directory?"))?;
        Ok(RelativePathBuf::from_path(base_directory)
            .map_err(|e| {
                rquickjs::Error::new_from_js_message("String", "RelativePath", e.to_string())
            })?
            .join_normalized(RelativePath::new(&path))
            .to_path("."))
    }

    #[rquickjs::function]
    #[tracing::instrument(level = "trace", skip(js_ctx))]
    pub fn read_file(
        js_ctx: Ctx<'_>,
        filename: String,
    ) -> rquickjs::Result<Promised<impl Future<Output = rquickjs::Result<StoreObject>>>> {
        // SAFETY: the only way these javascript functions get called is from inside a
        // `with_query_context()`
        let ctx = unsafe { &*get_context()? };
        let path = to_relative_path(js_ctx, filename)?;

        Ok(Promised(async move {
            let object = ReadFile(path)
                .query(ctx)
                .await
                .map_err(|e| error_message(format!("read_file: {e}")))?;
            Ok(StoreObject { object })
        }))
    }

    #[rquickjs::function]
    #[tracing::instrument(level = "trace", skip(js_ctx))]
    pub fn list_directory(
        js_ctx: Ctx<'_>,
        dirname: String,
    ) -> rquickjs::Result<Promised<impl Future<Output = rquickjs::Result<Vec<String>>>>> {
        // SAFETY: the only way these javascript functions get called is from inside a
        // `with_query_context()`
        let ctx = unsafe { &*get_context()? };
        let dirname = to_relative_path(js_ctx, dirname)?;
        Ok(Promised(async move {
            let contents = ListDirectory(dirname)
                .query(ctx)
                .await
                .map_err(|e| error_message(format!("list_directory: {e}")))?
                .into_iter()
                .map(|entry| entry.display().to_string())
                .collect();
            Ok(contents)
        }))
    }

    #[rquickjs::function]
    #[tracing::instrument(level = "trace", skip(js_ctx))]
    pub fn run_task(
        js_ctx: Ctx<'_>,
        filename: String,
        args: Option<RustValue>,
    ) -> rquickjs::Result<Promised<impl Future<Output = rquickjs::Result<RustValue>>>> {
        let file = to_relative_path(js_ctx, filename)?;
        Ok(Promised(async move {
            // SAFETY: the only way these javascript functions get called is from inside a
            // `with_query_context()`
            unsafe { super::run_task(file, args) }.await
        }))
    }

    #[rquickjs::function]
    #[tracing::instrument(level = "trace")]
    pub fn file_type(entry_name: String) -> rquickjs::Result<String> {
        let metadata = std::fs::metadata(PathBuf::from(entry_name))?;

        Ok(if metadata.is_file() {
            "file"
        } else if metadata.is_dir() {
            "dir"
        } else if metadata.is_symlink() {
            "symlink"
        } else {
            "unknown"
        }
        .to_string())
    }

    #[rquickjs::function]
    #[tracing::instrument(level = "trace", skip(value))]
    pub fn store<'js>(
        value: Either<String, rquickjs::TypedArray<'js, u8>>,
    ) -> rquickjs::Result<StoreObject> {
        // SAFETY: we are in a javascript context
        let ctx = unsafe { &*get_context()? };
        let contents = match value {
            Either::Left(s) => s.into_bytes(),
            Either::Right(arr) => Vec::from(AsRef::<[u8]>::as_ref(&arr)),
        };
        let object = ctx.db.objects.store(contents);
        Ok(StoreObject { object })
    }

    #[rquickjs::function]
    #[tracing::instrument(level = "trace")]
    pub async fn markdown_to_html(contents: StoreObject) -> rquickjs::Result<StoreObject> {
        // SAFETY: we are in a javascript context
        let ctx = unsafe { &*get_context()? };
        let object = MarkdownToHtml(contents.object)
            .query(ctx)
            .await
            .map_err(|e| error_message(format!("markdown_to_html: {e}")))?;
        Ok(StoreObject { object })
    }

    #[rquickjs::function]
    #[tracing::instrument(level = "trace")]
    pub async fn minify_html(contents: StoreObject) -> rquickjs::Result<StoreObject> {
        // SAFETY: we are in a javascript context
        let ctx = unsafe { &*get_context()? };
        let object = MinifyHtml(contents.object)
            .query(ctx)
            .await
            .map_err(|e| error_message(format!("minify_html: {e}")))?;
        Ok(StoreObject { object })
    }

    #[rquickjs::function]
    #[tracing::instrument(level = "trace")]
    pub fn write_output(name: String, contents: StoreObject) -> rquickjs::Result<()> {
        let path = PathBuf::from(name);
        if !path
            .components()
            .all(|component| matches!(component, Component::CurDir | Component::Normal(_)))
        {
            // Don't allow any path traversal outside the output directory
            return Err(error_message(format!(
                "directory traversal {}",
                path.display()
            )));
        }
        unsafe {
            super::push_outputs([WriteOutput {
                path,
                // SAFETY: being provided a StoreObject always means we've put it in the store
                // already
                object: contents.object,
            }])?
        };
        Ok(())
    }
}

query_key!(MarkdownToHtml(pub Object););

impl Producer for MarkdownToHtml {
    type Output = crate::Result<Object>;

    async fn produce(&self, ctx: &QueryContext) -> Self::Output {
        let contents = self.0.contents_as_string(ctx)?;

        let output = ctx
            .rt
            .spawn_blocking(move || {
                comrak::markdown_to_html_with_plugins(
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
                )
            })
            .await?;

        let object = ctx.db.objects.store(output.into_bytes());
        Ok(object)
    }
}

query_key!(MinifyHtml(pub Object););

impl Producer for MinifyHtml {
    type Output = crate::Result<Object>;

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
        let output = ctx
            .rt
            .spawn_blocking(move || minify_html::minify(contents.as_bytes(), &cfg))
            .await?;
        let object = ctx.db.objects.store(output);
        Ok(object)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteOutput {
    pub path: PathBuf,
    pub object: Object,
}

impl ToHash for WriteOutput {
    fn run_hash(&self, hasher: &mut sha2::Sha256) {
        hasher.update(b"WriteOutput(");
        self.path.run_hash(hasher);
        hasher.update("b)");
        self.object.run_hash(hasher);
    }
}

/// Custom loader that will track dependencies via ReadFile
struct MemoizedScriptLoader {
    rt: Arc<tokio::runtime::Runtime>,
    extensions: Vec<String>,
}

impl MemoizedScriptLoader {
    fn new(rt: Arc<tokio::runtime::Runtime>) -> Self {
        Self {
            rt,
            extensions: vec!["js".into()],
        }
    }
}

impl rquickjs::loader::Loader for MemoizedScriptLoader {
    #[tracing::instrument(level = "trace", skip(self, js_ctx))]
    fn load<'js>(
        &mut self,
        js_ctx: &rquickjs::Ctx<'js>,
        name: &str,
    ) -> rquickjs::Result<rquickjs::Module<'js>> {
        let path = PathBuf::from(name);
        if !path
            .extension()
            .map(|extension| {
                self.extensions
                    .iter()
                    .any(|known_extension| Some(known_extension.as_str()) == extension.to_str())
            })
            .unwrap_or(false)
        {
            return Err(rquickjs::Error::new_loading(name));
        }

        let ctx = unsafe { &*get_context()? };
        let object = self
            .rt
            .block_on(ReadFile(path).query(ctx))
            .map_err(|err| rquickjs::Error::new_loading_message(name, format!("{err}")))?;
        // Need to clone the source so we don't hang onto it for too long when reading from it in
        // the module; the module will clone it into a Vec anyways so no harm in doing that now.
        let source = Vec::<u8>::from(
            ctx.db
                .objects
                .get(&object)
                .expect("missing object")
                .as_ref(),
        );

        rquickjs::Module::declare(js_ctx.clone(), name, source)
    }
}

static RUNTIME: tokio::sync::OnceCell<rquickjs::AsyncRuntime> = tokio::sync::OnceCell::const_new();
static CONTEXT: tokio::sync::OnceCell<rquickjs::AsyncContext> = tokio::sync::OnceCell::const_new();

tokio::task_local! {
    // The current Ctx object that we are executing in, if any. Use with_js_ctx() to re-entrantly
    // access this.
    static CTX: std::ptr::NonNull<rquickjs::qjs::JSContext>;
}

async fn with_js_ctx<T, F, C>(rt: Arc<tokio::runtime::Runtime>, callback: C) -> T
where
    T: rquickjs::markers::ParallelSend + 'static,
    F: Future<Output = T>,
    C: (for<'js> FnOnce(rquickjs::Ctx<'js>) -> F) + rquickjs::markers::ParallelSend,
{
    match CTX.try_get() {
        Ok(ctx) => {
            // SAFETY: there is some larger `context.with` that we are borrowing from
            let ctx = unsafe { rquickjs::Ctx::from_raw(ctx) };
            callback(ctx).await
        }
        Err(_) => {
            let runtime = RUNTIME
                .get_or_init(async || {
                    let resolver = (
                        BuiltinResolver::default().with_module("driver"),
                        FileResolver::default(),
                    );
                    let loader = (
                        ModuleLoader::default().with_module("driver", js_driver),
                        MemoizedScriptLoader::new(rt),
                    );

                    let runtime = rquickjs::AsyncRuntime::new().expect("not enough memory?");
                    runtime.set_loader(resolver, loader).await;
                    runtime
                })
                .await;

            let context = CONTEXT
                .get_or_init(async || {
                    // TODO: it seems rquickjs isn't happy if it doesn't have the full set of features. Not
                    // that there's any IO features in here besides those we allow it, but ah well
                    rquickjs::AsyncContext::full(runtime)
                        .await
                        .expect("context failed to build")
                })
                .await;

            rquickjs::async_with!(context => |ctx| {
                CTX.scope(ctx.as_raw(), callback(ctx)).await
            })
            .await
        }
    }
}

query_key!(RunFile {
    pub file: PathBuf,
    pub args: Option<RustValue>,
});

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileOutput {
    value: RustValue,
    pub outputs: Vec<WriteOutput>,
}

impl ToHash for FileOutput {
    fn run_hash(&self, hasher: &mut sha2::Sha256) {
        hasher.update(b"FileOutput(");
        self.value.run_hash(hasher);
        hasher.update(b")(");
        self.outputs.run_hash(hasher);
        hasher.update(b")");
    }
}

impl Producer for RunFile {
    type Output = crate::Result<FileOutput>;

    #[tracing::instrument(level = "trace", skip(ctx))]
    async fn produce(&self, ctx: &QueryContext) -> Self::Output {
        let name = self.file.display().to_string();
        println!(
            "running {}({})",
            name,
            self.args
                .as_ref()
                .map(|args| args.to_string())
                .unwrap_or_default()
        );
        let object = ReadFile(self.file.clone()).query(ctx).await?;
        let contents = {
            // Need to shorten the lifetime of our read from the database so that we don't deadlock
            // trying to read from the map multiple times
            let contents = ctx.db.objects.get(&object).expect("missing object");
            String::from_utf8(contents.as_ref().to_vec())?
        };

        let (value, outputs) = with_query_context(ctx, async || {
            with_js_ctx(ctx.rt.clone(), |ctx| {
                let name = name.clone();
                // SAFETY: lifetimes work out trust me bro
                let ctx = unsafe { rquickjs::Ctx::from_raw(ctx.as_raw()) };
                async move {
                    let globals = ctx.globals();
                    globals
                        .set(
                            "print",
                            rquickjs::Function::new(ctx.clone(), |msg: String| println!("{msg}"))
                                .unwrap()
                                .with_name("print")
                                .unwrap(),
                        )
                        .unwrap();

                    globals
                        .set(
                            "ARGS",
                            match &self.args {
                                Some(args) => args.clone().into_js(&ctx).unwrap(),
                                None => Value::new_undefined(ctx.clone()),
                            },
                        )
                        .unwrap();

                    let catch = |err: rquickjs::Error| -> crate::Error {
                        match err {
                            rquickjs::Error::Exception => {
                                let value = ctx.catch();
                                if let Some(err) = value.as_exception() {
                                    let message = err.message().unwrap_or_default();
                                    let stack = err.stack().unwrap_or_default();
                                    eprintln!("js exception: {message}");
                                    eprintln!("{stack}");
                                } else if let Ok(value) = RustValue::from_js(&ctx, value.clone()) {
                                    eprintln!("js thrown value: {}", value);
                                } else {
                                    eprintln!("js error: {:?}", value);
                                }
                                crate::Error::from(rquickjs::Error::Exception)
                            }
                            otherwise => crate::Error::from(otherwise),
                        }
                    };
                    let module =
                        rquickjs::Module::declare(ctx.clone(), name, contents).map_err(catch)?;
                    let (module, promise) = module.eval().map_err(catch)?;
                    promise.into_future::<()>().await.map_err(catch)?;

                    let value: RustValue = module.get(rquickjs::atom::PredefinedAtom::Default)?;
                    Ok(value)
                }
            })
            .await
        })
        .await
        .map_err(|e| {
            crate::Error::new(&format!(
                "error running {}({}):\n\t{}",
                name,
                self.args
                    .as_ref()
                    .map(|args| args.to_string())
                    .unwrap_or_default(),
                e
            ))
        })?;

        Ok(FileOutput { value, outputs })
    }
}
