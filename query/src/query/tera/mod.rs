use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::task::Poll;

use futures_lite::future;
use serde::{Deserialize, Serialize};
use tera::Tera;

use crate::engine::db::Object;
use crate::engine::{Producer, QueryContext, Queryable};
use crate::query::ReadFile;
use crate::query::js::JsValue;
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
        let output = render_tera_async(ctx, &input, &self.arg).await?;
        Ok(output)
    }
}

fn render_tera_async(
    ctx: &QueryContext,
    input: &Object,
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
        let arg = arg.clone();
        let state = state.clone();
        let waker = poll_ctx.waker().clone();
        std::thread::spawn(move || {
            let output = render_tera(&ctx, &input, &arg);
            {
                *state.lock().unwrap() = State::Complete(output);
            }
            waker.wake();
        });
        Poll::Pending
    })
}

fn render_tera(ctx: &QueryContext, input: &Object, arg: &JsValue) -> crate::Result<String> {
    let input = input.contents_as_string(ctx)?;

    // TODO: is it worth having a global instance?
    // I think not probably, copying functions over is probably similar to registering them in the
    // first place.
    let mut tera = Tera::default();
    register_functions(&mut tera);

    const ONE_OFF_TEMPLATE: &str = "__tera_one_off";
    tera.add_raw_template(ONE_OFF_TEMPLATE, &input)?;
    let context = js_to_tera(arg)?;
    let output = tera.render(ONE_OFF_TEMPLATE, &context)?;

    Ok(output)
}

fn register_functions(tera: &mut Tera) {
    tera.register_function(
        "read",
        |args: &HashMap<String, tera::Value>| -> tera::Result<tera::Value> {
            // TODO: macro_rules if I have enough of these?
            let file = args
                .get("file")
                .ok_or_else(|| tera::Error::from("\"file\" parameter must be specified"))?
                .as_str()
                .ok_or_else(|| tera::Error::from("\"file\" must be a string"))?;

            todo!()
        },
    );

    tera.register_function(
        "list",
        |args: &HashMap<String, tera::Value>| -> tera::Result<tera::Value> {
            let dir = args
                .get("dir")
                .ok_or_else(|| tera::Error::from("\"dir\" parameter must be specified"))?
                .as_str()
                .ok_or_else(|| tera::Error::from("\"dir\" must be a string"))?;

            todo!()
        },
    );
}

fn js_to_tera(value: &JsValue) -> crate::Result<tera::Context> {
    todo!()
}
