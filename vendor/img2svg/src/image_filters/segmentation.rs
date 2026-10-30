//! Segmentation: watershed, mean shift, and SLIC superpixels.

use crate::image_processor::ImageData;
use rgb::RGBA8;
use std::collections::HashMap;

/// Marker-controlled watershed on gradient magnitude.
pub fn watershed_segment(image_data: &ImageData, num_markers: usize) -> Vec<u32> {
    let w = image_data.width as usize;
    let h = image_data.height as usize;
    let n = w * h;
    if n == 0 {
        return Vec::new();
    }

    let gray: Vec<f64> = image_data
        .pixels
        .iter()
        .map(|p| 0.299 * p.r as f64 + 0.587 * p.g as f64 + 0.114 * p.b as f64)
        .collect();

    let mut grad = vec![0.0f64; n];
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let gx = gray[y * w + x + 1] - gray[y * w + x - 1];
            let gy = gray[(y + 1) * w + x] - gray[(y - 1) * w + x];
            grad[y * w + x] = (gx * gx + gy * gy).sqrt();
        }
    }

    let mut labels = vec![0u32; n];
    let grid = (num_markers.max(1) as f64).sqrt().ceil() as usize;
    let step_x = (w / grid.max(1)).max(1);
    let step_y = (h / grid.max(1)).max(1);
    let mut next = 1u32;
    for y in (step_y / 2..h).step_by(step_y.max(1)) {
        for x in (step_x / 2..w).step_by(step_x.max(1)) {
            if next as usize > num_markers {
                break;
            }
            labels[y * w + x] = next;
            next += 1;
        }
    }

    for _ in 0..64 {
        let mut changed = false;
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                let idx = y * w + x;
                if labels[idx] != 0 {
                    continue;
                }
                let mut best = 0u32;
                let mut best_g = f64::MAX;
                for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                    let nx = (x as i32 + dx) as usize;
                    let ny = (y as i32 + dy) as usize;
                    let nl = labels[ny * w + nx];
                    if nl != 0 && grad[idx] < best_g {
                        best_g = grad[idx];
                        best = nl;
                    }
                }
                if best != 0 {
                    labels[idx] = best;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }

    labels
}

/// Mean shift filtering (spatial+range clustering per pixel, single pass).
pub fn mean_shift_filter(image_data: &ImageData, spatial_radius: f64, color_radius: f64) -> ImageData {
    let w = image_data.width as usize;
    let h = image_data.height as usize;
    let sr = spatial_radius as i32;
    let cr2 = color_radius * color_radius;
    let mut out = image_data.pixels.clone();

    for y in 0..h {
        for x in 0..w {
            let p = image_data.pixels[y * w + x];
            let mut sx = 0.0;
            let mut sy = 0.0;
            let mut sr_sum = 0.0;
            let mut sg = 0.0;
            let mut sb = 0.0;
            let mut count = 0.0;

            for dy in -sr..=sr {
                for dx in -sr..=sr {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        continue;
                    }
                    let q = image_data.pixels[ny as usize * w + nx as usize];
                    let dr = p.r as f64 - q.r as f64;
                    let dg = p.g as f64 - q.g as f64;
                    let db = p.b as f64 - q.b as f64;
                    if dr * dr + dg * dg + db * db <= cr2 {
                        sx += nx as f64;
                        sy += ny as f64;
                        sr_sum += q.r as f64;
                        sg += q.g as f64;
                        sb += q.b as f64;
                        count += 1.0;
                    }
                }
            }

            if count > 0.0 {
                out[y * w + x] = RGBA8::new(
                    (sr_sum / count).round() as u8,
                    (sg / count).round() as u8,
                    (sb / count).round() as u8,
                    p.a,
                );
            }
            let _ = (sx, sy);
        }
    }

    ImageData {
        width: image_data.width,
        height: image_data.height,
        pixels: out,
    }
}

/// SLIC superpixel segmentation. Returns label per pixel (1-based).
pub fn slic_superpixels(image_data: &ImageData, num_superpixels: usize, iterations: u32) -> Vec<u32> {
    let w = image_data.width as usize;
    let h = image_data.height as usize;
    let n = w * h;
    if n == 0 {
        return Vec::new();
    }

    let k = num_superpixels.max(1);
    let step = ((n as f64 / k as f64).sqrt()).max(1.0) as usize;
    let compactness = 10.0f64;

    let mut centers: Vec<(f64, f64, f64, f64, f64)> = Vec::new();
    for y in (step / 2..h).step_by(step.max(1)) {
        for x in (step / 2..w).step_by(step.max(1)) {
            let p = image_data.pixels[y * w + x];
            centers.push((
                x as f64,
                y as f64,
                p.r as f64,
                p.g as f64,
                p.b as f64,
            ));
            if centers.len() >= k {
                break;
            }
        }
        if centers.len() >= k {
            break;
        }
    }

    let mut labels = vec![0u32; n];
    let search = step as i32;

    for _ in 0..iterations.max(1) {
        labels.fill(0);
        let mut dists = vec![f64::MAX; n];

        for (ci, &(cx, cy, cr, cg, cb)) in centers.iter().enumerate() {
            let cid = (ci + 1) as u32;
            let x0 = ((cx - search as f64) as i32).max(0) as usize;
            let y0 = ((cy - search as f64) as i32).max(0) as usize;
            let x1 = ((cx + search as f64) as i32).min(w as i32 - 1) as usize;
            let y1 = ((cy + search as f64) as i32).min(h as i32 - 1) as usize;

            for y in y0..=y1 {
                for x in x0..=x1 {
                    let idx = y * w + x;
                    let p = image_data.pixels[idx];
                    let dc = ((p.r as f64 - cr).powi(2)
                        + (p.g as f64 - cg).powi(2)
                        + (p.b as f64 - cb).powi(2))
                    .sqrt();
                    let ds = ((x as f64 - cx).powi(2) + (y as f64 - cy).powi(2)).sqrt();
                    let d = ds + compactness * dc / step as f64;
                    if d < dists[idx] {
                        dists[idx] = d;
                        labels[idx] = cid;
                    }
                }
            }
        }

        for (ci, center) in centers.iter_mut().enumerate() {
            let cid = (ci + 1) as u32;
            let mut sx = 0.0;
            let mut sy = 0.0;
            let mut sr = 0.0;
            let mut sg = 0.0;
            let mut sb = 0.0;
            let mut count = 0.0;
            for (idx, &lab) in labels.iter().enumerate() {
                if lab == cid {
                    let x = idx % w;
                    let y = idx / w;
                    let p = image_data.pixels[idx];
                    sx += x as f64;
                    sy += y as f64;
                    sr += p.r as f64;
                    sg += p.g as f64;
                    sb += p.b as f64;
                    count += 1.0;
                }
            }
            if count > 0.0 {
                *center = (
                    sx / count,
                    sy / count,
                    sr / count,
                    sg / count,
                    sb / count,
                );
            }
        }
    }

    labels
}

/// Build quantized image from superpixel labels (mean color per label).
pub fn quantize_from_labels(image_data: &ImageData, labels: &[u32]) -> ImageData {
    let mut sums: HashMap<u32, (u64, u64, u64, u64, u64)> = HashMap::new();

    for (idx, &lab) in labels.iter().enumerate() {
        if lab == 0 {
            continue;
        }
        let p = image_data.pixels[idx];
        let e = sums.entry(lab).or_insert((0, 0, 0, 0, 0));
        e.0 += p.r as u64;
        e.1 += p.g as u64;
        e.2 += p.b as u64;
        e.3 += p.a as u64;
        e.4 += 1;
    }

    let mut means: HashMap<u32, RGBA8> = HashMap::new();
    for (lab, (r, g, b, a, c)) in sums {
        let c = c.max(1);
        means.insert(
            lab,
            RGBA8::new(
                (r / c) as u8,
                (g / c) as u8,
                (b / c) as u8,
                (a / c) as u8,
            ),
        );
    }

    let mut pixels = image_data.pixels.clone();
    for (idx, &lab) in labels.iter().enumerate() {
        if let Some(c) = means.get(&lab) {
            pixels[idx] = *c;
        }
    }

    ImageData {
        width: image_data.width,
        height: image_data.height,
        pixels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checker(w: u32, h: u32) -> ImageData {
        let mut pixels = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let v = if (x + y) % 2 == 0 { 255 } else { 0 };
                pixels.push(RGBA8::new(v, v, v, 255));
            }
        }
        ImageData { width: w, height: h, pixels }
    }

    #[test]
    fn watershed_produces_labels() {
        let labels = watershed_segment(&checker(20, 20), 4);
        assert_eq!(labels.len(), 400);
        assert!(labels.iter().any(|&l| l > 0));
    }

    #[test]
    fn mean_shift_runs() {
        let out = mean_shift_filter(&checker(8, 8), 2.0, 40.0);
        assert_eq!(out.pixels.len(), 64);
    }

    #[test]
    fn slic_labels_cover_image() {
        let labels = slic_superpixels(&checker(32, 32), 8, 3);
        assert_eq!(labels.len(), 1024);
        assert!(labels.iter().any(|&l| l > 0));
    }

    #[test]
    fn quantize_from_labels_reduces_colors() {
        let img = checker(16, 16);
        let labels = slic_superpixels(&img, 4, 2);
        let q = quantize_from_labels(&img, &labels);
        let unique: std::collections::HashSet<_> =
            q.pixels.iter().map(|p| (p.r, p.g, p.b)).collect();
        assert!(unique.len() <= 8);
    }
}
