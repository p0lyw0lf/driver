use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
    ops::DerefMut,
    path::PathBuf,
    rc::Rc,
};

use boa_engine::{
    Context, JsError, JsNativeError, JsResult, JsString, Module, NativeFunction,
    builtins::promise::PromiseState,
    context::{ContextBuilder, time::JsInstant},
    job::{GenericJob, Job, JobExecutor, NativeAsyncJob, PromiseJob, TimeoutJob},
    js_str,
    module::{ModuleLoader, resolve_module_specifier},
    property::Attribute,
    value::{TryFromJs, TryIntoJs},
};
use futures_concurrency::future::FutureGroup;
use futures_lite::StreamExt;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use tracing::trace;

use crate::{
    engine::{Producer, QueryContext, db::Object},
    query::ReadFile,
    query_key,
    to_hash::ToHash,
};

mod image;
mod macros;
mod object;
mod path;
mod value;

use self::{image::JsImage, path::JsPath};
#[cfg(test)]
pub use self::{object::JsObject, value::JsValue};
#[cfg(not(test))]
use self::{object::JsObject, value::JsValue};

pub type WriteOutputs = BTreeMap<PathBuf, Object>;

struct ContextFrame {
    ctx: QueryContext,
    outputs: WriteOutputs,
}

task_local::task_local! {
    static QUERY_CONTEXT: RefCell<ContextFrame>;
}

/// Runs a closure with a QueryContext pushed onto the stack. All calls to `get_context()` that run
/// as a result of that closure will access this ctx object. Therefore, all pointer accesses from
/// `get_context()` have safety ensured as a result of running in this function.
async fn with_query_context<T, F: Future<Output = crate::Result<T>>>(
    ctx: QueryContext,
    f: impl FnOnce() -> F,
) -> crate::Result<(T, WriteOutputs)> {
    let new_frame = ContextFrame {
        ctx,
        outputs: Default::default(),
    };
    let fut = QUERY_CONTEXT.scope(RefCell::new(new_frame), f());
    let fut = std::pin::pin!(fut);

    let out = (&mut fut).await?;
    let popped_frame = fut
        .take_value()
        .expect("no context frame to pop")
        .into_inner();
    Ok((out, popped_frame.outputs))
}

fn get_context() -> JsResult<QueryContext> {
    QUERY_CONTEXT
        .with(|ctx| ctx.borrow().ctx.clone())
        .map_err(JsError::from_rust)
}

/// SAFETY: only safe to call when running inside `with_query_context()`
unsafe fn push_outputs(outputs: impl IntoIterator<Item = (PathBuf, Object)>) -> JsResult<()> {
    QUERY_CONTEXT.with(|ctx| -> JsResult<_> {
        ctx.try_borrow_mut()
            .map_err(JsError::from_rust)?
            .outputs
            .extend(outputs);
        Ok(())
    })
}

/// An event loop using tokio to drive futures to completion.
struct Executor {
    async_jobs: RefCell<VecDeque<NativeAsyncJob>>,
    promise_jobs: RefCell<VecDeque<PromiseJob>>,
    timeout_jobs: RefCell<BTreeMap<JsInstant, TimeoutJob>>,
    generic_jobs: RefCell<VecDeque<GenericJob>>,
}

impl Executor {
    fn new() -> Self {
        Self {
            async_jobs: RefCell::default(),
            promise_jobs: RefCell::default(),
            timeout_jobs: RefCell::default(),
            generic_jobs: RefCell::default(),
        }
    }

    fn drain_timeout_jobs(&self, js_ctx: &mut Context) {
        let now = js_ctx.clock().now();

        let timed_out_jobs = {
            let mut timeout_jobs = self.timeout_jobs.borrow_mut();
            let mut jobs_to_keep = timeout_jobs.split_off(&now);
            jobs_to_keep.retain(|_, job| !job.is_cancelled());
            std::mem::replace(timeout_jobs.deref_mut(), jobs_to_keep)
        };

        for timeout_job in timed_out_jobs.into_values() {
            if let Err(e) = timeout_job.call(js_ctx) {
                eprintln!("Uncaught {e}");
            }
        }
    }

    fn drain_jobs(&self, js_ctx: &mut Context) {
        // Run the timeout jobs first.
        self.drain_timeout_jobs(js_ctx);

        if let Some(generic_job) = self.generic_jobs.borrow_mut().pop_front()
            && let Err(err) = generic_job.call(js_ctx)
        {
            eprintln!("Uncaught {err}");
        }

        let promise_jobs = std::mem::take(self.promise_jobs.borrow_mut().deref_mut());
        for promise_job in promise_jobs {
            if let Err(e) = promise_job.call(js_ctx) {
                eprintln!("Uncaught {e}");
            }
        }

        js_ctx.clear_kept_objects();
    }
}

impl JobExecutor for Executor {
    fn enqueue_job(self: Rc<Self>, job: Job, js_ctx: &mut Context) {
        match job {
            Job::PromiseJob(promise_job) => self.promise_jobs.borrow_mut().push_back(promise_job),
            Job::AsyncJob(native_async_job) => {
                self.async_jobs.borrow_mut().push_back(native_async_job)
            }
            Job::TimeoutJob(timeout_job) => {
                let now = js_ctx.clock().now();
                self.timeout_jobs
                    .borrow_mut()
                    .insert(now + timeout_job.timeout(), timeout_job);
            }
            Job::GenericJob(generic_job) => self.generic_jobs.borrow_mut().push_back(generic_job),
            _ => panic!("Unsupported job type"),
        }
    }

    // Sync flavor that needs to be provided
    fn run_jobs(self: Rc<Self>, js_ctx: &mut Context) -> JsResult<()> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();

        task::LocalSet::default().block_on(&runtime, self.run_jobs_async(&RefCell::new(js_ctx)))
    }

    async fn run_jobs_async(self: Rc<Self>, js_ctx: &RefCell<&mut Context>) -> JsResult<()> {
        let mut group = FutureGroup::new();
        loop {
            for async_job in std::mem::take(self.async_jobs.borrow_mut().deref_mut()) {
                trace!("inserting another async job");
                group.insert(async_job.call(js_ctx));
            }

            if group.is_empty()
                && self.promise_jobs.borrow().is_empty()
                && self.timeout_jobs.borrow().is_empty()
                && self.generic_jobs.borrow().is_empty()
            {
                // All queue empty
                return Ok(());
            }

            // For perf, it really helps if we await these futures one at a time instead of
            // poll_once-ing them. Also, it really does improve perf to keep them in a group
            // instead of awaiting one at a time. Also also, it's an incorrect strategy to poll all
            // the pending futures at once.
            if let Some(Err(err)) = group.next().await {
                eprintln!("Uncaught {err}");
            }

            self.drain_jobs(&mut js_ctx.borrow_mut());
            tokio::task::yield_now().await
        }
    }
}

/// Custom loader that will track dependencies via ReadFile
struct MemoizedModuleLoader {
    ctx: QueryContext,
    builtin_module_map: scc::HashMap<String, Module>,
    js_module_map: scc::HashMap<PathBuf, Module>,
}

impl MemoizedModuleLoader {
    fn new(ctx: QueryContext) -> Self {
        Self {
            ctx,
            builtin_module_map: Default::default(),
            js_module_map: Default::default(),
        }
    }

    fn set_builtin_module(&self, name: String, module: Module) {
        let _ = self.builtin_module_map.upsert_sync(name, module);
    }
}

impl ModuleLoader for MemoizedModuleLoader {
    async fn load_imported_module(
        self: Rc<Self>,
        referrer: boa_engine::module::Referrer,
        specifier: JsString,
        js_ctx: &RefCell<&mut Context>,
    ) -> JsResult<Module> {
        let short_path = specifier.to_std_string_escaped();
        if let Some(module) = self.builtin_module_map.get_async(&short_path).await {
            return Ok(module.clone());
        }

        // TODO: specify a base directory to sandbox the module imports
        let path =
            resolve_module_specifier(None, &specifier, referrer.path(), &mut js_ctx.borrow_mut())?;

        // Shortcutting here is OK because we create a new loader for each file we execute, so we
        // do exactly one ReadFile for each import dependency we have.
        if let Some(module) = self.js_module_map.get_async(&path).await {
            return Ok(module.get().clone());
        }

        let source_bytes = ReadFile(path.clone())
            .query(&self.ctx)
            .await
            .map_err(|e| {
                JsNativeError::eval()
                    .with_message(format!("reading imported module '{}': {}", short_path, e))
            })?
            .contents_as_bytes(&self.ctx)?;
        let source = boa_engine::Source::from_bytes(&source_bytes).with_path(&path);
        let module =
            boa_engine::Module::parse(source, None, &mut js_ctx.borrow_mut()).map_err(|err| {
                eprintln!("error in module {err}");
                JsNativeError::syntax()
                    .with_message(format!("could not parse module '{short_path}'"))
                    .with_cause(err)
            })?;
        let _ = self.js_module_map.insert_async(path, module.clone()).await;
        Ok(module)
    }
}

async fn with_js_ctx<T, F>(ctx: QueryContext, arg: JsValue, f: F) -> crate::Result<T>
where
    F: (AsyncFnOnce(&mut Context) -> crate::Result<T>) + Send + 'static,
    T: Send + 'static,
{
    let rt = ctx.rt.clone();
    let (send, recv) = oneshot::channel();
    let handle = std::thread::spawn(move || {
        send.send((|| -> crate::Result<T> {
            // I wish I could only have one runtime, but unfortunately not, the ctx stuff just doesn't work
            // out... Startup costs are a real thing we have to pay unfortunately. Hopefully multithreaded
            // pays off some of that!!
            let executor = Rc::new(Executor::new());
            let loader = Rc::new(MemoizedModuleLoader::new(ctx));

            let js_ctx = &mut ContextBuilder::new()
                .job_executor(executor.clone())
                .module_loader(loader.clone())
                .build()?;

            js_ctx.register_global_class::<JsImage>()?;
            js_ctx.register_global_class::<JsObject>()?;

            js_ctx.register_global_builtin_callable(
                js_str!("print").into(),
                1,
                NativeFunction::from_fn_ptr(|_this, args, js_ctx| {
                    let mut s = String::new();
                    for (i, arg) in args.iter().enumerate() {
                        if i != 0 {
                            s.push(' ');
                        }
                        s.push_str(&arg.to_string(js_ctx)?.to_std_string_lossy());
                    }
                    println!("{}", s);
                    Ok(boa_engine::JsValue::undefined())
                }),
            )?;

            let arg = arg.try_into_js(js_ctx)?;
            js_ctx.register_global_property(js_str!("ARG"), arg, Attribute::READONLY)?;

            let driver_module = make_driver_module(js_ctx)?;
            loader.set_builtin_module("driver".to_string(), driver_module);

            let local_set = &mut tokio::task::LocalSet::default();
            rt.block_on(async { local_set.run_until(async { f(js_ctx).await }).await })
        })())
        .unwrap_or_else(|_| panic!("failed to send"))
    });
    // It's very important we drop this handle here actually! Improves performance by a lot to
    // detatch the thread, for some reason.
    drop(handle);

    recv.await.expect("failed to receive")
}

fn make_driver_module(js_ctx: &mut Context) -> JsResult<Module> {
    use crate::js::macros::module;
    use driver_module::*;
    Ok(module!(
        use js_ctx;
        fn store(value: String) -> JsResult<JsObject>;

        async fn read_file(path: JsPath) -> JsResult<JsObject>;
        async fn list_directory(dirname: JsPath) -> JsResult<Vec<String>>;
        fn file_type(entry_name: String) -> JsResult<String>;

        async fn get_url(url: String) -> JsResult<JsObject>;

        async fn markdown_to_html(contents: JsObject) -> JsResult<JsObject>;
        async fn minify_html(contents: JsObject) -> JsResult<JsObject>;

        async fn parse_image(object: JsObject) -> JsResult<JsImage>;
        async fn convert_image(
            image: JsImage,
            opts: boa_engine::JsObject,
            [js_ctx: &mut Context],
        ) -> JsResult<JsImage>;

        async fn run_task(filename: JsPath, args: Option<JsValue>) -> JsResult<JsValue>;
        fn write_output(name: String, contents: JsObject) -> JsResult<()>;
    ))
}

mod driver_module {
    use std::cell::RefCell;
    use std::ops::DerefMut;
    use std::path::{Component, PathBuf};

    use boa_engine::value::TryFromJs;
    use boa_engine::{Context, js_str};
    use boa_engine::{JsError, JsNativeError, JsResult};
    use url::Url;

    use super::{FileOutput, RunFile, get_context, push_outputs};

    use crate::js::{image::JsImage, object::JsObject, path::JsPath, value::JsValue};
    use crate::query::{
        context::Producer,
        files::{ListDirectory, ReadFile},
        html::{MarkdownToHtml, MinifyHtml},
        image::{ConvertImage, ParseImage},
        remote::GetUrl,
    };

    pub async fn read_file(path: JsPath) -> JsResult<JsObject> {
        let ctx = &get_context()?;

        let object = ReadFile(path.0)
            .query(ctx)
            .await
            .map_err(|e| JsNativeError::eval().with_message(format!("read_file: {e}")))?;

        Ok(JsObject { object })
    }

    pub async fn list_directory(dirname: JsPath) -> JsResult<Vec<String>> {
        let ctx = &get_context()?;

        let contents = ListDirectory(dirname.0)
            .query(ctx)
            .await
            .map_err(|e| JsNativeError::eval().with_message(format!("list_directory: {e}")))?
            .into_iter()
            .map(|entry| entry.display().to_string())
            .collect();

        Ok(contents)
    }

    pub async fn run_task(filename: JsPath, arg: Option<JsValue>) -> JsResult<JsValue> {
        let ctx = &get_context()?;

        let filename = filename.0;
        let task = RunFile {
            file: filename.clone(),
            arg: arg.clone(),
        };

        let FileOutput { value, outputs } = task.query(ctx).await.map_err(|e| {
            JsNativeError::eval().with_message(format!(
                "error running {}({}):\n\t{}",
                filename.display(),
                arg.as_ref().map(JsValue::to_string).unwrap_or_default(),
                e
            ))
        })?;

        // Limit the amount of time we borrow QUERY_CONTEXT
        unsafe { push_outputs(outputs) }?;

        Ok(value)
    }

    pub fn file_type(entry_name: String) -> JsResult<String> {
        let metadata = std::fs::metadata(PathBuf::from(entry_name)).map_err(JsError::from_rust)?;

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

    // TODO: eventually, I want this to be able to take in JsUint8Array. However, that has some
    // weird lifetime implications w/ js_ctx, so I won't bother for now.
    pub fn store(value: String) -> JsResult<JsObject> {
        let ctx = &get_context()?;

        let object = ctx.db().objects.store(value.into_bytes());
        Ok(JsObject { object })
    }

    pub async fn get_url(url: String) -> JsResult<JsObject> {
        let ctx = &get_context()?;
        let url = Url::parse(&url)
            .map_err(|e| JsNativeError::eval().with_message(format!("parsing url: {e}")))?;

        let object = GetUrl(url)
            .query(ctx)
            .await
            .map_err(|e| JsNativeError::eval().with_message(format!("fetching url: {e}")))?;
        Ok(JsObject { object })
    }

    pub async fn markdown_to_html(contents: JsObject) -> JsResult<JsObject> {
        let ctx = &get_context()?;

        let object = MarkdownToHtml(contents.object.clone())
            .query(ctx)
            .await
            .map_err(|e| JsNativeError::eval().with_message(format!("markdown_to_html: {e}")))?;
        Ok(JsObject { object })
    }

    pub async fn minify_html(contents: JsObject) -> JsResult<JsObject> {
        let ctx = &get_context()?;

        let object = MinifyHtml(contents.object.clone())
            .query(ctx)
            .await
            .map_err(|e| JsNativeError::eval().with_message(format!("minify_html: {e}")))?;
        Ok(JsObject { object })
    }

    pub async fn parse_image(object: JsObject) -> JsResult<JsImage> {
        let ctx = &get_context()?;

        let image = ParseImage(object.object.clone())
            .query(ctx)
            .await
            .map_err(|e| JsNativeError::eval().with_message(format!("parse_image: {e}")))?;
        Ok(JsImage { image })
    }

    pub async fn convert_image(
        image: JsImage,
        opts: boa_engine::JsObject,
        js_ctx: &RefCell<&mut Context>,
    ) -> JsResult<JsImage> {
        let ctx = &get_context()?;

        let convert_image = {
            let mut js_ctx = js_ctx.borrow_mut();
            let js_ctx = js_ctx.deref_mut();
            let format = if opts.has_property(js_str!("format"), js_ctx)? {
                Some(TryFromJs::try_from_js(
                    &opts.get(js_str!("format"), js_ctx)?,
                    js_ctx,
                )?)
            } else {
                None
            };
            let size = if opts.has_property(js_str!("size"), js_ctx)? {
                Some(TryFromJs::try_from_js(
                    &opts.get(js_str!("size"), js_ctx)?,
                    js_ctx,
                )?)
            } else {
                None
            };
            let fit = if opts.has_property(js_str!("fit"), js_ctx)? {
                Some(TryFromJs::try_from_js(
                    &opts.get(js_str!("fit"), js_ctx)?,
                    js_ctx,
                )?)
            } else {
                None
            };

            ConvertImage {
                input: image.image.clone(),
                format,
                size,
                fit,
            }
        };

        let image = convert_image
            .query(ctx)
            .await
            .map_err(|e| JsNativeError::eval().with_message(format!("convert_image: {e}")))?;
        Ok(JsImage { image })
    }

    pub fn write_output(name: String, contents: JsObject) -> JsResult<()> {
        let path = PathBuf::from(name);
        if !path
            .components()
            .all(|component| matches!(component, Component::CurDir | Component::Normal(_)))
        {
            // Don't allow any path traversal outside the output directory
            return Err(JsNativeError::eval()
                .with_message(format!("directory traversal {}", path.display()))
                .into());
        }
        unsafe {
            super::push_outputs([(
                path,
                // SAFETY: being provided a StoreObject always means we've put it in the store
                // already
                contents.object.clone(),
            )])?
        };
        Ok(())
    }
}

query_key!(RunFile {
    pub file: PathBuf,
    pub arg: Option<JsValue>,
});

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileOutput {
    #[cfg(test)]
    pub value: JsValue,
    #[cfg(not(test))]
    value: JsValue,
    pub outputs: WriteOutputs,
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

    #[tracing::instrument(level = "debug", skip(ctx))]
    async fn produce(&self, ctx: &QueryContext) -> Self::Output {
        println!(
            "running {}({})",
            self.file.display(),
            self.arg
                .as_ref()
                .map(JsValue::to_string)
                .unwrap_or_default()
        );

        let file = self.file.clone();
        let arg = self.arg.clone().unwrap_or_default();

        let object = ReadFile(self.file.clone()).query(ctx).await?;
        let contents = object.contents_as_bytes(ctx)?;

        let ctx = ctx.clone();
        let (value, outputs) = with_js_ctx(ctx.clone(), arg.clone(), async move |js_ctx| {
            trace!("with_js_ctx start");
            let out = with_query_context(ctx, async move || {
                trace!("with_query_context start {}({})", file.display(), arg);
                // TODO: print stack traces
                /*
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
                */

                let source = boa_engine::Source::from_bytes(&contents).with_path(&file);
                let module = boa_engine::Module::parse(source, None, js_ctx)?;
                let promise = module.load_link_evaluate(js_ctx);
                let executor = js_ctx.downcast_job_executor::<Executor>().unwrap();
                trace!("starting to run jobs");
                executor.run_jobs_async(&RefCell::new(js_ctx)).await?;

                match promise.state() {
                    PromiseState::Pending => {
                        return Err(crate::Error::new("module didn't execute!"));
                    }
                    PromiseState::Fulfilled(v) => assert_eq!(v, boa_engine::JsValue::undefined()),
                    PromiseState::Rejected(err) => {
                        return Err(JsError::from_opaque(err).try_native(js_ctx)?.into());
                    }
                }

                let value = module.namespace(js_ctx).get(js_str!("default"), js_ctx)?;
                let value = JsValue::try_from_js(&value, js_ctx)?;
                trace!(
                    "with_query_context end {}({}) = {}",
                    file.display(),
                    arg,
                    value
                );
                Ok(value)
            })
            .await;

            trace!("with_js_ctx end");
            out
        })
        .await?;
        Ok(FileOutput { value, outputs })
    }
}
