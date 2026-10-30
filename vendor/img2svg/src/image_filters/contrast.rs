//! Contrast enhancement: histogram equalization and CLAHE

use crate::image_processor::ImageData;
use rgb::RGBA8;

pub struct HistogramEqualizer;

impl HistogramEqualizer {
    pub fn equalize(image_data: &ImageData) -> ImageData {
        let mut histogram = [0u32; 256];

        for pixel in &image_data.pixels {
            let gray = ((0.299 * pixel.r as f64 + 0.587 * pixel.g as f64 + 0.114 * pixel.b as f64)
                .round()) as usize;
            histogram[gray.min(255)] += 1;
        }

        let total = image_data.pixels.len() as f64;
        let mut cdf = [0f64; 256];
        let mut sum = 0.0;

        for i in 0..256 {
            sum += histogram[i] as f64;
            cdf[i] = (sum / total * 255.0).round();
        }

        let output: Vec<RGBA8> = image_data
            .pixels
            .iter()
            .map(|pixel| {
                let gray =
                    ((0.299 * pixel.r as f64 + 0.587 * pixel.g as f64 + 0.114 * pixel.b as f64)
                        .round()) as usize;
                let new_val = cdf[gray.min(255)] as u8;

                let scale = new_val as f64 / (gray as f64 + 1.0).max(1.0);

                RGBA8::new(
                    (pixel.r as f64 * scale).round().clamp(0.0, 255.0) as u8,
                    (pixel.g as f64 * scale).round().clamp(0.0, 255.0) as u8,
                    (pixel.b as f64 * scale).round().clamp(0.0, 255.0) as u8,
                    pixel.a,
                )
            })
            .collect();

        ImageData {
            width: image_data.width,
            height: image_data.height,
            pixels: output,
        }
    }
}

pub struct CLAHE {
    pub clip_limit: f64,
    pub grid_size: usize,
}

impl Default for CLAHE {
    fn default() -> Self {
        Self {
            clip_limit: 2.0,
            grid_size: 8,
        }
    }
}

impl CLAHE {
    pub fn new(clip_limit: f64, grid_size: usize) -> Self {
        Self {
            clip_limit,
            grid_size,
        }
    }

    pub fn equalize(&self, image_data: &ImageData) -> ImageData {
        let w = image_data.width as usize;
        let h = image_data.height as usize;
        let tile_w = w.div_ceil(self.grid_size);
        let tile_h = h.div_ceil(self.grid_size);

        let gray: Vec<u8> = image_data
            .pixels
            .iter()
            .map(|p| (0.299 * p.r as f64 + 0.587 * p.g as f64 + 0.114 * p.b as f64) as u8)
            .collect();

        let mut output = Vec::with_capacity(w * h);

        for y in 0..h {
            for x in 0..w {
                let tile_y = y / tile_h;
                let tile_x = x / tile_w;

                let y_start = tile_y.saturating_sub(1);
                let y_end = (tile_y + 2).min(self.grid_size);
                let x_start = tile_x.saturating_sub(1);
                let x_end = (tile_x + 2).min(self.grid_size);

                let mut weights = Vec::new();
                let mut cdfs = Vec::new();

                for ty in y_start..y_end {
                    for tx in x_start..x_end {
                        let tile_center_y = (ty * tile_h + tile_h / 2) as f64;
                        let tile_center_x = (tx * tile_w + tile_w / 2) as f64;

                        let dx = (x as f64 - tile_center_x) / (tile_w as f64);
                        let dy = (y as f64 - tile_center_y) / (tile_h as f64);
                        let weight = (1.0 - dx.abs()) * (1.0 - dy.abs());

                        weights.push(weight);
                        cdfs.push(self.compute_tile_cdf(&gray, w, h, tx, ty, tile_w, tile_h));
                    }
                }

                let gray_val = gray[y * w + x] as usize;
                let mut result = 0f64;
                let mut total_weight = 0f64;

                for (i, cdf) in cdfs.iter().enumerate() {
                    result += cdf[gray_val.min(255)] * weights[i];
                    total_weight += weights[i];
                }

                let new_val = if total_weight > 0.0 {
                    (result / total_weight).round() as u8
                } else {
                    gray[y * w + x]
                };

                let pixel = &image_data.pixels[y * w + x];
                let scale = new_val as f64 / (gray_val as f64 + 1.0).max(1.0);

                output.push(RGBA8::new(
                    (pixel.r as f64 * scale).round().clamp(0.0, 255.0) as u8,
                    (pixel.g as f64 * scale).round().clamp(0.0, 255.0) as u8,
                    (pixel.b as f64 * scale).round().clamp(0.0, 255.0) as u8,
                    pixel.a,
                ));
            }
        }

        ImageData {
            width: image_data.width,
            height: image_data.height,
            pixels: output,
        }
    }

    fn compute_tile_cdf(
        &self,
        gray: &[u8],
        w: usize,
        h: usize,
        tile_x: usize,
        tile_y: usize,
        tile_w: usize,
        tile_h: usize,
    ) -> [f64; 256] {
        let mut histogram = [0u32; 256];

        let y_start = tile_y * tile_h;
        let y_end = (y_start + tile_h).min(h);
        let x_start = tile_x * tile_w;
        let x_end = (x_start + tile_w).min(w);

        for y in y_start..y_end {
            for x in x_start..x_end {
                histogram[gray[y * w + x] as usize] += 1;
            }
        }

        if self.clip_limit > 0.0 {
            let total: u32 = histogram.iter().sum();
            let clip_limit = (total as f64 / 256.0 * self.clip_limit) as u32;

            let mut excess = 0i32;
            for i in 0..256 {
                if histogram[i] > clip_limit {
                    excess += (histogram[i] - clip_limit) as i32;
                    histogram[i] = clip_limit;
                }
            }

            let bonus = (excess as f64 / 256.0).round() as u32;
            for i in 0..256 {
                histogram[i] = histogram[i].saturating_add(bonus);
            }
        }

        let total: f64 = histogram.iter().map(|&v| v as f64).sum();
        let mut cdf = [0f64; 256];
        let mut sum = 0f64;

        for i in 0..256 {
            sum += histogram[i] as f64;
            cdf[i] = (sum / total * 255.0).round();
        }

        cdf
    }
}

/// Adaptive tone mapping using local gamma adjustment.
/// Computes local mean luminance per pixel and applies a per-pixel gamma
/// to improve visibility in both shadows and highlights.
pub struct AdaptiveToneMapper {
    pub gamma_shadows: f32,
    pub gamma_highlights: f32,
    pub window_size: usize,
}

impl Default for AdaptiveToneMapper {
    fn default() -> Self {
        Self {
            gamma_shadows: 0.7,
            gamma_highlights: 1.4,
            window_size: 15,
        }
    }
}

impl AdaptiveToneMapper {
    pub fn new(gamma_shadows: f32, gamma_highlights: f32, window_size: usize) -> Self {
        Self {
            gamma_shadows: gamma_shadows.max(0.1),
            gamma_highlights: gamma_highlights.max(0.1),
            window_size: window_size.max(3),
        }
    }

    pub fn apply(&self, image_data: &ImageData) -> ImageData {
        let w = image_data.width as usize;
        let h = image_data.height as usize;
        let r = self.window_size / 2;

        let gray: Vec<f32> = image_data
            .pixels
            .iter()
            .map(|p| 0.299 * p.r as f32 + 0.587 * p.g as f32 + 0.114 * p.b as f32)
            .collect();

        let mut local_mean = vec![0f32; w * h];

        for y in 0..h {
            for x in 0..w {
                let mut sum = 0f32;
                let mut count = 0usize;
                let y0 = y.saturating_sub(r);
                let y1 = (y + r + 1).min(h);
                let x0 = x.saturating_sub(r);
                let x1 = (x + r + 1).min(w);
                for yy in y0..y1 {
                    for xx in x0..x1 {
                        sum += gray[yy * w + xx];
                        count += 1;
                    }
                }
                local_mean[y * w + x] = sum / count.max(1) as f32;
            }
        }

        let output: Vec<RGBA8> = image_data
            .pixels
            .iter()
            .enumerate()
            .map(|(i, pixel)| {
                let lum = gray[i] / 255.0;
                let mean = local_mean[i] / 255.0;

                // If pixel is darker than local average, boost shadows;
                // if brighter, compress highlights.
                let gamma = if lum < mean {
                    self.gamma_shadows
                } else {
                    self.gamma_highlights
                };

                let adjust = |v: u8| {
                    let n = v as f32 / 255.0;
                    let corrected = n.powf(1.0 / gamma);
                    (corrected * 255.0).round().clamp(0.0, 255.0) as u8
                };

                RGBA8::new(adjust(pixel.r), adjust(pixel.g), adjust(pixel.b), pixel.a)
            })
            .collect();

        ImageData {
            width: image_data.width,
            height: image_data.height,
            pixels: output,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_histogram_equalization() {
        let mut pixels = Vec::new();
        for i in 0..100 {
            let val = (i * 2) as u8;
            pixels.push(RGBA8::new(val, val, val, 255));
        }
        let img = ImageData {
            width: 10,
            height: 10,
            pixels,
        };
        let result = HistogramEqualizer::equalize(&img);
        assert_eq!(result.pixels.len(), 100);
    }

    #[test]
    fn test_adaptive_tone_mapping() {
        let pixels = vec![RGBA8::new(128, 128, 128, 255); 100];
        let img = ImageData {
            width: 10,
            height: 10,
            pixels,
        };
        let mapper = AdaptiveToneMapper::default();
        let result = mapper.apply(&img);
        assert_eq!(result.pixels.len(), 100);
    }

    #[test]
    fn test_clahe_uniform() {
        let pixels = vec![RGBA8::new(128, 128, 128, 255); 100];
        let img = ImageData {
            width: 10,
            height: 10,
            pixels,
        };
        let clahe = CLAHE::default();
        let result = clahe.equalize(&img);
        assert_eq!(result.pixels.len(), 100);
    }
}
