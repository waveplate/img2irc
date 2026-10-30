//! Color operations: temperature, saturation, HSL conversion, LAB conversion

use crate::image_processor::ImageData;
use rgb::RGBA8;

pub struct ColorTemperature {
    pub adjustment: i32,
}

impl ColorTemperature {
    pub fn new(adjustment: i32) -> Self {
        Self { adjustment }
    }

    pub fn apply(&self, image_data: &ImageData) -> ImageData {
        let output: Vec<RGBA8> = image_data
            .pixels
            .iter()
            .map(|pixel| {
                let (mut r, g, mut b) = (pixel.r as i32, pixel.g as i32, pixel.b as i32);

                if self.adjustment > 0 {
                    r = (r as f32 * (1.0 + self.adjustment as f32 * 0.002)).round() as i32;
                    b = (b as f32 * (1.0 - self.adjustment as f32 * 0.001)).round() as i32;
                } else {
                    r = (r as f32 * (1.0 + self.adjustment as f32 * 0.001)).round() as i32;
                    b = (b as f32 * (1.0 - self.adjustment as f32 * 0.002)).round() as i32;
                }

                RGBA8::new(
                    r.clamp(0, 255) as u8,
                    g.clamp(0, 255) as u8,
                    b.clamp(0, 255) as u8,
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

pub struct Saturation {
    pub factor: f32,
}

impl Saturation {
    pub fn new(factor: f32) -> Self {
        Self { factor }
    }

    pub fn apply(&self, image_data: &ImageData) -> ImageData {
        let output: Vec<RGBA8> = image_data
            .pixels
            .iter()
            .map(|pixel| {
                let gray =
                    ((0.299 * pixel.r as f64 + 0.587 * pixel.g as f64 + 0.114 * pixel.b as f64)
                        .round()) as i32;

                RGBA8::new(
                    (gray as f32 + self.factor * (pixel.r as f32 - gray as f32))
                        .round()
                        .clamp(0.0, 255.0) as u8,
                    (gray as f32 + self.factor * (pixel.g as f32 - gray as f32))
                        .round()
                        .clamp(0.0, 255.0) as u8,
                    (gray as f32 + self.factor * (pixel.b as f32 - gray as f32))
                        .round()
                        .clamp(0.0, 255.0) as u8,
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

pub struct HSL {
    pub h: f32,
    pub s: f32,
    pub l: f32,
}

impl Default for HSL {
    fn default() -> Self {
        Self {
            h: 0.0,
            s: 1.0,
            l: 1.0,
        }
    }
}

impl HSL {
    pub fn new(h: f32, s: f32, l: f32) -> Self {
        Self { h, s, l }
    }

    pub fn apply(&self, image_data: &ImageData) -> ImageData {
        let output: Vec<RGBA8> = image_data
            .pixels
            .iter()
            .map(|pixel| {
                let (mut h, mut s, mut l) = rgb_to_hsl(pixel.r, pixel.g, pixel.b);

                h = (h + self.h) % 360.0;
                if h < 0.0 {
                    h += 360.0;
                }
                s = (s * self.s).clamp(0.0, 1.0);
                l = (l * self.l).clamp(0.0, 1.0);

                let (r, g, b) = hsl_to_rgb(h, s, l);
                RGBA8::new(r, g, b, pixel.a)
            })
            .collect();

        ImageData {
            width: image_data.width,
            height: image_data.height,
            pixels: output,
        }
    }
}

fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let rf = r as f32 / 255.0;
    let gf = g as f32 / 255.0;
    let bf = b as f32 / 255.0;

    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let delta = max - min;

    let l = (max + min) / 2.0;

    let (h, s) = if delta == 0.0 {
        (0.0, 0.0)
    } else {
        let s = if l < 0.5 {
            delta / (max + min)
        } else {
            delta / (2.0 - max - min)
        };

        let h = if max == rf {
            60.0 * (((gf - bf) / delta) % 6.0)
        } else if max == gf {
            60.0 * (((bf - rf) / delta) + 2.0)
        } else {
            60.0 * (((rf - gf) / delta) + 4.0)
        };

        (h, s)
    };

    (h, s, l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;

    let (rf, gf, bf) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };

    let r = ((rf + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    let g = ((gf + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    let b = ((bf + m) * 255.0).round().clamp(0.0, 255.0) as u8;

    (r, g, b)
}

pub struct LAB;

impl LAB {
    pub fn rgb_to_lab(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
        let (mut r_norm, mut g_norm, mut b_norm) =
            (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);

        for v in [&mut r_norm, &mut g_norm, &mut b_norm] {
            *v = if *v > 0.04045 {
                (*v / 1.055).powf(2.4)
            } else {
                *v / 12.92
            };
        }

        let x = (r_norm * 0.4124 + g_norm * 0.3576 + b_norm * 0.1805) * 100.0;
        let y = (r_norm * 0.2126 + g_norm * 0.7152 + b_norm * 0.0722) * 100.0;
        let z = (r_norm * 0.0193 + g_norm * 0.1192 + b_norm * 0.9505) * 100.0;

        let (fx, fy, fz) = (Self::lab_f(x), Self::lab_f(y), Self::lab_f(z));

        let l = 116.0 * fy - 16.0;
        let a = 500.0 * (fx - fy);
        let b_lab = 200.0 * (fy - fz);

        (l, a, b_lab)
    }

    pub fn lab_to_rgb(l: f32, a: f32, b_lab: f32) -> (u8, u8, u8) {
        let fy = (l + 16.0) / 116.0;
        let fx = a / 500.0 + fy;
        let fz = fy - b_lab / 200.0;

        let x = Self::lab_inv_f(fx) * 95.047;
        let y = Self::lab_inv_f(fy) * 100.0;
        let z = Self::lab_inv_f(fz) * 108.883;

        let mut r_norm = x * 3.2406 + y * -1.5372 + z * -0.4986;
        let mut g_norm = x * -0.9689 + y * 1.8758 + z * 0.0415;
        let mut b_norm = x * 0.0557 + y * -0.2040 + z * 1.0570;

        for v in [&mut r_norm, &mut g_norm, &mut b_norm] {
            *v = if *v > 0.0031308 {
                1.055 * v.powf(1.0 / 2.4) - 0.055
            } else {
                12.92 * *v
            };
        }

        let r = (r_norm * 255.0).round().clamp(0.0, 255.0) as u8;
        let g = (g_norm * 255.0).round().clamp(0.0, 255.0) as u8;
        let b = (b_norm * 255.0).round().clamp(0.0, 255.0) as u8;

        (r, g, b)
    }

    fn lab_f(t: f32) -> f32 {
        let val = 6.0_f32 / 29.0_f32;
        if t > val.powi(3) {
            t.cbrt()
        } else {
            t / (3.0_f32 * val.powi(2))
        }
    }

    fn lab_inv_f(t: f32) -> f32 {
        let val = 6.0_f32 / 29.0_f32;
        if t > val {
            t.powi(3)
        } else {
            3.0_f32 * val.powi(2) * t
        }
    }
}

pub struct ColorOps;

impl ColorOps {
    pub fn balance(image_data: &ImageData, r_shift: i32, g_shift: i32, b_shift: i32) -> ImageData {
        let output: Vec<RGBA8> = image_data
            .pixels
            .iter()
            .map(|pixel| {
                RGBA8::new(
                    (pixel.r as i32 + r_shift).clamp(0, 255) as u8,
                    (pixel.g as i32 + g_shift).clamp(0, 255) as u8,
                    (pixel.b as i32 + b_shift).clamp(0, 255) as u8,
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

    pub fn invert(image_data: &ImageData) -> ImageData {
        let output: Vec<RGBA8> = image_data
            .pixels
            .iter()
            .map(|pixel| RGBA8::new(255 - pixel.r, 255 - pixel.g, 255 - pixel.b, pixel.a))
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
    fn test_color_temperature() {
        let pixels = vec![RGBA8::new(128, 128, 128, 255); 100];
        let img = ImageData {
            width: 10,
            height: 10,
            pixels,
        };
        let temp = ColorTemperature::new(50);
        let result = temp.apply(&img);
        assert_eq!(result.pixels.len(), 100);
    }

    #[test]
    fn test_saturation() {
        let pixels = vec![RGBA8::new(128, 128, 128, 255); 100];
        let img = ImageData {
            width: 10,
            height: 10,
            pixels,
        };
        let sat = Saturation::new(1.5);
        let result = sat.apply(&img);
        assert_eq!(result.pixels.len(), 100);
    }

    #[test]
    fn test_hsl_conversion() {
        let pixels = vec![RGBA8::new(255, 0, 0, 255); 100];
        let img = ImageData {
            width: 10,
            height: 10,
            pixels,
        };
        let hsl = HSL::new(120.0, 1.0, 1.0);
        let result = hsl.apply(&img);
        assert_eq!(result.pixels.len(), 100);
        assert_eq!(result.pixels[0].g, 255);
    }

    #[test]
    fn test_lab_conversion() {
        let (l, a, b) = LAB::rgb_to_lab(255, 0, 0);
        let (r, _g, _b_out) = LAB::lab_to_rgb(l, a, b);
        assert!(r > 200);
    }

    #[test]
    fn test_color_balance() {
        let pixels = vec![RGBA8::new(128, 128, 128, 255); 100];
        let img = ImageData {
            width: 10,
            height: 10,
            pixels,
        };
        let result = ColorOps::balance(&img, 50, 0, 0);
        assert_eq!(result.pixels.len(), 100);
        assert_eq!(result.pixels[0].r, 178);
    }

    #[test]
    fn test_color_invert() {
        let pixels = vec![RGBA8::new(255, 128, 64, 255); 100];
        let img = ImageData {
            width: 10,
            height: 10,
            pixels,
        };
        let result = ColorOps::invert(&img);
        assert_eq!(result.pixels.len(), 100);
        assert_eq!(result.pixels[0].r, 0);
        assert_eq!(result.pixels[0].g, 127);
        assert_eq!(result.pixels[0].b, 191);
    }
}
