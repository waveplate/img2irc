//! Enhanced color quantization with k-means++ initialization, k-means refinement,
//! perceptual color distance, and edge-aware majority-vote smoothing.
//!
//! Ported from the vec project's ColorQuantizer for significantly better
//! palette quality and cleaner region boundaries.

use crate::edge_detector::EdgeMap;
use crate::image_processor::ImageData;
use rand::RngExt;
use rgb::RGBA8;

/// Perceptual color distance squared (weighted RGB, approximates human vision).
/// Weights: R=2, G=4, B=3 (green most sensitive). Alpha is included so that
/// regions differing only in transparency (e.g. anti-aliased edges, soft
/// shadows) are not collapsed together — a fully transparent pixel is very
/// far from an opaque one regardless of RGB.
#[inline]
pub fn perceptual_dist_sq(a: &RGBA8, b: &RGBA8) -> i32 {
    let dr = a.r as i32 - b.r as i32;
    let dg = a.g as i32 - b.g as i32;
    let db = a.b as i32 - b.b as i32;
    let da = a.a as i32 - b.a as i32;
    2 * dr * dr + 4 * dg * dg + 3 * db * db + 3 * da * da
}

/// K-means++ initialization: choose centroids with probability proportional
/// to squared distance from nearest existing centroid.
fn kmeans_plusplus_init(samples: &[RGBA8], k: usize) -> Vec<RGBA8> {
    let mut rng = rand::rng();
    let n = samples.len();
    if n == 0 || k == 0 {
        return Vec::new();
    }

    let mut centroids = Vec::with_capacity(k);
    centroids.push(samples[rng.random_range(0..n)]);

    let mut distances = vec![0.0f64; n];

    for iteration in 1..k {
        let new_cent = centroids.last().unwrap();
        let mut total_dist = 0.0f64;

        for (i, sample) in samples.iter().enumerate() {
            let dist_sq = perceptual_dist_sq(sample, new_cent) as f64;
            if iteration == 1 || dist_sq < distances[i] {
                distances[i] = dist_sq;
            }
            total_dist += distances[i];
        }

        if total_dist == 0.0 {
            break;
        }

        let mut rand_val = rng.random::<f64>() * total_dist;
        let mut chosen = false;
        for (i, &dist) in distances.iter().enumerate() {
            rand_val -= dist;
            if rand_val <= 0.0 {
                centroids.push(samples[i]);
                chosen = true;
                break;
            }
        }
        if !chosen {
            centroids.push(samples[rng.random_range(0..n)]);
        }
    }

    centroids
}

/// Refine palette using k-means iterations with perceptual distance.
/// Empty clusters (centroids that attract no samples) are reseeded to the
/// sample farthest from its current centroid — i.e. the largest cluster is
/// split. This keeps all `k` palette slots productive instead of wasting them,
/// which materially improves palette coverage for images with skew color
/// distributions (a few dominant colors + many rare accents).
fn kmeans_refine(mut palette: Vec<RGBA8>, samples: &[RGBA8], iterations: usize) -> Vec<RGBA8> {
    if palette.is_empty() || samples.is_empty() {
        return palette;
    }

    for _ in 0..iterations {
        let k = palette.len();
        let mut sums = vec![[0u64; 4]; k];
        let mut counts = vec![0u64; k];
        // Track each sample's distance to its nearest centroid so empty
        // clusters can be reseeded to the worst-fit sample.
        let mut dist_sq = vec![0i32; samples.len()];

        for (i, s) in samples.iter().enumerate() {
            let mut best_idx = 0;
            let mut best_dist = i32::MAX;
            for (j, c) in palette.iter().enumerate() {
                let d = perceptual_dist_sq(s, c);
                if d < best_dist {
                    best_dist = d;
                    best_idx = j;
                }
            }
            dist_sq[i] = best_dist;
            sums[best_idx][0] += s.r as u64;
            sums[best_idx][1] += s.g as u64;
            sums[best_idx][2] += s.b as u64;
            sums[best_idx][3] += s.a as u64;
            counts[best_idx] += 1;
        }

        // Reseed empty clusters: replace them with the worst-fit sample
        // (the sample whose nearest centroid is farthest away). This splits
        // the region that currently has the largest representational error.
        let empty: Vec<usize> = (0..k).filter(|&j| counts[j] == 0).collect();
        if !empty.is_empty() {
            // Order samples by descending distance to their nearest centroid.
            let mut order: Vec<usize> = (0..samples.len()).collect();
            order.sort_unstable_by(|&a, &b| dist_sq[b].cmp(&dist_sq[a]));
            let mut taken = vec![false; samples.len()];
            for &j in &empty {
                // Pick the next worst-fit sample that is not already a centroid.
                let pick = order.iter().copied().find(|&i| {
                    !taken[i] && !palette.contains(&samples[i])
                });
                if let Some(i) = pick {
                    taken[i] = true;
                    palette[j] = samples[i];
                    counts[j] = 1; // mark non-empty so the changed check sees it
                    sums[j] = [samples[i].r as u64, samples[i].g as u64,
                               samples[i].b as u64, samples[i].a as u64];
                }
            }
            // Recompute assignments for the reseeded centroids by restarting
            // the iteration — simplest correct approach. The next loop pass
            // will re-assign samples to the new centroids.
            continue;
        }

        let mut changed = false;
        for (j, c) in palette.iter_mut().enumerate() {
            if counts[j] == 0 {
                continue;
            }
            let n = counts[j];
            let new_c = RGBA8::new(
                (sums[j][0] / n) as u8,
                (sums[j][1] / n) as u8,
                (sums[j][2] / n) as u8,
                (sums[j][3] / n) as u8,
            );
            if c.r != new_c.r || c.g != new_c.g || c.b != new_c.b {
                changed = true;
                *c = new_c;
            }
        }

        if !changed {
            break;
        }
    }

    palette
}

/// Find nearest palette color index using perceptual distance.
#[inline]
fn nearest_palette_index(pixel: &RGBA8, palette: &[RGBA8]) -> usize {
    let mut best_idx = 0usize;
    let mut best_dist = i32::MAX;
    for (idx, c) in palette.iter().enumerate() {
        let d = perceptual_dist_sq(pixel, c);
        if d < best_dist {
            best_dist = d;
            best_idx = idx;
        }
    }
    best_idx
}

/// Enhanced quantization: k-means++ init → k-means refinement → perceptual mapping.
/// Returns (quantized image, palette indices, palette).
pub fn quantize_enhanced(
    image_data: &ImageData,
    num_colors: usize,
) -> (ImageData, Vec<usize>, Vec<RGBA8>) {
    let n_pixels = image_data.pixels.len();

    // Downsample for palette building: cap at 100K samples
    let sample_step = (n_pixels / 100_000).max(1);
    let samples: Vec<RGBA8> = image_data
        .pixels
        .iter()
        .step_by(sample_step)
        .copied()
        .collect();

    // K-means++ init → k-means refinement (12 iterations; empty clusters are
    // reseeded to the worst-fit sample, so more iterations improve coverage
    // without wasting palette slots).
    let initial_palette = kmeans_plusplus_init(&samples, num_colors);
    let palette = kmeans_refine(initial_palette, &samples, 12);

    // Map each pixel to nearest palette color
    let mut indices = vec![0usize; n_pixels];
    let mut quantized_pixels = Vec::with_capacity(n_pixels);

    for (i, pixel) in image_data.pixels.iter().enumerate() {
        let idx = nearest_palette_index(pixel, &palette);
        indices[i] = idx;
        quantized_pixels.push(palette[idx]);
    }

    let quantized = ImageData {
        width: image_data.width,
        height: image_data.height,
        pixels: quantized_pixels,
    };

    (quantized, indices, palette)
}

/// Edge-aware quantization: after initial quantization, apply majority-vote
/// smoothing on non-edge pixels to reduce speckling in smooth gradients.
pub fn quantize_edge_aware(
    image_data: &ImageData,
    num_colors: usize,
    edges: &EdgeMap,
    edge_threshold: u8,
    num_passes: usize,
) -> (ImageData, Vec<usize>, Vec<RGBA8>) {
    let (_, mut indices, palette) = quantize_enhanced(image_data, num_colors);

    let w = image_data.width as usize;
    let h = image_data.height as usize;
    let k = palette.len();

    // Multi-pass majority-vote smoothing
    for _pass in 0..num_passes {
        let mut next_indices = indices.clone();

        for y in 0..h {
            let row = y * w;
            for x in 0..w {
                let idx = row + x;
                if edges.data[idx] >= edge_threshold {
                    continue;
                }

                // Count neighbor indices in 3×3 window
                let mut counts = vec![0u16; k];
                let mut best_count = 0u16;
                let mut best_idx = indices[idx];

                let y_start = y.saturating_sub(1);
                let y_end = (y + 2).min(h);
                let x_start = x.saturating_sub(1);
                let x_end = (x + 2).min(w);

                for ny in y_start..y_end {
                    let nrow = ny * w;
                    for nx in x_start..x_end {
                        let nidx = nrow + nx;
                        if edges.data[nidx] < edge_threshold {
                            let ci = indices[nidx];
                            counts[ci] += 1;
                            if counts[ci] > best_count {
                                best_count = counts[ci];
                                best_idx = ci;
                            }
                        }
                    }
                }

                next_indices[idx] = best_idx;
            }
        }

        indices = next_indices;
    }

    // Build quantized image from smoothed indices
    let quantized_pixels: Vec<RGBA8> = indices.iter().map(|&i| palette[i]).collect();

    let quantized = ImageData {
        width: image_data.width,
        height: image_data.height,
        pixels: quantized_pixels,
    };

    (quantized, indices, palette)
}

/// Superpixel-based quantization using SLIC clustering.
pub fn quantize_superpixel(
    image_data: &ImageData,
    num_superpixels: usize,
) -> (ImageData, Vec<usize>, Vec<RGBA8>) {
    use crate::image_filters::{quantize_from_labels, slic_superpixels};
    use std::collections::HashMap;

    let labels = slic_superpixels(image_data, num_superpixels, 5);
    let quantized = quantize_from_labels(image_data, &labels);

    let mut palette_map: HashMap<u32, usize> = HashMap::new();
    let mut indices = vec![0usize; labels.len()];
    let mut palette: Vec<RGBA8> = Vec::new();

    for (i, &lab) in labels.iter().enumerate() {
        let c = quantized.pixels[i];
        if let Some(&idx) = palette_map.get(&lab) {
            indices[i] = idx;
        } else {
            let idx = palette.len();
            palette.push(c);
            palette_map.insert(lab, idx);
            indices[i] = idx;
        }
    }

    (quantized, indices, palette)
}

/// Count distinct colors in an image.
pub fn count_distinct_colors(image_data: &ImageData) -> usize {
    use std::collections::HashSet;
    let mut seen = HashSet::new();
    for p in &image_data.pixels {
        seen.insert((p.r, p.g, p.b));
    }
    seen.len()
}

/// Determine adaptive target color count based on image size and complexity.
pub fn adaptive_color_count(image_data: &ImageData) -> usize {
    let pixel_count = image_data.width as usize * image_data.height as usize;
    if pixel_count < 10_000 {
        64
    } else if pixel_count < 100_000 {
        128
    } else {
        256
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_perceptual_dist_green_weighted_more() {
        let a = RGBA8::new(100, 100, 100, 255);
        let b_red = RGBA8::new(110, 100, 100, 255);
        let b_green = RGBA8::new(100, 110, 100, 255);
        assert!(
            perceptual_dist_sq(&a, &b_green) > perceptual_dist_sq(&a, &b_red),
            "green should be weighted more than red"
        );
    }

    #[test]
    fn test_perceptual_dist_alpha_aware() {
        // Same RGB, different alpha → non-zero distance (transparency matters).
        let opaque = RGBA8::new(50, 100, 150, 255);
        let transparent = RGBA8::new(50, 100, 150, 0);
        assert!(perceptual_dist_sq(&opaque, &transparent) > 0);
        // A small alpha gap should be smaller than a large one.
        let small_a = RGBA8::new(50, 100, 150, 250);
        assert!(
            perceptual_dist_sq(&opaque, &small_a) < perceptual_dist_sq(&opaque, &transparent),
            "larger alpha gap should yield larger distance"
        );
    }

    #[test]
    fn test_perceptual_dist_zero_for_same() {
        let a = RGBA8::new(42, 42, 42, 255);
        assert_eq!(perceptual_dist_sq(&a, &a), 0);
    }

    #[test]
    fn test_kmeans_plusplus_init_returns_k_centroids() {
        let samples: Vec<RGBA8> = (0..100)
            .map(|i| RGBA8::new(i as u8 * 2, 0, 0, 255))
            .collect();
        let centroids = kmeans_plusplus_init(&samples, 8);
        assert_eq!(centroids.len(), 8);
    }

    #[test]
    fn test_kmeans_plusplus_init_empty() {
        let centroids = kmeans_plusplus_init(&[], 5);
        assert!(centroids.is_empty());
    }

    #[test]
    fn test_quantize_enhanced_reduces_colors() {
        let mut pixels = Vec::new();
        for i in 0..100 {
            pixels.push(RGBA8::new(i as u8 * 2, i as u8, 0, 255));
        }
        let img = ImageData {
            width: 10,
            height: 10,
            pixels,
        };
        let (quantized, indices, palette) = quantize_enhanced(&img, 4);
        assert_eq!(quantized.pixels.len(), 100);
        assert_eq!(indices.len(), 100);
        assert!(palette.len() <= 4);
        // All indices should be valid
        for &idx in &indices {
            assert!(idx < palette.len());
        }
    }

    #[test]
    fn test_quantize_enhanced_single_color() {
        let pixels = vec![RGBA8::new(128, 128, 128, 255); 25];
        let img = ImageData {
            width: 5,
            height: 5,
            pixels,
        };
        let (quantized, _, palette) = quantize_enhanced(&img, 4);
        // Should produce 1 color since input is uniform
        assert!(!palette.is_empty());
        // All output pixels should be the same
        let first = quantized.pixels[0];
        for p in &quantized.pixels {
            assert_eq!(p.r, first.r);
            assert_eq!(p.g, first.g);
            assert_eq!(p.b, first.b);
        }
    }

    #[test]
    fn test_quantize_enhanced_reseeds_empty_clusters() {
        // Skewed distribution: 99% one color, 1% a distinct accent.
        // Without empty-cluster reseeding, k-means++ often leaves palette
        // slots empty (stuck on the dominant color), so the accent is lost.
        let mut pixels = vec![RGBA8::new(20, 20, 20, 255); 990];
        for _ in 0..10 {
            pixels.push(RGBA8::new(240, 240, 240, 255));
        }
        let img = ImageData {
            width: 50,
            height: 20,
            pixels,
        };
        let (_quantized, _indices, palette) = quantize_enhanced(&img, 4);
        // The accent (near-white) must be represented in the palette.
        let has_light = palette
            .iter()
            .any(|c| c.r > 200 && c.g > 200 && c.b > 200);
        assert!(has_light, "accent color should survive quantization: {:?}", palette);
    }

    #[test]
    fn test_count_distinct_colors() {
        let pixels = vec![
            RGBA8::new(255, 0, 0, 255),
            RGBA8::new(0, 255, 0, 255),
            RGBA8::new(0, 0, 255, 255),
            RGBA8::new(255, 0, 0, 255), // duplicate
        ];
        let img = ImageData {
            width: 2,
            height: 2,
            pixels,
        };
        assert_eq!(count_distinct_colors(&img), 3);
    }

    #[test]
    fn test_adaptive_color_count() {
        let small = ImageData {
            width: 50,
            height: 50,
            pixels: vec![RGBA8::new(0, 0, 0, 255); 2500],
        };
        let medium = ImageData {
            width: 200,
            height: 200,
            pixels: vec![RGBA8::new(0, 0, 0, 255); 40000],
        };
        let large = ImageData {
            width: 1000,
            height: 1000,
            pixels: vec![RGBA8::new(0, 0, 0, 255); 1000000],
        };
        assert_eq!(adaptive_color_count(&small), 64);
        assert_eq!(adaptive_color_count(&medium), 128);
        assert_eq!(adaptive_color_count(&large), 256);
    }

    #[test]
    fn test_edge_aware_quantize() {
        // Create a simple image with an edge
        let mut pixels = Vec::new();
        for _y in 0..10 {
            for x in 0..10 {
                if x < 5 {
                    pixels.push(RGBA8::new(200, 0, 0, 255));
                } else {
                    pixels.push(RGBA8::new(0, 0, 200, 255));
                }
            }
        }
        let img = ImageData {
            width: 10,
            height: 10,
            pixels,
        };
        let edges = crate::edge_detector::detect_edges_sobel(&img);
        let (quantized, indices, palette) = quantize_edge_aware(&img, 4, &edges, 25, 2);
        assert_eq!(quantized.pixels.len(), 100);
        assert_eq!(indices.len(), 100);
        assert!(palette.len() <= 4);
    }
}
