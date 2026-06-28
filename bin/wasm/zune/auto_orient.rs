/*
 * Copyright (c) 2023.
 *
 * This software is free software;
 *
 * You can redistribute it or modify it under terms of the MIT, Apache License or Zlib license
 */

//! Perform auto orientation of the image
//!
//! This is a replacement for [`zune_imageprocs::auto_orient::AutoOrient`] until it uses a
//! non-broken Rotate implementation internally.

use super::rotate::Rotate;
use zune_core::bit_depth::BitType;
use zune_core::log::warn;
use zune_image::errors::ImageErrors;
use zune_image::image::Image;
use zune_image::traits::OperationsTrait;
use zune_imageprocs::flip::{Flip, FlipDirection};
use zune_imageprocs::transpose::Transpose;

/// Auto orient the image based on the exif metadata
///
/// This operation is a no-op if `metadata` feature is not specified
/// in the crate level docs
///
/// This operation is also a no-op if the image does not have
/// exif metadata
///
/// If orientation is applied, it will also modify the exif tag to indicate
/// the image was oriented
pub struct AutoOrient;

/// Corresponds to the EXIF orientation values. Each of these describe the transformation that needs
/// to be done to the raw pixel data to display the image. All rotations are **counterclockwise**.
#[derive(Copy, Clone)]
pub enum Orientation {
    Horizontal = 1,
    MirrorHorizontal = 2,
    Rotate180 = 3,
    MirrorVertical = 4,
    MirrorHorizontalRotate270 = 5,
    Rotate90 = 6,
    MirrorHorizontalRotate90 = 7,
    Rotate270 = 8,
}

impl Orientation {
    pub fn parse_from_exif<'a>(data: impl IntoIterator<Item = &'a exif::Field>) -> Option<Self> {
        for field in data.into_iter() {
            // look for the orientation tag
            if field.tag != exif::Tag::Orientation {
                continue;
            }
            if let exif::Value::Short(bytes) = &field.value {
                if bytes.is_empty() {
                    warn!("The exif value is empty, cannot orient");
                    return None;
                }
                return Some(match bytes[0] {
                    1 => Orientation::Horizontal,
                    2 => Orientation::MirrorHorizontal,
                    3 => Orientation::Rotate180,
                    4 => Orientation::MirrorVertical,
                    5 => Orientation::MirrorHorizontalRotate270,
                    6 => Orientation::Rotate90,
                    7 => Orientation::MirrorHorizontalRotate90,
                    8 => Orientation::Rotate270,

                    _ => {
                        warn!(
                            "Unknown exif orientation tag {:?}, ignoring it",
                            &field.value
                        );
                        return None;
                    }
                });
            } else {
                warn!("Invalid exif orientation type, ignoring it");
            }
        }

        None
    }

    pub fn swaps_dims(&self) -> bool {
        match self {
            Orientation::Horizontal => false,
            Orientation::MirrorHorizontal => false,
            Orientation::Rotate180 => false,
            Orientation::MirrorVertical => false,
            Orientation::MirrorHorizontalRotate270 => true,
            Orientation::Rotate90 => true,
            Orientation::MirrorHorizontalRotate90 => true,
            Orientation::Rotate270 => true,
        }
    }
}

impl OperationsTrait for AutoOrient {
    fn name(&self) -> &'static str {
        "Auto orient"
    }

    fn execute_impl(&self, image: &mut Image) -> Result<(), ImageErrors> {
        // check if we have exif orientation metadata and transform it
        // to be this orientation

        if let Some(data) = image.metadata().clone().exif() {
            let orientation = match Orientation::parse_from_exif(data) {
                Some(orientation) => orientation,
                None => return Ok(()),
            };
            match orientation {
                Orientation::Horizontal => {}
                Orientation::MirrorHorizontal => {

                    // Flip::new(FlipDirection::Horizontal).execute(image)?;
                }

                Orientation::Rotate180 => {
                    Rotate::A180.execute(image)?;
                }

                Orientation::MirrorVertical => {
                    Flip::new(FlipDirection::Vertical).execute(image)?;
                }

                Orientation::MirrorHorizontalRotate270 => {
                    Transpose::new().execute_impl(image)?;
                }

                Orientation::Rotate90 => {
                    Rotate::A90.execute(image)?;
                }

                Orientation::MirrorHorizontalRotate90 => {
                    Rotate::A270.execute(image)?;
                    Flip::new(FlipDirection::Horizontal).execute(image)?;
                }

                Orientation::Rotate270 => {
                    Rotate::A270.execute(image)?;
                }
            }
        }

        // update exif
        if let Some(data) = image.metadata_mut().exif_mut() {
            for field in data {
                // set orientation to do nothing
                if field.tag == exif::Tag::Orientation {
                    field.value = exif::Value::Byte(vec![1]);
                }
            }
        }
        Ok(())
    }

    fn supported_types(&self) -> &'static [BitType] {
        &[BitType::U16, BitType::U8, BitType::F32]
    }
}
