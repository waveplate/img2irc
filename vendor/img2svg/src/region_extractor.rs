//! Background color detection via border-pixel frequency.
//!
//! This module is a small helper used by the enhanced vectorizer to choose
//! a sensible background fill when rendering the SVG. Heavier region
//! extraction utilities previously housed here were removed once the
//! pipeline migrated to marching-squares contour tracing.

use crate::image_processor::ImageData;
use std::collections::HashMap;

/// Detect background color by sampling border pixels (most frequent color).
pub fn detect_background_color(image_data: &ImageData) -> (u8, u8, u8, u8) {
    let w = image_data.width as usize;
    let h = image_data.height as usize;
    if w == 0 || h == 0 {
        return (255, 255, 255, 255);
    }

    let mut color_counts: HashMap<u32, (usize, (u8, u8, u8, u8))> = HashMap::new();

    let mut sample_border = |x: usize, y: usize| {
        let p = &image_data.pixels[y * w + x];
        let key = (p.r as u32) << 16 | (p.g as u32) << 8 | p.b as u32;
        let entry = color_counts.entry(key).or_insert((0, (p.r, p.g, p.b, p.a)));
        entry.0 += 1;
    };

    for x in 0..w {
        sample_border(x, 0);
        sample_border(x, h - 1);
    }
    for y in 1..h - 1 {
        sample_border(0, y);
        sample_border(w - 1, y);
    }

    // Deterministic tie-breaking: highest count wins; on tie, prefer lighter color
    // (lighter colors are more common backgrounds). Uses luminance as tie-breaker.
    color_counts
        .values()
        .max_by(|(count_a, ca), (count_b, cb)| {
            count_a.cmp(count_b).then_with(|| {
                let lum_a = ca.0 as u32 * 299 + ca.1 as u32 * 587 + ca.2 as u32 * 114;
                let lum_b = cb.0 as u32 * 299 + cb.1 as u32 * 587 + cb.2 as u32 * 114;
                lum_a.cmp(&lum_b)
            })
        })
        .map(|(_, color)| *color)
        .unwrap_or((255, 255, 255, 255))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rgb::RGBA8;

    #[test]
    fn test_detect_background_color() {
        // 4x4 image, border is red, center is blue
        let mut pixels = vec![RGBA8::new(255, 0, 0, 255); 16];
        pixels[5] = RGBA8::new(0, 0, 255, 255);
        pixels[6] = RGBA8::new(0, 0, 255, 255);
        pixels[9] = RGBA8::new(0, 0, 255, 255);
        pixels[10] = RGBA8::new(0, 0, 255, 255);
        let img = ImageData {
            width: 4,
            height: 4,
            pixels,
        };
        let bg = detect_background_color(&img);
        assert_eq!(bg, (255, 0, 0, 255));
    }
}
