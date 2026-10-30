//! Advanced image processing filters and algorithms
//!
//! This module provides a comprehensive set of image processing operations
//! including noise reduction, edge enhancement, contrast adjustment, thresholding,
//! morphological operations, and color transformations.
//!
//! Items here are part of the documented public image-filter toolkit.
//! They are surfaced through crate-root re-exports rather than used
//! internally, so the `dead_code` lint is silenced locally.
#![allow(dead_code)]

use crate::image_processor::ImageData;

// Re-exports of the optional image filter toolkit. These items are surfaced
// to the crate root from `lib.rs`; clippy cannot see through the pub-use
// chain, so the lint is silenced locally.
#[allow(unused_imports)]
pub use self::canny::CannyEdgeDetector;
#[allow(unused_imports)]
pub use self::color_ops::{ColorOps, ColorTemperature, Saturation, HSL, LAB};
#[allow(unused_imports)]
pub use self::contrast::{AdaptiveToneMapper, HistogramEqualizer, CLAHE};
#[allow(unused_imports)]
pub use self::denoising::{NonLocalMeans, PeronaMalik};
#[allow(unused_imports)]
pub use self::effects::{EdgeGlowFilter, EmbossFilter, SepiaFilter, VignetteFilter, VintageFilter};
#[allow(unused_imports)]
pub use self::enhancement::{GammaCorrection, HighBoostFilter, LaplacianSharpen, UnsharpMask};
#[allow(unused_imports)]
pub use self::morphology::{MorphologyKind, MorphologyOp};
#[allow(unused_imports)]
pub use self::segmentation::{
    mean_shift_filter, quantize_from_labels, slic_superpixels, watershed_segment,
};
#[allow(unused_imports)]
pub use self::thresholding::{AdaptiveThreshold, OtsuThreshold, SmartThreshold};

mod canny;
mod color_ops;
mod contrast;
mod denoising;
mod effects;
mod enhancement;
mod morphology;
mod segmentation;
mod thresholding;

/// Apply median filter for salt-and-pepper noise reduction
pub fn median_filter(image_data: &ImageData, radius: u32) -> ImageData {
    let w = image_data.width as usize;
    let h = image_data.height as usize;
    let r = radius as usize;

    let mut output = image_data.pixels.clone();

    for y in 0..h {
        for x in 0..w {
            let mut neighbors = Vec::new();

            let y_start = y.saturating_sub(r);
            let y_end = (y + r + 1).min(h);
            let x_start = x.saturating_sub(r);
            let x_end = (x + r + 1).min(w);

            for ny in y_start..y_end {
                for nx in x_start..x_end {
                    neighbors.push(image_data.pixels[ny * w + nx]);
                }
            }

            neighbors.sort_by_key(|p| ((p.r as u32) << 16) | ((p.g as u32) << 8) | (p.b as u32));
            output[y * w + x] = neighbors[neighbors.len() / 2];
        }
    }

    ImageData {
        width: image_data.width,
        height: image_data.height,
        pixels: output,
    }
}

/// Apply weighted median filter (center pixel has higher weight).
/// Weights follow a 3×3 Gaussian-like pattern: center=4, edge=2, corner=1.
pub fn weighted_median_filter(image_data: &ImageData, radius: u32) -> ImageData {
    let w = image_data.width as usize;
    let h = image_data.height as usize;
    let r = radius as usize;

    let mut output = image_data.pixels.clone();

    for y in 0..h {
        for x in 0..w {
            let mut weighted = Vec::new();

            let y_start = y.saturating_sub(r);
            let y_end = (y + r + 1).min(h);
            let x_start = x.saturating_sub(r);
            let x_end = (x + r + 1).min(w);

            for ny in y_start..y_end {
                for nx in x_start..x_end {
                    let dx = (nx as isize - x as isize).abs();
                    let dy = (ny as isize - y as isize).abs();
                    let weight = if dx == 0 && dy == 0 {
                        4
                    } else if dx <= 1 && dy <= 1 {
                        2
                    } else {
                        1
                    };
                    let p = image_data.pixels[ny * w + nx];
                    for _ in 0..weight {
                        weighted.push(p);
                    }
                }
            }

            weighted.sort_by_key(|p| ((p.r as u32) << 16) | ((p.g as u32) << 8) | (p.b as u32));
            output[y * w + x] = weighted[weighted.len() / 2];
        }
    }

    ImageData {
        width: image_data.width,
        height: image_data.height,
        pixels: output,
    }
}

/// Apply Prewitt edge detector
pub fn prewitt_edge_detection(image_data: &ImageData) -> EdgeMap {
    let w = image_data.width as usize;
    let h = image_data.height as usize;

    let gray: Vec<u8> = image_data
        .pixels
        .iter()
        .map(|p| (0.299 * p.r as f64 + 0.587 * p.g as f64 + 0.114 * p.b as f64) as u8)
        .collect();

    let mut edge_buf = vec![0u8; w * h];

    let prewitt_x: [i32; 9] = [-1, 0, 1, -1, 0, 1, -1, 0, 1];
    let prewitt_y: [i32; 9] = [-1, -1, -1, 0, 0, 0, 1, 1, 1];

    for y in 1..(h - 1) {
        for x in 1..(w - 1) {
            let mut gx = 0i32;
            let mut gy = 0i32;

            for ky in 0..3 {
                for kx in 0..3 {
                    let px = x + kx - 1;
                    let py = y + ky - 1;
                    let pixel = gray[py * w + px] as i32;
                    let idx = ky * 3 + kx;
                    gx += pixel * prewitt_x[idx];
                    gy += pixel * prewitt_y[idx];
                }
            }

            let magnitude = ((gx * gx + gy * gy) as f64).sqrt();
            edge_buf[y * w + x] = magnitude.min(255.0) as u8;
        }
    }

    EdgeMap {
        width: image_data.width,
        height: image_data.height,
        data: edge_buf,
    }
}

/// Apply Roberts cross-gradient edge detector
pub fn roberts_edge_detection(image_data: &ImageData) -> EdgeMap {
    let w = image_data.width as usize;
    let h = image_data.height as usize;

    let gray: Vec<u8> = image_data
        .pixels
        .iter()
        .map(|p| (0.299 * p.r as f64 + 0.587 * p.g as f64 + 0.114 * p.b as f64) as u8)
        .collect();

    let mut edge_buf = vec![0u8; w * h];

    for y in 0..(h - 1) {
        for x in 0..(w - 1) {
            let g1 = (gray[y * w + x] as i32 - gray[(y + 1) * w + x + 1] as i32).abs();
            let g2 = (gray[y * w + x + 1] as i32 - gray[(y + 1) * w + x] as i32).abs();

            let magnitude = ((g1 * g1 + g2 * g2) as f64).sqrt();
            edge_buf[y * w + x] = magnitude.min(255.0) as u8;
        }
    }

    EdgeMap {
        width: image_data.width,
        height: image_data.height,
        data: edge_buf,
    }
}

#[derive(Debug, Clone)]
pub struct EdgeMap {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rgb::RGBA8;

    #[test]
    fn test_median_filter_uniform() {
        let pixels = vec![RGBA8::new(128, 128, 128, 255); 25];
        let img = ImageData {
            width: 5,
            height: 5,
            pixels,
        };
        let result = median_filter(&img, 1);
        assert_eq!(result.pixels.len(), 25);
        for p in result.pixels {
            assert_eq!(p.r, 128);
        }
    }

    #[test]
    fn test_prewitt_vertical_edge() {
        let mut pixels = Vec::new();
        for _y in 0..10 {
            for x in 0..10 {
                if x < 5 {
                    pixels.push(RGBA8::new(0, 0, 0, 255));
                } else {
                    pixels.push(RGBA8::new(255, 255, 255, 255));
                }
            }
        }
        let img = ImageData {
            width: 10,
            height: 10,
            pixels,
        };
        let edges = prewitt_edge_detection(&img);
        assert!(edges.data[5 * 10 + 5] > 0);
    }

    #[test]
    fn test_weighted_median_filter() {
        let pixels = vec![RGBA8::new(128, 128, 128, 255); 25];
        let img = ImageData {
            width: 5,
            height: 5,
            pixels,
        };
        let result = weighted_median_filter(&img, 1);
        assert_eq!(result.pixels.len(), 25);
        for p in result.pixels {
            assert_eq!(p.r, 128);
        }
    }

    #[test]
    fn test_roberts_edge_detection() {
        let mut pixels = Vec::new();
        for _y in 0..10 {
            for x in 0..10 {
                if x < 5 {
                    pixels.push(RGBA8::new(0, 0, 0, 255));
                } else {
                    pixels.push(RGBA8::new(255, 255, 255, 255));
                }
            }
        }
        let img = ImageData {
            width: 10,
            height: 10,
            pixels,
        };
        let edges = roberts_edge_detection(&img);
        let edge_count = edges.data.iter().filter(|&&v| v > 0).count();
        assert!(edge_count > 0);
    }
}
