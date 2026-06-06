use std::fmt::Display;

use serde::{Deserialize, Serialize};
use zune_core::bytestream::ZCursor;
use zune_core::options::DecoderOptions;
use zune_image::codecs::{
    jpeg::JpegDecoder, jpeg_xl::JxlDecoder, png::PngDecoder, webp::ZuneWebpDecoder,
};
use zune_image::traits::{DecoderTrait, OperationsTrait};

use driver_engine::Object;

mod auto_orient;
mod rotate;

#[derive(
    Default, Hash, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Debug, Serialize, Deserialize,
)]
pub enum ImageFormat {
    Jpeg,
    Jxl,
    Png,
    #[default]
    Webp,
}

#[derive(Hash, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Debug, Serialize, Deserialize)]
pub struct ImageSize {
    pub width: usize,
    pub height: usize,
}

impl ImageSize {
    /// Returns (width, height)
    fn as_dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }
}

/// Parsed data about an image, so that we can access cruicial information about it without having
/// to re-parse the headers.
#[derive(Hash, PartialEq, Eq, PartialOrd, Ord, Clone, Debug, Serialize, Deserialize)]
pub struct ImageObject {
    pub object: Object,
    pub format: ImageFormat,
    pub size: ImageSize,
}

driver_engine::key!(
    #[input=|_| false]
    struct ParseImage(pub Object);
);

driver_engine::producer!(ParseImage(self, ctx) -> driver_util::Result<ImageObject> {
    let contents = ZCursor::new(ctx.load_bytes(&self.0)?);
    let object = self.0.clone();

    let metadata = zune_image::utils::decode_info(contents)
        .ok_or_else(|| driver_util::Error::new("could not parse image metadata"))?;

    let format = match metadata.image_format() {
        Some(zune_image::codecs::ImageFormat::JPEG) => ImageFormat::Jpeg,
        Some(zune_image::codecs::ImageFormat::JPEG_XL) => ImageFormat::Jxl,
        Some(zune_image::codecs::ImageFormat::PNG) => ImageFormat::Png,
        Some(zune_image::codecs::ImageFormat::WEBP) => ImageFormat::Webp,
        Some(other) => {
            return Err(driver_util::Error::new(&format!(
                "invalid image format {other:?}"
            )));
        }
        None => return Err(driver_util::Error::new("could not get image format")),
    };

    let (width, height) = metadata.dimensions();
    let (width, height) = if
        let Some(data) = metadata.exif() &&
        let Some(orientation) = auto_orient::Orientation::parse_from_exif(data) &&
        orientation.swaps_dims() {
        (height, width)
    } else {
        (width, height)
    };

    Ok(ImageObject {
        object,
        format,
        size: ImageSize { width, height },
    })
});

impl Display for ParseImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "parse_image({})", self.0)
    }
}

/// <https://developer.mozilla.org/en-US/docs/Web/CSS/Reference/Properties/object-fit>
#[derive(
    Default, Hash, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Debug, Serialize, Deserialize,
)]
pub enum ImageFit {
    /// scales both dimensions to match exactly.
    Fill,
    /// scales preserving aspect ratio to fit the entire image within the bounds.
    #[default]
    Contain,
    /// scales preserving aspect ratio to cover the entire bounds.
    Cover,
}

/// Struct that sets things for [`zune_core::options::EncoderOptions`] in a serializable way.
#[derive(Hash, PartialEq, Eq, PartialOrd, Ord, Clone, Debug, Serialize, Deserialize)]
pub struct EncoderOptions {
    /// Clamped from 0-100. Interpretation differs per-codec.
    pub quality: u8,
    /// Interpretation differs per-codec.
    pub effort: u8,
    /// Whether to not preserve metadata across image transformations
    pub strip_metadata: bool,
}

impl Default for EncoderOptions {
    fn default() -> Self {
        Self {
            quality: 80,
            effort: 4,
            strip_metadata: true,
        }
    }
}

impl From<EncoderOptions> for zune_core::options::EncoderOptions {
    fn from(value: EncoderOptions) -> Self {
        zune_core::options::EncoderOptions::default()
            .set_quality(value.quality)
            .set_effort(value.effort)
            .set_strip_metadata(value.strip_metadata)
    }
}

/// Enum for supported [`zune_imageprocs::resize::ResizeMethod`]s
#[derive(
    Default, Hash, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Debug, Serialize, Deserialize,
)]
pub enum ResizeMethod {
    /// Lanczos with a=3 (highest quality, slowest)
    #[default]
    Lanczos3,
    /// Lanczos with a=2
    Lanczos2,
    /// Bicubic interpolation (Mitchell-Netravali, B=1/3, C=1/3)
    Bicubic,
    /// B-Spline (B=1, C=0)
    BSpline,
    /// Hermite filter (B=0, C=0)
    Hermite,
    /// Sinc with window radius 3
    Sinc,
    /// Bilinear (for completeness, 2x2 kernel)
    Bilinear,
}

impl From<ResizeMethod> for zune_imageprocs::resize::ResizeMethod {
    fn from(value: ResizeMethod) -> Self {
        match value {
            ResizeMethod::Lanczos3 => zune_imageprocs::resize::ResizeMethod::Lanczos3,
            ResizeMethod::Lanczos2 => zune_imageprocs::resize::ResizeMethod::Lanczos2,
            ResizeMethod::Bicubic => zune_imageprocs::resize::ResizeMethod::Bicubic,
            ResizeMethod::BSpline => zune_imageprocs::resize::ResizeMethod::BSpline,
            ResizeMethod::Hermite => zune_imageprocs::resize::ResizeMethod::Hermite,
            ResizeMethod::Sinc => zune_imageprocs::resize::ResizeMethod::Sinc,
            ResizeMethod::Bilinear => zune_imageprocs::resize::ResizeMethod::Bilinear,
        }
    }
}

driver_engine::key!(
    #[input=|_| false]
    struct ConvertImage {
        pub input: ImageObject,
        /// If None, will preserve the format of the source image if possible, defaulting to Webp
        /// if not.
        pub format: Option<ImageFormat>,
        /// If None, will preserve the dimensions of the source image.
        pub size: Option<ImageSize>,
        /// If None, will use ImageFit::Contain
        pub fit: Option<ImageFit>,
        /// If None, will use Zune's defaults
        pub encoder_options: Option<EncoderOptions>,
        /// If None, will use Lanczos3
        pub resize_method: Option<ResizeMethod>,
    }
);

driver_engine::producer!(ConvertImage(self, ctx) -> driver_util::Result<ImageObject> {
    // NOTE: I know that reading this into memory only to read it into more memory is wasteful,
    // but this is the best way to not have a dependency on ctx while we do the main
    // processing.

    let input_format = self.input.format;
    let (source_width, source_height) = self.input.size.as_dimensions();
    let input_contents = ZCursor::new(ctx.load_bytes(&self.input.object)?);
    let decoder_options = DecoderOptions::new_fast();

    let size = self.size;
    let fit = self.fit.unwrap_or_default();
    let format = self.format.unwrap_or_default();

    let (target_width, target_height) = size
        .as_ref()
        .map(ImageSize::as_dimensions)
        .unwrap_or((source_width, source_height));
    let (dest_width, dest_height) = match fit {
        ImageFit::Fill => (target_width, target_height),
        ImageFit::Contain => (
            std::cmp::min(target_width, source_width * target_height / source_height),
            std::cmp::min(target_height, source_height * target_width / source_width),
        ),
        ImageFit::Cover => (
            std::cmp::max(target_width, source_width * target_height / source_height),
            std::cmp::max(target_height, source_height * target_width / source_width),
        ),
    };

    let changed_size = source_width != dest_width || source_height != dest_height;
    let changed_format = input_format != format;
    let changed_encode = self.encoder_options.is_some();

    if !changed_size && !changed_format && !changed_encode {
        // If the image didn't change, just return the original image again
        return Ok(self.input.clone());
    }

    let mut image = match input_format {
        ImageFormat::Jpeg => {
            DecoderTrait::decode(&mut JpegDecoder::new_with_options(
                input_contents,
                decoder_options,
            ))?
        },
        ImageFormat::Jxl => {
            DecoderTrait::decode(&mut JxlDecoder::try_new(input_contents, decoder_options)?)?
        }
        ImageFormat::Png => DecoderTrait::decode(&mut PngDecoder::new_with_options(
            input_contents,
            decoder_options,
        ))?,
        ImageFormat::Webp => DecoderTrait::decode(&mut ZuneWebpDecoder::new(input_contents)?)?,
    };

    // Always auto-orient images; there seems to be some problems writing to image formats not
    // containing exif metadata unless we do this.
    auto_orient::AutoOrient.execute(&mut image)?;

    if image.dimensions() != (source_width, source_height) {
        panic!("corrupted image dimensions");
    }

    if changed_size {
        let resize_op = zune_imageprocs::resize::Resize::new(
            dest_width,
            dest_height,
            self.resize_method.unwrap_or_default().into(),
        );
        resize_op.execute(&mut image)?;
    }

    let object = {
        let mut sink = vec![];
        let format: zune_image::codecs::ImageFormat = format.into();
        format.encode(&image, self.encoder_options.clone().unwrap_or_default().into(), &mut sink)?;
        sink
    };
    let object = ctx.store(object)?;

    Ok(ImageObject {
        object,
        format,
        size: ImageSize {
            width: dest_width,
            height: dest_height,
        },
    })
});

impl Display for ImageSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{ width: {}, height: {} }}", self.width, self.height)
    }
}

impl Display for ImageFit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ImageFit::Fill => "fill",
            ImageFit::Contain => "contain",
            ImageFit::Cover => "cover",
        })
    }
}

impl From<ImageFormat> for zune_image::codecs::ImageFormat {
    fn from(value: ImageFormat) -> Self {
        match value {
            ImageFormat::Jpeg => Self::JPEG,
            ImageFormat::Jxl => Self::JPEG_XL,
            ImageFormat::Png => Self::PNG,
            ImageFormat::Webp => Self::WEBP,
        }
    }
}

impl Display for ImageFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ImageFormat::Jpeg => "jpeg",
            ImageFormat::Jxl => "jxl",
            ImageFormat::Png => "png",
            ImageFormat::Webp => "webp",
        })
    }
}

impl Display for ImageObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{{ object: {}, format: \"{}\", size: {} }}",
            self.object, self.format, self.size,
        )
    }
}

impl Display for EncoderOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{{ quality: {}, effort: {}, strip_metadata: {} }}",
            self.quality, self.effort, self.strip_metadata
        )
    }
}

impl Display for ResizeMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ResizeMethod::Lanczos3 => "lanczos3",
            ResizeMethod::Lanczos2 => "lanczos2",
            ResizeMethod::Bicubic => "bicubic",
            ResizeMethod::BSpline => "bspline",
            ResizeMethod::Hermite => "hermite",
            ResizeMethod::Sinc => "sinc",
            ResizeMethod::Bilinear => "bilinear",
        })
    }
}

impl Display for ConvertImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("convert_image(")?;
        Display::fmt(&self.input, f)?;
        f.write_str(", {")?;

        let mut had_some = false;
        let mut prefix = |f: &mut std::fmt::Formatter<'_>| {
            let out = f.write_str(if had_some { ", " } else { " " });
            had_some = true;
            out
        };

        if let Some(format) = self.format.as_ref() {
            prefix(f)?;
            write!(f, "format: \"{}\"", format)?;
        }
        if let Some(size) = self.size.as_ref() {
            prefix(f)?;
            write!(f, "size: {}", size)?;
        }
        if let Some(fit) = self.fit.as_ref() {
            prefix(f)?;
            write!(f, "fit: \"{}\"", fit)?;
        }
        if let Some(encoder_options) = self.encoder_options.as_ref() {
            prefix(f)?;
            write!(f, "encoder_options: {}", encoder_options)?;
        }
        if let Some(resize_method) = self.resize_method.as_ref() {
            prefix(f)?;
            write!(f, "resize_method: \"{}\"", resize_method)?;
        }

        if had_some {
            f.write_str(" ")?;
        }
        f.write_str("})")
    }
}
