//! Sobel edge detection for edge-aware quantization.
//!
//! Detects edges in the image using Sobel gradient operators,
//! producing a grayscale edge map used to preserve boundaries
//! during color quantization.

use crate::image_processor::ImageData;

/// Edge detection result: grayscale edge map (0=no edge, 255=strong edge)
#[derive(Debug, Clone)]
pub struct EdgeMap {
    /// Image width in pixels (mirrors `data.len() / height`).
    #[allow(dead_code)] // Read by tests; kept for public API clarity.
    pub width: u32,
    /// Image height in pixels (mirrors `data.len() / width`).
    #[allow(dead_code)] // Read by tests; kept for public API clarity.
    pub height: u32,
    pub data: Vec<u8>,
}

/// Detect edges using Sobel operator on the grayscale version of the image.
pub fn detect_edges_sobel(image_data: &ImageData) -> EdgeMap {
    let w = image_data.width as usize;
    let h = image_data.height as usize;

    // Convert to grayscale luminance
    let gray: Vec<u8> = image_data
        .pixels
        .iter()
        .map(|p| (0.299 * p.r as f64 + 0.587 * p.g as f64 + 0.114 * p.b as f64) as u8)
        .collect();

    let mut edge_buf = vec![0u8; w * h];

    let sobel_x: [i32; 9] = [-1, 0, 1, -2, 0, 2, -1, 0, 1];
    let sobel_y: [i32; 9] = [-1, -2, -1, 0, 0, 0, 1, 2, 1];

    for y in 1..(h - 1) {
        for x in 1..(w - 1) {
            let mut gx = 0i32;
            let mut gy = 0i32;

            for ky in 0..3usize {
                for kx in 0..3usize {
                    let px = x + kx - 1;
                    let py = y + ky - 1;
                    let pixel = gray[py * w + px] as i32;
                    let idx = ky * 3 + kx;
                    gx += pixel * sobel_x[idx];
                    gy += pixel * sobel_y[idx];
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

#[cfg(test)]
mod tests {
    use super::*;
    use rgb::RGBA8;

    #[test]
    fn test_sobel_detects_vertical_edge() {
        // Image with a sharp vertical edge at x=5
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
        let edges = detect_edges_sobel(&img);
        assert_eq!(edges.width, 10);
        assert_eq!(edges.height, 10);
        // Edge should be detected near x=5
        assert!(edges.data[5 * 10 + 5] > 0);
    }

    #[test]
    fn test_sobel_uniform_image_no_edges() {
        let pixels = vec![RGBA8::new(128, 128, 128, 255); 100];
        let img = ImageData {
            width: 10,
            height: 10,
            pixels,
        };
        let edges = detect_edges_sobel(&img);
        // Uniform image should have no edges
        for &v in &edges.data {
            assert_eq!(v, 0);
        }
    }

    #[test]
    fn test_sobel_horizontal_edge() {
        let mut pixels = Vec::new();
        for y in 0..10 {
            for _x in 0..10 {
                if y < 5 {
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
        let edges = detect_edges_sobel(&img);
        // Edge should be detected near y=5
        assert!(edges.data[5 * 10 + 5] > 0);
    }
}
