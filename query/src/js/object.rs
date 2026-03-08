use rquickjs::{Ctx, JsLifetime, atom::PredefinedAtom, class::Trace};
use serde::{Deserialize, Serialize};

use crate::db::object::Object;
use crate::js::get_context;

#[derive(
    Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Trace, JsLifetime, Serialize, Deserialize,
)]
#[rquickjs::class]
pub struct JsObject {
    #[qjs(skip_trace)]
    pub object: Object,
}

impl JsObject {
    /// SAFETY: only safe to call when in a javascript context
    pub unsafe fn contents_as_bytes(self, js_ctx: &Ctx<'_>) -> rquickjs::Result<Vec<u8>> {
        let ctx = &get_context(js_ctx)?;
        self.object.contents_as_bytes(ctx)
    }

    /// SAFETY: only safe to call when in a javascript context
    pub unsafe fn contents_as_string(self, js_ctx: &Ctx<'_>) -> rquickjs::Result<String> {
        let ctx = &get_context(js_ctx)?;
        self.object.contents_as_string(ctx)
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl JsObject {
    #[qjs(get)]
    fn data<'js>(&self, js_ctx: Ctx<'js>) -> rquickjs::Result<rquickjs::TypedArray<'js, u8>> {
        // SAFETY: we are in a javascript context
        let src = unsafe { self.clone().contents_as_bytes(&js_ctx)? };
        rquickjs::TypedArray::new(js_ctx, src)
    }

    #[qjs(get)]
    fn hash(&self) -> String {
        self.object.to_string()
    }

    #[allow(clippy::inherent_to_string)]
    #[qjs(rename = PredefinedAtom::ToString)]
    fn to_string(&self, js_ctx: Ctx<'_>) -> rquickjs::Result<String> {
        // SAFETY: we are in a javascript context
        unsafe { self.clone().contents_as_string(&js_ctx) }
    }
}
