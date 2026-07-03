use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::task::Poll;

use futures_lite::future;
use relative_path::{RelativePath, RelativePathBuf};
use serde::{Deserialize, Serialize};
use tera::Tera;

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

macro_rules! get_arg {
    ($name:ident <- $args: expr) => {
        let $name = $args.get(stringify!($name)).ok_or_else(|| {
            tera::Error::from(concat!(
                "\"",
                stringify!($name),
                "\" parameter must be specified"
            ))
        })?;
    };
    ($name:ident : $fn:ident <- $args: expr) => {
        let $name = $args
            .get(stringify!($name))
            .ok_or_else(|| {
                tera::Error::from(concat!(
                    "\"",
                    stringify!($name),
                    "\" parameter must be specified"
                ))
            })?
            .$fn()
            .ok_or_else(|| {
                tera::Error::from(concat!("\"", stringify!($name), "\" must be a string"))
            })?;
    };
}

fn register_functions(tera: &mut Tera, ctx: &QueryContext, writes: Arc<Mutex<WriteOutput>>) {
    macro_rules! wrap_function {
        (move ($($i:ident),*) |$args:ident| $body:tt) => {{
            $(
                let $i = $i.clone();
            )*
            move |$args: &HashMap<String, tera::Value>| -> tera::Result<tera::Value> {
                $body
            }
        }};
    }

    macro_rules! wrap_filter {
        (move ($($i:ident),*) |$arg:ident, $args:ident| $body:tt) => {{
            $(
                let $i = $i.clone();
            )*
            move |$arg: &tera::Value,
                  $args: &HashMap<String, tera::Value>|
                  -> tera::Result<tera::Value> {
                $body
            }
        }};
    }

    tera.register_function(
        "read",
        wrap_function!(move(ctx) |args| {
            get_arg!(file: as_str <- args);
            let file = resolve_path(file)?;
            let read_file = ReadFile(file);
            let blob = future::block_on(query(&ctx, read_file.clone())).map_err(|e| format!("{read_file}: {e}"))?;
            js_to_tera_value(&JsValue::Store(JsBlob { blob }))
        }),
    );

    tera.register_function(
        "list",
        wrap_function!(move(ctx) |args| {
                get_arg!(dir: as_str <- args);
                let dir = resolve_path(dir)?;
                let list_directory = ListDirectory(dir);
                let files =
                    future::block_on(query(&ctx, list_directory.clone())).map_err(|e| format!("{list_directory}: {e}"))?;
                Ok(files
                    .into_iter()
                    .map(|f| format!("{}", f.display()))
                    .collect::<Vec<String>>()
                    .into())
            }
        ),
    );

    tera.register_function(
        "file_type",
        wrap_function!(move() |args| {
            get_arg!(entry: as_str <- args);
            let entry = resolve_path(entry)?;

            let metadata = std::fs::metadata(entry).map_err(|e| e.to_string())?;

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
            get_arg!(file: as_str <- args);
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
                .map_err(|e| format!("{run_js}:\n\t{e}"))?
            )?;

            Ok(output)
        }),
    );

    tera.register_function(
        "run_tera",
        wrap_function!(move(ctx, writes) |args| {
            get_arg!(template: as_str <- args);
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
                blob: output.export.map_err(|e| format!("{run_tera}:\n\t{e}"))?,
            }))?;

            Ok(output)
        }),
    );

    tera.register_filter(
        "store",
        wrap_filter!(move(ctx) |arg, _args| {
            let tera::Value::String(s) = arg else {
                return Err("store must take in str".into());
            };
            let blob = ctx
                .store(s.clone().into_bytes())
                .map_err(|e| e.to_string())?;
            js_to_tera_value(&JsValue::Store(JsBlob { blob }))
        }),
    );

    tera.register_filter(
        "unstore",
        wrap_filter!(move(ctx) |arg, _args| {
            let tera::Value::Object(obj) = arg else {
                return Err("unstore must take in object".into());
            };
            let blob = tera_to_js_store_object(obj)?;
            let output = ctx.load_string(&blob).map_err(|e| e.to_string())?;
            Ok(tera::Value::String(output))
        }),
    );

    tera.register_function(
        "zip",
        wrap_function!(move() |args| {
            get_arg!(fst: as_array <- args);
            get_arg!(snd: as_array <- args);

            #[derive(Clone, Debug, Serialize, Deserialize)]
            struct Tuple {
                fst: tera::Value,
                snd: tera::Value,
            }

            let output = fst.iter().cloned()
                .zip(snd.iter().cloned())
                .map(|(fst, snd)| serde_json::json!(Tuple { fst, snd }))
                .collect();
            Ok(tera::Value::Array(output))
        }),
    );
}

/// Resolves a path to normalized relative to the cwd
fn resolve_path(path: &str) -> tera::Result<PathBuf> {
    Ok(RelativePathBuf::from_path(".")
        .map_err(|e| e.to_string())?
        .join_normalized(RelativePath::new(path))
        .to_path("."))
}

fn js_to_tera_context(value: &JsValue) -> driver_util::Result<tera::Context> {
    match value {
        JsValue::Object(obj) => {
            let mut ctx = tera::Context::new();
            for (key, value) in obj.iter() {
                let value = js_to_tera_value(value)?;
                ctx.insert(key, &value);
            }

            Ok(ctx)
        }
        JsValue::Null | JsValue::Undefined => Ok(tera::Context::default()),
        _ => Err(driver_util::Error::new("template arg must be object")),
    }
}

fn tera_to_js_context(args: &HashMap<String, tera::Value>, ignore: &str) -> tera::Result<JsValue> {
    let mut arg = BTreeMap::new();
    for (key, value) in args.iter() {
        if key == ignore {
            continue;
        }
        let value = tera_to_js_value(value)?;
        let _ = arg.insert(key.clone(), value);
    }
    Ok(if arg.is_empty() {
        JsValue::Null
    } else if arg.len() == 1
        && let Some(value) = arg.get("arg")
    {
        value.clone()
    } else {
        JsValue::Object(arg)
    })
}

const STORE_OBJECT_MAGIC: &str = "__store_object";

fn js_to_tera_value(value: &JsValue) -> tera::Result<tera::Value> {
    Ok(match value {
        JsValue::Undefined => tera::Value::Null,
        JsValue::Null => tera::Value::Null,
        JsValue::Bool(b) => tera::Value::Bool(*b),
        JsValue::Int(i) => tera::Value::Number((*i).into()),
        JsValue::String(s) => tera::Value::String(s.clone()),
        JsValue::Array(arr) => {
            tera::Value::Array(arr.iter().map(js_to_tera_value).collect::<Result<_, _>>()?)
        }
        JsValue::Object(obj) => tera::Value::Object(
            obj.iter()
                .map(|(key, value)| -> tera::Result<_> {
                    Ok((key.clone(), js_to_tera_value(value)?))
                })
                .collect::<Result<_, _>>()?,
        ),
        JsValue::Store(s) => {
            let mut map = tera::Map::new();
            map.insert(
                STORE_OBJECT_MAGIC.to_string(),
                tera::to_value(s.blob.clone())?,
            );
            tera::Value::Object(map)
        }
        JsValue::Image(_) => {
            todo!("support passing image objects to tera")
        }
    })
}

fn tera_to_js_value(value: &tera::Value) -> tera::Result<JsValue> {
    Ok(match value {
        tera::Value::Null => JsValue::Null,
        tera::Value::Bool(b) => JsValue::Bool(*b),
        tera::Value::Number(number) => JsValue::Int(
            number
                .as_i64()
                .ok_or("can only take i32")?
                .try_into()
                .map_err(|_| "can only take i32")?,
        ),
        tera::Value::String(s) => JsValue::String(s.clone()),
        tera::Value::Array(arr) => {
            JsValue::Array(arr.iter().map(tera_to_js_value).collect::<Result<_, _>>()?)
        }
        tera::Value::Object(obj) => match tera_to_js_store_object(obj) {
            Ok(blob) => JsValue::Store(JsBlob { blob }),
            Err(_) => JsValue::Object(
                obj.into_iter()
                    .map(|(key, value)| -> tera::Result<_> {
                        Ok((key.clone(), tera_to_js_value(value)?))
                    })
                    .collect::<Result<_, _>>()?,
            ),
        },
    })
}

fn tera_to_js_store_object(obj: &tera::Map<String, tera::Value>) -> tera::Result<Blob> {
    match obj.get(STORE_OBJECT_MAGIC) {
        Some(hash) => {
            let hash = tera::from_value(hash.clone())?;
            // SAFETY: we have no choice but to trust this. The user has purposefully messed us
            // up otherwise, worst case we will find there is no backing file.
            let blob = unsafe { Blob::from_hash(hash) };
            Ok(blob)
        }
        None => Err("not a store object".into()),
    }
}
