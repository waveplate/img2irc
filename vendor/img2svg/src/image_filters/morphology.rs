//! Morphological operations: erosion, dilation, opening, closing, etc.

use crate::image_processor::ImageData;
use rgb::RGBA8;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MorphologyKind {
    Erosion,
    Dilation,
    Opening,
    Closing,
    Gradient,
    TopHat,
    BottomHat,
}

pub struct MorphologyOp {
    pub kind: MorphologyKind,
    pub radius: u32,
}

impl MorphologyOp {
    pub fn new(kind: MorphologyKind, radius: u32) -> Self {
        Self { kind, radius }
    }

    pub fn apply(&self, image_data: &ImageData) -> ImageData {
        match self.kind {
            MorphologyKind::Erosion => self.erosion(image_data),
            MorphologyKind::Dilation => self.dilation(image_data),
            MorphologyKind::Opening => {
                let eroded = self.erosion(image_data);
                Self::new(MorphologyKind::Dilation, self.radius).dilation(&eroded)
            }
            MorphologyKind::Closing => {
                let dilated = self.dilation(image_data);
                Self::new(MorphologyKind::Erosion, self.radius).erosion(&dilated)
            }
            MorphologyKind::Gradient => {
                let dilated = self.dilation(image_data);
                let eroded = self.erosion(image_data);
                self.gradient(&dilated, &eroded)
            }
            MorphologyKind::TopHat => {
                let opened = Self::new(MorphologyKind::Opening, self.radius).apply(image_data);
                self.subtract(image_data, &opened)
            }
            MorphologyKind::BottomHat => {
                let closed = Self::new(MorphologyKind::Closing, self.radius).apply(image_data);
                self.subtract(&closed, image_data)
            }
        }
    }

    fn erosion(&self, image_data: &ImageData) -> ImageData {
        let w = image_data.width as usize;
        let h = image_data.height as usize;
        let r = self.radius as usize;

        let mut output = Vec::with_capacity(w * h);

        for y in 0..h {
            for x in 0..w {
                let mut min_r = 255u8;
                let mut min_g = 255u8;
                let mut min_b = 255u8;

                let y_start = y.saturating_sub(r);
                let y_end = (y + r + 1).min(h);
                let x_start = x.saturating_sub(r);
                let x_end = (x + r + 1).min(w);

                for ny in y_start..y_end {
                    for nx in x_start..x_end {
                        let p = image_data.pixels[ny * w + nx];
                        min_r = min_r.min(p.r);
                        min_g = min_g.min(p.g);
                        min_b = min_b.min(p.b);
                    }
                }

                output.push(RGBA8::new(
                    min_r,
                    min_g,
                    min_b,
                    image_data.pixels[y * w + x].a,
                ));
            }
        }

        ImageData {
            width: image_data.width,
            height: image_data.height,
            pixels: output,
        }
    }

    fn dilation(&self, image_data: &ImageData) -> ImageData {
        let w = image_data.width as usize;
        let h = image_data.height as usize;
        let r = self.radius as usize;

        let mut output = Vec::with_capacity(w * h);

        for y in 0..h {
            for x in 0..w {
                let mut max_r = 0u8;
                let mut max_g = 0u8;
                let mut max_b = 0u8;

                let y_start = y.saturating_sub(r);
                let y_end = (y + r + 1).min(h);
                let x_start = x.saturating_sub(r);
                let x_end = (x + r + 1).min(w);

                for ny in y_start..y_end {
                    for nx in x_start..x_end {
                        let p = image_data.pixels[ny * w + nx];
                        max_r = max_r.max(p.r);
                        max_g = max_g.max(p.g);
                        max_b = max_b.max(p.b);
                    }
                }

                output.push(RGBA8::new(
                    max_r,
                    max_g,
                    max_b,
                    image_data.pixels[y * w + x].a,
                ));
            }
        }

        ImageData {
            width: image_data.width,
            height: image_data.height,
            pixels: output,
        }
    }

    fn gradient(&self, dilated: &ImageData, eroded: &ImageData) -> ImageData {
        let mut output = Vec::with_capacity(dilated.pixels.len());

        for (i, d) in dilated.pixels.iter().enumerate() {
            let e = &eroded.pixels[i];
            output.push(RGBA8::new(
                d.r.saturating_sub(e.r),
                d.g.saturating_sub(e.g),
                d.b.saturating_sub(e.b),
                d.a,
            ));
        }

        ImageData {
            width: dilated.width,
            height: dilated.height,
            pixels: output,
        }
    }

    fn subtract(&self, img1: &ImageData, img2: &ImageData) -> ImageData {
        let mut output = Vec::with_capacity(img1.pixels.len());

        for (i, p1) in img1.pixels.iter().enumerate() {
            let p2 = &img2.pixels[i];
            output.push(RGBA8::new(
                p1.r.saturating_sub(p2.r),
                p1.g.saturating_sub(p2.g),
                p1.b.saturating_sub(p2.b),
                p1.a,
            ));
        }

        ImageData {
            width: img1.width,
            height: img1.height,
            pixels: output,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_erosion_darkens() {
        let mut pixels = Vec::new();
        for y in 0..5 {
            for x in 0..5 {
                pixels.push(RGBA8::new((y * 50) as u8, (x * 50) as u8, 128, 255));
            }
        }
        let img = ImageData {
            width: 5,
            height: 5,
            pixels,
        };
        let op = MorphologyOp::new(MorphologyKind::Erosion, 1);
        let result = op.apply(&img);

        let original_sum: u32 = img.pixels.iter().map(|p| p.r as u32).sum();
        let result_sum: u32 = result.pixels.iter().map(|p| p.r as u32).sum();
        assert!(result_sum <= original_sum);
    }

    #[test]
    fn test_dilation_brightens() {
        let mut pixels = Vec::new();
        for y in 0..5 {
            for x in 0..5 {
                pixels.push(RGBA8::new((y * 50) as u8, (x * 50) as u8, 128, 255));
            }
        }
        let img = ImageData {
            width: 5,
            height: 5,
            pixels,
        };
        let op = MorphologyOp::new(MorphologyKind::Dilation, 1);
        let result = op.apply(&img);

        let original_sum: u32 = img.pixels.iter().map(|p| p.r as u32).sum();
        let result_sum: u32 = result.pixels.iter().map(|p| p.r as u32).sum();
        assert!(result_sum >= original_sum);
    }

    #[test]
    fn test_opening() {
        let mut pixels = Vec::new();
        for _ in 0..25 {
            pixels.push(RGBA8::new(128, 128, 128, 255));
        }
        let img = ImageData {
            width: 5,
            height: 5,
            pixels,
        };
        let op = MorphologyOp::new(MorphologyKind::Opening, 1);
        let result = op.apply(&img);
        assert_eq!(result.pixels.len(), 25);
    }

    #[test]
    fn test_closing() {
        let mut pixels = Vec::new();
        for _ in 0..25 {
            pixels.push(RGBA8::new(128, 128, 128, 255));
        }
        let img = ImageData {
            width: 5,
            height: 5,
            pixels,
        };
        let op = MorphologyOp::new(MorphologyKind::Closing, 1);
        let result = op.apply(&img);
        assert_eq!(result.pixels.len(), 25);
    }

    #[test]
    fn test_gradient() {
        let mut pixels = Vec::new();
        for _y in 0..5 {
            for x in 0..5 {
                if x < 3 {
                    pixels.push(RGBA8::new(255, 255, 255, 255));
                } else {
                    pixels.push(RGBA8::new(0, 0, 0, 255));
                }
            }
        }
        let img = ImageData {
            width: 5,
            height: 5,
            pixels,
        };
        let op = MorphologyOp::new(MorphologyKind::Gradient, 1);
        let result = op.apply(&img);
        assert_eq!(result.pixels.len(), 25);
    }
}
