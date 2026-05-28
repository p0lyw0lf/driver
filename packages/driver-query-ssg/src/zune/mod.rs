use std::fmt::Display;

use serde::{Deserialize, Serialize};
use zune_core::{bytestream::ZCursor, options::DecoderOptions};
use zune_image::codecs::{
    jpeg::JpegDecoder, jpeg_xl::JxlDecoder, png::PngDecoder, webp::ZuneWebpDecoder,
};
use zune_image::traits::{DecoderTrait, OperationsTrait};

use driver_db::Object;

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

/// https://developer.mozilla.org/en-US/docs/Web/CSS/Reference/Properties/object-fit
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

    let mut image = match input_format {
        ImageFormat::Jpeg => DecoderTrait::decode(&mut JpegDecoder::new_with_options(
            input_contents,
            decoder_options,
        ))?,
        ImageFormat::Jxl => {
            DecoderTrait::decode(&mut JxlDecoder::try_new(input_contents, decoder_options)?)?
        }
        ImageFormat::Png => DecoderTrait::decode(&mut PngDecoder::new_with_options(
            input_contents,
            decoder_options,
        ))?,
        ImageFormat::Webp => DecoderTrait::decode(&mut ZuneWebpDecoder::new(input_contents)?)?,
    };

    if image.dimensions() != (source_width, source_height) {
        panic!("corrupted image dimensions");
    }

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

    if source_width != dest_width || source_height != dest_height {
        // TODO: I should probably allow customizing the resize method; however, this
        // is probably fine and seems to give the overall best results.
        let resize_op = zune_imageprocs::resize::Resize::new(
            dest_width,
            dest_height,
            zune_imageprocs::resize::ResizeMethod::Lanczos3,
        );
        resize_op.execute(&mut image)?;
    }

    let object = image.write_to_vec(format.into())?;
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

impl Display for ConvertImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("convert_image(")?;
        Display::fmt(&self.input, f)?;
        f.write_str(", {{")?;

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

        if had_some {
            f.write_str(" ")?;
        }
        f.write_str("}})")
    }
}
