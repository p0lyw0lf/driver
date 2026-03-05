use std::fmt::Display;

use serde::Deserialize;
use serde::Serialize;

use zune_core::bytestream::ZCursor;
use zune_core::options::DecoderOptions;
use zune_image::image::Image;
use zune_image::traits::OperationsTrait;

use crate::{
    db::object::Object,
    query::context::{Producer, QueryContext},
    query_key,
};

#[derive(Default, Hash, PartialEq, Eq, PartialOrd, Ord, Clone, Debug, Serialize, Deserialize)]
pub enum ImageFormat {
    Jpeg,
    Jxl,
    Png,
    #[default]
    Webp,
}

#[derive(Hash, PartialEq, Eq, PartialOrd, Ord, Clone, Debug, Serialize, Deserialize)]
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

/// https://developer.mozilla.org/en-US/docs/Web/CSS/Reference/Properties/object-fit
#[derive(Default, Hash, PartialEq, Eq, PartialOrd, Ord, Clone, Debug, Serialize, Deserialize)]
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
        pub input: Object,
        /// If None, will preserve the dimensions of the source image.
        pub size: Option<ImageSize>,
        /// If None, will use ImageFit::Contain
        pub fit: Option<ImageFit>,
        /// If None, will preserve the format of the source image if possible, defaulting to Webp
        /// if not.
        pub format: Option<ImageFormat>,
    }
);

impl Producer for ConvertImage {
    type Output = crate::Result<Object>;

    #[tracing::instrument(level = "trace", skip_all)]
    async fn produce(&self, ctx: &QueryContext) -> Self::Output {
        // NOTE: I know that reading this into memory only to read it into more memory is wasteful,
        // but this is the best way to not have a dependency on ctx while we do the main
        // processing.

        let contents = self.input.contents_as_bytes(ctx)?;

        let size = self.size.clone();
        let fit = self.fit.clone().unwrap_or_default();
        let format = self.format.clone().unwrap_or_default();

        let output = ctx
            .rt
            .spawn_blocking(move || -> crate::Result<_> {
                let mut image = Image::read(ZCursor::new(contents), DecoderOptions::new_fast())?;

                let (source_width, source_height) = image.dimensions();
                let (target_width, target_height) = size
                    .as_ref()
                    .map(ImageSize::as_dimensions)
                    .unwrap_or((source_width, source_height));
                let (dest_width, dest_height) = match fit {
                    ImageFit::Fill => (target_width, target_height),
                    ImageFit::Contain => (
                        std::cmp::min(source_width, source_width * source_height / target_height),
                        std::cmp::min(source_height, source_height * source_width / target_width),
                    ),
                    ImageFit::Cover => (
                        std::cmp::max(source_width, source_width * source_height / target_height),
                        std::cmp::max(source_height, source_height * source_width / target_width),
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

                Ok(image.write_to_vec(format.into())?)
            })
            .await??;

        let object = ctx.db.objects.store(output);
        Ok(object)
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
            ImageFit::Fill => "\"fill\"",
            ImageFit::Contain => "\"contain\"",
            ImageFit::Cover => "\"cover\"",
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
            ImageFormat::Jpeg => "\"jpeg\"",
            ImageFormat::Jxl => "\"jxl\"",
            ImageFormat::Png => "\"png\"",
            ImageFormat::Webp => "\"webp\"",
        })
    }
}
