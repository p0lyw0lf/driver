use std::{cell::RefCell, path::PathBuf};

use rquickjs::{
    Context, IntoJs, Module, Runtime, Value,
    loader::{BuiltinResolver, ModuleLoader},
};

use crate::{
    js::value::RustValue,
    query::context::{Producer, QueryContext},
    query::files::ReadFile,
};

mod value;

struct ContextFrame {
    curr: *const QueryContext,
    prev: Option<Box<ContextFrame>>,
    task_queue: Vec<RunFile>,
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
    ctx: &QueryContext,
    f: impl FnOnce() -> crate::Result<T>,
) -> crate::Result<(T, Vec<RunFile>)> {
    let prev = QUERY_CONTEXT.take().map(Box::new);
    let curr = ctx as *const _;
    let new_frame = ContextFrame {
        curr,
        prev,
        task_queue: vec![],
    };
    QUERY_CONTEXT.set(Some(new_frame));

    let out = f();

    let popped = QUERY_CONTEXT.take().expect("popped nothing");
    assert!(std::ptr::eq(curr, popped.curr));
    QUERY_CONTEXT.set(popped.prev.map(|x| *x));

    out.map(|t| (t, popped.task_queue))
}

/// Only safe to dereference the returned pointer if running inside a call to `with_context()`.
fn get_context() -> rquickjs::Result<*const QueryContext> {
    QUERY_CONTEXT.with_borrow(|ctx| -> rquickjs::Result<_> {
        let ctx = ctx.as_ref().ok_or(rquickjs::Error::Unknown)?;
        Ok(ctx.curr)
    })
}

/// SAFETY: only safe to call when running inside `with_query_context()`
unsafe fn push_task(task: RunFile) {
    QUERY_CONTEXT.with_borrow_mut(|ctx| {
        ctx.as_mut().map(|ctx| {
            ctx.task_queue.push(task);
        })
    });
}

fn error_message(message: &str) -> rquickjs::Error {
    rquickjs::Error::Io(std::io::Error::other(message))
}

#[rquickjs::module]
mod memoized {
    use rquickjs::Ctx;
    use std::path::PathBuf;

    use super::error_message;
    use super::get_context;
    use super::value::RustValue;
    use crate::{
        js::{RunFile, push_task},
        query::context::Producer,
        query::files::{ListDirectory, ReadFile},
    };

    #[rquickjs::function]
    pub fn read_file(
        js_ctx: Ctx<'_>,
        filename: String,
    ) -> rquickjs::Result<rquickjs::TypedArray<'_, u8>> {
        // SAFETY: the only way these javascript functions get called is from inside a
        // `with_query_context()`
        let ctx = unsafe { &*get_context()? };
        let contents = ReadFile(PathBuf::from(filename))
            .query(ctx)
            .map_err(|e| error_message(&format!("{e}")))?;
        rquickjs::TypedArray::new(js_ctx, contents)
    }

    #[rquickjs::function]
    pub fn list_directory(dirname: String) -> rquickjs::Result<Vec<String>> {
        // SAFETY: the only way these javascript functions get called is from inside a
        // `with_query_context()`
        let ctx = unsafe { &*get_context()? };
        let contents = ListDirectory(PathBuf::from(dirname))
            .query(ctx)
            .map_err(|e| error_message(&format!("{e}")))?
            .into_iter()
            .map(|entry| entry.display().to_string())
            .collect();
        Ok(contents)
    }

    #[rquickjs::function]
    pub fn queue_task(filename: String, args: Option<RustValue>) -> rquickjs::Result<()> {
        // SAFETY: the only way these javascript functions get called is from inside a
        // `with_query_context()`
        unsafe {
            push_task(RunFile {
                file: PathBuf::from(filename),
                args,
            });
        }

        Ok(())
    }
}

#[rquickjs::module]
mod io {
    use std::path::{Component, PathBuf};

    use either::Either;

    use super::error_message;
    use crate::options::OPTIONS;

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
    pub fn write_output(
        name: String,
        contents: Either<String, rquickjs::TypedArray<'_, u8>>,
    ) -> rquickjs::Result<()> {
        let name = PathBuf::from(name);
        if !name
            .components()
            .all(|component| matches!(component, Component::CurDir | Component::Normal(_)))
        {
            // Don't allow any path traversal outside the output directory
            return Err(error_message("directory traversal"));
        }
        let full_name = OPTIONS.read().unwrap().output_dir.join(name);
        let dir = full_name
            .parent()
            .ok_or(error_message("no parent directory"))?;
        std::fs::create_dir_all(dir)?;
        let contents = match &contents {
            Either::Left(s) => s.as_bytes(),
            Either::Right(buf) => buf.as_bytes().ok_or(error_message("detached buffer"))?,
        };
        std::fs::write(full_name, contents)?;
        Ok(())
    }
}

thread_local! {
    static RUNTIME: RefCell<Runtime> = RefCell::new({
        let resolver = (BuiltinResolver::default().with_module("io").with_module("memoized"),);
        let loader = (ModuleLoader::default().with_module("io", js_io).with_module("memoized", js_memoized),);

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
}

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
pub struct RunFile {
    pub file: PathBuf,
    pub args: Option<RustValue>,
}

impl Producer for RunFile {
    type Output = crate::Result<RustValue>;
    fn produce(&self, ctx: &QueryContext) -> Self::Output {
        let name = self.file.display().to_string();
        println!("running {name}");
        let contents = ReadFile(self.file.clone()).query(ctx)?;
        let contents = String::from_utf8(contents)?;

        let (value, tasks) = with_query_context(ctx, || -> crate::Result<_> {
            CONTEXT.with_borrow(|ctx| {
                ctx.with(|ctx| -> crate::Result<_> {
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

                    let module = Module::declare(ctx.clone(), name, contents)?;
                    let (module, promise) = module.eval()?;
                    promise.finish::<()>()?;

                    let value: RustValue = module.get(rquickjs::atom::PredefinedAtom::Default)?;
                    Ok(value)
                })
            })
        })?;

        for task in tasks {
            task.query(ctx)?;
        }

        Ok(value)
    }
}
