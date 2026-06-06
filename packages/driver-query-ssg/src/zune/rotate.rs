/*
 * Copyright (c) 2023.
 *
 * This software is free software;
 *
 * You can redistribute it or modify it under terms of the MIT, Apache License or Zlib license
 */
//! Rotate an image
//!
//! # WARNING
//!
//! This only works for rotating in 90-degree increments. It will not support any other sort of rotation.
//!
//! This is a simpler implementation of [`zune_imageprocs::rotate::Rotate`], which is necessary
//! until it also gets support for 90-degree rotations.

use zune_core::bit_depth::BitType;
use zune_image::channel::Channel;
use zune_image::errors::ImageErrors;
use zune_image::image::Image;
use zune_image::traits::OperationsTrait;
use zune_imageprocs::traits::NumOps;

/// Represents a clockwise angle of rotation, in degrees.
#[derive(Copy, Clone)]
pub enum Rotate {
    A90,
    A180,
    A270,
}

impl Rotate {
    fn get_rotated_dimensions(&self, (width, height): (usize, usize)) -> (usize, usize) {
        match self {
            Rotate::A90 => (height, width),
            Rotate::A180 => (width, height),
            Rotate::A270 => (height, width),
        }
    }

    fn change_image_dims(&self, image: &mut Image) {
        let (new_width, new_height) = self.get_rotated_dimensions(image.dimensions());
        image.set_dimensions(new_width, new_height)
    }
}

impl OperationsTrait for Rotate {
    fn name(&self) -> &'static str {
        "Rotate"
    }

    fn execute_impl(&self, image: &mut Image) -> Result<(), ImageErrors> {
        let im_type = image.depth().bit_type();

        let (width, height) = image.dimensions();

        let will_change_dims = matches!(self, Rotate::A90 | Rotate::A270);

        let resize_fn = |channel: &mut Channel| -> Result<(), ImageErrors> {
            let (new_width, new_height) = self.get_rotated_dimensions((width, height));

            let mut new_channel =
                Channel::new_with_length_and_type(new_width * new_height, channel.type_id());

            match im_type {
                BitType::U8 => {
                    self.rotate::<u8>(
                        width,
                        height,
                        channel.reinterpret_as()?,
                        new_channel.reinterpret_as_mut()?,
                    );
                }
                BitType::U16 => {
                    self.rotate::<u16>(
                        width,
                        height,
                        channel.reinterpret_as()?,
                        new_channel.reinterpret_as_mut()?,
                    );
                }
                BitType::F32 => self.rotate::<f32>(
                    width,
                    height,
                    channel.reinterpret_as()?,
                    new_channel.reinterpret_as_mut()?,
                ),
                d => return Err(ImageErrors::ImageOperationNotImplemented(self.name(), d)),
            };
            *channel = new_channel;
            Ok(())
        };
        execute_on(resize_fn, image, false)?;

        if will_change_dims {
            self.change_image_dims(image);
        }

        Ok(())
    }

    fn supported_types(&self) -> &'static [BitType] {
        &[BitType::U8, BitType::U16, BitType::F32]
    }
}

pub fn execute_on<T: Fn(&mut Channel) -> Result<(), ImageErrors> + Send + Sync>(
    function: T,
    image: &mut Image,
    ignore_alpha: bool,
) -> Result<(), ImageErrors> {
    for channel in image.channels_mut(ignore_alpha) {
        function(channel)?;
    }
    Ok(())
}

impl Rotate {
    fn rotate<T: Copy + NumOps<T> + Default>(
        &self,
        width: usize,
        height: usize,
        in_image: &[T],
        out_image: &mut [T],
    ) {
        match self {
            Rotate::A90 => {
                rotate_90(in_image, out_image, width, height);
            }
            Rotate::A180 => {
                out_image.copy_from_slice(in_image);
                rotate_180(out_image, width);
            }
            Rotate::A270 => {
                rotate_270(in_image, out_image, width, height);
            }
        }
    }
}

fn rotate_180<T: Copy>(in_out_image: &mut [T], width: usize) {
    let half = in_out_image.len() / 2;
    let (top, bottom) = in_out_image.split_at_mut(half);

    for (top_chunk, bottom_chunk) in top
        .chunks_exact_mut(width)
        .zip(bottom.chunks_exact_mut(width).rev())
    {
        for (a, b) in top_chunk.iter_mut().zip(bottom_chunk.iter_mut()) {
            core::mem::swap(a, b);
        }
    }
}

fn rotate_90<T: Copy>(in_image: &[T], out_image: &mut [T], width: usize, height: usize) {
    // TODO: [cae]: Use loop tiling.
    // Does not matter that it is already good enough, we need it fast.
    for (y, pixels) in in_image.chunks_exact(width).enumerate() {
        let idx = height - y - 1;

        for (x, pix) in pixels.iter().enumerate() {
            if let Some(c) = out_image.get_mut((x * height) + idx) {
                *c = *pix;
            }
        }
    }
}

fn rotate_270<T: Copy>(in_image: &[T], out_image: &mut [T], width: usize, height: usize) {
    for (y, pixels) in in_image.chunks_exact(width).enumerate() {
        for (x, pix) in pixels.iter().enumerate() {
            let y_idx = (width - x - 1) * height;
            if let Some(c) = out_image.get_mut(y_idx + y) {
                *c = *pix;
            }
        }
    }
}
