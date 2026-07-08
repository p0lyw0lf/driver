use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::task::Poll;

use futures_lite::future;
use relative_path::{RelativePath, RelativePathBuf};
use serde::{Deserialize, Serialize};
use tera::{Tera, TeraResult};

use driver_engine::{Blob, query};
use driver_query_fs::{ListDirectory, ReadFile};
use driver_util::WriteOutput;

use crate::QueryContext;
use crate::boa::{JsBlob, JsValue, RunJs};

driver_engine::key!(
    #[input=|_| false]
    struct RunTera {
        pub file: PathBuf,
        pub arg: JsValue,
    }
);
driver_engine::blob_trace!(RunTera => { arg });

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunTeraOutput {
    pub export: driver_util::Result<Blob>,
    pub writes: WriteOutput,
}
driver_engine::blob_trace!(RunTeraOutput => {
    export,
    writes,
});

driver_engine::producer!(RunTera(self, ctx) as (crate::QueryKey) -> RunTeraOutput {
    println!("run_tera(\"{}\", {})", self.file.display(), self.arg);
    let input = match query(ctx, ReadFile(self.file.clone())).await {
        Ok(input) => input,
        Err(e) => return RunTeraOutput {
            export: Err(e),
            writes: Default::default(),
        },
    };
    render_tera_async(ctx, &input, &self.file, &self.arg).await
});

impl std::fmt::Display for RunTera {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "run_tera(\"{}\", {})", self.file.display(), self.arg)
    }
}

enum State<T> {
    NotStarted,
    Running,
    Complete(T),
}

fn render_tera_async(
    ctx: &QueryContext,
    input: &Blob,
    file: &Path,
    arg: &JsValue,
) -> impl Future<Output = RunTeraOutput> {
    // The main reason why I can't just block is because the blocking tera template will itself
    // call back into the async executor wanting other threads to complete, with potential for
    // deadlock if all such threads are occupied by tera template renders. So instead, I need
    // to spawn a thread for each template, which isn't ideal.
    // I'll be on the lookout for if tera will support async filters eventually...
    let state = Arc::new(Mutex::new(State::<RunTeraOutput>::NotStarted));
    future::poll_fn(move |poll_ctx| {
        // First, check if we are already running/finished. This ensures we only spawn one
        // thread of computation.
        {
            let mut state = state.lock().unwrap();
            match &*state {
                State::NotStarted => {
                    *state = State::Running;
                }
                State::Running => {
                    return Poll::Pending;
                }
                State::Complete(output) => {
                    return Poll::Ready(output.clone());
                }
            }
        }

        // If we passed the check, spawn the thread
        let ctx = ctx.clone();
        let input = input.clone();
        let file = PathBuf::from(file);
        let arg = arg.clone();
        let state = state.clone();
        let waker = poll_ctx.waker().clone();
        std::thread::spawn(move || {
            let output = render_tera(&ctx, &input, &file, &arg);
            {
                *state.lock().unwrap() = State::Complete(output);
            }
            waker.wake();
        });
        Poll::Pending
    })
}

fn render_tera(ctx: &QueryContext, input: &Blob, file: &Path, arg: &JsValue) -> RunTeraOutput {
    let writes = Arc::new(Mutex::new(WriteOutput::default()));

    let export = (|| -> driver_util::Result<_> {
        let input = ctx.load_string(input)?;

        let mut tera = Tera::default();
        register_functions(&mut tera, ctx, writes.clone());

        let name = format!("{}", file.display());
        tera.add_raw_template(&name, &input)?;
        let context = js_to_tera_context(arg)?;
        let output = tera.render(&name, &context)?;

        ctx.store(output.into_bytes())
    })();

    let writes = Arc::into_inner(writes)
        .expect("should be finished rendering")
        .into_inner()
        .expect("mutex is poisoned");

    RunTeraOutput { export, writes }
}

fn register_functions(tera: &mut Tera, ctx: &QueryContext, writes: Arc<Mutex<WriteOutput>>) {
    macro_rules! wrap_function {
        (move ($($i:ident),*) |$args:ident| $body:tt) => {{
            $(
                let $i = $i.clone();
            )*
            move |$args: tera::Kwargs, _state: &tera::State<'_>| -> TeraResult<tera::Value> {
                $body
            }
        }};
    }

    macro_rules! wrap_filter {
        (move ($($i:ident),*) |$arg:ident: $ty:ty| $body:tt) => {{
            $(
                let $i = $i.clone();
            )*
            move |$arg: $ty, _args, _state: &tera::State<'_>| -> TeraResult<tera::Value> {
                $body
            }
        }};
    }

    tera.register_function(
        "read",
        wrap_function!(move(ctx) |args| {
            let file: &str = args.must_get("file")?;
            let file = resolve_path(file)?;
            let read_file = ReadFile(file);
            let blob = future::block_on(query(&ctx, read_file.clone())).map_err(|e| tera::Error::message(format!("{read_file}: {e}")))?;
            js_to_tera_value(&JsValue::Store(JsBlob { blob }))
        }),
    );

    tera.register_function(
        "list",
        wrap_function!(move(ctx) |args| {
            let dir: &str = args.must_get("dir")?;
            let dir = resolve_path(dir)?;
            let list_directory = ListDirectory(dir);
            let files =
                future::block_on(query(&ctx, list_directory.clone())).map_err(|e| tera::Error::message(format!("{list_directory}: {e}")))?;
            Ok(files
                .into_iter()
                .map(|f| format!("{}", f.display()))
                .collect::<Vec<String>>()
                .into())
        }),
    );

    tera.register_function(
        "file_type",
        wrap_function!(move() |args| {
            let entry: &str = args.must_get("entry")?;
            let entry = resolve_path(entry)?;

            let metadata = std::fs::metadata(entry)?;

            Ok(if metadata.is_file() {
                "file"
            } else if metadata.is_dir() {
                "dir"
            } else if metadata.is_symlink() {
                "symlink"
            } else {
                "unknown"
            }
            .into())
        }),
    );

    tera.register_function(
        "run_js",
        wrap_function!(move(ctx, writes) |args| {
            let file: &str = args.must_get("file")?;
            let file = resolve_path(file)?;
            let arg = tera_to_js_context(args, "file")?;

            let run_js = RunJs {
                file: file.clone(),
                arg: arg.clone(),
            };
            let output = future::block_on(
                query(&ctx, run_js.clone())
            );
            {
                writes.lock().unwrap().merge(output.writes);
            }
            let output = js_to_tera_value(
                &output.export
                .map_err(|e| tera::Error::message(format!("{run_js}:\n\t{e}")))?
            )?;

            Ok(output)
        }),
    );

    tera.register_function(
        "run_tera",
        wrap_function!(move(ctx, writes) |args| {
            let template: &str = args.must_get("template")?;
            let file = resolve_path(template)?;
            let arg = tera_to_js_context(args, "template")?;

            let run_tera = RunTera {
                file: file.clone(),
                arg: arg.clone(),
            };
            let output = future::block_on(query(&ctx, run_tera.clone()));
            {
                writes.lock().unwrap().merge(output.writes);
            };
            let output = js_to_tera_value(&JsValue::Store(JsBlob {
                blob: output.export.map_err(|e| tera::Error::message(format!("{run_tera}:\n\t{e}")))?,
            }))?;

            Ok(output)
        }),
    );

    tera.register_filter(
        "store",
        wrap_filter!(move(ctx) |arg: &str| {
            let blob = ctx
                .store(String::from(arg).into_bytes())
                .map_err(tera::Error::message)?;
            js_to_tera_value(&JsValue::Store(JsBlob { blob }))
        }),
    );

    tera.register_filter(
        "unstore",
        wrap_filter!(move(ctx) |arg: &tera::Map| {
            let blob = tera_to_js_store_object(arg)?;
            let output = ctx.load_string(&blob).map_err(tera::Error::message)?;
            Ok(output.into())
        }),
    );

    tera.register_function(
        "zip",
        wrap_function!(move() |args| {
            let fst: &[tera::Value] = args.must_get("fst")?;
            let snd: &[tera::Value] = args.must_get("snd")?;

            #[derive(Clone, Debug, Serialize)]
            struct Tuple {
                fst: tera::Value,
                snd: tera::Value,
            }

            let output = fst.iter().cloned()
                .zip(snd.iter().cloned())
                .map(|(fst, snd)| Tuple { fst, snd })
                .collect::<Vec<_>>();
            tera::Value::try_from_serializable(&output)
        }),
    );
}

/// Resolves a path to normalized relative to the cwd
fn resolve_path(path: &str) -> TeraResult<PathBuf> {
    Ok(RelativePathBuf::from_path(".")
        .map_err(tera::Error::message)?
        .join_normalized(RelativePath::new(path))
        .to_path("."))
}

fn js_to_tera_context(value: &JsValue) -> driver_util::Result<tera::Context> {
    match value {
        JsValue::Object(obj) => {
            let mut ctx = tera::Context::new();
            for (key, value) in obj.iter() {
                let value = js_to_tera_value(value)?;
                ctx.insert(key.clone(), &value);
            }

            Ok(ctx)
        }
        JsValue::Null | JsValue::Undefined => Ok(tera::Context::default()),
        _ => Err(driver_util::Error::new("template arg must be object")),
    }
}

fn tera_to_js_context(args: tera::Kwargs, ignore: &str) -> TeraResult<JsValue> {
    // TODO: this double-serialization is unavoidable because tera doesn't give us a way to iterate
    // over the input args...
    let mut output_args = BTreeMap::new();
    for (key, value) in args.iter() {
        let Some(key) = key.as_str() else {
            continue;
        };
        if key == ignore {
            continue;
        }
        let value = tera_to_js_value(value)?;
        let _ = output_args.insert(key.to_string(), value);
    }
    Ok(if output_args.is_empty() {
        JsValue::Null
    } else if output_args.len() == 1
        && let Some(value) = output_args.get("arg")
    {
        value.clone()
    } else {
        JsValue::Object(output_args)
    })
}

const STORE_OBJECT_MAGIC: &str = "__store_object";

fn js_to_tera_value(value: &JsValue) -> TeraResult<tera::Value> {
    Ok(match value {
        JsValue::Undefined => tera::Value::undefined(),
        JsValue::Null => tera::Value::none(),
        JsValue::Bool(b) => (*b).into(),
        JsValue::Int(i) => (*i).into(),
        JsValue::String(s) => s.to_string().into(),
        JsValue::Array(arr) => arr
            .iter()
            .map(js_to_tera_value)
            .collect::<Result<Vec<_>, _>>()?
            .as_slice()
            .into(),
        JsValue::Object(obj) => obj
            .iter()
            .map(|(key, value)| -> TeraResult<_> { Ok((key.clone(), js_to_tera_value(value)?)) })
            .collect::<Result<HashMap<_, _>, _>>()?
            .into(),
        JsValue::Store(s) => {
            let mut map = tera::Map::new();
            let hash: &[u8] = s.blob.as_ref();
            map.insert(STORE_OBJECT_MAGIC.into(), hash.into());
            map.into()
        }
        JsValue::Image(_) => {
            unimplemented!("support passing image objects to tera")
        }
    })
}

fn tera_to_js_value(value: &tera::Value) -> TeraResult<JsValue> {
    Ok(if value.is_undefined() {
        JsValue::Undefined
    } else if value.is_none() {
        JsValue::Null
    } else if let Some(b) = value.as_bool() {
        JsValue::Bool(b)
    } else if let Some(i) = value.as_i64() {
        JsValue::Int(i as i32)
    } else if let Some(s) = value.as_str() {
        JsValue::String(s.to_string())
    } else if let Some(arr) = value.as_array() {
        JsValue::Array(arr.iter().map(tera_to_js_value).collect::<Result<_, _>>()?)
    } else if let Some(obj) = value.as_map() {
        match tera_to_js_store_object(obj) {
            Ok(blob) => JsValue::Store(JsBlob { blob }),
            Err(_) => JsValue::Object(
                obj.iter()
                    .filter_map(|(key, value)| -> Option<TeraResult<_>> {
                        let key = key.as_str()?.to_string();
                        let value = tera_to_js_value(value);
                        Some(value.map(|value| (key, value)))
                    })
                    .collect::<Result<_, _>>()?,
            ),
        }
    } else {
        return Err(tera::Error::message("unsupported argument type"));
    })
}

fn tera_to_js_store_object(obj: &tera::Map) -> TeraResult<Blob> {
    let hash = obj
        .get(&STORE_OBJECT_MAGIC.into())
        .ok_or_else(|| tera::Error::message("not a store object"))?;
    let hash = hash
        .as_bytes()
        .ok_or_else(|| tera::Error::message("magic value wasn't bytes"))?
        .try_into()
        .map_err(|_| tera::Error::message("invalid byte length"))?;

    // SAFETY: we have no choice but to trust this. The user has purposefully messed us
    // up otherwise, worst case we will find there is no backing file.
    let blob = unsafe { Blob::from_hash(hash) };
    Ok(blob)
}
