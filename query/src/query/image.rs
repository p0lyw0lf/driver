use std::fmt::Display;

use serde::{Deserialize, Serialize};
use sha2::Digest;
use zune_core::{bytestream::ZCursor, options::DecoderOptions};
use zune_image::codecs::{
    jpeg::JpegDecoder, jpeg_xl::JxlDecoder, png::PngDecoder, webp::ZuneWebpDecoder,
};
use zune_image::traits::{DecoderTrait, OperationsTrait};

use crate::to_hash::ToHash;
use crate::{
    engine::{Producer, QueryContext, db::Object},
    query_key,
};

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

impl ToHash for ImageFormat {
    fn run_hash(&self, hasher: &mut sha2::Sha256) {
        hasher.update(b"ImageFormat(");
        hasher.update([*self as u8]);
        hasher.update(b")");
    }
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

impl ToHash for ImageSize {
    fn run_hash(&self, hasher: &mut sha2::Sha256) {
        hasher.update(b"ImageSize(");
        hasher.update(self.width.to_le_bytes());
        hasher.update(self.height.to_le_bytes());
        hasher.update(b")");
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

impl ToHash for ImageObject {
    fn run_hash(&self, hasher: &mut sha2::Sha256) {
        hasher.update(b"ImageObject(");
        self.object.run_hash(hasher);
        self.format.run_hash(hasher);
        self.size.run_hash(hasher);
        hasher.update(b")");
    }
}

query_key!(ParseImage(pub Object););

impl Producer for ParseImage {
    type Output = crate::Result<ImageObject>;

    async fn produce(&self, ctx: &QueryContext) -> Self::Output {
        let contents = ZCursor::new(self.0.contents_as_bytes(ctx)?);
        let object = self.0.clone();

        let metadata = zune_image::utils::decode_info(contents)
            .ok_or_else(|| crate::Error::new("could not parse image metadata"))?;

        let format = match metadata.image_format() {
            Some(zune_image::codecs::ImageFormat::JPEG) => ImageFormat::Jpeg,
            Some(zune_image::codecs::ImageFormat::JPEG_XL) => ImageFormat::Jxl,
            Some(zune_image::codecs::ImageFormat::PNG) => ImageFormat::Png,
            Some(zune_image::codecs::ImageFormat::WEBP) => ImageFormat::Webp,
            Some(other) => {
                return Err(crate::Error::new(&format!(
                    "invalid image format {other:?}"
                )));
            }
            None => return Err(crate::Error::new("could not get image format")),
        };

        let (width, height) = metadata.dimensions();

        Ok(ImageObject {
            object,
            format,
            size: ImageSize { width, height },
        })
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

query_key!(
    ConvertImage {
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

impl Producer for ConvertImage {
    type Output = crate::Result<ImageObject>;

    #[tracing::instrument(level = "debug", skip_all)]
    async fn produce(&self, ctx: &QueryContext) -> Self::Output {
        // NOTE: I know that reading this into memory only to read it into more memory is wasteful,
        // but this is the best way to not have a dependency on ctx while we do the main
        // processing.

        let input_format = self.input.format;
        let (source_width, source_height) = self.input.size.as_dimensions();
        let input_contents = ZCursor::new(self.input.object.contents_as_bytes(ctx)?);
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

        if target_width != dest_width || target_height != dest_height {
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
        let object = ctx.db().objects.store(object)?;

        Ok(ImageObject {
            object,
            format,
            size: ImageSize {
                width: dest_width,
                height: dest_height,
            },
        })
    }
}

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
