use rquickjs::{Ctx, JsLifetime, class::Trace};
use rquickjs::{FromJs, IntoJs};
use serde::{Deserialize, Serialize};

use crate::js::JsObject;
use crate::query::image::{ImageFit, ImageFormat, ImageObject, ImageSize};

impl<'js> IntoJs<'js> for ImageFormat {
    fn into_js(self, ctx: &Ctx<'js>) -> rquickjs::Result<rquickjs::Value<'js>> {
        self.to_string().into_js(ctx)
    }
}

impl<'js> FromJs<'js> for ImageFormat {
    fn from_js(_ctx: &Ctx<'js>, value: rquickjs::Value<'js>) -> rquickjs::Result<Self> {
        match value
            .as_string()
            .ok_or_else(|| rquickjs::Error::new_from_js(value.type_name(), "ImageFormat"))?
            .to_string()?
            .as_str()
        {
            "jpeg" => Ok(ImageFormat::Jpeg),
            "jpg" => Ok(ImageFormat::Jpeg),
            "jxl" => Ok(ImageFormat::Jxl),
            "jpeg_xl" => Ok(ImageFormat::Jxl),
            "png" => Ok(ImageFormat::Png),
            "webp" => Ok(ImageFormat::Webp),
            _ => Err(rquickjs::Error::new_from_js(
                "invalid string",
                "ImageFormat",
            )),
        }
    }
}

impl<'js> IntoJs<'js> for ImageSize {
    fn into_js(self, ctx: &Ctx<'js>) -> rquickjs::Result<rquickjs::Value<'js>> {
        let obj = rquickjs::Object::new(ctx.clone())?;
        obj.prop("width", self.width)?;
        obj.prop("height", self.height)?;
        Ok(obj.into_value())
    }
}

impl<'js> FromJs<'js> for ImageSize {
    fn from_js(_ctx: &Ctx<'js>, value: rquickjs::Value<'js>) -> rquickjs::Result<Self> {
        let obj = value
            .as_object()
            .ok_or_else(|| rquickjs::Error::new_from_js(value.type_name(), "ImageSize"))?;

        let width = obj.get("width")?;
        let height = obj.get("height")?;

        Ok(ImageSize { width, height })
    }
}

impl<'js> FromJs<'js> for ImageFit {
    fn from_js(_ctx: &Ctx<'js>, value: rquickjs::Value<'js>) -> rquickjs::Result<Self> {
        match value
            .as_string()
            .ok_or_else(|| rquickjs::Error::new_from_js(value.type_name(), "ImageFit"))?
            .to_string()?
            .as_str()
        {
            "fill" => Ok(ImageFit::Fill),
            "contain" => Ok(ImageFit::Contain),
            "cover" => Ok(ImageFit::Cover),
            _ => Err(rquickjs::Error::new_from_js("invalid string", "ImageFit")),
        }
    }
}

#[derive(
    Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Trace, JsLifetime, Serialize, Deserialize,
)]
#[rquickjs::class]
pub struct JsImage {
    #[qjs(skip_trace)]
    pub image: ImageObject,
}

#[rquickjs::methods(rename_all = "camelCase")]
impl JsImage {
    #[qjs(get)]
    fn object(&self) -> JsObject {
        let object = self.image.object.clone();
        JsObject { object }
    }

    #[qjs(get)]
    fn format<'js>(&self, js_ctx: Ctx<'js>) -> rquickjs::Result<rquickjs::Value<'js>> {
        self.image.format.into_js(&js_ctx)
    }

    #[qjs(get)]
    fn size<'js>(&self, js_ctx: Ctx<'js>) -> rquickjs::Result<rquickjs::Value<'js>> {
        self.image.size.into_js(&js_ctx)
    }
}
