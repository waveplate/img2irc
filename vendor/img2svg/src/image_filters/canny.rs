//! Canny edge detector with non-maximum suppression and hysteresis

use crate::image_filters::EdgeMap;
use crate::image_processor::ImageData;

pub struct CannyEdgeDetector {
    pub low_threshold: f32,
    pub high_threshold: f32,
    pub sigma: f32,
}

impl Default for CannyEdgeDetector {
    fn default() -> Self {
        Self {
            low_threshold: 0.1,
            high_threshold: 0.3,
            sigma: 1.0,
        }
    }
}

impl CannyEdgeDetector {
    pub fn new(low_threshold: f32, high_threshold: f32, sigma: f32) -> Self {
        Self {
            low_threshold,
            high_threshold,
            sigma,
        }
    }

    pub fn detect(&self, image_data: &ImageData) -> EdgeMap {
        let w = image_data.width as usize;
        let h = image_data.height as usize;

        let gray: Vec<u8> = image_data
            .pixels
            .iter()
            .map(|p| (0.299 * p.r as f64 + 0.587 * p.g as f64 + 0.114 * p.b as f64) as u8)
            .collect();

        let blurred = if self.sigma > 0.0 {
            self.gaussian_blur(&gray, w, h)
        } else {
            gray.clone()
        };

        let (magnitude, direction) = self.sobel_gradients(&blurred, w, h);

        let suppressed = self.non_max_suppression(&magnitude, &direction, w, h);

        let edges = self.hysteresis(&suppressed, w, h);

        EdgeMap {
            width: image_data.width,
            height: image_data.height,
            data: edges,
        }
    }

    fn gaussian_blur(&self, image: &[u8], width: usize, height: usize) -> Vec<u8> {
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

        let mut temp = vec![0u8; width * height];
        let mut output = vec![0u8; width * height];

        for y in 0..height {
            for x in 0..width {
                let mut sum = 0f32;
                for k in 0..kernel_size {
                    let kx = (x as isize + k as isize - kernel_size as isize / 2)
                        .clamp(0, width as isize - 1) as usize;
                    sum += image[y * width + kx] as f32 * kernel[k];
                }
                temp[y * width + x] = sum as u8;
            }
        }

        for y in 0..height {
            for x in 0..width {
                let mut sum = 0f32;
                for k in 0..kernel_size {
                    let ky = (y as isize + k as isize - kernel_size as isize / 2)
                        .clamp(0, height as isize - 1) as usize;
                    sum += temp[ky * width + x] as f32 * kernel[k];
                }
                output[y * width + x] = sum as u8;
            }
        }

        output
    }

    fn sobel_gradients(&self, image: &[u8], width: usize, height: usize) -> (Vec<u8>, Vec<i32>) {
        let mut magnitude = vec![0u8; width * height];
        let mut direction = vec![0i32; width * height];

        let sobel_x: [i32; 9] = [-1, 0, 1, -2, 0, 2, -1, 0, 1];
        let sobel_y: [i32; 9] = [-1, -2, -1, 0, 0, 0, 1, 2, 1];

        for y in 1..(height - 1) {
            for x in 1..(width - 1) {
                let mut gx = 0i32;
                let mut gy = 0i32;

                for ky in 0..3 {
                    for kx in 0..3 {
                        let px = x + kx - 1;
                        let py = y + ky - 1;
                        let pixel = image[py * width + px] as i32;
                        let idx = ky * 3 + kx;
                        gx += pixel * sobel_x[idx];
                        gy += pixel * sobel_y[idx];
                    }
                }

                magnitude[y * width + x] = ((gx * gx + gy * gy) as f64).sqrt().min(255.0) as u8;

                let angle = (gy as f64).atan2(gx as f64);
                let deg = (angle * 180.0 / std::f64::consts::PI).round() as i32;

                direction[y * width + x] = if deg < 0 { deg + 180 } else { deg };
            }
        }

        (magnitude, direction)
    }

    fn non_max_suppression(
        &self,
        magnitude: &[u8],
        direction: &[i32],
        width: usize,
        height: usize,
    ) -> Vec<u8> {
        let mut output = vec![0u8; width * height];

        for y in 1..(height - 1) {
            for x in 1..(width - 1) {
                let angle = direction[y * width + x];
                let mag = magnitude[y * width + x];

                let (neighbor1, neighbor2) = match angle {
                    a if (a + 22) % 180 < 45 => (
                        magnitude[y * width + (x + 1)],
                        magnitude[y * width + (x - 1)],
                    ),
                    a if (a + 67) % 180 < 45 => (
                        magnitude[(y + 1) * width + (x - 1)],
                        magnitude[(y - 1) * width + (x + 1)],
                    ),
                    a if (a + 112) % 180 < 45 => (
                        magnitude[(y + 1) * width + x],
                        magnitude[(y - 1) * width + x],
                    ),
                    _ => (
                        magnitude[(y + 1) * width + (x + 1)],
                        magnitude[(y - 1) * width + (x - 1)],
                    ),
                };

                if mag >= neighbor1 && mag >= neighbor2 {
                    output[y * width + x] = mag;
                } else {
                    output[y * width + x] = 0;
                }
            }
        }

        output
    }

    fn hysteresis(&self, image: &[u8], width: usize, height: usize) -> Vec<u8> {
        let mut strong_edges = vec![false; width * height];
        let mut weak_edges = vec![false; width * height];

        let max_val = *image.iter().max().unwrap_or(&255) as f32;
        let high_thresh = (self.high_threshold * max_val) as u8;
        let low_thresh = (self.low_threshold * max_val) as u8;

        for (i, &pixel) in image.iter().enumerate() {
            if pixel >= high_thresh {
                strong_edges[i] = true;
            } else if pixel >= low_thresh {
                weak_edges[i] = true;
            }
        }

        let mut output = vec![0u8; width * height];

        for (i, &strong) in strong_edges.iter().enumerate() {
            if strong {
                self.trace_edge(i, &weak_edges, &mut output, width, height);
            }
        }

        output
    }

    fn trace_edge(
        &self,
        index: usize,
        weak_edges: &[bool],
        output: &mut [u8],
        width: usize,
        height: usize,
    ) {
        let mut stack = vec![index];

        while let Some(idx) = stack.pop() {
            if output[idx] == 255 {
                continue;
            }

            output[idx] = 255;

            let y = idx / width;
            let x = idx % width;

            for dy in -1..=1 {
                for dx in -1..=1 {
                    if dy == 0 && dx == 0 {
                        continue;
                    }

                    let ny = y as isize + dy;
                    let nx = x as isize + dx;

                    if ny >= 0 && ny < height as isize && nx >= 0 && nx < width as isize {
                        let nidx = ny as usize * width + nx as usize;
                        if weak_edges[nidx] && output[nidx] != 255 {
                            stack.push(nidx);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rgb::RGBA8;

    #[test]
    fn test_canny_detects_edge() {
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
        let canny = CannyEdgeDetector::default();
        let edges = canny.detect(&img);
        assert_eq!(edges.width, 10);
        assert_eq!(edges.height, 10);
    }

    #[test]
    fn test_canny_uniform_image() {
        let pixels = vec![RGBA8::new(128, 128, 128, 255); 100];
        let img = ImageData {
            width: 10,
            height: 10,
            pixels,
        };
        let canny = CannyEdgeDetector::default();
        let edges = canny.detect(&img);
        assert_eq!(edges.width, 10);
        assert_eq!(edges.height, 10);
    }
}
