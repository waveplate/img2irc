//! Experimental reference-aware quality score for glyph art; lower is better.
//!
//! Continuous color gradients avoid global palette clustering. Orientation
//! dispersion measures rough contours, including short edges and junctions;
//! excess fine texture catches patterns that disappear when blurred. A
//! source-relative narrow-feature penalty catches unsupported tips and gaps
//! before geometry is downsampled or blurred. A
//! symmetric, locally tolerant edge comparison preserves source structure
//! without requiring pixel alignment. Coarse color agreement anchors regions.
//! These are engineering heuristics, not a calibrated model of human taste.

use serde::Serialize;

mod output_cache;
pub(crate) use output_cache::OutputCache;
mod geometry_cache;
pub(crate) use geometry_cache::GeometryCache;
mod sum_cache;
#[cfg(not(target_arch = "wasm32"))]
mod stage_profile;

type V3 = [f32; 3];
// Skipping a run of exact zeros preserves a finite nonnegative sum bit for
// bit. Nonempty blocks retain the original pixel/term addition order.
const REDUCTION_BLOCK: usize = 128;

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct PerceptualScore {
    pub total: f64,
    pub roughness: f64,
    pub texture: f64,
    pub missing: f64,
    pub extra: f64,
    pub colour: f64,
    /// Unsupported narrow gaps, bridges and spurs before geometry downsampling.
    pub thin_defects: f64,
}

/// Bounds cover the change from raster-order f64 accumulation to cached
/// pairwise accumulation. The original scorer resolves overlapping decisions.
#[derive(Clone, Copy)]
pub(crate) struct ScoreEstimate {
    pub(crate) value: f64,
    pub(crate) lower: f64,
    pub(crate) upper: f64,
}

impl ScoreEstimate {
    pub(crate) fn exact(value: f64) -> Self { Self { value, lower: value, upper: value } }
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
    let mut horizontal = Vec::new();
    let mut output = Vec::new();
    blur_into(pixels, width, height, radius, &mut horizontal, &mut output);
    output
}

fn blur_into(
    pixels: &[V3],
    width: usize,
    height: usize,
    radius: usize,
    horizontal: &mut Vec<V3>,
    output: &mut Vec<V3>,
) {
    horizontal.resize(pixels.len(), [0.0; 3]);
    for y in 0..height {
        blur_row(pixels, width, radius, y, |p, value| horizontal[p] = value);
    }
    output.resize(pixels.len(), [0.0; 3]);
    for x in 0..width {
        blur_column(horizontal, width, height, radius, x, |p, value| {
            output[p] = value
        });
    }
}

// Shared by fresh and incremental filters. Always start at the beginning of
// a row/column: changing the starting point changes floating-point rounding.
fn blur_row(pixels: &[V3], width: usize, radius: usize, y: usize, mut emit: impl FnMut(usize, V3)) {
    let mut sum = [0.0f64; 3];
    for x in 0..=radius.min(width - 1) {
        for c in 0..3 {
            sum[c] += pixels[y * width + x][c] as f64;
        }
    }
    for x in 0..width {
        let count = (x + radius + 1).min(width) - x.saturating_sub(radius);
        emit(y * width + x, sum.map(|v| (v / count as f64) as f32));
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

fn blur_column(
    horizontal: &[V3],
    width: usize,
    height: usize,
    radius: usize,
    x: usize,
    mut emit: impl FnMut(usize, V3),
) {
    let mut sum = [0.0f64; 3];
    for y in 0..=radius.min(height - 1) {
        for c in 0..3 {
            sum[c] += horizontal[y * width + x][c] as f64;
        }
    }
    for y in 0..height {
        let count = (y + radius + 1).min(height) - y.saturating_sub(radius);
        emit(y * width + x, sum.map(|v| (v / count as f64) as f32));
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

#[derive(Clone, Copy, Default)]
struct Edge {
    strength: f32,
    // Doubled angle: opposite normals describe the same contour orientation.
    // Length < 1 expresses ambiguity where several color gradients intersect.
    direction: [f32; 2],
}

fn edges(pixels: &[V3], width: usize, height: usize) -> Vec<Edge> {
    let mut result = Vec::new();
    edges_into(pixels, width, height, &mut result);
    result
}

fn edges_into(pixels: &[V3], width: usize, height: usize, result: &mut Vec<Edge>) {
    result.resize(pixels.len(), Edge::default());
    for y in 0..height {
        for x in 0..width {
            result[y * width + x] = edge_at(pixels, width, height, x, y);
        }
    }
}

fn edge_at(pixels: &[V3], width: usize, height: usize, x: usize, y: usize) -> Edge {
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
        Edge {
            strength: 0.5 * trace.sqrt(),
            direction: [(a - c) / trace, 2.0 * b / trace],
        }
    } else {
        Edge::default()
    }
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
            let best = matching_error(edge, b, width, height, x, y, radius);
            sum += (edge.strength * best) as f64;
        }
    }
    sum
}

fn matching_error(
    edge: Edge,
    other: &[Edge],
    width: usize,
    height: usize,
    x: usize,
    y: usize,
    radius: usize,
) -> f32 {
    if edge.strength <= 1e-5 {
        return 0.0;
    }
    let mut best = edge_error(edge, other[y * width + x]);
    if best > 1e-6 {
        for ny in y.saturating_sub(radius)..=(y + radius).min(height - 1) {
            for nx in x.saturating_sub(radius)..=(x + radius).min(width - 1) {
                let distance = x.abs_diff(nx).pow(2) + y.abs_diff(ny).pow(2);
                let movement = 0.02 * distance as f32 / radius.max(1).pow(2) as f32;
                best = best.min(edge_error(edge, other[ny * width + nx]) + movement);
            }
        }
    }
    best
}

fn dispersions(edges: &[Edge], width: usize, height: usize, radius: usize) -> Vec<f32> {
    let mut work = ScoreWorkspace::default();
    dispersions_into(edges, width, height, radius, &mut work);
    work.dispersions
}

fn dispersions_into(
    edges: &[Edge],
    width: usize,
    height: usize,
    radius: usize,
    work: &mut ScoreWorkspace,
) {
    work.moments.resize(edges.len(), [0.0; 3]);
    for (moment, edge) in work.moments.iter_mut().zip(edges) {
        *moment = [
            edge.strength,
            edge.strength * edge.direction[0],
            edge.strength * edge.direction[1],
        ];
    }
    blur_into(
        &work.moments,
        width,
        height,
        radius,
        &mut work.horizontal,
        &mut work.filtered,
    );
    work.dispersions.resize(edges.len(), 0.0);
    for (value, moment) in work.dispersions.iter_mut().zip(&work.filtered) {
        *value = moment_dispersion(*moment);
    }
}

fn moment_dispersion(moment: V3) -> f32 {
    if moment[0] <= 1e-5 {
        0.0
    } else {
        (1.0 - moment[1].hypot(moment[2]) / moment[0]).clamp(0.0, 1.0)
    }
}

/// A colour supported on only one side, or on two opposite sides, forms a
/// narrow tip/strip. Ordinary staircase corners have two adjacent supporters.
/// Both light gaps and dark bridges count; no foreground colour is assumed.
fn fragility(pixels: &[V3], width: usize, height: usize) -> Vec<f32> {
    let mut result = vec![0.0; pixels.len()];
    for y in 1..height.saturating_sub(1) {
        for x in 1..width.saturating_sub(1) {
            result[y * width + x] = fragility_at(pixels, width, height, x, y);
        }
    }
    result
}

fn fragility_at(pixels: &[V3], width: usize, height: usize, x: usize, y: usize) -> f32 {
    if x == 0 || y == 0 || x + 1 >= width || y + 1 >= height {
        return 0.0;
    }
    let p = y * width + x;
    let distances =
        [p - 1, p + 1, p - width, p + width].map(|q| norm(difference(pixels[p], pixels[q])));
    let contrast = distances.into_iter().fold(0.0_f32, f32::max);
    if contrast < 0.04 {
        return 0.0;
    }
    let support = distances.map(|distance| distance <= contrast * 0.35);
    let count = support.iter().filter(|&&same| same).count();
    let weight = if count <= 1 {
        1.0
    } else if count == 2 && ((support[0] && support[1]) || (support[2] && support[3])) {
        0.7
    } else {
        0.0
    };
    contrast * weight
}

/// A worker owns its temporary vectors; trials reuse allocations without
/// changing the filter kernels, precision or order of any score reduction.
#[derive(Default)]
pub(crate) struct ScoreWorkspace {
    horizontal: Vec<V3>,
    filtered: Vec<V3>,
    edges: Vec<Edge>,
    moments: Vec<V3>,
    dispersions: Vec<f32>,
}

fn nearby_max(values: &[f32], width: usize, height: usize, x: usize, y: usize) -> f32 {
    let mut result = 0.0_f32;
    for yy in y.saturating_sub(1)..=(y + 1).min(height - 1) {
        for xx in x.saturating_sub(1)..=(x + 1).min(width - 1) {
            result = result.max(values[yy * width + xx]);
        }
    }
    result
}

/// Source-only transforms are shared by every trial, including local ranking.
pub(crate) struct PreparedReference {
    fine_width: usize,
    fine_height: usize,
    fine_allowance: Vec<f32>,
    fine_denominator: f64,
    width: usize,
    height: usize,
    sample_step: usize,
    cell_scale: usize,
    source_edges: Vec<Edge>,
    source_dispersion: Vec<f32>,
    source_coarse: Vec<V3>,
    denominator: f64,
    variance: f64,
}

impl PreparedReference {
    pub(crate) fn geometry_pixels(&self) -> usize { self.width * self.height }

    pub(crate) fn new(
        reference: &[[u8; 3]],
        width: usize,
        height: usize,
        cell_scale: usize,
    ) -> Option<Self> {
        if width == 0 || height == 0 || width.checked_mul(height) != Some(reference.len()) {
            return None;
        }
        let fine: Vec<_> = reference.iter().copied().map(color).collect();
        let source_fragility = fragility(&fine, width, height);
        let fine_allowance = (0..fine.len())
            .map(|p| nearby_max(&source_fragility, width, height, p % width, p / width))
            .collect();
        let fine_denominator = total_variation(&fine, width, height).max(1.0);
        let sample_step = if width >= 4 && height >= 4 { 2 } else { 1 };
        let (reference, scaled_width, scaled_height) = if sample_step == 2 {
            (
                half_size(&fine, width, height),
                width.div_ceil(2),
                height.div_ceil(2),
            )
        } else {
            (fine, width, height)
        };
        let cell_scale = cell_scale.div_ceil(sample_step);
        let source_edges = edges(
            &blur(&reference, scaled_width, scaled_height, 1),
            scaled_width,
            scaled_height,
        );
        let denominator = source_edges
            .iter()
            .map(|edge| edge.strength as f64)
            .sum::<f64>()
            .max(1.0);
        let source_dispersion = dispersions(
            &source_edges,
            scaled_width,
            scaled_height,
            (cell_scale / 4).clamp(1, 4),
        );
        let source_coarse = blur(&reference, scaled_width, scaled_height, cell_scale.max(1));
        let mut mean = [0.0_f64; 3];
        let mut square = 0.0;
        for pixel in &source_coarse {
            for c in 0..3 {
                mean[c] += pixel[c] as f64;
                square += (pixel[c] as f64).powi(2);
            }
        }
        let n = source_coarse.len() as f64;
        let variance =
            (square / n - mean.iter().map(|value| (value / n).powi(2)).sum::<f64>()).max(0.0);
        Some(Self {
            fine_width: width,
            fine_height: height,
            fine_allowance,
            fine_denominator,
            width: scaled_width,
            height: scaled_height,
            sample_step,
            cell_scale,
            source_edges,
            source_dispersion,
            source_coarse,
            denominator,
            variance,
        })
    }

    pub(crate) fn score(&self, output: &[[u8; 3]]) -> PerceptualScore {
        if output.len() != self.fine_width * self.fine_height {
            return invalid_score();
        }
        let fine: Vec<_> = output.iter().copied().map(color).collect();
        let variation = total_variation(&fine, self.fine_width, self.fine_height);
        let fine_errors = fragility(&fine, self.fine_width, self.fine_height);
        let thin_defects = 100.0
            * fine_errors
                .iter()
                .zip(&self.fine_allowance)
                .map(|(&a, &b)| (a - b).max(0.0) as f64)
                .sum::<f64>()
            / self.fine_denominator;
        let output = if self.sample_step == 2 {
            half_size(&fine, self.fine_width, self.fine_height)
        } else {
            fine
        };
        self.finish_score(
            &output,
            variation,
            thin_defects,
            &mut ScoreWorkspace::default(),
        )
    }

    pub(crate) fn prepare_output(&self, output: &[[u8; 3]]) -> OutputCache {
        OutputCache::new(output, self.fine_width, self.fine_height, self.sample_step)
    }

    pub(crate) fn score_prepared(
        &self,
        output: &OutputCache,
        work: &mut ScoreWorkspace,
    ) -> PerceptualScore {
        let (variation, thin_defects) = self.fine_terms(output);
        self.finish_score(&output.scaled, variation, thin_defects, work)
    }

    pub(crate) fn score_incremental(
        &self,
        output: &OutputCache,
        geometry: &mut GeometryCache,
        rect: [usize; 4],
    ) -> PerceptualScore {
        geometry.update(self, output, rect);
        let (variation, thin_defects) = geometry.fine_totals(self, output);
        self.finish_totals(variation, thin_defects, geometry.totals())
    }

    pub(crate) fn score_incremental_estimate(&self, output: &OutputCache, geometry: &mut GeometryCache, rect: [usize; 4]) -> ScoreEstimate {
        geometry.update(self, output, rect);
        let (variation, thin_defects) = geometry.fine_totals(self, output);
        self.estimate_totals(variation, thin_defects, geometry.totals())
    }

    fn estimate_totals(&self, variation: f64, thin_defects: f64, totals: [f64; 6]) -> ScoreEstimate {
        // All reduced terms are nonnegative. gamma(n) bounds error in n
        // floating additions. Twice gamma includes both accumulation orders;
        // the longer original order also bounds the leaf/tree path length.
        let interval = |value: f64, additions: usize| {
            let ne = (additions as f64 * f64::EPSILON).min(0.25);
            let error = value.abs() * (2.0 * ne / (1.0 - ne));
            ((value - error).max(0.0), value + error)
        };
        let fine_pixels = self.fine_width * self.fine_height;
        let (variation_lo, variation_hi) = interval(variation, fine_pixels.saturating_mul(2));
        let (thin_lo, thin_hi) = interval(thin_defects, fine_pixels);
        let lower: [f64; 6] = std::array::from_fn(|c| interval(totals[c], self.geometry_pixels()).0);
        let upper: [f64; 6] = std::array::from_fn(|c| interval(totals[c], self.geometry_pixels()).1);
        // Texture decreases with output edge mass and increases with variation.
        // All other components increase with their corresponding totals.
        let mut lower_terms = lower;
        lower_terms[0] = upper[0];
        let mut upper_terms = upper;
        upper_terms[0] = lower[0];
        let value = self.finish_totals(variation, thin_defects, totals).total;
        let margin = (value.abs() + 1.0) * 1e-12;
        ScoreEstimate {
            value,
            lower: (self.finish_totals(variation_lo, thin_lo, lower_terms).total - margin).max(0.0),
            upper: self.finish_totals(variation_hi, thin_hi, upper_terms).total + margin,
        }
    }

    fn fine_terms(&self, output: &OutputCache) -> (f64, f64) {
        let thin_defects = 100.0 * output.thin_errors(&self.fine_allowance) / self.fine_denominator;
        (output.variation(), thin_defects)
    }

    fn finish_score(
        &self,
        output: &[V3],
        variation: f64,
        thin_defects: f64,
        work: &mut ScoreWorkspace,
    ) -> PerceptualScore {
        let mut totals = self.geometry_terms(output, work);
        let tolerance = (self.cell_scale / 4).clamp(1, 3);
        totals[3] = unmatched(&self.source_edges, &work.edges, self.width, self.height, tolerance);
        totals[4] = unmatched(&work.edges, &self.source_edges, self.width, self.height, tolerance);
        self.finish_totals(variation, thin_defects, totals)
    }

    // Reuse the same filters, floating-point precision and reductions.
    fn geometry_terms(&self, output: &[V3], work: &mut ScoreWorkspace) -> [f64; 6] {
        let (width, height, cell_scale) = (self.width, self.height, self.cell_scale);
        blur_into(
            output,
            width,
            height,
            1,
            &mut work.horizontal,
            &mut work.filtered,
        );
        edges_into(&work.filtered, width, height, &mut work.edges);
        // Borrow the edge vector separately while reusing the other buffers.
        let output_edges = std::mem::take(&mut work.edges);
        let output_mass = output_edges.iter().map(|e| e.strength as f64).sum::<f64>();
        dispersions_into(
            &output_edges,
            width,
            height,
            (cell_scale / 4).clamp(1, 4),
            work,
        );
        let mut dispersion = 0.0;
        let mut dispersion_squared = 0.0;
        for (edge, &value) in output_edges.iter().zip(&work.dispersions) {
            dispersion += edge.strength as f64 * value as f64;
            dispersion_squared += edge.strength as f64 * (value as f64).powi(2);
        }
        work.edges = output_edges;
        blur_into(
            output,
            width,
            height,
            cell_scale.max(1),
            &mut work.horizontal,
            &mut work.filtered,
        );
        let error: f64 = self
            .source_coarse
            .iter()
            .zip(&work.filtered)
            .map(|(a, b)| {
                (0..3)
                    .map(|c| (a[c] as f64 - b[c] as f64).powi(2))
                    .sum::<f64>()
            })
            .sum();
        [output_mass, dispersion, dispersion_squared, 0.0, 0.0, error]
    }

    fn finish_totals(
        &self,
        variation: f64,
        thin_defects: f64,
        totals: [f64; 6],
    ) -> PerceptualScore {
        let [output_mass, dispersion, dispersion_squared, missing, extra, error] = totals;
        let denominator = self.denominator;
        let roughness = 100.0
            * (0.6 * dispersion / denominator + 0.4 * (dispersion_squared / denominator).sqrt());
        let texture = 100.0
            * ((variation - 1.5 * output_mass * self.sample_step as f64) / variation.max(1e-6))
                .clamp(0.0, 1.0);
        let missing = 100.0 * missing / denominator;
        let extra = 100.0 * extra / denominator;
        let colour = 100.0
            * ((error / (self.width * self.height) as f64) / (self.variance + 0.01))
                .sqrt()
                .min(1.0);
        PerceptualScore {
            total: 0.8 * roughness
                + 0.15 * texture
                + 1.5 * thin_defects
                + (1.5 * missing * missing + 0.5 * extra * extra + 0.25 * colour * colour) / 100.0,
            roughness,
            texture,
            missing,
            extra,
            colour,
            thin_defects,
        }
    }

    /// Search heatmap: source-relative roughness, unsupported fine structures,
    /// and missing/invented edges. A perfect straight seam needs no proposals.
    pub(crate) fn defect_cells(
        &self,
        output: &[[u8; 3]],
        cell_width: usize,
        cell_height: usize,
    ) -> Vec<f32> {
        let grid_width = self.fine_width.div_ceil(cell_width);
        let grid_height = self.fine_height.div_ceil(cell_height);
        let mut priorities = vec![0.0_f32; grid_width * grid_height];
        let fine: Vec<_> = output.iter().copied().map(color).collect();
        for (p, value) in fragility(&fine, self.fine_width, self.fine_height)
            .into_iter()
            .enumerate()
        {
            let cell =
                (p / self.fine_width / cell_height) * grid_width + p % self.fine_width / cell_width;
            priorities[cell] =
                priorities[cell].max(100.0 * (value - self.fine_allowance[p]).max(0.0));
        }
        let output = if self.sample_step == 2 {
            half_size(&fine, self.fine_width, self.fine_height)
        } else {
            fine
        };
        let edges = edges(
            &blur(&output, self.width, self.height, 1),
            self.width,
            self.height,
        );
        let dispersion = dispersions(
            &edges,
            self.width,
            self.height,
            (self.cell_scale / 4).clamp(1, 4),
        );
        let tolerance = (self.cell_scale / 4).clamp(1, 3);
        for y in 0..self.height {
            for x in 0..self.width {
                let p = y * self.width + x;
                let roughness = (dispersion[p]
                    - nearby_max(&self.source_dispersion, self.width, self.height, x, y))
                .max(0.0)
                    * edges[p].strength;
                let missing = matching_error(
                    self.source_edges[p],
                    &edges,
                    self.width,
                    self.height,
                    x,
                    y,
                    tolerance,
                ) * self.source_edges[p].strength;
                let extra = matching_error(
                    edges[p],
                    &self.source_edges,
                    self.width,
                    self.height,
                    x,
                    y,
                    tolerance,
                ) * edges[p].strength;
                let cell = (y * self.sample_step / cell_height) * grid_width
                    + x * self.sample_step / cell_width;
                priorities[cell] = priorities[cell].max(100.0 * roughness.max(missing).max(extra));
            }
        }
        priorities
    }
}

fn invalid_score() -> PerceptualScore {
    PerceptualScore {
        total: f64::INFINITY,
        ..PerceptualScore::default()
    }
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
    PreparedReference::new(reference, width, height, cell_scale)
        .map_or_else(invalid_score, |reference| reference.score(output))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incremental_score_bounds_cover_original_scorer_after_local_edits() {
        let (width, height) = (65, 63);
        for pattern in 0..3 {
            let source: Vec<_> = (0..width * height).map(|p| {
                let (x, y) = (p % width, p / width);
                match pattern {
                    0 => [if x > y { 240 } else { 16 }; 3],
                    1 => [(x * 17 % 256) as u8, (y * 23 % 256) as u8, ((x + y) * 11 % 256) as u8],
                    _ => [128; 3],
                }
            }).collect();
            let reference = PreparedReference::new(&source, width, height, 8).unwrap();
            let mut output = source.clone();
            let mut features = reference.prepare_output(&output);
            let mut cache = GeometryCache::new(&reference, &features);
            for (edit, (x0, y0)) in [(0, 0), (57, 57), (29, 27), (1, 20), (40, 5), (29, 27)].into_iter().enumerate() {
                let rect = [x0, y0, x0 + 8, y0 + 6];
                for y in rect[1]..rect[3] {
                    for x in rect[0]..rect[2] {
                        output[y * width + x] = if edit % 2 == 0 { [255, 32, 0] } else { source[y * width + x] };
                    }
                }
                features.update(&output, rect);
                let estimate = reference.score_incremental_estimate(&features, &mut cache, rect);
                let original = reference.score(&output).total;
                assert!(estimate.lower <= original && original <= estimate.upper,
                    "pattern={pattern} edit={edit}: original={original:.15}, estimate={:.15} [{:.15}, {:.15}]", estimate.value, estimate.lower, estimate.upper);
            }
        }
    }

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
