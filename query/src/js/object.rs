use boa_engine::object::builtins::JsUint8Array;
use boa_engine::value::TryIntoJs;
use boa_engine::{JsData, JsResult};
use boa_gc::{Finalize, GcRef, Trace};
use serde::{Deserialize, Serialize};

use crate::db::object::Object;
use crate::js::get_context;

#[derive(
    Debug,
    Clone,
    Hash,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Trace,
    Finalize,
    JsData,
    Serialize,
    Deserialize,
)]
pub struct JsObject {
    #[unsafe_ignore_trace]
    pub object: Object,
}

impl JsObject {
    /// SAFETY: only safe to call when in a javascript context
    pub unsafe fn contents_as_bytes(self) -> JsResult<Vec<u8>> {
        let ctx = &get_context()?;
        self.object.contents_as_bytes(ctx)
    }

    /// SAFETY: only safe to call when in a javascript context
    pub unsafe fn contents_as_string(self) -> JsResult<String> {
        let ctx = &get_context()?;
        self.object.contents_as_string(ctx)
    }
}

crate::class_wrap!(class JsObject {
    length 0,
    methods {
        data: (0) |this: GcRef<'_, JsObject>, _args, js_ctx| {
            // SAFETY: we are in a javascript context
            let src = unsafe { this.clone().contents_as_bytes()? };
            JsUint8Array::from_iter(src, js_ctx)
        },
        hash: (0) |this: GcRef<'_, JsObject>, _args, _js_ctx| {
            JsResult::Ok(this.object.to_string())
        },
        toString: (0) |this: GcRef<'_, JsObject>, _args, _js_ctx| {
            // SAFETY: we are in a javascript context
            unsafe { this.clone().contents_as_string() }
        },
    },
});
