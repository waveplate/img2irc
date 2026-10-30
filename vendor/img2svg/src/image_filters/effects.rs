//! Special effects: emboss, sepia, vignette, edge glow

use crate::image_processor::ImageData;
use rgb::RGBA8;

pub struct EmbossFilter {
    pub strength: f32,
    pub angle: f32,
}

impl Default for EmbossFilter {
    fn default() -> Self {
        Self {
            strength: 1.0,
            angle: 135.0,
        }
    }
}

impl EmbossFilter {
    pub fn new(strength: f32, angle: f32) -> Self {
        Self { strength, angle }
    }

    pub fn apply(&self, image_data: &ImageData) -> ImageData {
        let w = image_data.width as usize;
        let h = image_data.height as usize;

        let gray: Vec<u8> = image_data
            .pixels
            .iter()
            .map(|p| (0.299 * p.r as f64 + 0.587 * p.g as f64 + 0.114 * p.b as f64) as u8)
            .collect();

        let angle_rad = self.angle * std::f32::consts::PI / 180.0_f32;
        let dx = angle_rad.cos().round() as i32;
        let dy = angle_rad.sin().round() as i32;

        let mut output = Vec::with_capacity(w * h);

        for y in 0..h {
            for x in 0..w {
                let idx1 = y * w + x;
                let idx2 = ((y as i32 + dy) as usize).clamp(0, h - 1) * w
                    + ((x as i32 + dx) as usize).clamp(0, w - 1);

                let diff = gray[idx1] as i32 - gray[idx2] as i32;
                let embossed = 128 + (diff as f32 * self.strength) as i32;
                let val = embossed.clamp(0, 255) as u8;

                output.push(RGBA8::new(val, val, val, image_data.pixels[idx1].a));
            }
        }

        ImageData {
            width: image_data.width,
            height: image_data.height,
            pixels: output,
        }
    }
}

pub struct SepiaFilter {
    pub intensity: f32,
}

impl Default for SepiaFilter {
    fn default() -> Self {
        Self { intensity: 1.0 }
    }
}

impl SepiaFilter {
    pub fn new(intensity: f32) -> Self {
        Self {
            intensity: intensity.clamp(0.0, 1.0),
        }
    }

    pub fn apply(&self, image_data: &ImageData) -> ImageData {
        let output: Vec<RGBA8> = image_data
            .pixels
            .iter()
            .map(|pixel| {
                let r = pixel.r as f32;
                let g = pixel.g as f32;
                let b = pixel.b as f32;

                let sepia_r = (r * 0.393 + g * 0.769 + b * 0.189).clamp(0.0, 255.0);
                let sepia_g = (r * 0.349 + g * 0.686 + b * 0.168).clamp(0.0, 255.0);
                let sepia_b = (r * 0.272 + g * 0.534 + b * 0.131).clamp(0.0, 255.0);

                let final_r = r + (sepia_r - r) * self.intensity;
                let final_g = g + (sepia_g - g) * self.intensity;
                let final_b = b + (sepia_b - b) * self.intensity;

                RGBA8::new(
                    final_r.round() as u8,
                    final_g.round() as u8,
                    final_b.round() as u8,
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

pub struct VignetteFilter {
    pub strength: f32,
    pub radius: f32,
}

impl Default for VignetteFilter {
    fn default() -> Self {
        Self {
            strength: 0.5,
            radius: 0.7,
        }
    }
}

impl VignetteFilter {
    pub fn new(strength: f32, radius: f32) -> Self {
        Self {
            strength,
            radius: radius.clamp(0.0, 1.0),
        }
    }

    pub fn apply(&self, image_data: &ImageData) -> ImageData {
        let w = image_data.width as f32;
        let h = image_data.height as f32;
        let cx = w / 2.0;
        let cy = h / 2.0;
        let max_dist = (cx.powi(2) + cy.powi(2)).sqrt();

        let output: Vec<RGBA8> = image_data
            .pixels
            .iter()
            .enumerate()
            .map(|(i, pixel)| {
                let y = (i as f32 / w).floor();
                let x = i as f32 % w;

                let dx = x - cx;
                let dy = y - cy;
                let dist = (dx.powi(2) + dy.powi(2)).sqrt();
                let normalized_dist = dist / (max_dist * self.radius);

                let vignette = (1.0 - normalized_dist.powi(2)).clamp(0.0, 1.0);
                let factor = 1.0 - (1.0 - vignette) * self.strength;

                RGBA8::new(
                    (pixel.r as f32 * factor).round().clamp(0.0, 255.0) as u8,
                    (pixel.g as f32 * factor).round().clamp(0.0, 255.0) as u8,
                    (pixel.b as f32 * factor).round().clamp(0.0, 255.0) as u8,
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

pub struct EdgeGlowFilter {
    pub strength: f32,
    pub color: RGBA8,
}

impl Default for EdgeGlowFilter {
    fn default() -> Self {
        Self {
            strength: 1.0,
            color: RGBA8::new(255, 255, 0, 255),
        }
    }
}

impl EdgeGlowFilter {
    pub fn new(strength: f32, color: RGBA8) -> Self {
        Self { strength, color }
    }

    pub fn apply(&self, image_data: &ImageData) -> ImageData {
        let w = image_data.width as usize;
        let h = image_data.height as usize;

        let gray: Vec<u8> = image_data
            .pixels
            .iter()
            .map(|p| (0.299 * p.r as f64 + 0.587 * p.g as f64 + 0.114 * p.b as f64) as u8)
            .collect();

        let mut edges = vec![0u8; w * h];

        let sobel_x: [i32; 9] = [-1, 0, 1, -2, 0, 2, -1, 0, 1];
        let sobel_y: [i32; 9] = [-1, -2, -1, 0, 0, 0, 1, 2, 1];

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
                        gx += pixel * sobel_x[idx];
                        gy += pixel * sobel_y[idx];
                    }
                }

                let magnitude = ((gx * gx + gy * gy) as f64).sqrt();
                edges[y * w + x] = magnitude.min(255.0) as u8;
            }
        }

        let output: Vec<RGBA8> = edges
            .iter()
            .enumerate()
            .map(|(i, &edge)| {
                let pixel = &image_data.pixels[i];
                if edge > 50 {
                    let glow_factor = (edge as f32 / 255.0) * self.strength;
                    RGBA8::new(
                        (pixel.r as f32 * (1.0 - glow_factor) + self.color.r as f32 * glow_factor)
                            .round()
                            .clamp(0.0, 255.0) as u8,
                        (pixel.g as f32 * (1.0 - glow_factor) + self.color.g as f32 * glow_factor)
                            .round()
                            .clamp(0.0, 255.0) as u8,
                        (pixel.b as f32 * (1.0 - glow_factor) + self.color.b as f32 * glow_factor)
                            .round()
                            .clamp(0.0, 255.0) as u8,
                        pixel.a,
                    )
                } else {
                    *pixel
                }
            })
            .collect();

        ImageData {
            width: image_data.width,
            height: image_data.height,
            pixels: output,
        }
    }
}

pub struct VintageFilter {
    pub contrast: f32,
    pub brightness: f32,
    pub sepia_intensity: f32,
    pub vignette_strength: f32,
}

impl Default for VintageFilter {
    fn default() -> Self {
        Self {
            contrast: 1.1,
            brightness: 1.05,
            sepia_intensity: 0.3,
            vignette_strength: 0.4,
        }
    }
}

impl VintageFilter {
    pub fn new(
        contrast: f32,
        brightness: f32,
        sepia_intensity: f32,
        vignette_strength: f32,
    ) -> Self {
        Self {
            contrast,
            brightness,
            sepia_intensity,
            vignette_strength,
        }
    }

    pub fn apply(&self, image_data: &ImageData) -> ImageData {
        let mut result = image_data.clone();

        result = SepiaFilter::new(self.sepia_intensity).apply(&result);

        let output: Vec<RGBA8> = result
            .pixels
            .iter()
            .map(|pixel| {
                let adjust = |v: u8| {
                    let val = (v as f32 - 128.0) * self.contrast + 128.0;
                    (val * self.brightness).round().clamp(0.0, 255.0) as u8
                };

                RGBA8::new(adjust(pixel.r), adjust(pixel.g), adjust(pixel.b), pixel.a)
            })
            .collect();

        result = ImageData {
            width: image_data.width,
            height: image_data.height,
            pixels: output,
        };

        VignetteFilter::new(self.vignette_strength, 0.7).apply(&result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_emboss_filter() {
        let pixels = vec![RGBA8::new(128, 128, 128, 255); 100];
        let img = ImageData {
            width: 10,
            height: 10,
            pixels,
        };
        let emboss = EmbossFilter::default();
        let result = emboss.apply(&img);
        assert_eq!(result.pixels.len(), 100);
    }

    #[test]
    fn test_sepia_filter() {
        let pixels = vec![RGBA8::new(200, 100, 50, 255); 100];
        let img = ImageData {
            width: 10,
            height: 10,
            pixels,
        };
        let sepia = SepiaFilter::default();
        let result = sepia.apply(&img);
        assert_eq!(result.pixels.len(), 100);
        assert_ne!(result.pixels[0].r, 200);
    }

    #[test]
    fn test_vignette_filter() {
        let pixels = vec![RGBA8::new(255, 255, 255, 255); 100];
        let img = ImageData {
            width: 10,
            height: 10,
            pixels,
        };
        let vignette = VignetteFilter::default();
        let result = vignette.apply(&img);
        assert_eq!(result.pixels.len(), 100);
    }

    #[test]
    fn test_edge_glow_filter() {
        let pixels = vec![RGBA8::new(128, 128, 128, 255); 100];
        let img = ImageData {
            width: 10,
            height: 10,
            pixels,
        };
        let glow = EdgeGlowFilter::default();
        let result = glow.apply(&img);
        assert_eq!(result.pixels.len(), 100);
    }

    #[test]
    fn test_vintage_filter() {
        let pixels = vec![RGBA8::new(128, 128, 128, 255); 100];
        let img = ImageData {
            width: 10,
            height: 10,
            pixels,
        };
        let vintage = VintageFilter::default();
        let result = vintage.apply(&img);
        assert_eq!(result.pixels.len(), 100);
    }
}
