use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::task::Poll;

use futures_lite::future;
use relative_path::{RelativePath, RelativePathBuf};
use serde::{Deserialize, Serialize};
use tera::Tera;

use crate::engine::db::Object;
use crate::engine::{Producer, QueryContext, Queryable};
use crate::query::js::JsValue;
use crate::query::{ListDirectory, ReadFile, RunFile};
use crate::query_key;

query_key!(RunTemplate { pub file: PathBuf, pub arg: JsValue });

enum State<T> {
    NotStarted,
    Running,
    Complete(T),
}

impl Producer for RunTemplate {
    type Output = crate::Result<String>;

    async fn produce(&self, ctx: &QueryContext) -> Self::Output {
        let input = ReadFile(self.file.clone()).query(ctx).await?;
        let output = render_tera_async(ctx, &input, &self.file, &self.arg).await?;
        Ok(output)
    }
}

fn render_tera_async(
    ctx: &QueryContext,
    input: &Object,
    file: &Path,
    arg: &JsValue,
) -> impl Future<Output = crate::Result<String>> {
    // The main reason why I can't just block is because the blocking tera template will itself
    // call back into the async executor wanting other threads to complete, with potential for
    // deadlock if all such threads are occupied by tera template renders. So instead, I need
    // to spawn a thread for each template, which isn't ideal.
    // I'll be on the lookout for if tera will support async filters eventually...
    let state = Arc::new(Mutex::new(State::<crate::Result<String>>::NotStarted));
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
) -> crate::Result<String> {
    let input = input.contents_as_string(ctx)?;

    let mut tera = Tera::default();
    register_functions(ctx, &mut tera);

    let name = format!("{}", file.display());
    tera.add_raw_template(&name, &input)?;
    let context = js_to_tera_context(arg)?;
    let output = tera.render(&name, &context)?;

    Ok(output)
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

fn register_functions(ctx: &QueryContext, tera: &mut Tera) {
    tera.register_function("read", {
        let ctx = ctx.clone();
        move |args: &HashMap<String, tera::Value>| -> tera::Result<tera::Value> {
            get_arg!(file: as_str <- args);
            let file = resolve_path(file)?;
            let object = future::block_on(ReadFile(file).query(&ctx)).map_err(|e| e.to_string())?;
            let output = object.contents_as_string(&ctx).map_err(|e| e.to_string())?;
            Ok(output.into())
        }
    });

    tera.register_function("list", {
        let ctx = ctx.clone();
        move |args: &HashMap<String, tera::Value>| -> tera::Result<tera::Value> {
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
    });

    tera.register_function("run_task", {
        let ctx = ctx.clone();
        move |args: &HashMap<String, tera::Value>| -> tera::Result<tera::Value> {
            get_arg!(file: as_str <- args);
            get_arg!(arg <- args);
            let file = resolve_path(file)?;
            let arg = tera_to_js_value(arg)?;

            let output = future::block_on(
                RunFile {
                    file: file.clone(),
                    arg: arg.clone(),
                }
                .query(&ctx),
            )
            .map_err(|e| format!("error running {}({}):\n\t{}", file.display(), arg, e))?;
            let output = js_to_tera_value(&output.value)?;

            Ok(output)
        }
    });

    tera.register_function("run_template", {
        let ctx = ctx.clone();
        move |args: &HashMap<String, tera::Value>| -> tera::Result<tera::Value> {
            get_arg!(template: as_str <- args);
            let file = resolve_path(template)?;

            let mut arg = BTreeMap::new();
            for (key, value) in args.iter() {
                if key == "template" {
                    continue;
                }
                let value = tera_to_js_value(value)?;
                let _ = arg.insert(key.clone(), value);
            }
            let arg = JsValue::Object(arg);

            let output = future::block_on(
                RunTemplate {
                    file: file.clone(),
                    arg: arg.clone(),
                }
                .query(&ctx),
            )
            .map_err(|e| format!("error templating {}({}):\n\t{}", file.display(), arg, e))?;

            Ok(output.into())
        }
    });
}

/// Resolves a path to normalized relative to the cwd
fn resolve_path(path: &str) -> tera::Result<PathBuf> {
    Ok(RelativePathBuf::from_path(".")
        .map_err(|e| e.to_string())?
        .join_normalized(RelativePath::new(path))
        .to_path("."))
}

fn js_to_tera_context(value: &JsValue) -> crate::Result<tera::Context> {
    let JsValue::Object(obj) = value else {
        return Err(crate::Error::new("template arg must be object"));
    };

    let mut ctx = tera::Context::new();
    for (key, value) in obj.iter() {
        ctx.try_insert(key, value)?;
    }

    Ok(ctx)
}

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
        JsValue::Store(_) => return Err("cannot pass StoreObject to tera".into()),
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
        tera::Value::Object(obj) => JsValue::Object(
            obj.into_iter()
                .map(|(key, value)| -> tera::Result<_> {
                    Ok((key.clone(), tera_to_js_value(value)?))
                })
                .collect::<Result<_, _>>()?,
        ),
    })
}
