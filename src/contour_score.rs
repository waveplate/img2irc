//! Output-only contour smoothness scoring.
//!
//! Lower scores are better. The bending term is continuous: it has no angle
//! threshold and increases smoothly as neighboring contour tangents diverge.

use std::collections::{HashMap, VecDeque};
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

const CONTOUR_CACHE_MAX_PIXELS: usize = 2_048;
const CONTOUR_CACHE_MAX_ENTRIES: usize = 32_768;

#[derive(Default)]
struct FastHasher(u64);

impl Hasher for FastHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        let mut hash = if self.0 == 0 {
            0xcbf29ce484222325
        } else {
            self.0
        };
        for &byte in bytes {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        self.0 = hash;
    }

    #[inline]
    fn write_u32(&mut self, value: u32) {
        self.0 = (value as u64).wrapping_mul(0x9e3779b97f4a7c15);
    }
}

type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<FastHasher>>;

#[derive(Hash, PartialEq, Eq)]
struct ContourCacheKey {
    // Labels have at most eight values, so two fit losslessly in one byte.
    // Keeping the exact packed sequence avoids collision-based cache results
    // while halving its dominant memory cost and hashing work.
    labels: Box<[u8]>,
    width: usize,
    height: usize,
    cell_scale: usize,
    weights: [u64; 5],
}

#[derive(Hash, PartialEq, Eq)]
struct PixelCacheKey {
    fingerprint: [u64; 2],
    width: usize,
    height: usize,
    cell_scale: usize,
    weights: [u64; 5],
}

thread_local! {
    static CONTOUR_SCORE_CACHE: std::cell::RefCell<FastMap<ContourCacheKey, ContourScore>> =
        std::cell::RefCell::new(FastMap::with_capacity_and_hasher(
            1_024,
            BuildHasherDefault::default(),
        ));
    static PIXEL_SCORE_CACHE: std::cell::RefCell<FastMap<PixelCacheKey, ContourScore>> =
        std::cell::RefCell::new(FastMap::with_capacity_and_hasher(
            1_024,
            BuildHasherDefault::default(),
        ));
}

static CONTOUR_CACHE_HITS: AtomicU64 = AtomicU64::new(0);
static CONTOUR_CACHE_MISSES: AtomicU64 = AtomicU64::new(0);

pub fn cache_stats() -> (u64, u64) {
    (
        CONTOUR_CACHE_HITS.load(Ordering::Relaxed),
        CONTOUR_CACHE_MISSES.load(Ordering::Relaxed),
    )
}

pub fn clear_cache() {
    CONTOUR_SCORE_CACHE.with(|cache| cache.borrow_mut().clear());
    PIXEL_SCORE_CACHE.with(|cache| cache.borrow_mut().clear());
}

fn packed_labels(labels: &[u8]) -> Box<[u8]> {
    let mut packed = Vec::with_capacity(labels.len().div_ceil(2));
    for pair in labels.chunks(2) {
        let high = pair.get(1).copied().unwrap_or(0) << 4;
        packed.push(pair[0] | high);
    }
    packed.into_boxed_slice()
}

fn pixel_fingerprint(pixels: &[[u8; 3]]) -> [u64; 2] {
    let mut first = 0xcbf29ce484222325u64;
    let mut second = 0x9e3779b97f4a7c15u64;
    for (index, pixel) in pixels.iter().enumerate() {
        let color = ((pixel[0] as u64) << 16) | ((pixel[1] as u64) << 8) | pixel[2] as u64;
        first ^= color;
        first = first.wrapping_mul(0x100000001b3);
        second = second.rotate_left(27)
            ^ color.wrapping_add((index as u64).wrapping_mul(0x9e3779b185ebca87));
    }
    [first, second]
}

fn cache_contour_score(key: ContourCacheKey, score: ContourScore) {
    CONTOUR_SCORE_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() >= CONTOUR_CACHE_MAX_ENTRIES {
            cache.clear();
        }
        cache.insert(key, score);
    });
}

fn cache_pixel_score(key: PixelCacheKey, score: ContourScore) {
    PIXEL_SCORE_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() >= CONTOUR_CACHE_MAX_ENTRIES {
            cache.clear();
        }
        cache.insert(key, score);
    });
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ContourScore {
    pub total: f64,
    pub bending: f64,
    pub endpoints: f64,
    pub junctions: f64,
    pub fragments: f64,
    pub boundary_length: usize,
    pub endpoint_count: usize,
    pub junction_count: usize,
    pub component_count: usize,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ReferenceContourScore {
    pub total: f64,
    pub contour: ContourScore,
    pub fidelity: f64,
    pub boundary_balance: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct ContourScoreWeights {
    pub bending: f64,
    pub endpoints: f64,
    pub junctions: f64,
    pub fragments: f64,
    pub fidelity: f64,
    pub boundary_balance: f64,
    /// Blend between RMS bending (0) and the more peak-sensitive fourth-power
    /// aggregate (1).
    pub peak_sensitivity: f64,
}

impl Default for ContourScoreWeights {
    fn default() -> Self {
        Self {
            bending: 1.0,
            endpoints: 1.0,
            junctions: 2.0,
            fragments: 1.0,
            fidelity: 0.1,
            boundary_balance: 0.1,
            peak_sensitivity: 0.4,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PreparedReference {
    blurred: Vec<[u8; 3]>,
    boundary_length: usize,
    width: usize,
    height: usize,
}

#[derive(Clone, Debug, Default)]
pub struct PreparedOutput {
    integral: Vec<[u64; 3]>,
    blurred: Vec<[u8; 3]>,
    fidelity_distance_sum: f64,
    width: usize,
    height: usize,
    blur_radius: usize,
}

pub const PREVIEW_CELL_WIDTH: usize = 8;
pub const PREVIEW_CELL_HEIGHT: usize = 16;

/// Normalize a font-dependent glyph raster to the terminal preview geometry.
/// Nearest-neighbor sampling deliberately preserves visible pixelation.
pub fn normalize_preview_raster(
    pixels: &[[u8; 3]],
    grid_width: usize,
    grid_height: usize,
    glyph_width: usize,
    glyph_height: usize,
) -> (Vec<[u8; 3]>, usize, usize) {
    let source_width = grid_width * glyph_width;
    let source_height = grid_height * glyph_height;
    let width = grid_width * PREVIEW_CELL_WIDTH;
    let height = grid_height * PREVIEW_CELL_HEIGHT;
    if source_width == 0 || source_height == 0 || pixels.len() != source_width * source_height {
        return (Vec::new(), 0, 0);
    }
    let mut normalized = Vec::with_capacity(width * height);
    for y in 0..height {
        let source_y = y * source_height / height;
        for x in 0..width {
            let source_x = x * source_width / width;
            normalized.push(pixels[source_y * source_width + source_x]);
        }
    }
    (normalized, width, height)
}

#[derive(Default)]
struct Graph {
    point_stride: usize,
    edges: Vec<(usize, usize)>,
    incident: Vec<[usize; 4]>,
    degree: Vec<u8>,
    active_points: Vec<usize>,
    component_lengths: Vec<usize>,
}

impl Graph {
    fn from_labels(
        labels: &[u8],
        width: usize,
        height: usize,
        collect_components: bool,
    ) -> Self {
        let point_stride = width + 1;
        let point_count = point_stride * (height + 1);
        let mut edges = Vec::new();
        let mut incident = vec![[usize::MAX; 4]; point_count];
        let mut degree = vec![0u8; point_count];
        let mut active_points = Vec::new();

        let mut add_edge = |a: usize, b: usize| {
            let edge_id = edges.len();
            edges.push((a, b));
            for point_id in [a, b] {
                let slot = degree[point_id] as usize;
                debug_assert!(slot < 4);
                if slot == 0 {
                    active_points.push(point_id);
                }
                incident[point_id][slot] = edge_id;
                degree[point_id] += 1;
            }
        };

        // Keep the same vertical-then-horizontal edge order as the original
        // segment builder. Stable ordering keeps floating-point aggregation
        // and candidate tie-breaking deterministic.
        for y in 0..height {
            for x in 1..width {
                if labels[y * width + x - 1] != labels[y * width + x] {
                    add_edge(y * point_stride + x, (y + 1) * point_stride + x);
                }
            }
        }
        for y in 1..height {
            for x in 0..width {
                if labels[(y - 1) * width + x] != labels[y * width + x] {
                    add_edge(y * point_stride + x, y * point_stride + x + 1);
                }
            }
        }

        let mut component_lengths = Vec::new();
        if collect_components {
            let mut visited = vec![false; edges.len()];
            for first in 0..edges.len() {
                if visited[first] {
                    continue;
                }
                let mut queue = VecDeque::from([first]);
                let mut component_length = 0usize;
                while let Some(edge_id) = queue.pop_front() {
                    if std::mem::replace(&mut visited[edge_id], true) {
                        continue;
                    }
                    component_length += 1;
                    let (a, b) = edges[edge_id];
                    for point_id in [a, b] {
                        for &next in &incident[point_id][..degree[point_id] as usize] {
                            if !visited[next] {
                                queue.push_back(next);
                            }
                        }
                    }
                }
                component_lengths.push(component_length);
            }
        }
        Self {
            point_stride,
            edges,
            incident,
            degree,
            active_points,
            component_lengths,
        }
    }

    #[inline]
    fn point(&self, point_id: usize) -> (i32, i32) {
        (
            (point_id % self.point_stride) as i32,
            (point_id / self.point_stride) as i32,
        )
    }

    fn other(&self, edge_id: usize, point_id: usize) -> usize {
        let (a, b) = self.edges[edge_id];
        if a == point_id {
            b
        } else {
            a
        }
    }

    fn walk_distances(
        &self,
        start: usize,
        first_edge: usize,
        distances: [usize; 3],
    ) -> [usize; 3] {
        let mut results = [usize::MAX; 3];
        let mut previous_edge = first_edge;
        let mut current = self.other(first_edge, start);
        let mut target_index = 0usize;
        if distances[0] == 1 {
            results[0] = current;
            target_index = 1;
        }
        for step in 2..=distances[2] {
            if self.degree[current] != 2 {
                break;
            }
            let next_edge = self.incident[current][..2]
                .iter()
                .copied()
                .find(|&edge_id| edge_id != previous_edge)
                .expect("degree-two contour point should have a forward edge");
            previous_edge = next_edge;
            current = self.other(next_edge, current);
            if current == start {
                break;
            }
            while target_index < distances.len() && step == distances[target_index] {
                results[target_index] = current;
                target_index += 1;
            }
        }
        results
    }
}

fn dominant_labels(pixels: &[[u8; 3]]) -> Vec<u8> {
    let mut counts: FastMap<u32, usize> = FastMap::default();
    for &pixel in pixels {
        let color = ((pixel[0] as u32) << 16) | ((pixel[1] as u32) << 8) | pixel[2] as u32;
        *counts.entry(color).or_default() += 1;
    }
    let mut palette = counts
        .iter()
        .map(|(&color, &count)| (color, count))
        .collect::<Vec<_>>();
    palette.sort_unstable_by(|(color_a, count_a), (color_b, count_b)| {
        count_b.cmp(count_a).then_with(|| color_a.cmp(color_b))
    });
    palette.truncate(8.min(palette.len()));
    let colors = palette
        .into_iter()
        .map(|(color, _)| {
            [
                ((color >> 16) & 0xff) as u8,
                ((color >> 8) & 0xff) as u8,
                (color & 0xff) as u8,
            ]
        })
        .collect::<Vec<_>>();

    // Candidate rasters are made from a small number of terminal colours.
    // Classify each distinct colour once instead of repeating eight RGB
    // distance calculations for every pixel in the raster.
    let mut color_labels: FastMap<u32, usize> = FastMap::default();
    for &color in counts.keys() {
        let pixel = [
            ((color >> 16) & 0xff) as u8,
            ((color >> 8) & 0xff) as u8,
            (color & 0xff) as u8,
        ];
        let label = colors
            .iter()
            .enumerate()
            .min_by_key(|(_, candidate)| {
                (0..3)
                    .map(|channel| {
                        let delta = pixel[channel] as i32 - candidate[channel] as i32;
                        delta * delta
                    })
                    .sum::<i32>()
            })
            .map(|(index, _)| index)
            .unwrap_or(0);
        color_labels.insert(color, label);
    }

    pixels
        .iter()
        .map(|pixel| {
            let color = ((pixel[0] as u32) << 16) | ((pixel[1] as u32) << 8) | pixel[2] as u32;
            color_labels.get(&color).copied().unwrap_or(0) as u8
        })
        .collect()
}

fn boundary_length(labels: &[u8], width: usize, height: usize) -> usize {
    let mut length = 0usize;
    for y in 0..height {
        for x in 1..width {
            if labels[y * width + x - 1] != labels[y * width + x] {
                length += 1;
            }
        }
    }
    for y in 1..height {
        for x in 0..width {
            if labels[(y - 1) * width + x] != labels[y * width + x] {
                length += 1;
            }
        }
    }
    length
}

/// Exact fast path for the common terminal/palette case. With at most eight
/// distinct colours, dominant-label quantization preserves equality exactly,
/// so the boundary can be counted without allocating or classifying labels.
fn pixel_boundary_length(pixels: &[[u8; 3]], width: usize, height: usize) -> usize {
    let mut colours = Vec::<[u8; 3]>::with_capacity(9);
    for &pixel in pixels {
        if !colours.contains(&pixel) {
            colours.push(pixel);
            if colours.len() > 8 {
                let labels = dominant_labels(pixels);
                return boundary_length(&labels, width, height);
            }
        }
    }

    let mut length = 0usize;
    for y in 0..height {
        for x in 1..width {
            if pixels[y * width + x - 1] != pixels[y * width + x] {
                length += 1;
            }
        }
    }
    for y in 1..height {
        for x in 0..width {
            if pixels[(y - 1) * width + x] != pixels[y * width + x] {
                length += 1;
            }
        }
    }
    length
}

fn continuous_bending(graph: &Graph, cell_scale: usize, peak_sensitivity: f64) -> f64 {
    static LATTICE_LENGTHS: OnceLock<Vec<f64>> = OnceLock::new();
    let lattice_lengths = LATTICE_LENGTHS.get_or_init(|| {
        (0..=512)
            .map(|squared_length| (squared_length as f64).sqrt())
            .collect()
    });
    let scale = cell_scale.max(2);
    let distances = [(scale / 2).max(1), scale, scale * 2];
    let endpoints = graph
        .active_points
        .iter()
        .map(|&center_id| {
            if graph.degree[center_id] != 2 {
                return [[usize::MAX; 3]; 2];
            }
            [
                graph.walk_distances(center_id, graph.incident[center_id][0], distances),
                graph.walk_distances(center_id, graph.incident[center_id][1], distances),
            ]
        })
        .collect::<Vec<_>>();
    let mut count = 0usize;
    let mut squared_sum = 0.0;
    let mut fourth_power_sum = 0.0;
    let peak_sensitivity = peak_sensitivity.clamp(0.0, 1.0);
    let need_l2 = peak_sensitivity < 1.0;
    let need_l4 = peak_sensitivity > 0.0;
    for distance_index in 0..distances.len() {
        for (active_index, &center_id) in graph.active_points.iter().enumerate() {
            let a_id = endpoints[active_index][0][distance_index];
            let b_id = endpoints[active_index][1][distance_index];
            if a_id == usize::MAX || b_id == usize::MAX {
                continue;
            }
            let center = graph.point(center_id);
            let a = graph.point(a_id);
            let b = graph.point(b_id);
            let va = (a.0 - center.0, a.1 - center.1);
            let vb = (b.0 - center.0, b.1 - center.1);
            let la_squared = (va.0 * va.0 + va.1 * va.1) as usize;
            let lb_squared = (vb.0 * vb.0 + vb.1 * vb.1) as usize;
            let la = lattice_lengths
                .get(la_squared)
                .copied()
                .unwrap_or_else(|| (la_squared as f64).sqrt());
            let lb = lattice_lengths
                .get(lb_squared)
                .copied()
                .unwrap_or_else(|| (lb_squared as f64).sqrt());
            if la == 0.0 || lb == 0.0 {
                continue;
            }
            let dot = (va.0 * vb.0 + va.1 * vb.1) as f64;
            let cosine = (dot / (la * lb)).clamp(-1.0, 1.0);
            let sample = 0.5 * (1.0 + cosine);
            let squared = sample * sample;
            count += 1;
            if need_l2 {
                squared_sum += squared;
            }
            if need_l4 {
                fourth_power_sum += squared * squared;
            }
        }
    }
    if count == 0 {
        return 0.0;
    }
    let count = count as f64;
    let l2 = if need_l2 {
        (squared_sum / count).sqrt()
    } else {
        0.0
    };
    let l4 = if need_l4 {
        (fourth_power_sum / count).powf(0.25)
    } else {
        0.0
    };
    100.0 * ((1.0 - peak_sensitivity) * l2 + peak_sensitivity * l4)
}

/// Score a rendered RGB raster. `cell_scale` is the smaller rendered glyph
/// dimension; it sets physical sampling distance, not an angle threshold.
pub fn score_rgb(
    pixels: &[[u8; 3]],
    width: usize,
    height: usize,
    cell_scale: usize,
) -> ContourScore {
    score_rgb_with_weights(
        pixels,
        width,
        height,
        cell_scale,
        &ContourScoreWeights::default(),
    )
}

pub fn score_rgb_with_weights(
    pixels: &[[u8; 3]],
    width: usize,
    height: usize,
    cell_scale: usize,
    weights: &ContourScoreWeights,
) -> ContourScore {
    if width == 0 || height == 0 || pixels.len() != width * height {
        return ContourScore::default();
    }
    let bending_enabled = weights.bending > 0.0;
    let endpoints_enabled = weights.endpoints > 0.0;
    let junctions_enabled = weights.junctions > 0.0;
    let fragments_enabled = weights.fragments > 0.0;
    if !bending_enabled && !endpoints_enabled && !junctions_enabled && !fragments_enabled {
        return ContourScore {
            boundary_length: pixel_boundary_length(pixels, width, height),
            ..ContourScore::default()
        };
    }

    let cache_weights = [
        weights.bending.to_bits(),
        weights.endpoints.to_bits(),
        weights.junctions.to_bits(),
        weights.fragments.to_bits(),
        weights.peak_sensitivity.to_bits(),
    ];
    let cacheable = pixels.len() <= CONTOUR_CACHE_MAX_PIXELS;
    let pixel_cache_key = cacheable.then(|| PixelCacheKey {
        fingerprint: pixel_fingerprint(pixels),
        width,
        height,
        cell_scale,
        weights: cache_weights,
    });
    if let Some(cached) = pixel_cache_key
        .as_ref()
        .and_then(|key| PIXEL_SCORE_CACHE.with(|cache| cache.borrow().get(key).copied()))
    {
        CONTOUR_CACHE_HITS.fetch_add(1, Ordering::Relaxed);
        return cached;
    }

    let labels = dominant_labels(pixels);
    let cache_key = cacheable.then(|| ContourCacheKey {
        labels: packed_labels(&labels),
        width,
        height,
        cell_scale,
        weights: cache_weights,
    });
    if let Some(cached) = cache_key
        .as_ref()
        .and_then(|key| CONTOUR_SCORE_CACHE.with(|cache| cache.borrow().get(key).copied()))
    {
        if let Some(key) = pixel_cache_key {
            cache_pixel_score(key, cached);
        }
        CONTOUR_CACHE_HITS.fetch_add(1, Ordering::Relaxed);
        return cached;
    }
    if cache_key.is_some() {
        CONTOUR_CACHE_MISSES.fetch_add(1, Ordering::Relaxed);
    }

    let graph = Graph::from_labels(&labels, width, height, fragments_enabled);
    if graph.edges.is_empty() {
        let score = ContourScore::default();
        if let Some(key) = cache_key {
            cache_contour_score(key, score);
        }
        if let Some(key) = pixel_cache_key {
            cache_pixel_score(key, score);
        }
        return score;
    }

    let length = graph.edges.len() as f64;
    let endpoints = if endpoints_enabled {
        graph
            .active_points
            .iter()
            .filter(|&&point_id| graph.degree[point_id] == 1)
            .count() as f64
    } else {
        0.0
    };
    let junctions = if junctions_enabled {
        graph
            .active_points
            .iter()
            .filter(|&&point_id| graph.degree[point_id] > 2)
            .count() as f64
    } else {
        0.0
    };
    let scale = cell_scale.max(1) as f64;
    let fragments = if fragments_enabled {
        graph
            .component_lengths
            .iter()
            .map(|&component_length| 1.0 / (1.0 + component_length as f64 / scale))
            .sum::<f64>()
    } else {
        0.0
    };

    let bending = if bending_enabled {
        continuous_bending(&graph, cell_scale, weights.peak_sensitivity)
    } else {
        0.0
    };
    let endpoint_score = 100.0 * endpoints / length;
    let junction_score = 100.0 * junctions / length;
    let fragment_score = 100.0 * fragments / length;
    let total = weights.bending.max(0.0) * bending
        + weights.endpoints.max(0.0) * endpoint_score
        + weights.junctions.max(0.0) * junction_score
        + weights.fragments.max(0.0) * fragment_score;
    let score = ContourScore {
        total,
        bending,
        endpoints: endpoint_score,
        junctions: junction_score,
        fragments: fragment_score,
        boundary_length: graph.edges.len(),
        endpoint_count: endpoints as usize,
        junction_count: junctions as usize,
        component_count: graph.component_lengths.len(),
    };
    if let Some(key) = cache_key {
        cache_contour_score(key, score);
    }
    if let Some(key) = pixel_cache_key {
        cache_pixel_score(key, score);
    }
    score
}

fn rgb_integral(pixels: &[[u8; 3]], width: usize, height: usize) -> Vec<[u64; 3]> {
    let stride = width + 1;
    let mut integral = vec![[0u64; 3]; (width + 1) * (height + 1)];
    for y in 0..height {
        let mut row = [0u64; 3];
        for x in 0..width {
            for channel in 0..3 {
                row[channel] += pixels[y * width + x][channel] as u64;
                integral[(y + 1) * stride + x + 1][channel] =
                    integral[y * stride + x + 1][channel] + row[channel];
            }
        }
    }
    integral
}

#[inline]
fn integral_sum(
    integral: &[[u64; 3]],
    stride: usize,
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
    channel: usize,
) -> u64 {
    integral[y1 * stride + x1][channel] + integral[y0 * stride + x0][channel]
        - integral[y0 * stride + x1][channel]
        - integral[y1 * stride + x0][channel]
}

fn box_blur_from_integral(
    pixels: &[[u8; 3]],
    integral: &[[u64; 3]],
    width: usize,
    height: usize,
    radius: usize,
) -> Vec<[u8; 3]> {
    if radius == 0 {
        return pixels.to_vec();
    }
    let stride = width + 1;
    let mut blurred = Vec::with_capacity(pixels.len());
    for y in 0..height {
        let y0 = y.saturating_sub(radius);
        let y1 = (y + radius + 1).min(height);
        for x in 0..width {
            let x0 = x.saturating_sub(radius);
            let x1 = (x + radius + 1).min(width);
            let count = ((x1 - x0) * (y1 - y0)) as u64;
            let mut pixel = [0u8; 3];
            for channel in 0..3 {
                let sum = integral_sum(integral, stride, x0, y0, x1, y1, channel);
                pixel[channel] = (sum / count) as u8;
            }
            blurred.push(pixel);
        }
    }
    blurred
}

fn box_blur(pixels: &[[u8; 3]], width: usize, height: usize, radius: usize) -> Vec<[u8; 3]> {
    if radius == 0 {
        return pixels.to_vec();
    }
    let integral = rgb_integral(pixels, width, height);
    box_blur_from_integral(pixels, &integral, width, height, radius)
}

#[inline]
fn normalized_rgb_distance(a: [u8; 3], b: [u8; 3]) -> f64 {
    let squared = (0..3)
        .map(|channel| {
            let delta = a[channel] as f64 - b[channel] as f64;
            delta * delta
        })
        .sum::<f64>();
    squared.sqrt() / (255.0 * 3.0f64.sqrt())
}

fn compose_reference_score(
    reference: &PreparedReference,
    contour: ContourScore,
    fidelity_distance_sum: f64,
    weights: &ContourScoreWeights,
) -> ReferenceContourScore {
    let fidelity = if weights.fidelity > 0.0 {
        fidelity_distance_sum / (reference.width * reference.height).max(1) as f64 * 100.0
    } else {
        0.0
    };
    let boundary_balance = if weights.boundary_balance > 0.0 {
        let reference_length = reference.boundary_length as f64;
        let output_length = contour.boundary_length as f64;
        100.0
            * ((output_length + 1.0) / (reference_length + 1.0))
                .ln()
                .abs()
    } else {
        0.0
    };
    let total = contour.total
        + weights.fidelity.max(0.0) * fidelity
        + weights.boundary_balance.max(0.0) * boundary_balance;
    ReferenceContourScore {
        total,
        contour,
        fidelity,
        boundary_balance,
    }
}

/// Combine output contour quality with continuous anti-degeneracy terms from
/// the source. No angle or pass/fail threshold is used.
pub fn score_rgb_against_reference(
    reference: &[[u8; 3]],
    output: &[[u8; 3]],
    width: usize,
    height: usize,
    cell_scale: usize,
) -> ReferenceContourScore {
    score_rgb_against_reference_with_weights(
        reference,
        output,
        width,
        height,
        cell_scale,
        &ContourScoreWeights::default(),
    )
}

pub fn score_rgb_against_reference_with_weights(
    reference: &[[u8; 3]],
    output: &[[u8; 3]],
    width: usize,
    height: usize,
    cell_scale: usize,
    weights: &ContourScoreWeights,
) -> ReferenceContourScore {
    if reference.len() != output.len() || reference.len() != width * height {
        return ReferenceContourScore::default();
    }
    let prepared = prepare_reference_with_weights(reference, width, height, cell_scale, weights);
    score_rgb_against_prepared_with_weights(&prepared, output, cell_scale, weights)
}

pub fn prepare_reference(
    reference: &[[u8; 3]],
    width: usize,
    height: usize,
    cell_scale: usize,
) -> PreparedReference {
    prepare_reference_with_weights(
        reference,
        width,
        height,
        cell_scale,
        &ContourScoreWeights::default(),
    )
}

pub fn prepare_reference_with_weights(
    reference: &[[u8; 3]],
    width: usize,
    height: usize,
    cell_scale: usize,
    weights: &ContourScoreWeights,
) -> PreparedReference {
    let blurred = if weights.fidelity > 0.0 {
        let blur_radius = (cell_scale / 4).max(1);
        box_blur(reference, width, height, blur_radius)
    } else {
        Vec::new()
    };
    let boundary_length = if weights.boundary_balance > 0.0 {
        pixel_boundary_length(reference, width, height)
    } else {
        0
    };
    PreparedReference {
        blurred,
        boundary_length,
        width,
        height,
    }
}

pub fn score_rgb_against_prepared(
    reference: &PreparedReference,
    output: &[[u8; 3]],
    cell_scale: usize,
) -> ReferenceContourScore {
    score_rgb_against_prepared_with_weights(
        reference,
        output,
        cell_scale,
        &ContourScoreWeights::default(),
    )
}

pub fn score_rgb_against_prepared_with_weights(
    reference: &PreparedReference,
    output: &[[u8; 3]],
    cell_scale: usize,
    weights: &ContourScoreWeights,
) -> ReferenceContourScore {
    score_rgb_and_prepare_output_with_weights(reference, output, cell_scale, weights).0
}

pub fn score_rgb_and_prepare_output(
    reference: &PreparedReference,
    output: &[[u8; 3]],
    cell_scale: usize,
) -> (ReferenceContourScore, PreparedOutput) {
    score_rgb_and_prepare_output_with_weights(
        reference,
        output,
        cell_scale,
        &ContourScoreWeights::default(),
    )
}

pub fn score_rgb_and_prepare_output_with_weights(
    reference: &PreparedReference,
    output: &[[u8; 3]],
    cell_scale: usize,
    weights: &ContourScoreWeights,
) -> (ReferenceContourScore, PreparedOutput) {
    if output.len() != reference.width * reference.height {
        return (ReferenceContourScore::default(), PreparedOutput::default());
    }
    let contour = score_rgb_with_weights(
        output,
        reference.width,
        reference.height,
        cell_scale,
        weights,
    );
    let (integral, blurred, fidelity_distance_sum, blur_radius) = if weights.fidelity > 0.0 {
        let blur_radius = (cell_scale / 4).max(1);
        let integral = rgb_integral(output, reference.width, reference.height);
        let blurred = box_blur_from_integral(
            output,
            &integral,
            reference.width,
            reference.height,
            blur_radius,
        );
        let fidelity_distance_sum = reference
            .blurred
            .iter()
            .zip(&blurred)
            .map(|(&a, &b)| normalized_rgb_distance(a, b))
            .sum::<f64>();
        (integral, blurred, fidelity_distance_sum, blur_radius)
    } else {
        (Vec::new(), Vec::new(), 0.0, 0)
    };
    let score = compose_reference_score(reference, contour, fidelity_distance_sum, weights);
    let prepared = PreparedOutput {
        integral,
        blurred,
        fidelity_distance_sum,
        width: reference.width,
        height: reference.height,
        blur_radius,
    };
    (score, prepared)
}

/// Score a candidate that differs from a prepared output only inside the
/// supplied rectangle. Contours are still evaluated exactly; only the blurred
/// fidelity term is updated incrementally.
pub fn score_rgb_candidate_against_prepared(
    reference: &PreparedReference,
    baseline: &PreparedOutput,
    baseline_pixels: &[[u8; 3]],
    candidate_pixels: &[[u8; 3]],
    change_x: usize,
    change_y: usize,
    change_width: usize,
    change_height: usize,
    cell_scale: usize,
) -> ReferenceContourScore {
    score_rgb_candidate_against_prepared_with_weights(
        reference,
        baseline,
        baseline_pixels,
        candidate_pixels,
        change_x,
        change_y,
        change_width,
        change_height,
        cell_scale,
        &ContourScoreWeights::default(),
    )
}

pub fn score_rgb_candidate_against_prepared_with_weights(
    reference: &PreparedReference,
    baseline: &PreparedOutput,
    baseline_pixels: &[[u8; 3]],
    candidate_pixels: &[[u8; 3]],
    change_x: usize,
    change_y: usize,
    change_width: usize,
    change_height: usize,
    cell_scale: usize,
    weights: &ContourScoreWeights,
) -> ReferenceContourScore {
    if baseline.width != reference.width
        || baseline.height != reference.height
        || baseline_pixels.len() != candidate_pixels.len()
        || candidate_pixels.len() != reference.width * reference.height
    {
        return score_rgb_against_prepared_with_weights(
            reference,
            candidate_pixels,
            cell_scale,
            weights,
        );
    }

    let contour = score_rgb_with_weights(
        candidate_pixels,
        reference.width,
        reference.height,
        cell_scale,
        weights,
    );
    if weights.fidelity <= 0.0 {
        return compose_reference_score(reference, contour, 0.0, weights);
    }
    let change_x1 = (change_x + change_width).min(reference.width);
    let change_y1 = (change_y + change_height).min(reference.height);
    if change_x >= change_x1 || change_y >= change_y1 {
        return compose_reference_score(
            reference,
            contour,
            baseline.fidelity_distance_sum,
            weights,
        );
    }

    let delta_width = change_x1 - change_x;
    let delta_height = change_y1 - change_y;
    let delta_stride = delta_width + 1;
    let mut delta_integral = vec![[0i64; 3]; (delta_width + 1) * (delta_height + 1)];
    for local_y in 0..delta_height {
        let mut row = [0i64; 3];
        for local_x in 0..delta_width {
            let pixel_idx = (change_y + local_y) * reference.width + change_x + local_x;
            for channel in 0..3 {
                row[channel] += candidate_pixels[pixel_idx][channel] as i64
                    - baseline_pixels[pixel_idx][channel] as i64;
                delta_integral[(local_y + 1) * delta_stride + local_x + 1][channel] =
                    delta_integral[local_y * delta_stride + local_x + 1][channel] + row[channel];
            }
        }
    }

    let radius = baseline.blur_radius;
    let affected_x0 = change_x.saturating_sub(radius);
    let affected_y0 = change_y.saturating_sub(radius);
    let affected_x1 = (change_x1 + radius).min(reference.width);
    let affected_y1 = (change_y1 + radius).min(reference.height);
    let output_stride = reference.width + 1;
    let mut fidelity_distance_sum = baseline.fidelity_distance_sum;

    for y in affected_y0..affected_y1 {
        let window_y0 = y.saturating_sub(radius);
        let window_y1 = (y + radius + 1).min(reference.height);
        for x in affected_x0..affected_x1 {
            let window_x0 = x.saturating_sub(radius);
            let window_x1 = (x + radius + 1).min(reference.width);
            let count = ((window_x1 - window_x0) * (window_y1 - window_y0)) as i64;
            let intersect_x0 = window_x0.max(change_x) - change_x;
            let intersect_y0 = window_y0.max(change_y) - change_y;
            let intersect_x1 = window_x1.min(change_x1) - change_x;
            let intersect_y1 = window_y1.min(change_y1) - change_y;
            let mut candidate_blurred = [0u8; 3];
            for channel in 0..3 {
                let baseline_sum = integral_sum(
                    &baseline.integral,
                    output_stride,
                    window_x0,
                    window_y0,
                    window_x1,
                    window_y1,
                    channel,
                ) as i64;
                let delta_sum = delta_integral[intersect_y1 * delta_stride + intersect_x1]
                    [channel]
                    + delta_integral[intersect_y0 * delta_stride + intersect_x0][channel]
                    - delta_integral[intersect_y0 * delta_stride + intersect_x1][channel]
                    - delta_integral[intersect_y1 * delta_stride + intersect_x0][channel];
                candidate_blurred[channel] = ((baseline_sum + delta_sum) / count) as u8;
            }
            let idx = y * reference.width + x;
            fidelity_distance_sum -=
                normalized_rgb_distance(reference.blurred[idx], baseline.blurred[idx]);
            fidelity_distance_sum +=
                normalized_rgb_distance(reference.blurred[idx], candidate_blurred);
        }
    }

    compose_reference_score(reference, contour, fidelity_distance_sum, weights)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raster(
        width: usize,
        height: usize,
        foreground: impl Fn(usize, usize) -> bool,
    ) -> Vec<[u8; 3]> {
        (0..height)
            .flat_map(|y| {
                let foreground = &foreground;
                (0..width).map(move |x| if foreground(x, y) { [255; 3] } else { [0; 3] })
            })
            .collect()
    }

    #[test]
    fn straight_boundary_scores_better_than_corner() {
        let straight = raster(64, 64, |x, _| x >= 32);
        let corner = raster(64, 64, |x, y| x >= 32 && y >= 32);
        assert!(score_rgb(&straight, 64, 64, 8).total < score_rgb(&corner, 64, 64, 8).total);
    }

    #[test]
    fn smooth_diagonal_scores_better_than_reversing_zigzag() {
        let diagonal = raster(64, 64, |x, y| y >= x);
        let zigzag = raster(64, 64, |x, y| {
            let offset = if (y / 4) % 2 == 0 { 8 } else { 0 };
            x <= y + offset
        });
        assert!(score_rgb(&diagonal, 64, 64, 8).total < score_rgb(&zigzag, 64, 64, 8).total);
    }

    #[test]
    fn blank_output_cannot_win_by_having_no_contour() {
        let reference = raster(64, 64, |x, y| {
            let dx = x as isize - 32;
            let dy = y as isize - 32;
            dx * dx + dy * dy < 18 * 18
        });
        let blank = raster(64, 64, |_, _| false);
        let exact = score_rgb_against_reference(&reference, &reference, 64, 64, 8);
        let missing = score_rgb_against_reference(&reference, &blank, 64, 64, 8);
        assert!(
            exact.total < missing.total,
            "exact={exact:?} missing={missing:?}"
        );
    }

    #[test]
    fn raw_topology_counts_distinguish_new_islands() {
        let one_island = raster(16, 16, |x, y| (3..7).contains(&x) && (3..7).contains(&y));
        let two_islands = raster(16, 16, |x, y| {
            ((2..5).contains(&x) && (2..5).contains(&y))
                || ((10..13).contains(&x) && (10..13).contains(&y))
        });
        let one = score_rgb(&one_island, 16, 16, 8);
        let two = score_rgb(&two_islands, 16, 16, 8);
        assert_eq!(one.component_count, 1);
        assert_eq!(two.component_count, 2);
        assert_eq!(one.endpoint_count, 0);
        assert_eq!(two.endpoint_count, 0);
        assert_eq!(one.junction_count, 0);
        assert_eq!(two.junction_count, 0);
    }

    #[test]
    fn preview_normalization_is_independent_of_font_raster_scale() {
        let small = raster(4, 4, |x, y| y >= x);
        let large = raster(32, 64, |x, y| y / 16 >= x / 8);
        let (small, sw, sh) = normalize_preview_raster(&small, 4, 4, 1, 1);
        let (large, lw, lh) = normalize_preview_raster(&large, 4, 4, 8, 16);
        assert_eq!((sw, sh), (lw, lh));
        assert_eq!(small, large);
        assert_eq!(
            score_rgb(&small, sw, sh, 8).total,
            score_rgb(&large, lw, lh, 8).total
        );
    }

    #[test]
    fn incremental_candidate_fidelity_matches_full_score() {
        let width = 24;
        let height = 48;
        let reference = raster(width, height, |x, y| x * 2 + y >= 40);
        let baseline = raster(width, height, |x, y| x + y / 2 >= 20);
        let mut candidate = baseline.clone();
        for y in 16..32 {
            for x in 8..16 {
                candidate[y * width + x] = if (x + y) % 3 == 0 {
                    [255; 3]
                } else {
                    [0; 3]
                };
            }
        }

        let prepared_reference = prepare_reference(&reference, width, height, 8);
        let (_, prepared_output) =
            score_rgb_and_prepare_output(&prepared_reference, &baseline, 8);
        let incremental = score_rgb_candidate_against_prepared(
            &prepared_reference,
            &prepared_output,
            &baseline,
            &candidate,
            8,
            16,
            8,
            16,
            8,
        );
        let full = score_rgb_against_prepared(&prepared_reference, &candidate, 8);
        assert!((incremental.fidelity - full.fidelity).abs() < 1e-10);
        assert!((incremental.total - full.total).abs() < 1e-10);
    }

    #[test]
    fn repeated_small_raster_uses_contour_cache_without_changing_score() {
        let width = 17;
        let height = 19;
        let pixels = raster(width, height, |x, y| (x * 7 + y * 11 + (x ^ y)) % 13 < 6);
        let before = cache_stats();
        let first = score_rgb(&pixels, width, height, 8);
        let second = score_rgb(&pixels, width, height, 8);
        let after = cache_stats();

        assert_eq!(first.total, second.total);
        assert_eq!(first.boundary_length, second.boundary_length);
        assert!(after.0 > before.0, "a repeated raster should hit the cache");
    }
}
