use std::{cell::RefCell, path::PathBuf};

use rquickjs::{
    Context, IntoJs, Module, Runtime, Value,
    loader::{BuiltinResolver, ModuleLoader},
};

use crate::{
    files::ReadFile,
    js::value::RustValue,
    query::context::{Producer, QueryContext},
};

mod value;

struct ContextFrame {
    curr: *const QueryContext,
    prev: Option<Box<ContextFrame>>,
}

thread_local! {
    static QUERY_CONTEXT: RefCell<Option<ContextFrame>> = const { RefCell::new(None) };
}

/// NOTE: ONLY SAFE IF SINGLE-THREADED
///
/// Runs a closure with a QueryContext pushed onto the stack. All calls to `get_context()` that run
/// as a result of that closure will access this ctx object. Therefore, all pointer accesses from
/// `get_context()` have safety ensured as a result of running in this function.
fn with_query_context<T>(ctx: &QueryContext, f: impl FnOnce() -> T) -> T {
    let prev = QUERY_CONTEXT.take().map(Box::new);
    let curr = ctx as *const _;
    let new_frame = ContextFrame { curr, prev };
    QUERY_CONTEXT.set(Some(new_frame));

    let out = f();

    let popped = QUERY_CONTEXT.take().expect("popped nothing");
    assert!(std::ptr::eq(curr, popped.curr));
    QUERY_CONTEXT.set(popped.prev.map(|x| *x));

    out
}

/// Only safe to dereference the returned pointer if running inside a call to `with_context()`.
fn get_context() -> rquickjs::Result<*const QueryContext> {
    QUERY_CONTEXT.with_borrow(|ctx| -> rquickjs::Result<_> {
        let ctx = ctx.as_ref().ok_or(rquickjs::Error::Unknown)?;
        Ok(ctx.curr)
    })
}

#[rquickjs::module]
mod memoized {
    use std::path::PathBuf;

    use super::get_context;
    use super::value::RustValue;
    use crate::{
        files::{ListDirectory, ReadFile},
        js::RunFile,
        query::context::Producer,
    };

    #[rquickjs::function]
    pub fn read_file(filename: String) -> rquickjs::Result<Vec<u8>> {
        let ctx = unsafe { &*get_context()? };
        let contents = ReadFile(PathBuf::from(filename))
            .query(ctx)
            .map_err(|_| rquickjs::Error::Exception)?;
        Ok(contents)
    }

    #[rquickjs::function]
    pub fn list_directory(dirname: String) -> rquickjs::Result<Vec<String>> {
        let ctx = unsafe { &*get_context()? };
        let contents = ListDirectory(PathBuf::from(dirname))
            .query(ctx)
            .map_err(|_| rquickjs::Error::Exception)?
            .into_iter()
            .map(|entry| entry.display().to_string())
            .collect();
        Ok(contents)
    }

    #[rquickjs::function]
    pub fn run(filename: String, arg: String) -> rquickjs::Result<()> {
        let ctx = unsafe { &*get_context()? };
        // TODO: it looks like we can't be re-entrant here. rquickjs only wants a single "Ctx<'_>"
        // around at once, because it doesn't like us holding the lock for longer than we have to.
        // Unfortunately I do want to be re-entrant with it, so I'll have to look into other
        // solutions. Like holding the lock by force.
        RunFile {
            file: PathBuf::from(filename),
            args: Some(RustValue::Array(vec![RustValue::String(arg)])),
        }
        .query(ctx)
        .map_err(|_| rquickjs::Error::Exception)?;

        Ok(())
    }
}

#[rquickjs::module]
mod io {
    use std::path::PathBuf;

    #[rquickjs::function]
    pub fn file_type(entry_name: String) -> rquickjs::Result<String> {
        let metadata =
            std::fs::metadata(PathBuf::from(entry_name)).map_err(|_| rquickjs::Error::Exception)?;

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
    type Output = crate::Result<()>;
    fn produce(&self, ctx: &QueryContext) -> Self::Output {
        let name = self.file.display().to_string();
        println!("running {name}");
        let contents = ReadFile(self.file.clone()).query(ctx)?;
        let contents = String::from_utf8(contents)?;

        with_query_context(ctx, || -> crate::Result<_> {
            CONTEXT.with_borrow(|js_ctx| -> crate::Result<_> {
                js_ctx.with(|js_ctx| -> crate::Result<_> {
                    let globals = js_ctx.globals();
                    globals
                        .set(
                            "print",
                            rquickjs::Function::new(js_ctx.clone(), |msg: String| {
                                println!("{msg}")
                            })
                            .unwrap()
                            .with_name("print")
                            .unwrap(),
                        )
                        .unwrap();

                    globals
                        .set(
                            "ARGS",
                            match &self.args {
                                Some(args) => args.clone().into_js(&js_ctx).unwrap(),
                                None => Value::new_undefined(js_ctx.clone()),
                            },
                        )
                        .unwrap();
                    Module::evaluate(js_ctx.clone(), "example", contents)?.finish::<()>()?;
                    Ok(())
                })
            })
        })?;

        println!("done running");
        Ok(())
    }
}
