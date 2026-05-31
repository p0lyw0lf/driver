use boa_engine::{
    Context, JsData, JsError, JsNativeError, JsResult, JsValue, js_object, js_str,
    value::{TryFromJs, TryIntoJs},
};
use boa_gc::{Finalize, GcRef, Trace};
use serde::{Deserialize, Serialize};

use crate::boa::JsObject;
use crate::boa::macros::class_wrap;
use crate::zune::{EncoderOptions, ImageFit, ImageFormat, ImageObject, ImageSize, ResizeMethod};

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

impl TryFromJs for EncoderOptions {
    fn try_from_js(value: &JsValue, context: &mut Context) -> JsResult<Self> {
        let obj = value
            .as_object()
            .ok_or_else(|| JsNativeError::typ().with_message("EncoderOptions must be object"))?;

        let quality = js_str!("quality");
        let quality = if obj.has_property(quality, context)? {
            u8::try_from_js(&obj.get(quality, context)?, context)?
        } else {
            80
        };

        let effort = js_str!("effort");
        let effort = if obj.has_property(effort, context)? {
            u8::try_from_js(&obj.get(effort, context)?, context)?
        } else {
            4
        };

        let strip_metadata = js_str!("strip_metadata");
        let strip_metadata = if obj.has_property(strip_metadata, context)? {
            bool::try_from_js(&obj.get(strip_metadata, context)?, context)?
        } else {
            true
        };

        Ok(EncoderOptions {
            quality,
            effort,
            strip_metadata,
        })
    }
}

impl TryFromJs for ResizeMethod {
    fn try_from_js(value: &JsValue, _js_ctx: &mut Context) -> JsResult<Self> {
        match value
            .as_string()
            .ok_or_else(|| JsNativeError::typ().with_message("ResizeMethod must be string"))?
            .to_std_string()
            .map_err(JsError::from_rust)?
            .as_str()
        {
            "lanczos3" => Ok(ResizeMethod::Lanczos3),
            "lanczos2" => Ok(ResizeMethod::Lanczos2),
            "bicubic" => Ok(ResizeMethod::Bicubic),
            "bspline" => Ok(ResizeMethod::BSpline),
            "hermite" => Ok(ResizeMethod::Hermite),
            "sinc" => Ok(ResizeMethod::Sinc),
            "bilinear" => Ok(ResizeMethod::Bilinear),
            _ => Err(JsNativeError::typ()
                .with_message("Invalid ResizeMethod")
                .into()),
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

class_wrap!(class JsImage {
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
