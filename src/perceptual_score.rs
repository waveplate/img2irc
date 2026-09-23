//! Experimental reference-aware quality score for glyph art; lower is better.
//!
//! Continuous color gradients avoid global palette clustering. Orientation
//! dispersion measures rough contours, including short edges and junctions;
//! excess fine texture catches patterns that disappear when blurred. A
//! symmetric, locally tolerant edge comparison preserves source structure
//! without requiring pixel alignment. Coarse color agreement anchors regions.
//! These are engineering heuristics, not a calibrated model of human taste.

use serde::Serialize;

type V3 = [f32; 3];

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct PerceptualScore {
    pub total: f64,
    pub roughness: f64,
    pub texture: f64,
    pub missing: f64,
    pub extra: f64,
    pub colour: f64,
}

fn color([r, g, b]: [u8; 3]) -> V3 {
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    // A cheap opponent transform, retaining chromatic edges at equal luma.
    // This is deliberately not advertised as a perceptually uniform space.
    [
        0.299 * r + 0.587 * g + 0.114 * b,
        0.5 * (r - g),
        0.25 * (r + g - 2.0 * b),
    ]
}

fn norm(v: V3) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

fn difference(a: V3, b: V3) -> V3 {
    std::array::from_fn(|c| a[c] - b[c])
}

fn half_size(pixels: &[V3], width: usize, height: usize) -> Vec<V3> {
    let mut result = Vec::with_capacity(width.div_ceil(2) * height.div_ceil(2));
    for y in (0..height).step_by(2) {
        for x in (0..width).step_by(2) {
            let mut sum = [0.0; 3];
            let mut count = 0.0;
            for yy in y..(y + 2).min(height) {
                for xx in x..(x + 2).min(width) {
                    for c in 0..3 {
                        sum[c] += pixels[yy * width + xx][c];
                    }
                    count += 1.0;
                }
            }
            result.push(sum.map(|v| v / count));
        }
    }
    result
}

/// Separable running sums: linear work, independent of the filter radius.
fn blur(pixels: &[V3], width: usize, height: usize, radius: usize) -> Vec<V3> {
    let mut horizontal = vec![[0.0; 3]; pixels.len()];
    for y in 0..height {
        let mut sum = [0.0f64; 3];
        for x in 0..=radius.min(width - 1) {
            for c in 0..3 {
                sum[c] += pixels[y * width + x][c] as f64;
            }
        }
        for x in 0..width {
            let count = (x + radius + 1).min(width) - x.saturating_sub(radius);
            horizontal[y * width + x] = sum.map(|v| (v / count as f64) as f32);
            for c in 0..3 {
                if x >= radius {
                    sum[c] -= pixels[y * width + x - radius][c] as f64;
                }
                if x + radius + 1 < width {
                    sum[c] += pixels[y * width + x + radius + 1][c] as f64;
                }
            }
        }
    }
    let mut output = vec![[0.0; 3]; pixels.len()];
    for x in 0..width {
        let mut sum = [0.0f64; 3];
        for y in 0..=radius.min(height - 1) {
            for c in 0..3 {
                sum[c] += horizontal[y * width + x][c] as f64;
            }
        }
        for y in 0..height {
            let count = (y + radius + 1).min(height) - y.saturating_sub(radius);
            output[y * width + x] = sum.map(|v| (v / count as f64) as f32);
            for c in 0..3 {
                if y >= radius {
                    sum[c] -= horizontal[(y - radius) * width + x][c] as f64;
                }
                if y + radius + 1 < height {
                    sum[c] += horizontal[(y + radius + 1) * width + x][c] as f64;
                }
            }
        }
    }
    output
}

#[derive(Clone, Copy, Default)]
struct Edge {
    strength: f32,
    // Doubled angle: opposite normals describe the same contour orientation.
    // Length < 1 expresses ambiguity where several color gradients intersect.
    direction: [f32; 2],
}

fn edges(pixels: &[V3], width: usize, height: usize) -> Vec<Edge> {
    let mut result = vec![Edge::default(); pixels.len()];
    for y in 0..height {
        for x in 0..width {
            let dx = difference(
                pixels[y * width + (x + 1).min(width - 1)],
                pixels[y * width + x.saturating_sub(1)],
            );
            let dy = difference(
                pixels[(y + 1).min(height - 1) * width + x],
                pixels[y.saturating_sub(1) * width + x],
            );
            let a = dx.iter().map(|v| v * v).sum::<f32>();
            let b = dx.iter().zip(dy).map(|(u, v)| u * v).sum::<f32>();
            let c = dy.iter().map(|v| v * v).sum::<f32>();
            let trace = a + c;
            if trace > 1e-10 {
                result[y * width + x] = Edge {
                    strength: 0.5 * trace.sqrt(),
                    direction: [(a - c) / trace, 2.0 * b / trace],
                };
            }
        }
    }
    result
}

fn total_variation(pixels: &[V3], width: usize, height: usize) -> f64 {
    let mut sum = 0.0;
    for y in 0..height {
        for x in 0..width {
            let p = y * width + x;
            if x > 0 {
                sum += norm(difference(pixels[p], pixels[p - 1])) as f64;
            }
            if y > 0 {
                sum += norm(difference(pixels[p], pixels[p - width])) as f64;
            }
        }
    }
    sum
}

fn edge_error(a: Edge, b: Edge) -> f32 {
    if b.strength <= 1e-5 {
        return 1.0;
    }
    // Ask whether an edge is present. Sharpening a soft source boundary
    // should not look like deleting it; coarse color agreement separately
    // limits contrast/palette changes.
    let contrast = (1.0 - 2.0 * b.strength / a.strength).max(0.0);
    // Also zero for identical ambiguous junctions, without normalizing a
    // near-zero vector or taking roots inside the neighborhood search.
    let angle = 0.125
        * ((a.direction[0] - b.direction[0]).powi(2) + (a.direction[1] - b.direction[1]).powi(2));
    (contrast + angle).min(1.0)
}

fn unmatched(a: &[Edge], b: &[Edge], width: usize, height: usize, radius: usize) -> f64 {
    let mut sum = 0.0;
    for y in 0..height {
        for x in 0..width {
            let edge = a[y * width + x];
            if edge.strength <= 1e-5 {
                continue;
            }
            let mut best = edge_error(edge, b[y * width + x]);
            if best > 1e-6 {
                for ny in y.saturating_sub(radius)..=(y + radius).min(height - 1) {
                    for nx in x.saturating_sub(radius)..=(x + radius).min(width - 1) {
                        let distance = x.abs_diff(nx).pow(2) + y.abs_diff(ny).pow(2);
                        // Small movement is cheap, not free. Independent
                        // local matches are a tolerance, not a deformation map.
                        let movement = 0.02 * distance as f32 / radius.max(1).pow(2) as f32;
                        best = best.min(edge_error(edge, b[ny * width + nx]) + movement);
                    }
                }
            }
            sum += (edge.strength * best) as f64;
        }
    }
    sum
}

/// Score equally sized, aligned source and output rasters in preview pixels.
/// The reference should be the image after intentional crop/color edits and
/// before glyph matching. Invalid inputs score infinity, never a perfect zero.
pub fn score(
    reference: &[[u8; 3]],
    output: &[[u8; 3]],
    width: usize,
    height: usize,
    cell_scale: usize,
) -> PerceptualScore {
    if width == 0
        || height == 0
        || width.checked_mul(height) != Some(output.len())
        || reference.len() != output.len()
    {
        return PerceptualScore {
            total: f64::INFINITY,
            ..PerceptualScore::default()
        };
    }
    let reference: Vec<_> = reference.iter().copied().map(color).collect();
    let output: Vec<_> = output.iter().copied().map(color).collect();
    let variation = total_variation(&output, width, height);
    // Keep pixel-scale texture evidence, but evaluate geometry after area
    // sampling: fewer raster stair steps and a quarter of the search work.
    let sample_step = if width >= 4 && height >= 4 { 2 } else { 1 };
    let (reference, output, width, height) = if sample_step == 2 {
        (
            half_size(&reference, width, height),
            half_size(&output, width, height),
            width.div_ceil(2),
            height.div_ceil(2),
        )
    } else {
        (reference, output, width, height)
    };
    let cell_scale = cell_scale.div_ceil(sample_step);
    let source_edges = edges(&blur(&reference, width, height, 1), width, height);
    let output_edges = edges(&blur(&output, width, height, 1), width, height);
    let source_mass = source_edges.iter().map(|e| e.strength as f64).sum::<f64>();
    let output_mass = output_edges.iter().map(|e| e.strength as f64).sum::<f64>();
    // A fixed reference denominator prevents adding easy edges from diluting
    // existing defects; erasing edges is handled by the missing-structure term.
    let denominator = source_mass.max(1.0);
    let moments: Vec<_> = output_edges
        .iter()
        .map(|e| {
            [
                e.strength,
                e.strength * e.direction[0],
                e.strength * e.direction[1],
            ]
        })
        .collect();
    let moments = blur(&moments, width, height, (cell_scale / 4).clamp(1, 4));
    let mut dispersion = 0.0;
    let mut dispersion_squared = 0.0;
    for (edge, moment) in output_edges.iter().zip(moments) {
        if moment[0] <= 1e-5 {
            continue;
        }
        let value = (1.0 - moment[1].hypot(moment[2]) / moment[0]).clamp(0.0, 1.0) as f64;
        dispersion += edge.strength as f64 * value;
        dispersion_squared += edge.strength as f64 * value * value;
    }
    let roughness =
        100.0 * (0.6 * dispersion / denominator + 0.4 * (dispersion_squared / denominator).sqrt());
    let texture = 100.0
        * ((variation - 1.5 * output_mass * sample_step as f64) / variation.max(1e-6))
            .clamp(0.0, 1.0);
    let tolerance = (cell_scale / 4).clamp(1, 3);
    let missing =
        100.0 * unmatched(&source_edges, &output_edges, width, height, tolerance) / denominator;
    let extra =
        100.0 * unmatched(&output_edges, &source_edges, width, height, tolerance) / denominator;
    let source_coarse = blur(&reference, width, height, cell_scale.max(1));
    let output_coarse = blur(&output, width, height, cell_scale.max(1));
    let mut mean = [0.0f64; 3];
    let mut square = 0.0;
    let mut error = 0.0;
    for (a, b) in source_coarse.iter().zip(output_coarse) {
        for c in 0..3 {
            mean[c] += a[c] as f64;
            square += (a[c] as f64).powi(2);
            error += (a[c] as f64 - b[c] as f64).powi(2);
        }
    }
    let n = output.len() as f64;
    let variance = (square / n - mean.iter().map(|v| (v / n).powi(2)).sum::<f64>()).max(0.0);
    let colour = 100.0 * ((error / n) / (variance + 0.01)).sqrt().min(1.0);
    PerceptualScore {
        // Soft structural constraints: small approximation errors should
        // not overwhelm a noticeable improvement in contour quality, while
        // wholesale feature loss is expensive. Losing an edge costs more
        // than adding one because displacement contributes to BOTH errors
        // but erasure only to missing.
        total: 0.8 * roughness
            + 0.15 * texture
            + (1.5 * missing * missing + 0.5 * extra * extra + 0.25 * colour * colour) / 100.0,
        roughness,
        texture,
        missing,
        extra,
        colour,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bitmap(f: impl Fn(usize, usize) -> bool) -> Vec<[u8; 3]> {
        (0..64 * 64)
            .map(|i| [if f(i % 64, i / 64) { 240 } else { 16 }; 3])
            .collect()
    }

    fn measure(reference: &[[u8; 3]], output: &[[u8; 3]]) -> PerceptualScore {
        score(reference, output, 64, 64, 8)
    }

    #[test]
    fn perceptual_continuity_beats_zigzags_breaks_and_erasure() {
        let straight = bitmap(|x, y| x >= y);
        let zigzag = bitmap(|x, y| x >= y + (y / 4 % 2) * 4);
        let broken = bitmap(|x, y| x >= y && !(y % 16 == 8 && x < y + 12));
        let blank = bitmap(|_, _| false);
        let good = measure(&straight, &straight);
        for other in [&zigzag, &broken, &blank] {
            let bad = measure(&straight, other);
            assert!(good.total < bad.total, "good={good:?}, bad={bad:?}");
        }
        assert!(good.missing < 1e-5 && good.extra < 1e-5);
    }

    #[test]
    fn perceptual_allows_small_displacements_but_retains_structure() {
        let circle =
            |cx: i32| bitmap(|x, y| (x as i32 - cx).pow(2) + (y as i32 - 32).pow(2) < 18 * 18);
        let reference = circle(32);
        let nearby = measure(&reference, &circle(33));
        let far = measure(&reference, &circle(40));
        let blank = measure(&reference, &bitmap(|_, _| false));
        assert!(
            nearby.total < far.total && far.total < blank.total,
            "nearby={nearby:?}, far={far:?}, blank={blank:?}"
        );
    }

    #[test]
    fn perceptual_catches_short_fragments_and_texture_lost_in_blur() {
        let reference = vec![[128; 3]; 64 * 64];
        let mut fragments = reference.clone();
        for y in (8..56).step_by(8) {
            for x in (8..56).step_by(8) {
                fragments[y * 64 + x] = [0; 3];
            }
        }
        let stripes = bitmap(|x, _| x % 3 == 0);
        let exact = measure(&reference, &reference);
        assert!(measure(&reference, &fragments).total > exact.total);
        assert!(measure(&reference, &stripes).texture > 50.0);
    }

    #[test]
    fn perceptual_weights_defects_by_contrast_without_palette_truncation() {
        let reference = bitmap(|x, _| x >= 32);
        let mut faint = reference.clone();
        let mut strong = reference.clone();
        for i in 0..16 {
            let p = (8 + i / 4 * 4) * 64 + 4 + i % 4 * 4;
            faint[p] = [17 + i as u8; 3];
            strong[p] = [128 + i as u8 * 7; 3];
        }
        let subtle = measure(&reference, &faint);
        let obvious = measure(&reference, &strong);
        assert!(subtle.extra > 0.0);
        assert!(
            obvious.total > subtle.total,
            "faint={subtle:?}, strong={obvious:?}"
        );
    }

    #[test]
    fn perceptual_invalid_images_cannot_score_as_perfect_and_tiny_images_work() {
        assert!(score(&[], &[], 0, 0, 8).total.is_infinite());
        assert!(score(&[[0; 3]], &[], 1, 1, 8).total.is_infinite());
        assert_eq!(score(&[[128; 3]], &[[128; 3]], 1, 1, 8).total, 0.0);
        assert!(score(&[[128; 3]], &[[0; 3]], 1, 1, 8).colour > 0.0);
    }
}
