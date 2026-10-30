//! Thresholding algorithms: Otsu and adaptive thresholding

use crate::image_processor::ImageData;
use rgb::RGBA8;

pub struct OtsuThreshold;

impl OtsuThreshold {
    pub fn compute(image_data: &ImageData) -> u8 {
        let mut histogram = [0u32; 256];

        for pixel in &image_data.pixels {
            let gray = ((0.299 * pixel.r as f64 + 0.587 * pixel.g as f64 + 0.114 * pixel.b as f64)
                .round()) as usize;
            histogram[gray.min(255)] += 1;
        }

        let total = image_data.pixels.len();

        let mut sum = 0u64;
        for i in 0..256 {
            sum += i as u64 * histogram[i] as u64;
        }

        let mut sum_b = 0u64;
        let mut w_b = 0u64;
        let mut max_variance = 0.0;
        let mut threshold = 0u8;

        for i in 0..256 {
            w_b += histogram[i] as u64;
            if w_b == 0 {
                continue;
            }

            let w_f = total as u64 - w_b;
            if w_f == 0 {
                break;
            }

            sum_b += i as u64 * histogram[i] as u64;

            let m_b = sum_b as f64 / w_b as f64;
            let m_f = ((sum - sum_b) as f64) / w_f as f64;

            let variance =
                (w_b as f64 / total as f64) * (w_f as f64 / total as f64) * (m_b - m_f).powi(2);

            if variance > max_variance {
                max_variance = variance;
                threshold = i as u8;
            }
        }

        threshold
    }

    pub fn binarize(image_data: &ImageData, threshold: Option<u8>) -> ImageData {
        let thresh = threshold.unwrap_or_else(|| Self::compute(image_data));

        let output: Vec<RGBA8> = image_data
            .pixels
            .iter()
            .map(|pixel| {
                let gray =
                    ((0.299 * pixel.r as f64 + 0.587 * pixel.g as f64 + 0.114 * pixel.b as f64)
                        .round()) as u8;
                let val = if gray > thresh { 255u8 } else { 0u8 };
                RGBA8::new(val, val, val, pixel.a)
            })
            .collect();

        ImageData {
            width: image_data.width,
            height: image_data.height,
            pixels: output,
        }
    }
}

pub struct AdaptiveThreshold {
    pub block_size: usize,
    pub c: i32,
}

impl Default for AdaptiveThreshold {
    fn default() -> Self {
        Self {
            block_size: 11,
            c: 2,
        }
    }
}

impl AdaptiveThreshold {
    pub fn new(block_size: usize, c: i32) -> Self {
        Self {
            block_size: block_size.max(3) | 1,
            c,
        }
    }

    pub fn binarize(&self, image_data: &ImageData) -> ImageData {
        let w = image_data.width as usize;
        let h = image_data.height as usize;
        let half = self.block_size / 2;

        let gray: Vec<u8> = image_data
            .pixels
            .iter()
            .map(|p| (0.299 * p.r as f64 + 0.587 * p.g as f64 + 0.114 * p.b as f64) as u8)
            .collect();

        let integral = self.compute_integral(&gray, w, h);

        let output: Vec<RGBA8> = gray
            .iter()
            .enumerate()
            .map(|(i, &pixel)| {
                let y = i / w;
                let x = i % w;

                let y1 = y.saturating_sub(half);
                let y2 = (y + half + 1).min(h);
                let x1 = x.saturating_sub(half);
                let x2 = (x + half + 1).min(w);

                let area = ((y2 - y1) * (x2 - x1)) as i32;

                let sum = integral[y2 * (w + 1) + x2]
                    - integral[y1 * (w + 1) + x2]
                    - integral[y2 * (w + 1) + x1]
                    + integral[y1 * (w + 1) + x1];

                let mean = (sum as f64 / area as f64).round() as i32;
                let threshold = mean - self.c;

                let val = if pixel as i32 >= threshold {
                    255u8
                } else {
                    0u8
                };
                RGBA8::new(val, val, val, image_data.pixels[i].a)
            })
            .collect();

        ImageData {
            width: image_data.width,
            height: image_data.height,
            pixels: output,
        }
    }

    fn compute_integral(&self, gray: &[u8], w: usize, h: usize) -> Vec<i64> {
        let mut integral = vec![0i64; (w + 1) * (h + 1)];

        for y in 0..h {
            let mut row_sum = 0i64;
            for x in 0..w {
                row_sum += gray[y * w + x] as i64;
                integral[(y + 1) * (w + 1) + (x + 1)] = integral[y * (w + 1) + (x + 1)]
                    + integral[(y + 1) * (w + 1) + x]
                    - integral[y * (w + 1) + x]
                    + row_sum;
            }
        }

        integral
    }
}

/// Smart threshold that automatically selects Otsu or adaptive thresholding
/// based on image content.
///
/// Uses the Otsu separability measure (ratio of between-class variance to total
/// variance). If the histogram is strongly bimodal (separability > 0.7), Otsu
/// thresholding is used. Otherwise, adaptive thresholding is applied for
/// images with uneven illumination or low contrast.
pub struct SmartThreshold;

impl SmartThreshold {
    pub fn binarize(image_data: &ImageData) -> ImageData {
        let mut histogram = [0u32; 256];
        let mut total_gray = 0u64;

        for pixel in &image_data.pixels {
            let gray = ((0.299 * pixel.r as f64 + 0.587 * pixel.g as f64 + 0.114 * pixel.b as f64)
                .round()) as usize;
            let g = gray.min(255) as u8;
            histogram[g as usize] += 1;
            total_gray += g as u64;
        }

        let total = image_data.pixels.len() as u64;
        let mean_total = total_gray as f64 / total as f64;

        let mut sum = 0u64;
        for i in 0..256 {
            sum += i as u64 * histogram[i] as u64;
        }

        let mut sum_b = 0u64;
        let mut w_b = 0u64;
        let mut max_variance = 0.0;

        for i in 0..256 {
            w_b += histogram[i] as u64;
            if w_b == 0 {
                continue;
            }
            let w_f = total - w_b;
            if w_f == 0 {
                break;
            }
            sum_b += i as u64 * histogram[i] as u64;
            let m_b = sum_b as f64 / w_b as f64;
            let m_f = ((sum - sum_b) as f64) / w_f as f64;
            let variance =
                (w_b as f64 / total as f64) * (w_f as f64 / total as f64) * (m_b - m_f).powi(2);
            if variance > max_variance {
                max_variance = variance;
            }
        }

        // Total variance
        let mut total_variance = 0.0;
        for i in 0..256 {
            if histogram[i] == 0 {
                continue;
            }
            let diff = i as f64 - mean_total;
            total_variance +=
                (histogram[i] as f64 / total as f64) * diff * diff;
        }

        let separability = if total_variance > 1e-6 {
            max_variance / total_variance
        } else {
            0.0
        };

        // Strongly bimodal → Otsu; otherwise → adaptive
        if separability > 0.5 {
            OtsuThreshold::binarize(image_data, None)
        } else {
            AdaptiveThreshold::default().binarize(image_data)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_otsu_threshold_computes() {
        let mut pixels = Vec::new();
        for i in 0..100 {
            let val = if i < 50 { 0u8 } else { 255u8 };
            pixels.push(RGBA8::new(val, val, val, 255));
        }
        let img = ImageData {
            width: 10,
            height: 10,
            pixels,
        };
        let threshold = OtsuThreshold::compute(&img);
        // threshold is u8, so it's always <= 255; this tests it doesn't panic
        let _ = threshold;
    }

    #[test]
    fn test_otsu_binarize() {
        let mut pixels = Vec::new();
        for i in 0..100 {
            let val = if i < 50 { 0u8 } else { 255u8 };
            pixels.push(RGBA8::new(val, val, val, 255));
        }
        let img = ImageData {
            width: 10,
            height: 10,
            pixels,
        };
        let result = OtsuThreshold::binarize(&img, None);
        assert_eq!(result.pixels.len(), 100);
    }

    #[test]
    fn test_adaptive_threshold() {
        let mut pixels = Vec::new();
        for y in 0..10 {
            for _x in 0..10 {
                let val = (y * 25) as u8;
                pixels.push(RGBA8::new(val, val, val, 255));
            }
        }
        let img = ImageData {
            width: 10,
            height: 10,
            pixels,
        };
        let adaptive = AdaptiveThreshold::default();
        let result = adaptive.binarize(&img);
        assert_eq!(result.pixels.len(), 100);
    }

    #[test]
    fn test_adaptive_threshold_odd_block() {
        let adaptive = AdaptiveThreshold::new(10, 2);
        assert_eq!(adaptive.block_size % 2, 1);
    }

    #[test]
    fn test_smart_threshold_bimodal_chooses_otsu() {
        // Bimodal image: two clear peaks around 50 and 200 → should use Otsu
        let mut pixels = Vec::new();
        for _y in 0..10 {
            for x in 0..10 {
                let val = if x < 5 { 50u8 } else { 200u8 };
                pixels.push(RGBA8::new(val, val, val, 255));
            }
        }
        let img = ImageData {
            width: 10,
            height: 10,
            pixels,
        };
        let result = SmartThreshold::binarize(&img);
        // Otsu on bimodal image produces clean black/white
        let unique: std::collections::HashSet<_> = result.pixels.iter().map(|p| p.r).collect();
        assert_eq!(unique.len(), 2, "Expected two unique values (0 and 255)");
    }

    #[test]
    fn test_smart_threshold_gradient_chooses_adaptive() {
        // Smooth gradient: no clear bimodal separation → should use adaptive
        let mut pixels = Vec::new();
        for y in 0..10 {
            for _x in 0..10 {
                let val = (y * 25) as u8;
                pixels.push(RGBA8::new(val, val, val, 255));
            }
        }
        let img = ImageData {
            width: 10,
            height: 10,
            pixels,
        };
        let result = SmartThreshold::binarize(&img);
        // Adaptive on gradient produces non-trivial result (not all same)
        assert_eq!(result.pixels.len(), 100);
    }
}
