use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::task::Poll;

use futures_lite::future;
use relative_path::{RelativePath, RelativePathBuf};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use tera::Tera;

use crate::engine::db::Object;
use crate::engine::{Producer, QueryContext, Queryable};
use crate::query::js::{JsObject, JsValue, WriteOutputs};
use crate::query::{ListDirectory, ReadFile, RunFile};
use crate::query_key;
use crate::to_hash::ToHash;

query_key!(RunTemplate { pub file: PathBuf, pub arg: JsValue });

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateOutput {
    pub value: Object,
    pub outputs: WriteOutputs,
}

impl ToHash for TemplateOutput {
    fn run_hash(&self, hasher: &mut sha2::Sha256) {
        hasher.update(b"TemplateOutput(");
        self.value.run_hash(hasher);
        hasher.update(b")(");
        self.outputs.run_hash(hasher);
        hasher.update(b")");
    }
}

enum State<T> {
    NotStarted,
    Running,
    Complete(T),
}

impl Producer for RunTemplate {
    type Output = crate::Result<TemplateOutput>;

    #[tracing::instrument(level = "debug", skip(ctx))]
    async fn produce(&self, ctx: &QueryContext) -> Self::Output {
        println!("run_tera(\"{}\", {})", self.file.display(), self.arg);
        let input = ReadFile(self.file.clone()).query(ctx).await?;
        render_tera_async(ctx, &input, &self.file, &self.arg).await
    }
}

fn render_tera_async(
    ctx: &QueryContext,
    input: &Object,
    file: &Path,
    arg: &JsValue,
) -> impl Future<Output = crate::Result<TemplateOutput>> {
    // The main reason why I can't just block is because the blocking tera template will itself
    // call back into the async executor wanting other threads to complete, with potential for
    // deadlock if all such threads are occupied by tera template renders. So instead, I need
    // to spawn a thread for each template, which isn't ideal.
    // I'll be on the lookout for if tera will support async filters eventually...
    let state = Arc::new(Mutex::new(
        State::<crate::Result<TemplateOutput>>::NotStarted,
    ));
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

fn render_tera(
    ctx: &QueryContext,
    input: &Object,
    file: &Path,
    arg: &JsValue,
) -> crate::Result<TemplateOutput> {
    let input = input.contents_as_string(ctx)?;
    let outputs = Arc::new(Mutex::new(WriteOutputs::new()));

    let output = {
        let mut tera = Tera::default();
        let key = format!("{}({})", file.display(), arg);
        register_functions(&mut tera, ctx, key, outputs.clone());

        let name = format!("{}", file.display());
        tera.add_raw_template(&name, &input)?;
        let context = js_to_tera_context(arg)?;
        tera.render(&name, &context)?
    };
    let outputs = Arc::into_inner(outputs)
        .expect("should be finished rendering")
        .into_inner()
        .expect("mutex is poisoned");
    let object = ctx.db().objects.store(output.into_bytes())?;

    Ok(TemplateOutput {
        value: object,
        outputs,
    })
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

fn register_functions(
    tera: &mut Tera,
    ctx: &QueryContext,
    key: String,
    outputs: Arc<Mutex<WriteOutputs>>,
) {
    macro_rules! wrap_function {
        (move ($key:ident, $($i:ident),*) |$args:ident| $body:tt) => {{
            let key = $key.clone();
            $(
                let $i = $i.clone();
            )*
            move |$args: &HashMap<String, tera::Value>| -> tera::Result<tera::Value> {
                let out = $body;
                if let Err(ref e) = out {
                    eprintln!("error templating {key}:\n\t{e}");
                }
                out
            }
        }};
    }

    macro_rules! wrap_filter {
        (move ($key:ident, $($i:ident),*) |$arg:ident, $args:ident| $body:tt) => {{
            let key = $key.clone();
            $(
                let $i = $i.clone();
            )*
            move |$arg: &tera::Value,
                  $args: &HashMap<String, tera::Value>|
                  -> tera::Result<tera::Value> {
                let out = $body;
                if let Err(ref e) = out {
                    eprintln!("error templating {key}:\n\t{e}");
                }
                out
            }
        }};
    }

    tera.register_function(
        "read",
        wrap_function!(move(key, ctx) |args| {
            get_arg!(file: as_str <- args);
            let file = resolve_path(file)?;
            let object = future::block_on(ReadFile(file).query(&ctx)).map_err(|e| e.to_string())?;
                js_to_tera_value(&JsValue::Store(JsObject { object }))
        }),
    );

    tera.register_function(
        "list",
        wrap_function!(move(key, ctx) |args| {
                get_arg!(dir: as_str <- args);
                let dir = resolve_path(dir)?;
                let files =
                    future::block_on(ListDirectory(dir).query(&ctx)).map_err(|e| e.to_string())?;
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
        wrap_function!(move(key,) |args| {
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
        wrap_function!(move(key, ctx, outputs) |args| {
            get_arg!(file: as_str <- args);
            let file = resolve_path(file)?;
            let arg = tera_to_js_context(args, "file")?;

            let output = future::block_on(
                RunFile {
                    file: file.clone(),
                    arg: arg.clone(),
                }
                .query(&ctx),
            )
            .map_err(|e| {
                let e = format!("error running {}({}):\n\t{}", file.display(), arg, e);
                eprintln!("{e}");
                e
            })?;
            {
                outputs.lock().unwrap().extend(output.outputs);
            }
            let output = js_to_tera_value(&output.value)?;

            Ok(output)
        }),
    );

    tera.register_function(
        "run_tera",
        wrap_function!(move(key, ctx, outputs) |args| {
            get_arg!(template: as_str <- args);
            let file = resolve_path(template)?;
            let arg = tera_to_js_context(args, "template")?;

            let output = future::block_on(
                RunTemplate {
                    file: file.clone(),
                    arg: arg.clone(),
                }
                .query(&ctx),
            )
            .map_err(|e| format!("error templating {}({}):\n\t{}", file.display(), arg, e))?;
            {
                outputs.lock().unwrap().extend(output.outputs);
            };
            let output = js_to_tera_value(&JsValue::Store(JsObject { object: output.value }))?;

            Ok(output)
        }),
    );

    tera.register_filter(
        "store",
        wrap_filter!(move(key, ctx) |arg, _args| {
            let tera::Value::String(s) = arg else {
                return Err("store must take in str".into());
            };
            let object = ctx
                .db()
                .objects
                .store(s.clone().into_bytes())
                .map_err(|e| e.to_string())?;
            js_to_tera_value(&JsValue::Store(JsObject { object }))
        }),
    );

    tera.register_filter(
        "unstore",
        wrap_filter!(move(key, ctx) |arg, _args| {
            let tera::Value::Object(obj) = arg else {
                return Err("unstore must take in object".into());
            };
            let object = tera_to_js_store_object(obj)?;
            let output = object.contents_as_string(&ctx).map_err(|e| e.to_string())?;
            Ok(tera::Value::String(output))
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

fn js_to_tera_context(value: &JsValue) -> crate::Result<tera::Context> {
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
        _ => Err(crate::Error::new("template arg must be object")),
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
                tera::to_value(s.object.clone())?,
            );
            tera::Value::Object(map)
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
            Ok(object) => JsValue::Store(JsObject { object }),
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

fn tera_to_js_store_object(obj: &tera::Map<String, tera::Value>) -> tera::Result<Object> {
    match obj.get(STORE_OBJECT_MAGIC) {
        Some(hash) => {
            let hash = tera::from_value(hash.clone())?;
            // SAFETY: we have no choice but to trust this. The user has purposefully messed us
            // up otherwise, worst case we will find there is no backing file.
            let object = unsafe { Object::from_hash(hash) };
            Ok(object)
        }
        None => Err("not a store object".into()),
    }
}
