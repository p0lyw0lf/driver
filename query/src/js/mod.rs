use std::{path::PathBuf, sync::Arc};

use rquickjs::{
    Ctx, FromJs, Function, JsLifetime,
    loader::{BuiltinResolver, FileResolver, ModuleLoader},
};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use tokio::sync::Mutex;
use tracing::trace;

use crate::{
    db::object::Object,
    query::{
        context::{Producer, QueryContext},
        files::ReadFile,
    },
    query_key,
    to_hash::ToHash,
};

mod image;
mod object;
mod path;
mod value;

#[cfg(test)]
pub use self::{object::JsObject, value::JsValue};

#[cfg(not(test))]
use self::{object::JsObject, value::JsValue};

#[derive(JsLifetime)]
struct ContextFrame {
    ctx: QueryContext,
    arg: JsValue,
    output_queue: Mutex<Vec<WriteOutput>>,
}

/// Runs a closure with a QueryContext pushed onto the stack. All calls to `get_context()` that run
/// as a result of that closure will access this ctx object. Therefore, all pointer accesses from
/// `get_context()` have safety ensured as a result of running in this function.
async fn with_query_context<'js, T>(
    js_ctx: &Ctx<'js>,
    ctx: QueryContext,
    arg: JsValue,
    f: impl AsyncFnOnce() -> crate::Result<T>,
) -> crate::Result<(T, Vec<WriteOutput>)> {
    let new_frame = ContextFrame {
        ctx,
        arg,
        output_queue: Mutex::new(vec![]),
    };

    let prev = js_ctx
        .store_userdata(new_frame)
        .expect("could not insert userdata");
    let out = f().await;
    let popped_frame: Box<ContextFrame> = match prev {
        Some(prev) => js_ctx
            .store_userdata(*prev)
            .expect("could not swap in old userdata"),
        None => js_ctx
            .remove_userdata()
            .expect("could not remove last userdata"),
    }
    .ok_or_else(|| crate::Error::new("got empty userdata after pop"))?;

    Ok((out?, popped_frame.output_queue.into_inner()))
}

fn error_message(
    message: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>,
) -> rquickjs::Error {
    rquickjs::Error::Io(std::io::Error::other(message))
}

fn get_context(js_ctx: &Ctx<'_>) -> rquickjs::Result<QueryContext> {
    Ok(js_ctx
        .userdata::<ContextFrame>()
        .ok_or_else(|| error_message("get_context: no userdata"))?
        .ctx
        .clone())
}

async fn run_task(
    js_ctx: &Ctx<'_>,
    file: PathBuf,
    args: Option<JsValue>,
) -> rquickjs::Result<JsValue> {
    let ctx = &get_context(js_ctx)?;
    let task = RunFile {
        file: file.clone(),
        args,
    };
    let FileOutput { value, outputs } = task.query(ctx).await.map_err(|e| {
        rquickjs::Error::new_loading_message(file.display().to_string(), e.to_string())
    })?;

    push_outputs(js_ctx, outputs).await?;

    Ok(value)
}

async fn push_outputs(
    js_ctx: &Ctx<'_>,
    outputs: impl IntoIterator<Item = WriteOutput>,
) -> rquickjs::Result<()> {
    js_ctx
        .userdata::<ContextFrame>()
        .ok_or_else(|| error_message("push_outputs: no userdata"))?
        .output_queue
        .lock()
        .await
        .extend(outputs);
    Ok(())
}

#[rquickjs::module]
mod driver {
    use std::path::{Component, PathBuf};

    use either::Either;
    use rquickjs::Ctx;
    use url::Url;

    use super::{WriteOutput, error_message, get_context};

    use crate::js::{image::JsImage, object::JsObject, path::JsPath, value::JsValue};
    use crate::query::{
        context::Producer,
        files::{ListDirectory, ReadFile},
        html::{MarkdownToHtml, MinifyHtml},
        image::{ConvertImage, ParseImage},
        remote::GetUrl,
    };

    #[rquickjs::function]
    pub fn get_arg(js_ctx: Ctx<'_>) -> rquickjs::Result<JsValue> {
        Ok(js_ctx
            .userdata::<super::ContextFrame>()
            .ok_or_else(|| error_message("get_arg: no userdata"))?
            .arg
            .clone())
    }

    #[rquickjs::function]
    pub async fn read_file(js_ctx: Ctx<'_>, path: JsPath) -> rquickjs::Result<JsObject> {
        // SAFETY: the only way these javascript functions get called is from inside a
        // `with_query_context()`
        let ctx = &get_context(&js_ctx)?;

        let object = ReadFile(path.0)
            .query(ctx)
            .await
            .map_err(|e| error_message(format!("read_file: {e}")))?;
        Ok(JsObject { object })
    }

    #[rquickjs::function]
    pub async fn list_directory(js_ctx: Ctx<'_>, dirname: JsPath) -> rquickjs::Result<Vec<String>> {
        // SAFETY: the only way these javascript functions get called is from inside a
        // `with_query_context()`
        let ctx = &get_context(&js_ctx)?;
        let contents = ListDirectory(dirname.0)
            .query(ctx)
            .await
            .map_err(|e| error_message(format!("list_directory: {e}")))?
            .into_iter()
            .map(|entry| entry.display().to_string())
            .collect();
        Ok(contents)
    }

    #[rquickjs::function]
    pub async fn run_task(
        js_ctx: Ctx<'_>,
        filename: JsPath,
        args: Option<JsValue>,
    ) -> rquickjs::Result<JsValue> {
        super::CTX
            .scope(js_ctx.as_raw(), async {
                super::run_task(&js_ctx, filename.0, args).await
            })
            .await
    }

    #[rquickjs::function]
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
    pub fn store<'js>(
        js_ctx: Ctx<'js>,
        value: Either<String, rquickjs::TypedArray<'js, u8>>,
    ) -> rquickjs::Result<JsObject> {
        // SAFETY: we are in a javascript context
        let ctx = &get_context(&js_ctx)?;
        let contents = match value {
            Either::Left(s) => s.into_bytes(),
            Either::Right(arr) => Vec::from(AsRef::<[u8]>::as_ref(&arr)),
        };
        let object = ctx.db.objects.store(contents);
        Ok(JsObject { object })
    }

    #[rquickjs::function]
    pub async fn get_url(js_ctx: Ctx<'_>, url: String) -> rquickjs::Result<JsObject> {
        // SAFETY: we are in a javascript context
        let ctx = &get_context(&js_ctx)?;
        let url = Url::parse(&url).map_err(|e| error_message(format!("parsing url: {e}")))?;

        let object = GetUrl(url)
            .query(ctx)
            .await
            .map_err(|e| error_message(format!("fetching url: {e}")))?;
        Ok(JsObject { object })
    }

    #[rquickjs::function]
    pub async fn markdown_to_html(
        js_ctx: Ctx<'_>,
        contents: JsObject,
    ) -> rquickjs::Result<JsObject> {
        // SAFETY: we are in a javascript context
        let ctx = &get_context(&js_ctx)?;
        let object = MarkdownToHtml(contents.object)
            .query(ctx)
            .await
            .map_err(|e| error_message(format!("markdown_to_html: {e}")))?;
        Ok(JsObject { object })
    }

    #[rquickjs::function]
    pub async fn minify_html(js_ctx: Ctx<'_>, contents: JsObject) -> rquickjs::Result<JsObject> {
        // SAFETY: we are in a javascript context
        let ctx = &get_context(&js_ctx)?;
        let object = MinifyHtml(contents.object)
            .query(ctx)
            .await
            .map_err(|e| error_message(format!("minify_html: {e}")))?;
        Ok(JsObject { object })
    }

    #[rquickjs::function]
    pub async fn parse_image(js_ctx: Ctx<'_>, object: JsObject) -> rquickjs::Result<JsImage> {
        // SAFETY: we are in a javascript context
        let ctx = &get_context(&js_ctx)?;
        let image = ParseImage(object.object)
            .query(ctx)
            .await
            .map_err(|e| error_message(format!("parse_image: {e}")))?;
        Ok(JsImage { image })
    }

    #[rquickjs::function]
    pub async fn convert_image<'js>(
        js_ctx: Ctx<'js>,
        image: JsImage,
        opts: rquickjs::Object<'js>,
    ) -> rquickjs::Result<JsImage> {
        // SAFETY: we are in a javascript context
        let ctx = &get_context(&js_ctx)?;
        let format = if opts.contains_key("format")? {
            Some(opts.get("format")?)
        } else {
            None
        };
        let size = if opts.contains_key("size")? {
            Some(opts.get("size")?)
        } else {
            None
        };
        let fit = if opts.contains_key("fit")? {
            Some(opts.get("fit")?)
        } else {
            None
        };

        let image = ConvertImage {
            input: image.image,
            format,
            size,
            fit,
        }
        .query(ctx)
        .await
        .map_err(|e| error_message(format!("convert_image: {e}")))?;

        Ok(JsImage { image })
    }

    #[rquickjs::function]
    pub async fn write_output(
        js_ctx: Ctx<'_>,
        name: String,
        contents: JsObject,
    ) -> rquickjs::Result<()> {
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
        super::push_outputs(
            &js_ctx,
            [WriteOutput {
                path,
                // SAFETY: being provided a StoreObject always means we've put it in the store
                // already
                object: contents.object,
            }],
        )
        .await?;
        Ok(())
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
    #[tracing::instrument(level = "debug", skip(self, js_ctx))]
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

        let ctx = &get_context(js_ctx)?;
        // Need to spawn a new thread to make tokio happy.
        // Probably not a way around this until rquickjs supports asynchronous loaders.
        let object = std::thread::scope(|s| {
            let rt = self.rt.clone();
            s.spawn(move || rt.block_on(ReadFile(path).query(ctx)))
                .join()
        })
        .map_err(|err| {
            rquickjs::Error::new_loading_message(name, format!("joining reader thread: {err:?}"))
        })?
        .map_err(|err| rquickjs::Error::new_loading_message(name, format!("{err}")))?;

        // Need to clone the source so we don't hang onto it for too long when reading from it in
        // the module; the module will clone it into a Vec anyways so no harm in doing that now.
        let source = object.contents_as_bytes(ctx)?;

        rquickjs::Module::declare(js_ctx.clone(), name, source)
    }
}

tokio::task_local! {
    // The current Ctx object that we are executing in, if any. Use with_js_ctx() to re-entrantly
    // access this.
    static CTX: std::ptr::NonNull<rquickjs::qjs::JSContext>;
}

async fn with_js_ctx<T, C>(rt: Arc<tokio::runtime::Runtime>, callback: C) -> T
where
    T: rquickjs::markers::ParallelSend + 'static,
    C: (AsyncFnOnce(rquickjs::Ctx<'_>) -> T) + rquickjs::markers::ParallelSend,
{
    match CTX.try_get() {
        Ok(ctx) => {
            // SAFETY: there is some larger `context.with` that we are borrowing from
            let ctx = unsafe { rquickjs::Ctx::from_raw(ctx) };
            callback(ctx).await
        }
        Err(_) => {
            // Realistically, there will be just one `with_js_ctx` at the top-level, so it's OK to
            // make another
            tracing::info!("creating new js ctx");
            let runtime = {
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
            };
            // TODO: it seems rquickjs isn't happy if it doesn't have the full set of features. Not
            // that there's any IO features in here besides those we allow it, but ah well
            let context = rquickjs::AsyncContext::full(&runtime)
                .await
                .expect("context failed to build");

            rquickjs::async_with!(context => |js_ctx| {
                js_ctx.globals().set("print", Function::new(js_ctx.clone(), |arg: String| println!("{}", arg))).expect("setting print global");
                callback(js_ctx).await
            })
            .await
        }
    }
}

query_key!(RunFile {
    pub file: PathBuf,
    pub args: Option<JsValue>,
});

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileOutput {
    #[cfg(test)]
    pub value: JsValue,
    #[cfg(not(test))]
    value: JsValue,
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

    #[tracing::instrument(level = "debug", skip_all)]
    async fn produce(&self, ctx: &QueryContext) -> Self::Output {
        let name = self.file.display().to_string();
        let key = format!(
            "{}({})",
            name,
            self.args
                .as_ref()
                .map(|args| args.to_string())
                .unwrap_or_default()
        );
        println!("running {key}");
        let object = ReadFile(self.file.clone()).query(ctx).await?;
        trace!("read file");
        let contents = object.contents_as_string(ctx)?;
        trace!("read file contents");

        let (value, outputs) = with_js_ctx(ctx.rt.clone(), async |js_ctx| {
            trace!("with_js_ctx start");
            let catch = |err: rquickjs::Error| -> crate::Error {
                match err {
                    rquickjs::Error::Exception => {
                        let value = js_ctx.catch();
                        if let Some(err) = value.as_exception() {
                            let message = err.message().unwrap_or_default();
                            let stack = err.stack().unwrap_or_default();
                            eprintln!("js exception: {message}");
                            eprintln!("{stack}");
                        } else if let Ok(value) = JsValue::from_js(&js_ctx, value.clone()) {
                            eprintln!("js thrown value: {}", value);
                        } else {
                            eprintln!("js error: {:?}", value);
                        }
                        crate::Error::from(rquickjs::Error::Exception)
                    }
                    otherwise => crate::Error::from(otherwise),
                }
            };

            let out = with_query_context(
                &js_ctx,
                ctx.clone(),
                self.args.clone().unwrap_or_default(),
                async || {
                    trace!("with_query_context start");
                    let module = rquickjs::Module::declare(js_ctx.clone(), name.clone(), contents)
                        .map_err(catch)?;
                    let (module, promise) = module.eval().map_err(catch)?;
                    promise.into_future::<()>().await.map_err(catch)?;

                    let value: JsValue = module.get(rquickjs::atom::PredefinedAtom::Default)?;
                    trace!("with_query_context end");
                    Ok(value)
                },
            )
            .await;

            trace!("with_js_ctx end");
            out
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
        trace!("finished");
        Ok(FileOutput { value, outputs })
    }
}
