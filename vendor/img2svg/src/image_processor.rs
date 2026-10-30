pub use anyhow::Result;
use rgb::RGBA8;

/// Source color space of the loaded image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Tested in image_processor_tests; re-exported at crate root.
pub enum ColorSpace {
    /// Standard sRGB or generic RGB.
    Rgb,
    /// CMYK (subtractive) color space, common in print workflows.
    Cmyk,
    /// Grayscale.
    Grayscale,
}

#[derive(Debug, Clone)]
pub struct ImageData {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<RGBA8>,
}

/// Convert CMYK (0-255) to RGB (0-255) using the standard formula:
///
/// R = 255 * (1 - C/255) * (1 - K/255)
/// G = 255 * (1 - M/255) * (1 - K/255)
/// B = 255 * (1 - Y/255) * (1 - K/255)
#[allow(dead_code)] // Tested in image_processor_tests; re-exported at crate root.
pub fn cmyk_to_rgb(c: u8, m: u8, y: u8, k: u8) -> (u8, u8, u8) {
    let c = c as f64 / 255.0;
    let m = m as f64 / 255.0;
    let y = y as f64 / 255.0;
    let k = k as f64 / 255.0;

    let r = 255.0 * (1.0 - c) * (1.0 - k);
    let g = 255.0 * (1.0 - m) * (1.0 - k);
    let b = 255.0 * (1.0 - y) * (1.0 - k);

    (
        r.round().clamp(0.0, 255.0) as u8,
        g.round().clamp(0.0, 255.0) as u8,
        b.round().clamp(0.0, 255.0) as u8,
    )
}

/// Create `ImageData` from raw CMYK pixels (each pixel = 4 bytes: C, M, Y, K).
/// Applies standard CMYK -> RGB conversion.
#[allow(dead_code)] // Tested in image_processor_tests; re-exported at crate root.
pub fn image_data_from_cmyk(cmyk_pixels: &[(u8, u8, u8, u8)], width: u32, height: u32) -> ImageData {
    let pixels: Vec<RGBA8> = cmyk_pixels
        .iter()
        .map(|&(c, m, y, k)| {
            let (r, g, b) = cmyk_to_rgb(c, m, y, k);
            RGBA8::new(r, g, b, 255)
        })
        .collect();
    ImageData {
        width,
        height,
        pixels,
    }
}

/// Try to detect the source color space of an image file by inspecting its bytes.
/// For JPEG, checks the APP14 Adobe marker for CMYK (transform == 2).
/// Falls back to `Rgb` for other formats.
#[allow(dead_code)] // Tested in image_processor_tests; re-exported at crate root.
pub fn detect_color_space(path: &std::path::Path) -> ColorSpace {
    use std::fs::read;

    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if !ext.eq_ignore_ascii_case("jpg") && !ext.eq_ignore_ascii_case("jpeg") {
        return ColorSpace::Rgb;
    }

    let bytes = match read(path) {
        Ok(b) => b,
        Err(_) => return ColorSpace::Rgb,
    };

    // Scan for APP14 'Adobe' marker (0xFFEE followed by length and 'Adobe\0')
    let mut i = 0;
    while i + 16 < bytes.len() {
        if bytes[i] == 0xFF && bytes[i + 1] == 0xEE {
            // APP14 marker found
            let seg_len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
            if i + seg_len + 2 <= bytes.len()
                && &bytes[i + 4..i + 10] == b"Adobe\0"
            {
                // Transform byte at offset 11 within the APP14 segment
                let transform = bytes.get(i + 4 + 11).copied().unwrap_or(0);
                // transform == 0: YCbCr, 1: YCCK (CMYK), 2: unknown
                if transform == 1 || transform == 2 {
                    return ColorSpace::Cmyk;
                }
            }
        }
        i += 1;
    }

    ColorSpace::Rgb
}

pub fn load_image(path: &std::path::Path) -> Result<ImageData> {
    let img = image::open(path)?;
    let rgba = img.to_rgba8();

    let pixels: Vec<RGBA8> = rgba
        .pixels()
        .map(|p| RGBA8::new(p[0], p[1], p[2], p[3]))
        .collect();

    Ok(ImageData {
        width: rgba.width(),
        height: rgba.height(),
        pixels,
    })
}

/// A single frame from an animated GIF with its delay in milliseconds.
#[derive(Debug, Clone)]
pub struct GifFrame {
    pub image_data: ImageData,
    /// Frame delay in milliseconds.
    #[allow(dead_code)] // Read via re-export at crate root.
    pub delay_ms: u32,
}

/// Load all frames from an animated GIF file.
/// Returns each frame as `ImageData` along with its display delay in milliseconds.
/// For static GIFs, returns a single frame with delay=0.
pub fn load_animated_gif(path: &std::path::Path) -> Result<Vec<GifFrame>> {
    use image::codecs::gif::GifDecoder;
    use image::AnimationDecoder;
    use std::fs::File;
    use std::io::BufReader;

    let file = File::open(path)?;
    let decoder = GifDecoder::new(BufReader::new(file))?;
    let frames = decoder.into_frames().collect_frames()?;

    let mut result = Vec::with_capacity(frames.len());
    for frame in frames {
        let delay = frame.delay();
        let buf = frame.into_buffer();
        // image::Delay stores delay as (numerator, denominator) in milliseconds.
        let (numer, denom) = delay.numer_denom_ms();
        let delay_ms = if denom == 0 {
            100
        } else {
            let ms = (numer as f64 / denom as f64).round() as u32;
            ms.max(10) // GIF minimum practical delay
        };

        let pixels: Vec<RGBA8> = buf
            .pixels()
            .map(|p| RGBA8::new(p[0], p[1], p[2], p[3]))
            .collect();

        result.push(GifFrame {
            image_data: ImageData {
                width: buf.width(),
                height: buf.height(),
                pixels,
            },
            delay_ms,
        });
    }

    Ok(result)
}

/// Resize image if either dimension exceeds max_size, maintaining aspect ratio.
/// Uses Lanczos3 for high-quality downscaling.
pub fn resize_if_needed(image_data: ImageData, max_size: u32) -> ImageData {
    let (w, h) = (image_data.width, image_data.height);
    if w <= max_size && h <= max_size {
        return image_data;
    }

    let scale = (max_size as f64 / w as f64).min(max_size as f64 / h as f64);
    let new_w = (w as f64 * scale).round() as u32;
    let new_h = (h as f64 * scale).round() as u32;

    eprintln!(
        "  Auto-resizing {}x{} -> {}x{} (max_size={})",
        w, h, new_w, new_h, max_size
    );

    // Convert to image::RgbaImage for resizing
    let mut rgba_img = image::RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let idx = (y * w + x) as usize;
            let p = image_data.pixels[idx];
            rgba_img.put_pixel(x, y, image::Rgba([p.r, p.g, p.b, p.a]));
        }
    }

    let resized = image::imageops::resize(
        &rgba_img,
        new_w,
        new_h,
        image::imageops::FilterType::Lanczos3,
    );

    let pixels: Vec<RGBA8> = resized
        .pixels()
        .map(|p| RGBA8::new(p[0], p[1], p[2], p[3]))
        .collect();

    ImageData {
        width: new_w,
        height: new_h,
        pixels,
    }
}

/// Median-cut color quantization for better color space coverage.
pub fn quantize_colors(image_data: &ImageData, num_colors: usize) -> Result<ImageData> {
    if num_colors == 0 {
        return Err(anyhow::anyhow!("num_colors must be greater than 0"));
    }

    let palette = median_cut(&image_data.pixels, num_colors);

    let mut quantized_pixels = Vec::with_capacity(image_data.pixels.len());
    for pixel in &image_data.pixels {
        let closest = palette
            .iter()
            .min_by_key(|c| {
                let dr = c.r as i32 - pixel.r as i32;
                let dg = c.g as i32 - pixel.g as i32;
                let db = c.b as i32 - pixel.b as i32;
                dr * dr + dg * dg + db * db
            })
            .ok_or_else(|| anyhow::anyhow!("Failed to quantize: empty palette"))?;
        quantized_pixels.push(RGBA8::new(closest.r, closest.g, closest.b, pixel.a));
    }

    Ok(ImageData {
        width: image_data.width,
        height: image_data.height,
        pixels: quantized_pixels,
    })
}

/// Median-cut: recursively split the color box along its widest channel.
fn median_cut(pixels: &[RGBA8], num_colors: usize) -> Vec<RGBA8> {
    if num_colors == 0 {
        return vec![];
    }

    // Collect unique-ish colors (sample for performance on large images)
    let mut colors: Vec<(u8, u8, u8)> = Vec::new();
    let step = (pixels.len() / 50000).max(1);
    for (i, p) in pixels.iter().enumerate() {
        if i % step == 0 {
            colors.push((p.r, p.g, p.b));
        }
    }
    if colors.is_empty() {
        return vec![RGBA8::new(0, 0, 0, 255)];
    }

    let mut boxes: Vec<Vec<(u8, u8, u8)>> = vec![colors];
    while boxes.len() < num_colors {
        // Find the box with the largest range to split
        let mut best_idx = 0;
        let mut best_range = 0u16;
        for (i, b) in boxes.iter().enumerate() {
            let range = box_max_range(b);
            if range > best_range || (range == best_range && b.len() > boxes[best_idx].len()) {
                best_range = range;
                best_idx = i;
            }
        }
        if boxes[best_idx].len() < 2 {
            break;
        }
        let to_split = boxes.remove(best_idx);
        let (a, b) = split_box(to_split);
        if !a.is_empty() {
            boxes.push(a);
        }
        if !b.is_empty() {
            boxes.push(b);
        }
    }

    boxes.iter().map(|b| box_average(b)).collect()
}

pub fn box_max_range(colors: &[(u8, u8, u8)]) -> u16 {
    let (mut rmin, mut rmax) = (255u8, 0u8);
    let (mut gmin, mut gmax) = (255u8, 0u8);
    let (mut bmin, mut bmax) = (255u8, 0u8);
    for &(r, g, b) in colors {
        rmin = rmin.min(r);
        rmax = rmax.max(r);
        gmin = gmin.min(g);
        gmax = gmax.max(g);
        bmin = bmin.min(b);
        bmax = bmax.max(b);
    }
    let rr = (rmax - rmin) as u16;
    let gr = (gmax - gmin) as u16;
    let br = (bmax - bmin) as u16;
    rr.max(gr).max(br)
}

pub fn split_box(mut colors: Vec<(u8, u8, u8)>) -> (Vec<(u8, u8, u8)>, Vec<(u8, u8, u8)>) {
    let (mut rmin, mut rmax) = (255u8, 0u8);
    let (mut gmin, mut gmax) = (255u8, 0u8);
    let (mut bmin, mut bmax) = (255u8, 0u8);
    for &(r, g, b) in &colors {
        rmin = rmin.min(r);
        rmax = rmax.max(r);
        gmin = gmin.min(g);
        gmax = gmax.max(g);
        bmin = bmin.min(b);
        bmax = bmax.max(b);
    }
    let rr = rmax - rmin;
    let gr = gmax - gmin;
    let br = bmax - bmin;

    if rr >= gr && rr >= br {
        colors.sort_by_key(|c| c.0);
    } else if gr >= br {
        colors.sort_by_key(|c| c.1);
    } else {
        colors.sort_by_key(|c| c.2);
    }

    let mid = colors.len() / 2;
    let right = colors.split_off(mid);
    (colors, right)
}

pub fn box_average(colors: &[(u8, u8, u8)]) -> RGBA8 {
    if colors.is_empty() {
        return RGBA8::new(0, 0, 0, 255);
    }
    let (mut sr, mut sg, mut sb) = (0u64, 0u64, 0u64);
    for &(r, g, b) in colors {
        sr += r as u64;
        sg += g as u64;
        sb += b as u64;
    }
    let n = colors.len() as u64;
    RGBA8::new((sr / n) as u8, (sg / n) as u8, (sb / n) as u8, 255)
}

#[cfg(test)]
mod tests {
    include!("image_processor_tests.rs");
}
