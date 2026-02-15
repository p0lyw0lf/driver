use rquickjs::{Ctx, JsLifetime, atom::PredefinedAtom, class::Trace};
use serde::{Deserialize, Serialize};

use crate::db::object::Object;
use crate::js::get_context;

#[derive(
    Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Trace, JsLifetime, Serialize, Deserialize,
)]
#[rquickjs::class]
pub struct StoreObject {
    #[qjs(skip_trace)]
    pub object: Object,
}

impl StoreObject {
    /// SAFETY: only safe to call when in a javascript context
    pub unsafe fn contents_as_bytes(self) -> rquickjs::Result<Vec<u8>> {
        let ctx = unsafe { &*get_context()? };
        self.object.contents_as_bytes(ctx)
    }

    /// SAFETY: only safe to call when in a javascript context
    pub unsafe fn contents_as_string(self) -> rquickjs::Result<String> {
        let ctx = unsafe { &*get_context()? };
        self.object.contents_as_string(ctx)
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl StoreObject {
    #[qjs(get)]
    fn data<'js>(&self, js_ctx: Ctx<'js>) -> rquickjs::Result<rquickjs::TypedArray<'js, u8>> {
        let src = unsafe { self.clone().contents_as_bytes()? };
        rquickjs::TypedArray::new(js_ctx, src)
    }

    #[allow(clippy::inherent_to_string)]
    #[qjs(rename = PredefinedAtom::ToString)]
    fn to_string(&self) -> rquickjs::Result<String> {
        // SAFETY: we are in a javascript context
        unsafe { self.clone().contents_as_string() }
    }
}
