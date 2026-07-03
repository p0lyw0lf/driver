use boa_engine::object::builtins::JsUint8Array;
use boa_engine::{JsData, JsNativeError, JsResult};
use boa_gc::{Finalize, GcRef, Trace};
use serde::{Deserialize, Serialize};

use driver_engine::Blob;

use crate::boa::get_context;
use crate::boa::macros::class_wrap;

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
pub struct JsBlob {
    #[unsafe_ignore_trace]
    pub blob: Blob,
}
driver_engine::blob_trace!(JsBlob => { blob });

impl JsBlob {
    /// # Safety
    /// Only safe to call when in a javascript context
    pub unsafe fn contents_as_bytes(self) -> JsResult<Vec<u8>> {
        let ctx = &get_context()?;
        ctx.load_bytes(&self.blob).map_err(|e| {
            JsNativeError::eval()
                .with_message(format!("loading {}: {}", self.blob, e))
                .into()
        })
    }

    /// # Safety
    /// Only safe to call when in a javascript context
    pub unsafe fn contents_as_string(self) -> JsResult<String> {
        let ctx = &get_context()?;
        ctx.load_string(&self.blob).map_err(|e| {
            JsNativeError::eval()
                .with_message(format!("loading {}: {}", self.blob, e))
                .into()
        })
    }
}

class_wrap!(class JsBlob {
    length 0,
    methods {
        data: (0) |this: GcRef<'_, JsBlob>, _args, js_ctx| {
            // SAFETY: we are in a javascript context
            let src = unsafe { this.clone().contents_as_bytes()? };
            JsUint8Array::from_iter(src, js_ctx)
        },
        hash: (0) |this: GcRef<'_, JsBlob>, _args, _js_ctx| {
            JsResult::Ok(this.blob.to_string())
        },
        toString: (0) |this: GcRef<'_, JsBlob>, _args, _js_ctx| {
            // SAFETY: we are in a javascript context
            unsafe { this.clone().contents_as_string() }
        },
    },
});
