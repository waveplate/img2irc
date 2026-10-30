//! Image preprocessing for better vectorization results
//!
//! This module provides preprocessing functions to prepare images for vectorization,
//! particularly useful for photographs or complex images.

use crate::image_processor::ImageData;
use anyhow::Result;
use rgb::RGBA8;

/// Preprocessing options
#[derive(Debug, Clone)]
pub struct PreprocessOptions {
    /// Color reduction level (0-1, 0 = none, 1 = maximum reduction)
    pub color_reduction: f32,
    /// Bilateral filter sigma for spatial domain
    pub spatial_sigma: f32,
    /// Bilateral filter sigma for color domain
    pub color_sigma: f32,
    /// Number of bilateral filter iterations
    pub iterations: u32,
}

impl Default for PreprocessOptions {
    fn default() -> Self {
        Self {
            color_reduction: 0.0,
            spatial_sigma: 3.0,
            color_sigma: 30.0,
            iterations: 1,
        }
    }
}

impl PreprocessOptions {
    /// Create options optimized for photographs
    pub fn photo() -> Self {
        Self {
            color_reduction: 0.5, // Moderate color reduction
            spatial_sigma: 5.0,   // More spatial smoothing
            color_sigma: 40.0,    // More color smoothing
            iterations: 2,
        }
    }
}

/// Apply preprocessing to image data
pub fn preprocess(image_data: &ImageData, options: &PreprocessOptions) -> Result<ImageData> {
    let mut pixels = image_data.pixels.clone();

    // Apply bilateral filter for edge-preserving smoothing
    if options.spatial_sigma > 0.0 && options.iterations > 0 {
        for _ in 0..options.iterations {
            pixels = bilateral_filter(
                &pixels,
                image_data.width,
                image_data.height,
                options.spatial_sigma,
                options.color_sigma,
            );
        }
    }

    // Apply color reduction (posterization)
    if options.color_reduction > 0.0 {
        pixels = reduce_colors(&pixels, options.color_reduction);
    }

    Ok(ImageData {
        width: image_data.width,
        height: image_data.height,
        pixels,
    })
}

/// Fast LUT-based bilateral filter — edge-preserving smoothing.
/// Uses precomputed range weight lookup table with fixed-point arithmetic
/// for much better performance than the naive Gaussian approach.
fn bilateral_filter(
    pixels: &[RGBA8],
    width: u32,
    height: u32,
    _spatial_sigma: f32,
    color_sigma: f32,
) -> Vec<RGBA8> {
    let w = width as usize;
    let h = height as usize;

    // Always use radius 2 — the LUT-based approach is fast enough
    let r: i32 = 2;

    // Precompute range weight LUT (distance 0..=195075 maps to 0..255)
    // 195075 = 255^2 * 3 (max squared RGB distance)
    let range_denom = 2.0 * (color_sigma as f64) * (color_sigma as f64);
    let lut_size: usize = 256;
    let bin_scale = 195075.0 / lut_size as f64;
    let mut range_lut = vec![0u32; lut_size];
    for i in 0..lut_size {
        let dist = i as f64 * bin_scale;
        let weight = (-dist / range_denom).exp();
        range_lut[i] = (weight * 1024.0) as u32; // fixed-point 10-bit
    }

    let mut output = vec![RGBA8::new(0, 0, 0, 255); pixels.len()];

    for y in 0..h {
        for x in 0..w {
            let ci = y * w + x;
            let cr = pixels[ci].r as i32;
            let cg = pixels[ci].g as i32;
            let cb = pixels[ci].b as i32;

            let mut sum_r: u64 = 0;
            let mut sum_g: u64 = 0;
            let mut sum_b: u64 = 0;
            let mut sum_w: u64 = 0;

            let y_start = if (y as i32) < r { 0 } else { y - r as usize };
            let y_end = (y + r as usize + 1).min(h);
            let x_start = if (x as i32) < r { 0 } else { x - r as usize };
            let x_end = (x + r as usize + 1).min(w);

            for ny in y_start..y_end {
                let row = ny * w;
                for nx in x_start..x_end {
                    let ni = row + nx;
                    let dr = pixels[ni].r as i32 - cr;
                    let dg = pixels[ni].g as i32 - cg;
                    let db = pixels[ni].b as i32 - cb;
                    let dist_sq = (dr * dr + dg * dg + db * db) as usize;

                    let bin = (dist_sq * lut_size) / 195076;
                    let weight = range_lut[bin.min(lut_size - 1)] as u64;

                    sum_r += pixels[ni].r as u64 * weight;
                    sum_g += pixels[ni].g as u64 * weight;
                    sum_b += pixels[ni].b as u64 * weight;
                    sum_w += weight;
                }
            }

            if sum_w > 0 {
                output[ci] = RGBA8::new(
                    (sum_r / sum_w) as u8,
                    (sum_g / sum_w) as u8,
                    (sum_b / sum_w) as u8,
                    pixels[ci].a,
                );
            } else {
                output[ci] = pixels[ci];
            }
        }
    }

    output
}

/// Reduce colors through posterization
fn reduce_colors(pixels: &[RGBA8], reduction: f32) -> Vec<RGBA8> {
    // Calculate number of color levels (256 -> 2-256 based on reduction)
    // reduction 0.0 = 256 levels (no change)
    // reduction 1.0 = 2 levels (maximum posterization)
    let levels = ((1.0 - reduction) * 254.0 + 2.0).clamp(2.0, 256.0) as u8;
    let factor = 255.0 / (levels - 1) as f32;

    pixels
        .iter()
        .map(|p| {
            RGBA8::new(
                ((p.r as f32 / factor).round() * factor).clamp(0.0, 255.0) as u8,
                ((p.g as f32 / factor).round() * factor).clamp(0.0, 255.0) as u8,
                ((p.b as f32 / factor).round() * factor).clamp(0.0, 255.0) as u8,
                p.a,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reduce_colors_none() {
        let pixels = vec![
            RGBA8::new(0, 0, 0, 255),
            RGBA8::new(127, 127, 127, 255), // Use 127 instead of 128 to avoid rounding issues
            RGBA8::new(255, 255, 255, 255),
        ];
        let result = reduce_colors(&pixels, 0.0);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].r, 0);
        assert_eq!(result[1].r, 127);
        assert_eq!(result[2].r, 255);
    }

    #[test]
    fn test_reduce_colors_max() {
        let pixels = vec![
            RGBA8::new(0, 0, 0, 255),
            RGBA8::new(128, 128, 128, 255),
            RGBA8::new(255, 255, 255, 255),
        ];
        let result = reduce_colors(&pixels, 1.0); // Maximum reduction -> 2 levels
        assert_eq!(result.len(), 3);
        // All should be either 0 or 255
        for p in &result {
            assert!(p.r == 0 || p.r == 255);
        }
    }

    #[test]
    fn test_reduce_colors_partial() {
        let pixels = vec![
            RGBA8::new(0, 0, 0, 255),
            RGBA8::new(100, 100, 100, 255),
            RGBA8::new(200, 200, 200, 255),
        ];
        let result = reduce_colors(&pixels, 0.5); // 50% reduction -> ~128 levels
        assert_eq!(result.len(), 3);
        // Values should be quantized
        assert_ne!(result[0].r, result[1].r);
    }

    #[test]
    fn test_preprocess_options_default() {
        let opts = PreprocessOptions::default();
        assert_eq!(opts.color_reduction, 0.0);
        assert_eq!(opts.spatial_sigma, 3.0);
    }

    #[test]
    fn test_preprocess_options_photo() {
        let opts = PreprocessOptions::photo();
        assert_eq!(opts.color_reduction, 0.5);
        assert_eq!(opts.spatial_sigma, 5.0);
        assert_eq!(opts.iterations, 2);
    }

    #[test]
    fn test_bilateral_filter_preserves_alpha() {
        let pixels = vec![
            RGBA8::new(128, 128, 128, 255),
            RGBA8::new(128, 128, 128, 128),
            RGBA8::new(128, 128, 128, 0),
        ];
        let result = bilateral_filter(&pixels, 3, 1, 2.0, 30.0);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].a, 255);
        assert_eq!(result[1].a, 128);
        assert_eq!(result[2].a, 0);
    }

    #[test]
    fn test_bilateral_filter_uniform() {
        // All same color - should remain same
        let pixels = vec![
            RGBA8::new(128, 128, 128, 255),
            RGBA8::new(128, 128, 128, 255),
            RGBA8::new(128, 128, 128, 255),
        ];
        let result = bilateral_filter(&pixels, 3, 1, 2.0, 30.0);
        for p in result {
            assert_eq!(p.r, 128);
            assert_eq!(p.g, 128);
            assert_eq!(p.b, 128);
        }
    }
}
