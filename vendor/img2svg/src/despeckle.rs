//! Potrace-style despeckle: remove small connected color islands from quantized images.

use crate::image_processor::ImageData;
use rgb::RGBA8;
use std::collections::HashMap;

/// Remove connected components smaller than `min_area` pixels by merging into neighboring colors.
pub fn despeckle(image: &ImageData, min_area: usize) -> ImageData {
    if min_area <= 1 {
        return image.clone();
    }

    let w = image.width as usize;
    let h = image.height as usize;
    let n = w * h;
    if n == 0 {
        return image.clone();
    }

    let mut labels = vec![0u32; n];
    let mut palette: Vec<RGBA8> = Vec::new();
    let mut color_to_label: HashMap<[u8; 4], u32> = HashMap::new();

    for (i, p) in image.pixels.iter().enumerate() {
        let key = [p.r, p.g, p.b, p.a];
        let lab = *color_to_label.entry(key).or_insert_with(|| {
            let id = palette.len() as u32;
            palette.push(*p);
            id
        });
        labels[i] = lab;
    }

    let mut visited = vec![false; n];
    let mut out_labels = labels.clone();

    for y in 0..h {
        for x in 0..w {
            let start = y * w + x;
            if visited[start] {
                continue;
            }
            let seed = labels[start];
            let region = flood_same_color(x, y, w, h, &labels, seed, &mut visited);
            if region.len() >= min_area {
                continue;
            }

            for &(rx, ry) in &region {
                let idx = ry * w + rx;
                let mut neighbor_counts: HashMap<u32, usize> = HashMap::new();
                for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                    let nx = rx as i32 + dx;
                    let ny = ry as i32 + dy;
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        continue;
                    }
                    let nl = out_labels[ny as usize * w + nx as usize];
                    if nl != seed {
                        *neighbor_counts.entry(nl).or_default() += 1;
                    }
                }
                let replacement = neighbor_counts
                    .into_iter()
                    .max_by_key(|(_, count)| *count)
                    .map(|(lab, _)| lab)
                    .unwrap_or(seed);
                out_labels[idx] = replacement;
            }
        }
    }

    let mut pixels = Vec::with_capacity(n);
    for &lab in &out_labels {
        pixels.push(palette[lab as usize]);
    }

    ImageData {
        width: image.width,
        height: image.height,
        pixels,
    }
}

fn flood_same_color(
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    labels: &[u32],
    seed: u32,
    visited: &mut [bool],
) -> Vec<(usize, usize)> {
    let mut stack = vec![(x, y)];
    let mut region = Vec::new();
    visited[y * w + x] = true;

    while let Some((cx, cy)) = stack.pop() {
        region.push((cx, cy));
        for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            let nx = cx as i32 + dx;
            let ny = cy as i32 + dy;
            if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                continue;
            }
            let ni = ny as usize * w + nx as usize;
            if visited[ni] || labels[ni] != seed {
                continue;
            }
            visited[ni] = true;
            stack.push((nx as usize, ny as usize));
        }
    }

    region
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_with_speckle() -> ImageData {
        let w = 10;
        let h = 10;
        let mut pixels = vec![RGBA8::new(255, 255, 255, 255); w * h];
        for y in 0..5 {
            for x in 0..5 {
                pixels[y * w + x] = RGBA8::new(255, 0, 0, 255);
            }
        }
        pixels[8 * w + 8] = RGBA8::new(0, 255, 0, 255);
        ImageData {
            width: w as u32,
            height: h as u32,
            pixels,
        }
    }

    #[test]
    fn removes_single_pixel_speckle() {
        let img = solid_with_speckle();
        let out = despeckle(&img, 2);
        let speckle = out.pixels[8 * 10 + 8];
        assert_ne!(speckle, RGBA8::new(0, 255, 0, 255));
    }

    #[test]
    fn preserves_large_regions() {
        let img = solid_with_speckle();
        let out = despeckle(&img, 2);
        assert_eq!(out.pixels[0], RGBA8::new(255, 0, 0, 255));
        assert_eq!(out.pixels[4 * 10 + 4], RGBA8::new(255, 0, 0, 255));
    }

    #[test]
    fn min_area_one_is_noop() {
        let img = solid_with_speckle();
        let out = despeckle(&img, 1);
        assert_eq!(out.pixels[8 * 10 + 8], RGBA8::new(0, 255, 0, 255));
    }
}
