use boa_engine::{
    Context, JsData, JsError, JsNativeError, JsResult, JsValue, js_object, js_str,
    value::{TryFromJs, TryIntoJs},
};
use boa_gc::{Finalize, GcRef, Trace};
use serde::{Deserialize, Serialize};

use crate::js::JsObject;
use crate::query::image::{ImageFit, ImageFormat, ImageObject, ImageSize};

impl TryIntoJs for ImageFormat {
    fn try_into_js(&self, context: &mut Context) -> JsResult<JsValue> {
        self.to_string().try_into_js(context)
    }
}

impl TryFromJs for ImageFormat {
    fn try_from_js(value: &JsValue, _js_ctx: &mut Context) -> JsResult<Self> {
        match value
            .as_string()
            .ok_or_else(|| JsNativeError::typ().with_message("ImageFormat must be string"))?
            .to_std_string()
            .map_err(JsError::from_rust)?
            .as_str()
        {
            "jpeg" => Ok(ImageFormat::Jpeg),
            "jpg" => Ok(ImageFormat::Jpeg),
            "jxl" => Ok(ImageFormat::Jxl),
            "jpeg_xl" => Ok(ImageFormat::Jxl),
            "png" => Ok(ImageFormat::Png),
            "webp" => Ok(ImageFormat::Webp),
            _ => Err(JsNativeError::typ()
                .with_message("Invalid ImageFormat")
                .into()),
        }
    }
}

impl TryIntoJs for ImageSize {
    fn try_into_js(&self, context: &mut Context) -> JsResult<JsValue> {
        Ok(js_object!({
            "width": self.width,
            "height": self.height,
        }, context)
        .into())
    }
}

impl TryFromJs for ImageSize {
    fn try_from_js(value: &JsValue, context: &mut Context) -> JsResult<Self> {
        let obj = value
            .as_object()
            .ok_or_else(|| JsNativeError::typ().with_message("ImageSize must be object"))?;

        let width = usize::try_from_js(&obj.get(js_str!("width"), context)?, context)?;
        let height = usize::try_from_js(&obj.get(js_str!("height"), context)?, context)?;

        Ok(ImageSize { width, height })
    }
}

impl TryFromJs for ImageFit {
    fn try_from_js(value: &JsValue, _js_ctx: &mut Context) -> JsResult<Self> {
        match value
            .as_string()
            .ok_or_else(|| JsNativeError::typ().with_message("ImageFit must be string"))?
            .to_std_string()
            .map_err(JsError::from_rust)?
            .as_str()
        {
            "fill" => Ok(ImageFit::Fill),
            "contain" => Ok(ImageFit::Contain),
            "cover" => Ok(ImageFit::Cover),
            _ => Err(JsNativeError::typ().with_message("Invalid ImageFit").into()),
        }
    }
}

#[derive(
    Debug,
    Clone,
    Hash,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    JsData,
    Finalize,
    Trace,
    Serialize,
    Deserialize,
)]
pub struct JsImage {
    #[unsafe_ignore_trace]
    pub image: ImageObject,
}

crate::js::macros::class_wrap!(class JsImage {
    length 0,
    methods {
        object: (0) |this: GcRef<'_, JsImage>, _args, _js_ctx| -> JsResult<_> {
            let object = this.image.object.clone();
            Ok(JsObject { object })
        },
        format: (0) |this: GcRef<'_, JsImage>, _args, js_ctx| {
            this.image.format.try_into_js(js_ctx)
        },
        size: (0) |this: GcRef<'_, JsImage>, _args, js_ctx| {
            this.image.size.try_into_js(js_ctx)
        },
    },
});
