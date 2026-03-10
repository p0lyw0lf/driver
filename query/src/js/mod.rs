use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
    ops::DerefMut,
    path::PathBuf,
    rc::Rc,
};

use boa_engine::{
    Context, JsError, JsNativeError, JsResult, JsString, Module,
    builtins::promise::PromiseState,
    context::{ContextBuilder, time::JsInstant},
    job::{GenericJob, Job, JobExecutor, NativeAsyncJob, PromiseJob, TimeoutJob},
    js_str,
    module::{ModuleLoader, resolve_module_specifier},
    value::TryFromJs,
};
use futures_concurrency::future::FutureGroup;
use futures_lite::{StreamExt, future};
use scc::HashMap;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use tokio::task;
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

mod class_wrap;
mod image;
mod object;
mod path;
mod value;

#[cfg(test)]
pub use self::{object::JsObject, value::JsValue};

#[cfg(not(test))]
use self::{object::JsObject, value::JsValue};

struct ContextFrame {
    ctx: QueryContext,
    output_queue: Vec<WriteOutput>,
}

// SAFETY: I'm pretty sure know what I'm doing
unsafe impl Send for ContextFrame {}

tokio::task_local! {
    static QUERY_CONTEXT: RefCell<ContextFrame>;
}

/// Runs a closure with a QueryContext pushed onto the stack. All calls to `get_context()` that run
/// as a result of that closure will access this ctx object. Therefore, all pointer accesses from
/// `get_context()` have safety ensured as a result of running in this function.
async fn with_query_context<T, F: Future<Output = crate::Result<T>>>(
    ctx: QueryContext,
    f: impl FnOnce() -> F,
) -> crate::Result<(T, Vec<WriteOutput>)> {
    let new_frame = ContextFrame {
        ctx,
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

fn get_context() -> JsResult<QueryContext> {
    QUERY_CONTEXT
        .try_with(|ctx| ctx.borrow().ctx.clone())
        .map_err(JsError::from_rust)
}

/// SAFETY: only safe to call when running inside `with_query_context()`
unsafe fn push_outputs(outputs: impl IntoIterator<Item = WriteOutput>) -> JsResult<()> {
    QUERY_CONTEXT.with(|ctx| -> JsResult<_> {
        ctx.try_borrow_mut()
            .map_err(JsError::from_rust)?
            .output_queue
            .extend(outputs);
        Ok(())
    })
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

        let jobs_to_run = {
            let mut timeout_jobs = self.timeout_jobs.borrow_mut();
            let mut jobs_to_keep = timeout_jobs.split_off(&now);
            jobs_to_keep.retain(|_, job| !job.is_cancelled());
            std::mem::replace(timeout_jobs.deref_mut(), jobs_to_keep)
        };

        for job in jobs_to_run.into_values() {
            if let Err(e) = job.call(js_ctx) {
                eprintln!("Uncaught {e}");
            }
        }
    }

    fn drain_jobs(&self, js_ctx: &mut Context) {
        // Run the timeout jobs first.
        self.drain_timeout_jobs(js_ctx);

        if let Some(generic) = self.generic_jobs.borrow_mut().pop_front()
            && let Err(err) = generic.call(js_ctx)
        {
            eprintln!("Uncaught {err}");
        }

        let jobs = std::mem::take(self.promise_jobs.borrow_mut().deref_mut());
        for job in jobs {
            if let Err(e) = job.call(js_ctx) {
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
            for job in std::mem::take(self.async_jobs.borrow_mut().deref_mut()) {
                group.insert(job.call(js_ctx));
            }

            if group.is_empty()
                && self.promise_jobs.borrow().is_empty()
                && self.timeout_jobs.borrow().is_empty()
                && self.generic_jobs.borrow().is_empty()
            {
                // All queue empty
                return Ok(());
            }

            if let Some(Err(err)) = future::poll_once(group.next()).await.flatten() {
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
    driver_module: RefCell<Option<Module>>,
    module_map: HashMap<PathBuf, Module>,
}

impl MemoizedModuleLoader {
    fn new(ctx: QueryContext) -> Self {
        Self {
            ctx,
            driver_module: RefCell::new(None),
            module_map: Default::default(),
        }
    }

    fn set_driver_module(&self, module: Module) {
        self.driver_module.borrow_mut().insert(module);
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
        if &short_path == "driver" {
            return Ok(self
                .driver_module
                .borrow()
                .as_ref()
                .expect("MUST call initialize_driver first")
                .clone());
        }

        // TODO: specify a base directory
        let path =
            resolve_module_specifier(None, &specifier, referrer.path(), &mut js_ctx.borrow_mut())?;

        // Shortcutting here is OK because we create a new loader for each file we execute, so we
        // do exactly one ReadFile for each import dependency we have.
        if let Some(module) = self.module_map.get_async(&path).await {
            return Ok(module.get().clone());
        }

        let source_bytes = ReadFile(path.clone())
            .query(&self.ctx)
            .await
            .map_err(|e| JsNativeError::eval().with_message(e.to_string()))?
            .contents_as_bytes(&self.ctx)?;
        let source = boa_engine::Source::from_bytes(&source_bytes);
        let module =
            boa_engine::Module::parse(source, None, &mut js_ctx.borrow_mut()).map_err(|err| {
                JsNativeError::syntax()
                    .with_message(format!("could not parse module `{short_path}'"))
                    .with_cause(err)
            })?;

        self.module_map.insert_async(path, module.clone()).await;
        Ok(module)
    }
}

async fn with_js_ctx<T, F>(ctx: QueryContext, f: F) -> crate::Result<T>
where
    F: (AsyncFnOnce(&mut Context) -> crate::Result<T>),
{
    // I wish I could only have one runtime, but unfortunately not, the ctx stuff just doesn't work
    // out... Startup costs are a real thing we have to pay unfortunately. Hopefully multithreaded
    // pays off some of that!!
    let executor = Rc::new(Executor::new());
    let loader = Rc::new(MemoizedModuleLoader::new(ctx));

    let js_ctx = &mut ContextBuilder::new()
        .job_executor(executor.clone())
        .module_loader(loader.clone())
        .build()?;

    let driver_module = make_driver_module(js_ctx)?;
    loader.set_driver_module(driver_module);

    let local_set = &mut tokio::task::LocalSet::default();
    local_set
        .run_until(async { crate::Result::Ok(f(js_ctx).await?) })
        .await
}

fn make_driver_module(js_ctx: &mut Context) -> JsResult<Module> {
    todo!()
}

mod driver_module {
    use std::path::{Component, PathBuf};

    use boa_engine::value::TryFromJs;
    use boa_engine::{Context, js_str};
    use boa_engine::{JsError, JsNativeError, JsResult, object::builtins::JsUint8Array};
    use either::Either;
    use url::Url;

    use super::{FileOutput, RunFile, WriteOutput, get_context, push_outputs};

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

    pub async fn run_task(filename: JsPath, args: Option<JsValue>) -> JsResult<JsValue> {
        let ctx = &get_context()?;

        let filename = filename.0;
        let task = RunFile {
            file: filename.clone(),
            args,
        };

        let FileOutput { value, outputs } = task.query(ctx).await.map_err(|e| {
            JsNativeError::eval().with_message(format!(
                "error running {}: {}",
                filename.display().to_string(),
                e.to_string()
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

    pub fn store(value: Either<String, JsUint8Array>, js_ctx: &mut Context) -> JsResult<JsObject> {
        let ctx = &get_context()?;

        let contents = match value {
            Either::Left(s) => s.into_bytes(),
            Either::Right(arr) => arr.iter(js_ctx).collect(),
        };

        let object = ctx.db.objects.store(contents);
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
        js_ctx: &mut Context,
    ) -> JsResult<JsImage> {
        let ctx = &get_context()?;

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

        let image = ConvertImage {
            input: image.image.clone(),
            format,
            size,
            fit,
        }
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
            super::push_outputs([WriteOutput {
                path,
                // SAFETY: being provided a StoreObject always means we've put it in the store
                // already
                object: contents.object.clone(),
            }])?
        };
        Ok(())
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
        let contents = object.contents_as_bytes(ctx)?;

        let (value, outputs) = with_query_context(ctx.clone(), async || {
            trace!("with_query_context start");
            let out = with_js_ctx(ctx.clone(), async |js_ctx| {
                trace!("with_js_ctx start");
                // TODO: set ARGS global variable

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

                let source = boa_engine::Source::from_reader(&contents[..], Some(&self.file));
                let module = boa_engine::Module::parse(source, None, js_ctx)?;

                let promise = module.load_link_evaluate(js_ctx);
                let executor = js_ctx.downcast_job_executor::<Executor>().unwrap();
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
                trace!("with_js_ctx end");
                Ok(value)
            })
            .await;

            trace!("with_query_context end");
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
