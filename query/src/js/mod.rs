use std::{cell::RefCell, path::PathBuf};

use rquickjs::{
    Context, Module, Runtime,
    context::intrinsic,
    loader::{BuiltinResolver, ModuleLoader},
};

use crate::{
    files::ReadFile,
    query::context::{Producer, QueryContext},
};

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
#[allow(non_upper_case_globals)]
mod builtins {
    use std::path::PathBuf;

    use super::get_context;
    use crate::{
        files::{ListDirectory, ReadFile},
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
}

thread_local! {
    static RUNTIME: RefCell<Runtime> = RefCell::new({
        let resolver = (BuiltinResolver::default().with_module("bundle/driver"),);
        let loader = (ModuleLoader::default().with_module("bundle/driver", js_builtins),);

        let runtime = Runtime::new().expect("not enough memory?");
        runtime.set_loader(resolver, loader);
        runtime
    });

    static CONTEXT: RefCell<Context> = RefCell::new({
        RUNTIME.with_borrow(|runtime| {
            Context::builder()
                .with::<intrinsic::BigInt>()
                .with::<intrinsic::Date>()
                .with::<intrinsic::Json>()
                .with::<intrinsic::MapSet>()
                .with::<intrinsic::RegExp>()
                .build(runtime)
                .expect("context failed to build")
        })
    });
}

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
pub struct RunFile(pub PathBuf);

impl Producer for RunFile {
    type Output = crate::Result<()>;
    fn produce(&self, ctx: &QueryContext) -> Self::Output {
        let name = self.0.display().to_string();
        println!("running {name}");
        let contents = ReadFile(self.0.clone()).query(ctx)?;
        let contents = String::from_utf8(contents)?;

        with_query_context(ctx, || -> crate::Result<_> {
            CONTEXT.with_borrow(|js_ctx| -> crate::Result<_> {
                js_ctx.with(|js_ctx| -> crate::Result<_> {
                    Module::evaluate(js_ctx, name, contents)?.finish::<()>()?;
                    Ok(())
                })
            })
        })?;

        Ok(())
    }
}
