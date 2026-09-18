//! Source-pixel adjustments shared by the browser and terminal pipeline.
use photon_rs::PhotonImage;

pub fn quantize_rgb(rgb: [u8; 3], palette: &[u32]) -> [u8; 3] {
    if palette.is_empty() {
        return rgb;
    }
    let packed = u32::from(rgb[0]) << 16 | u32::from(rgb[1]) << 8 | u32::from(rgb[2]);
    let index = crate::draw::nearest_hex_colour_fast(packed, palette) as usize;
    let colour = palette[index.min(palette.len() - 1)];
    [
        ((colour >> 16) & 0xff) as u8,
        ((colour >> 8) & 0xff) as u8,
        (colour & 0xff) as u8,
    ]
}

pub fn replace_colour(image: &mut PhotonImage, from: [u8; 3], to: [u8; 3], tolerance: f32) {
    let mut pixels = image.get_raw_pixels();
    let limit = tolerance.clamp(0.0, 100.0) * 255.0 / 100.0;
    for pixel in pixels.chunks_exact_mut(4) {
        if pixel[3] == 0 {
            continue;
        }
        let distance = (0..3)
            .map(|c| (pixel[c] as f32 - from[c] as f32).powi(2))
            .sum::<f32>()
            / 3.0;
        if distance <= limit * limit {
            pixel[..3].copy_from_slice(&to);
        }
    }
    *image = PhotonImage::new(pixels, image.get_width(), image.get_height());
}

/// Replace palette-visible colours rather than inaccessible source RGB values.
/// Every source pixel is first assigned to the same palette bucket used by the
/// renderer, so selecting a displayed colour replaces the complete bucket.
pub fn replace_colour_quantized(
    image: &mut PhotonImage,
    from: [u8; 3],
    to: [u8; 3],
    tolerance: f32,
    palette: &[u32],
) {
    let mut pixels = image.get_raw_pixels();
    let from = quantize_rgb(from, palette);
    let to = quantize_rgb(to, palette);
    let limit = tolerance.clamp(0.0, 100.0) * 255.0 / 100.0;
    for pixel in pixels.chunks_exact_mut(4) {
        if pixel[3] == 0 {
            continue;
        }
        let visible = quantize_rgb([pixel[0], pixel[1], pixel[2]], palette);
        let distance = (0..3)
            .map(|c| (visible[c] as f32 - from[c] as f32).powi(2))
            .sum::<f32>()
            / 3.0;
        if distance <= limit * limit {
            pixel[..3].copy_from_slice(&to);
        }
    }
    *image = PhotonImage::new(pixels, image.get_width(), image.get_height());
}

/// Shift all three channels equally, preserving their differences (chroma).
/// Limit the shift at the RGB gamut boundary instead of clipping channels separately.
pub fn luma_contrast(image: &mut PhotonImage, contrast: f32) {
    if contrast == 0.0 || !contrast.is_finite() {
        return;
    }
    let c = contrast.clamp(-255.0, 255.0);
    let factor = 259.0 * (c + 255.0) / (255.0 * (259.0 - c));
    let mut pixels = image.get_raw_pixels();
    for pixel in pixels.chunks_exact_mut(4) {
        if pixel[3] == 0 {
            continue;
        }
        let y = 0.2126 * pixel[0] as f32 + 0.7152 * pixel[1] as f32 + 0.0722 * pixel[2] as f32;
        let lo = *pixel[..3].iter().min().unwrap() as f32;
        let hi = *pixel[..3].iter().max().unwrap() as f32;
        let shift = ((factor - 1.0) * (y - 128.0))
            .clamp(-lo, 255.0 - hi)
            .round() as i16;
        for channel in &mut pixel[..3] {
            *channel = (*channel as i16 + shift) as u8;
        }
    }
    *image = PhotonImage::new(pixels, image.get_width(), image.get_height());
}

/// Sliding RGB histograms give a median without sorting each neighbourhood.
/// Transparent neighbours are excluded; the centre pixel's alpha is retained.
pub fn median_blur(image: &mut PhotonImage, radius: i32) {
    let radius = radius.clamp(0, 10) as usize;
    if radius == 0 {
        return;
    }
    let (w, h) = (image.get_width() as usize, image.get_height() as usize);
    if w == 0 || h == 0 {
        return;
    }
    let source = image.get_raw_pixels();
    let mut output = source.clone();
    for y in 0..h {
        let mut histogram = [[0u32; 256]; 3];
        let mut count = 0u32;
        let mut column = |x: usize, add: bool, histogram: &mut [[u32; 256]; 3]| {
            for ny in y.saturating_sub(radius)..=(y + radius).min(h - 1) {
                let p = &source[(ny * w + x) * 4..][..4];
                if p[3] == 0 {
                    continue;
                }
                if add {
                    count += 1;
                } else {
                    count -= 1;
                }
                for c in 0..3 {
                    if add {
                        histogram[c][p[c] as usize] += 1;
                    } else {
                        histogram[c][p[c] as usize] -= 1;
                    }
                }
            }
            count
        };
        let mut size = 0;
        for x in 0..=radius.min(w - 1) {
            size = column(x, true, &mut histogram);
        }
        for x in 0..w {
            let offset = (y * w + x) * 4;
            if source[offset + 3] != 0 && size > 0 {
                for c in 0..3 {
                    let mut sum = 0;
                    for value in 0..256 {
                        sum += histogram[c][value];
                        if sum > size / 2 {
                            output[offset + c] = value as u8;
                            break;
                        }
                    }
                }
            }
            if x >= radius {
                size = column(x - radius, false, &mut histogram);
            }
            if x + radius + 1 < w {
                size = column(x + radius + 1, true, &mut histogram);
            }
        }
    }
    *image = PhotonImage::new(output, w as u32, h as u32);
}

/// Positive amounts grow lines; negative amounts shrink them. Select whole RGB
/// pixels by luma so morphology does not invent colours at coloured edges.
pub fn line_thickness(image: &mut PhotonImage, amount: i32, light_lines: bool) {
    let radius = amount.unsigned_abs().min(10) as usize;
    if radius == 0 {
        return;
    }
    let (w, h) = (image.get_width() as usize, image.get_height() as usize);
    if w == 0 || h == 0 {
        return;
    }
    let choose_light = (amount > 0) == light_lines;
    let mut source = image.get_raw_pixels();
    // A square min/max filter is separable into horizontal and vertical passes.
    for vertical in [false, true] {
        let mut output = source.clone();
        for y in 0..h {
            for x in 0..w {
                let offset = (y * w + x) * 4;
                if source[offset + 3] == 0 {
                    continue;
                }
                let mut best = offset;
                let luma = |i: usize| {
                    2126u32 * source[i] as u32
                        + 7152u32 * source[i + 1] as u32
                        + 722u32 * source[i + 2] as u32
                };
                let pos = if vertical { y } else { x };
                let end = if vertical { h } else { w };
                for n in pos.saturating_sub(radius)..=(pos + radius).min(end - 1) {
                    let candidate = if vertical {
                        (n * w + x) * 4
                    } else {
                        (y * w + n) * 4
                    };
                    if source[candidate + 3] != 0
                        && if choose_light {
                            luma(candidate) > luma(best)
                        } else {
                            luma(candidate) < luma(best)
                        }
                    {
                        best = candidate;
                    }
                }
                output[offset..offset + 3].copy_from_slice(&source[best..best + 3]);
            }
        }
        source = output;
    }
    *image = PhotonImage::new(source, w as u32, h as u32);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_tolerance_and_alpha() {
        let mut img = PhotonImage::new(
            vec![
                255, 255, 255, 255, 248, 248, 248, 128, 0, 0, 0, 255, 255, 255, 255, 0,
            ],
            4,
            1,
        );
        replace_colour(&mut img, [255; 3], [30, 40, 50], 3.0);
        assert_eq!(
            img.get_raw_pixels(),
            vec![30, 40, 50, 255, 30, 40, 50, 128, 0, 0, 0, 255, 255, 255, 255, 0]
        );
    }

    #[test]
    fn quantized_replacement_matches_the_visible_palette_bucket() {
        let mut img = PhotonImage::new(vec![250, 8, 8, 255, 180, 20, 20, 255, 250, 8, 8, 0], 3, 1);
        replace_colour_quantized(
            &mut img,
            [255, 0, 0],
            [0, 255, 0],
            0.0,
            &crate::palette::ANSI256,
        );
        let pixels = img.get_raw_pixels();
        assert_eq!(&pixels[0..3], &[0, 255, 0]);
        assert_eq!(&pixels[4..7], &[180, 20, 20]);
        assert_eq!(&pixels[8..11], &[250, 8, 8]);
    }
    #[test]
    fn contrast_preserves_chroma_even_at_gamut_boundary() {
        let mut img = PhotonImage::new(vec![180, 150, 130, 77, 250, 210, 180, 255], 2, 1);
        luma_contrast(&mut img, 200.0);
        let p = img.get_raw_pixels();
        assert!(p[0] > 180);
        assert_eq!(p[0] - p[1], 30);
        assert_eq!(p[1] - p[2], 20);
        assert_eq!(&p[4..7], &[255, 215, 185]);
        assert_eq!(p[3], 77);
    }
    #[test]
    fn median_removes_isolated_noise_and_zero_is_identity() {
        let mut pixels = vec![255; 5 * 5 * 4];
        pixels[48..51].fill(0);
        let mut img = PhotonImage::new(pixels.clone(), 5, 5);
        median_blur(&mut img, 0);
        assert_eq!(img.get_raw_pixels(), pixels);
        median_blur(&mut img, 1);
        assert!(img.get_raw_pixels().iter().all(|v| *v == 255));
    }
    #[test]
    fn dark_and_light_lines_grow_and_shrink() {
        for light in [false, true] {
            let bg = if light { 0 } else { 255 };
            let fg = 255 - bg;
            let pixels: Vec<u8> = (0..7)
                .flat_map(|x| {
                    let c = if x == 3 { fg } else { bg };
                    [c, c, c, 255]
                })
                .collect();
            let mut img = PhotonImage::new(pixels, 7, 1);
            line_thickness(&mut img, 1, light);
            assert_eq!(
                img.get_raw_pixels()
                    .chunks_exact(4)
                    .filter(|p| p[0] == fg)
                    .count(),
                3
            );
            line_thickness(&mut img, -1, light);
            assert_eq!(
                img.get_raw_pixels()
                    .chunks_exact(4)
                    .filter(|p| p[0] == fg)
                    .count(),
                1
            );
        }
    }
}
