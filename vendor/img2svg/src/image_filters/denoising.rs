//! Noise reduction: non-local means and Perona-Malik anisotropic diffusion.

use crate::image_processor::ImageData;
use rgb::RGBA8;

/// Non-local means denoising (grayscale, simplified patch search).
pub struct NonLocalMeans {
    pub patch_radius: u32,
    pub search_radius: u32,
    pub h: f64,
}

impl Default for NonLocalMeans {
    fn default() -> Self {
        Self {
            patch_radius: 1,
            search_radius: 4,
            h: 10.0,
        }
    }
}

impl NonLocalMeans {
    pub fn apply(&self, image_data: &ImageData) -> ImageData {
        let w = image_data.width as usize;
        let h = image_data.height as usize;
        let pr = self.patch_radius as i32;
        let sr = self.search_radius as i32;
        let h2 = self.h * self.h;

        let gray: Vec<f64> = image_data
            .pixels
            .iter()
            .map(|p| 0.299 * p.r as f64 + 0.587 * p.g as f64 + 0.114 * p.b as f64)
            .collect();

        let mut out = image_data.pixels.clone();

        for y in 0..h {
            for x in 0..w {
                let mut weight_sum = 0.0;
                let mut value_sum = 0.0;

                for dy in -sr..=sr {
                    for dx in -sr..=sr {
                        let nx = x as i32 + dx;
                        let ny = y as i32 + dy;
                        if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                            continue;
                        }
                        let nx = nx as usize;
                        let ny = ny as usize;

                        let mut dist = 0.0;
                        for py in -pr..=pr {
                            for px in -pr..=pr {
                                let ax = x as i32 + px;
                                let ay = y as i32 + py;
                                let bx = nx as i32 + px;
                                let by = ny as i32 + py;
                                if ax < 0
                                    || ay < 0
                                    || bx < 0
                                    || by < 0
                                    || ax >= w as i32
                                    || ay >= h as i32
                                    || bx >= w as i32
                                    || by >= h as i32
                                {
                                    continue;
                                }
                                let d = gray[ay as usize * w + ax as usize]
                                    - gray[by as usize * w + bx as usize];
                                dist += d * d;
                            }
                        }

                        let wgt = (-dist / h2).exp();
                        weight_sum += wgt;
                        value_sum += wgt * gray[ny * w + nx];
                    }
                }

                if weight_sum > 0.0 {
                    let g = (value_sum / weight_sum).clamp(0.0, 255.0) as u8;
                    let idx = y * w + x;
                    let a = out[idx].a;
                    out[idx] = RGBA8::new(g, g, g, a);
                }
            }
        }

        ImageData {
            width: image_data.width,
            height: image_data.height,
            pixels: out,
        }
    }
}

/// Perona-Malik anisotropic diffusion on luminance.
pub struct PeronaMalik {
    pub iterations: u32,
    pub kappa: f64,
    pub lambda: f64,
}

impl Default for PeronaMalik {
    fn default() -> Self {
        Self {
            iterations: 10,
            kappa: 15.0,
            lambda: 0.15,
        }
    }
}

impl PeronaMalik {
    pub fn apply(&self, image_data: &ImageData) -> ImageData {
        let w = image_data.width as usize;
        let h = image_data.height as usize;
        let mut lum: Vec<f64> = image_data
            .pixels
            .iter()
            .map(|p| 0.299 * p.r as f64 + 0.587 * p.g as f64 + 0.114 * p.b as f64)
            .collect();

        for _ in 0..self.iterations {
            let prev = lum.clone();
            for y in 1..h - 1 {
                for x in 1..w - 1 {
                    let idx = y * w + x;
                    let n = prev[idx - w] - prev[idx];
                    let s = prev[idx + w] - prev[idx];
                    let e = prev[idx + 1] - prev[idx];
                    let wv = prev[idx - 1] - prev[idx];

                    let cn = diffusion_coeff(n, self.kappa);
                    let cs = diffusion_coeff(s, self.kappa);
                    let ce = diffusion_coeff(e, self.kappa);
                    let cw = diffusion_coeff(wv, self.kappa);

                    lum[idx] = prev[idx]
                        + self.lambda
                            * (cn * n + cs * s + ce * e + cw * wv);
                }
            }
        }

        let pixels: Vec<RGBA8> = lum
            .iter()
            .zip(image_data.pixels.iter())
            .map(|(&g, p)| {
                let v = g.clamp(0.0, 255.0) as u8;
                RGBA8::new(v, v, v, p.a)
            })
            .collect();

        ImageData {
            width: image_data.width,
            height: image_data.height,
            pixels,
        }
    }
}

fn diffusion_coeff(grad: f64, kappa: f64) -> f64 {
    let g = grad.abs();
    (-(g / kappa).powi(2)).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(v: u8) -> ImageData {
        ImageData {
            width: 4,
            height: 4,
            pixels: vec![RGBA8::new(v, v, v, 255); 16],
        }
    }

    #[test]
    fn nlm_uniform_unchanged() {
        let img = solid(128);
        let out = NonLocalMeans::default().apply(&img);
        assert!((out.pixels[0].r as i32 - 128).abs() <= 2);
    }

    #[test]
    fn perona_malik_uniform_stable() {
        let img = solid(100);
        let out = PeronaMalik {
            iterations: 5,
            ..Default::default()
        }
        .apply(&img);
        assert!((out.pixels[0].r as i32 - 100).abs() <= 2);
    }
}
