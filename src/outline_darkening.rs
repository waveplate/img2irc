//! Darken narrow dark marks without applying a global brightness adjustment.
use photon_rs::PhotonImage;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, clap::Args)]
#[serde(default)]
#[group(id = "outline-darkening-options")]
pub struct Options {
    /// Darken detected outlines by 0-100%; 100 makes them exactly black.
    #[arg(id = "outline-strength", long = "outline-strength", default_value_t = 100.0, value_parser = crate::args::parse_percentage)]
    pub strength: f32,
    /// Detection radius in source pixels; increase to include thicker outlines.
    #[arg(id = "outline-radius", long = "outline-radius", default_value_t = 3, value_parser = clap::value_parser!(u32).range(1..=64))]
    pub radius: u32,
    /// Maximum outline luminance, 0-100%; lower excludes lighter coloured details.
    #[arg(id = "outline-threshold", long = "outline-threshold", default_value_t = 40.0, value_parser = crate::args::parse_percentage)]
    pub threshold: f32,
    /// Minimum luminance difference from the surrounding fill, 0-100%.
    #[arg(id = "outline-contrast", long = "outline-contrast", default_value_t = 8.0, value_parser = crate::args::parse_percentage)]
    pub contrast: f32,
}

impl Options {
    pub const DEFAULT: Self = Self {
        strength: 100.0,
        radius: 3,
        threshold: 40.0,
        contrast: 8.0,
    };
}

impl Default for Options {
    fn default() -> Self {
        Self::DEFAULT
    }
}

pub fn apply(image: &mut PhotonImage, options: &Options) {
    let (width, height) = (image.get_width() as usize, image.get_height() as usize);
    if width == 0 || height == 0 || !options.strength.is_finite() || options.strength <= 0.0 {
        return;
    }
    let mut pixels = image.get_raw_pixels();
    let luma: Vec<u8> = pixels
        .chunks_exact(4)
        .map(|p| {
            if p[3] == 0 {
                255
            } else {
                ((2126 * u32::from(p[0]) + 7152 * u32::from(p[1]) + 722 * u32::from(p[2]) + 5000)
                    / 10000) as u8
            }
        })
        .collect();
    // Black top-hat: closing fills narrow dark valleys, while broad flat areas
    // retain their luminance. The residual identifies the marks to darken.
    let radius = options.radius.clamp(1, 64) as usize;
    let closed = extreme(
        &extreme(&luma, width, height, radius, true),
        width,
        height,
        radius,
        false,
    );
    let ceiling = options.threshold.clamp(0.0, 100.0) * 2.55;
    let contrast = options.contrast.clamp(0.0, 100.0) * 2.55;
    let retained = 1.0 - options.strength.clamp(0.0, 100.0) / 100.0;
    for ((pixel, &source), filled) in pixels.chunks_exact_mut(4).zip(&luma).zip(closed) {
        let difference = filled.saturating_sub(source);
        if pixel[3] != 0
            && f32::from(source) <= ceiling
            && difference > 0
            && f32::from(difference) >= contrast
        {
            for channel in &mut pixel[..3] {
                *channel = (f32::from(*channel) * retained).round() as u8;
            }
        }
    }
    *image = PhotonImage::new(pixels, width as u32, height as u32);
}

/// Separable sliding min/max. Each index enters/leaves the queue once, keeping
/// detection O(pixels) even for large radii or consecutive video frames.
fn extreme(source: &[u8], width: usize, height: usize, radius: usize, maximum: bool) -> Vec<u8> {
    let mut input = source.to_vec();
    let mut output = vec![0; source.len()];
    let mut queue = VecDeque::<usize>::with_capacity(radius * 2 + 1);
    for vertical in [false, true] {
        let (lines, length, stride) = if vertical {
            (width, height, width)
        } else {
            (height, width, 1)
        };
        for line in 0..lines {
            queue.clear();
            let base = if vertical { line } else { line * width };
            let mut next = 0;
            for position in 0..length {
                let end = position.saturating_add(radius).min(length - 1);
                while next <= end {
                    let value = input[base + next * stride];
                    while queue.back().is_some_and(|&index| {
                        let old = input[base + index * stride];
                        if maximum {
                            old <= value
                        } else {
                            old >= value
                        }
                    }) {
                        queue.pop_back();
                    }
                    queue.push_back(next);
                    next += 1;
                }
                let start = position.saturating_sub(radius);
                while queue.front().is_some_and(|&index| index < start) {
                    queue.pop_front();
                }
                output[base + position * stride] = input[base + queue[0] * stride];
            }
        }
        std::mem::swap(&mut input, &mut output);
    }
    input
}
