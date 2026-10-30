//! Image enhancement: unsharp masking, Laplacian sharpening, gamma correction

use crate::image_processor::ImageData;
use rgb::RGBA8;

pub struct UnsharpMask {
    pub sigma: f32,
    pub amount: f32,
    pub threshold: u8,
}

impl Default for UnsharpMask {
    fn default() -> Self {
        Self {
            sigma: 1.0,
            amount: 1.5,
            threshold: 0,
        }
    }
}

impl UnsharpMask {
    pub fn new(sigma: f32, amount: f32, threshold: u8) -> Self {
        Self {
            sigma,
            amount,
            threshold,
        }
    }

    pub fn apply(&self, image_data: &ImageData) -> ImageData {
        let blurred = self.gaussian_blur(image_data);
        let w = image_data.width as usize;
        let h = image_data.height as usize;

        let mut output = Vec::with_capacity(w * h);

        for (i, pixel) in image_data.pixels.iter().enumerate() {
            let blurred_pixel = &blurred.pixels[i];
            let gray = ((0.299 * pixel.r as f64 + 0.587 * pixel.g as f64 + 0.114 * pixel.b as f64)
                .round()) as u8;
            let blurred_gray = ((0.299 * blurred_pixel.r as f64
                + 0.587 * blurred_pixel.g as f64
                + 0.114 * blurred_pixel.b as f64)
                .round()) as u8;

            let diff = (gray as i16 - blurred_gray as i16).abs().clamp(0, 255) as u8;

            if diff > self.threshold {
                output.push(RGBA8::new(
                    (pixel.r as f32 + self.amount * (pixel.r as f32 - blurred_pixel.r as f32))
                        .round()
                        .clamp(0.0, 255.0) as u8,
                    (pixel.g as f32 + self.amount * (pixel.g as f32 - blurred_pixel.g as f32))
                        .round()
                        .clamp(0.0, 255.0) as u8,
                    (pixel.b as f32 + self.amount * (pixel.b as f32 - blurred_pixel.b as f32))
                        .round()
                        .clamp(0.0, 255.0) as u8,
                    pixel.a,
                ));
            } else {
                output.push(*pixel);
            }
        }

        ImageData {
            width: image_data.width,
            height: image_data.height,
            pixels: output,
        }
    }

    pub(crate) fn gaussian_blur(&self, image_data: &ImageData) -> ImageData {
        let w = image_data.width as usize;
        let h = image_data.height as usize;
        let kernel_size = (self.sigma * 3.0).ceil() as usize * 2 + 1;
        let mut kernel = vec![0f32; kernel_size];

        let sum: f32 = kernel
            .iter_mut()
            .enumerate()
            .map(|(i, v)| {
                let x = (i as isize) - (kernel_size as isize / 2);
                *v = (-((x as f32) * (x as f32)) / (2.0 * self.sigma * self.sigma)).exp();
                *v
            })
            .sum();

        for v in kernel.iter_mut() {
            *v /= sum;
        }

        let mut temp = vec![RGBA8::new(0, 0, 0, 255); w * h];
        let mut output = vec![RGBA8::new(0, 0, 0, 255); w * h];

        for y in 0..h {
            for x in 0..w {
                let mut sum_r = 0f32;
                let mut sum_g = 0f32;
                let mut sum_b = 0f32;
                for k in 0..kernel_size {
                    let kx = (x as isize + k as isize - kernel_size as isize / 2)
                        .clamp(0, w as isize - 1) as usize;
                    sum_r += image_data.pixels[y * w + kx].r as f32 * kernel[k];
                    sum_g += image_data.pixels[y * w + kx].g as f32 * kernel[k];
                    sum_b += image_data.pixels[y * w + kx].b as f32 * kernel[k];
                }
                temp[y * w + x] = RGBA8::new(
                    sum_r.round().clamp(0.0, 255.0) as u8,
                    sum_g.round().clamp(0.0, 255.0) as u8,
                    sum_b.round().clamp(0.0, 255.0) as u8,
                    image_data.pixels[y * w + x].a,
                );
            }
        }

        for y in 0..h {
            for x in 0..w {
                let mut sum_r = 0f32;
                let mut sum_g = 0f32;
                let mut sum_b = 0f32;
                for k in 0..kernel_size {
                    let ky = (y as isize + k as isize - kernel_size as isize / 2)
                        .clamp(0, h as isize - 1) as usize;
                    sum_r += temp[ky * w + x].r as f32 * kernel[k];
                    sum_g += temp[ky * w + x].g as f32 * kernel[k];
                    sum_b += temp[ky * w + x].b as f32 * kernel[k];
                }
                output[y * w + x] = RGBA8::new(
                    sum_r.round().clamp(0.0, 255.0) as u8,
                    sum_g.round().clamp(0.0, 255.0) as u8,
                    sum_b.round().clamp(0.0, 255.0) as u8,
                    image_data.pixels[y * w + x].a,
                );
            }
        }

        ImageData {
            width: image_data.width,
            height: image_data.height,
            pixels: output,
        }
    }
}

pub struct LaplacianSharpen {
    pub amount: f32,
}

impl Default for LaplacianSharpen {
    fn default() -> Self {
        Self { amount: 1.0 }
    }
}

impl LaplacianSharpen {
    pub fn new(amount: f32) -> Self {
        Self { amount }
    }

    pub fn apply(&self, image_data: &ImageData) -> ImageData {
        let w = image_data.width as usize;
        let h = image_data.height as usize;

        let gray: Vec<u8> = image_data
            .pixels
            .iter()
            .map(|p| (0.299 * p.r as f64 + 0.587 * p.g as f64 + 0.114 * p.b as f64) as u8)
            .collect();

        let mut output = Vec::with_capacity(w * h);

        for y in 0..h {
            for x in 0..w {
                let pixel = &image_data.pixels[y * w + x];

                if y == 0 || y == h - 1 || x == 0 || x == w - 1 {
                    output.push(*pixel);
                    continue;
                }

                let center = gray[y * w + x] as f32;
                let neighbors: Vec<f32> = vec![
                    gray[(y - 1) * w + x] as f32,
                    gray[(y + 1) * w + x] as f32,
                    gray[y * w + (x - 1)] as f32,
                    gray[y * w + (x + 1)] as f32,
                ];

                let laplacian = center * 4.0 - neighbors.iter().sum::<f32>();
                let sharpened = center + self.amount * laplacian;

                let scale = (sharpened / center).clamp(0.5, 2.0);

                output.push(RGBA8::new(
                    (pixel.r as f32 * scale).round().clamp(0.0, 255.0) as u8,
                    (pixel.g as f32 * scale).round().clamp(0.0, 255.0) as u8,
                    (pixel.b as f32 * scale).round().clamp(0.0, 255.0) as u8,
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
}

/// High-boost filtering: aggressive sharpening by boosting high-frequency detail.
/// Similar to unsharp masking but with a boost factor > 1 for stronger edge emphasis.
pub struct HighBoostFilter {
    pub sigma: f32,
    pub boost: f32,
}

impl Default for HighBoostFilter {
    fn default() -> Self {
        Self {
            sigma: 1.0,
            boost: 2.0,
        }
    }
}

impl HighBoostFilter {
    pub fn new(sigma: f32, boost: f32) -> Self {
        Self {
            sigma,
            boost: boost.max(1.0),
        }
    }

    pub fn apply(&self, image_data: &ImageData) -> ImageData {
        let blurred = UnsharpMask::new(self.sigma, 1.0, 0).gaussian_blur(image_data);
        let w = image_data.width as usize;
        let h = image_data.height as usize;

        let mut output = Vec::with_capacity(w * h);

        for (i, pixel) in image_data.pixels.iter().enumerate() {
            let blurred_pixel = &blurred.pixels[i];
            output.push(RGBA8::new(
                (pixel.r as f32 + self.boost * (pixel.r as f32 - blurred_pixel.r as f32))
                    .round()
                    .clamp(0.0, 255.0) as u8,
                (pixel.g as f32 + self.boost * (pixel.g as f32 - blurred_pixel.g as f32))
                    .round()
                    .clamp(0.0, 255.0) as u8,
                (pixel.b as f32 + self.boost * (pixel.b as f32 - blurred_pixel.b as f32))
                    .round()
                    .clamp(0.0, 255.0) as u8,
                pixel.a,
            ));
        }

        ImageData {
            width: image_data.width,
            height: image_data.height,
            pixels: output,
        }
    }
}

pub struct GammaCorrection {
    pub gamma: f32,
}

impl Default for GammaCorrection {
    fn default() -> Self {
        Self { gamma: 1.0 }
    }
}

impl GammaCorrection {
    pub fn new(gamma: f32) -> Self {
        Self {
            gamma: gamma.max(0.1),
        }
    }

    pub fn apply(&self, image_data: &ImageData) -> ImageData {
        let output: Vec<RGBA8> = image_data
            .pixels
            .iter()
            .map(|pixel| {
                let apply_gamma = |v: u8| {
                    let normalized = v as f32 / 255.0;
                    let corrected = normalized.powf(1.0 / self.gamma);
                    (corrected * 255.0).round().clamp(0.0, 255.0) as u8
                };

                RGBA8::new(
                    apply_gamma(pixel.r),
                    apply_gamma(pixel.g),
                    apply_gamma(pixel.b),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unsharp_mask() {
        let pixels = vec![RGBA8::new(128, 128, 128, 255); 100];
        let img = ImageData {
            width: 10,
            height: 10,
            pixels,
        };
        let unsharp = UnsharpMask::default();
        let result = unsharp.apply(&img);
        assert_eq!(result.pixels.len(), 100);
    }

    #[test]
    fn test_laplacian_sharpen() {
        let pixels = vec![RGBA8::new(128, 128, 128, 255); 100];
        let img = ImageData {
            width: 10,
            height: 10,
            pixels,
        };
        let sharpen = LaplacianSharpen::default();
        let result = sharpen.apply(&img);
        assert_eq!(result.pixels.len(), 100);
    }

    #[test]
    fn test_gamma_correction() {
        let pixels = vec![RGBA8::new(128, 128, 128, 255); 100];
        let img = ImageData {
            width: 10,
            height: 10,
            pixels,
        };
        let gamma = GammaCorrection::new(2.2);
        let result = gamma.apply(&img);
        assert_eq!(result.pixels.len(), 100);
    }

    #[test]
    fn test_high_boost_filter() {
        let pixels = vec![RGBA8::new(128, 128, 128, 255); 100];
        let img = ImageData {
            width: 10,
            height: 10,
            pixels,
        };
        let filter = HighBoostFilter::default();
        let result = filter.apply(&img);
        assert_eq!(result.pixels.len(), 100);
    }

    #[test]
    fn test_gamma_correction_inverse() {
        let pixels = vec![RGBA8::new(128, 128, 128, 255); 100];
        let img = ImageData {
            width: 10,
            height: 10,
            pixels,
        };
        let gamma = GammaCorrection::new(0.5);
        let result = gamma.apply(&img);
        assert_eq!(result.pixels.len(), 100);
    }
}
