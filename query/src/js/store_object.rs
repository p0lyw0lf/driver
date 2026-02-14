use rquickjs::{Ctx, JsLifetime, atom::PredefinedAtom, class::Trace};
use serde::{Deserialize, Serialize};

use crate::db::object::Object;
use crate::js::get_context;

#[derive(Debug, Clone, Hash, PartialEq, Eq, Trace, JsLifetime, Serialize, Deserialize)]
#[rquickjs::class]
pub struct StoreObject {
    #[qjs(skip_trace)]
    pub object: Object,
}

impl StoreObject {
    /// SAFETY: only safe to call when in a javascript context
    pub unsafe fn contents_as_bytes(self) -> rquickjs::Result<Vec<u8>> {
        let ctx = unsafe { &*get_context()? };
        Ok(ctx
            .db
            .objects
            .get(&self.object)
            .ok_or(rquickjs::Error::new_into_js_message(
                "StoreObject",
                "TypedArray",
                format!("object {} not found", self.object),
            ))?
            .as_ref()
            .iter()
            .map(Clone::clone)
            .collect::<Vec<u8>>())
    }

    /// SAFETY: only safe to call when in a javascript context
    pub unsafe fn contents_as_string(self) -> rquickjs::Result<String> {
        let bytes = unsafe { self.contents_as_bytes()? };
        String::from_utf8(bytes).map_err(|err| {
            rquickjs::Error::new_into_js_message("StoreObject", "String", err.to_string())
        })
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
