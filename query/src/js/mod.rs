use std::{cell::RefCell, path::PathBuf};

use rquickjs::{
    Context, FromJs, IntoJs, Runtime, Value,
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
    file: PathBuf,
    curr: *const QueryContext,
    prev: Option<Box<ContextFrame>>,
    output_queue: Vec<WriteOutput>,
}

thread_local! {
    static QUERY_CONTEXT: RefCell<Option<ContextFrame>> = const { RefCell::new(None) };
}

/// NOTE: ONLY SAFE IF SINGLE-THREADED
///
/// Runs a closure with a QueryContext pushed onto the stack. All calls to `get_context()` that run
/// as a result of that closure will access this ctx object. Therefore, all pointer accesses from
/// `get_context()` have safety ensured as a result of running in this function.
fn with_query_context<T>(
    file: PathBuf,
    ctx: &QueryContext,
    f: impl FnOnce() -> crate::Result<T>,
) -> crate::Result<(T, Vec<WriteOutput>)> {
    let prev = QUERY_CONTEXT.take().map(Box::new);
    let curr = ctx as *const _;
    let new_frame = ContextFrame {
        file,
        curr,
        prev,
        output_queue: vec![],
    };
    QUERY_CONTEXT.set(Some(new_frame));

    let out = f();

    let popped = QUERY_CONTEXT.take().expect("popped nothing");
    assert!(std::ptr::eq(curr, popped.curr));
    QUERY_CONTEXT.set(popped.prev.map(|x| *x));

    out.map(|t| (t, popped.output_queue))
}

/// Only safe to dereference the returned pointer if running inside a call to `with_context()`.
fn get_context() -> rquickjs::Result<*const QueryContext> {
    QUERY_CONTEXT.with_borrow(|ctx| -> rquickjs::Result<_> {
        let ctx = ctx.as_ref().ok_or(rquickjs::Error::Unknown)?;
        Ok(ctx.curr)
    })
}

/// SAFETY: only safe to call when running inside `with_query_context()`
unsafe fn get_current_file() -> rquickjs::Result<PathBuf> {
    QUERY_CONTEXT.with_borrow(|ctx| -> rquickjs::Result<_> {
        let ctx = ctx.as_ref().ok_or(rquickjs::Error::Unknown)?;
        Ok(ctx.file.clone())
    })
}

/// SAFETY: only safe to call when running inside `with_query_context()`
unsafe fn push_task(task: RunFile) -> rquickjs::Result<RustValue> {
    // Purposefully limit how much we borrow QUERY_CONTEXT for, since re-querying the RunFile will
    // cause it to be borrowed again.
    let ctx = unsafe { &*get_context()? };
    let FileOutput { value, outputs } = task.clone().query(ctx).map_err(|e| {
        rquickjs::Error::new_loading_message(task.file.display().to_string(), e.to_string())
    })?;

    QUERY_CONTEXT.with_borrow_mut(|ctx| -> rquickjs::Result<_> {
        let ctx = ctx.as_mut().ok_or(rquickjs::Error::Unknown)?;
        ctx.output_queue.extend(outputs);
        Ok(value)
    })
}

/// SAFETY: only safe to call when running inside `with_query_context()`
unsafe fn push_output(output: WriteOutput) -> rquickjs::Result<()> {
    QUERY_CONTEXT.with_borrow_mut(|ctx| -> rquickjs::Result<_> {
        let ctx = ctx.as_mut().ok_or(rquickjs::Error::Unknown)?;
        ctx.output_queue.push(output);
        Ok(())
    })
}

fn error_message(message: &str) -> rquickjs::Error {
    rquickjs::Error::Io(std::io::Error::other(message))
}

#[rquickjs::module]
mod memoized {
    use std::path::PathBuf;

    use relative_path::RelativePath;
    use relative_path::RelativePathBuf;

    use super::error_message;
    use super::get_context;
    use super::value::RustValue;
    use crate::js::get_current_file;
    use crate::js::store_object::StoreObject;
    use crate::{
        js::{RunFile, push_task},
        query::context::Producer,
        query::files::{ListDirectory, ReadFile},
    };

    /// Helper function that formats a path relative to the current file
    /// SAFETY: MUST be called inside a javascript context
    unsafe fn to_relative_path(path: String) -> rquickjs::Result<PathBuf> {
        let base_file = unsafe { get_current_file()? };
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
    pub fn read_file(filename: String) -> rquickjs::Result<StoreObject> {
        // SAFETY: the only way these javascript functions get called is from inside a
        // `with_query_context()`
        let ctx = unsafe { &*get_context()? };
        let object = ReadFile(unsafe { to_relative_path(filename)? })
            .query(ctx)
            .map_err(|e| error_message(&format!("{e}")))?;
        Ok(StoreObject { object })
    }

    #[rquickjs::function]
    pub fn list_directory(dirname: String) -> rquickjs::Result<Vec<String>> {
        // SAFETY: the only way these javascript functions get called is from inside a
        // `with_query_context()`
        let ctx = unsafe { &*get_context()? };
        let contents = ListDirectory(unsafe { to_relative_path(dirname)? })
            .query(ctx)
            .map_err(|e| error_message(&format!("{e}")))?
            .into_iter()
            .map(|entry| entry.display().to_string())
            .collect();
        Ok(contents)
    }

    #[rquickjs::function]
    pub fn run_task(filename: String, args: Option<RustValue>) -> rquickjs::Result<RustValue> {
        // SAFETY: the only way these javascript functions get called is from inside a
        // `with_query_context()`
        unsafe {
            push_task(RunFile {
                file: to_relative_path(filename)?,
                args,
            })
        }
    }
}

// TODO: most of these should actually be memoized as well I think.
#[rquickjs::module]
mod io {
    use std::path::{Component, PathBuf};

    use either::Either;

    use crate::js::store_object::StoreObject;

    use super::WriteOutput;
    use super::error_message;
    use super::get_context;
    use super::push_output;

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
    pub fn markdown_to_html(contents: StoreObject) -> rquickjs::Result<String> {
        // SAFETY: we are in a javascript context
        let contents = unsafe { contents.contents_as_string()? };

        Ok(comrak::markdown_to_html_with_plugins(
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
        ))
    }

    #[rquickjs::function]
    pub fn minify_html(contents: StoreObject) -> rquickjs::Result<StoreObject> {
        // SAFETY: we are in a javascript context
        let ctx = unsafe { &*get_context()? };
        let contents = unsafe { contents.contents_as_string()? };
        let cfg = minify_html::Cfg {
            keep_closing_tags: true,
            keep_comments: true,
            keep_html_and_head_opening_tags: true,
            minify_css: true,
            minify_js: true,
            ..Default::default()
        };
        let output = minify_html::minify(contents.as_bytes(), &cfg);
        let object = ctx.db.objects.store(output);
        Ok(StoreObject { object })
    }

    #[rquickjs::function]
    pub fn write_output(name: String, contents: StoreObject) -> rquickjs::Result<()> {
        let path = PathBuf::from(name);
        if !path
            .components()
            .all(|component| matches!(component, Component::CurDir | Component::Normal(_)))
        {
            // Don't allow any path traversal outside the output directory
            return Err(error_message(&format!(
                "directory traversal {}",
                path.display()
            )));
        }
        unsafe {
            push_output(WriteOutput {
                path,
                // SAFETY: being provided a StoreObject always means we've put it in the store
                // already
                object: contents.object,
            })?
        };
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
    extensions: Vec<String>,
}

impl Default for MemoizedScriptLoader {
    fn default() -> Self {
        Self {
            extensions: vec!["js".into()],
        }
    }
}

impl rquickjs::loader::Loader for MemoizedScriptLoader {
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
        let object = ReadFile(path)
            .query(ctx)
            .map_err(|err| rquickjs::Error::new_loading_message(name, format!("{err}")))?;
        let source = ctx.db.objects.get(&object).expect("missing object");

        rquickjs::Module::declare(js_ctx.clone(), name, source.as_ref())
    }
}

thread_local! {
    static RUNTIME: RefCell<Runtime> = RefCell::new({
        let resolver = (
            BuiltinResolver::default()
                .with_module("io")
                .with_module("memoized"),
            FileResolver::default(),
        );
        let loader = (
            ModuleLoader::default()
                .with_module("io", js_io)
                .with_module("memoized", js_memoized),
            MemoizedScriptLoader::default(),
        );

        let runtime = Runtime::new().expect("not enough memory?");
        runtime.set_loader(resolver, loader);
        runtime
    });

    static CONTEXT: RefCell<Context> = RefCell::new({
        RUNTIME.with_borrow(|runtime| {
            // TODO: it seems rquickjs isn't happy if it doesn't have the full set of features. Not
            // that there's any IO features in here besides those we allow it, but ah well
            Context::full(runtime)
                .expect("context failed to build")
        })
    });

    // The current Ctx object that we are executing in, if any. Use with_js_ctx() to re-entrantly
    // access this.
    static CTX: RefCell<Option<std::ptr::NonNull<rquickjs::qjs::JSContext>>> = const { RefCell::new(None) };
}

fn with_js_ctx<T>(f: impl FnOnce(&rquickjs::Ctx<'_>) -> T) -> T {
    let maybe_ctx = CTX.replace(None);
    match maybe_ctx {
        Some(ctx) => {
            CTX.set(Some(ctx));
            // SAFETY: there is some larger `context.with` that we are borrowing from
            let ctx = unsafe { rquickjs::Ctx::from_raw(ctx) };
            f(&ctx)
        }
        None => CONTEXT.with_borrow(|context| {
            context.with(|ctx| {
                CTX.set(Some(ctx.as_raw()));
                let out = f(&ctx);
                CTX.set(None);
                out
            })
        }),
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
    fn produce(&self, ctx: &QueryContext) -> Self::Output {
        let name = self.file.display().to_string();
        println!(
            "running {}({})",
            name,
            self.args
                .as_ref()
                .map(|args| args.to_string())
                .unwrap_or_default()
        );
        let object = ReadFile(self.file.clone()).query(ctx)?;
        let contents = {
            // Need to shorten the lifetime of our read from the database so that we don't deadlock
            // trying to read from the map multiple times
            let contents = ctx.db.objects.get(&object).expect("missing object");
            String::from_utf8(contents.as_ref().to_vec())?
        };

        let (value, outputs) = with_query_context(self.file.clone(), ctx, || {
            with_js_ctx(|ctx| -> crate::Result<_> {
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
                            Some(args) => args.clone().into_js(ctx).unwrap(),
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
                            } else if let Ok(value) = RustValue::from_js(ctx, value.clone()) {
                                eprintln!("js thrown value: {}", value);
                            } else {
                                eprintln!("js error: {:?}", value);
                            }
                            crate::Error::from(rquickjs::Error::Exception)
                        }
                        otherwise => crate::Error::from(otherwise),
                    }
                };

                let module = rquickjs::Module::declare(ctx.clone(), name.clone(), contents)
                    .map_err(catch)?;
                let (module, promise) = module.eval().map_err(catch)?;
                promise.finish::<()>().map_err(catch)?;

                let value: RustValue = module.get(rquickjs::atom::PredefinedAtom::Default)?;
                Ok(value)
            })
        })
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
