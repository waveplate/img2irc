//! Exact incremental full-image geometry. A changed row/column is filtered
//! from its original start; bitwise differences propagate through every stage,
//! including floating-point tails outside the nominal spatial filter support.

use super::sum_cache::SumCache;
use super::*;

fn same(a: V3, b: V3) -> bool {
    a.map(f32::to_bits) == b.map(f32::to_bits)
}

struct CachedBlur {
    horizontal: Vec<V3>,
    output: Vec<V3>,
    rows: Vec<bool>,
    columns: Vec<bool>,
    changed: Vec<usize>,
    radius: usize,
}

impl CachedBlur {
    fn new(pixels: &[V3], width: usize, height: usize, radius: usize) -> Self {
        let mut horizontal = Vec::new();
        let mut output = Vec::new();
        blur_into(pixels, width, height, radius, &mut horizontal, &mut output);
        Self {
            horizontal,
            output,
            radius,
            rows: vec![false; height],
            columns: vec![false; width],
            changed: Vec::new(),
        }
    }

    fn update(&mut self, pixels: &[V3], width: usize, height: usize, changed: &[usize]) {
        self.rows.fill(false);
        self.columns.fill(false);
        self.changed.clear();
        for &p in changed {
            self.rows[p / width] = true;
        }
        for y in 0..height {
            if !self.rows[y] {
                continue;
            }
            blur_row(pixels, width, self.radius, y, |p, value| {
                if !same(self.horizontal[p], value) {
                    self.columns[p % width] = true;
                    self.horizontal[p] = value;
                }
            });
        }
        for x in 0..width {
            if !self.columns[x] {
                continue;
            }
            blur_column(
                &self.horizontal,
                width,
                height,
                self.radius,
                x,
                |p, value| {
                    if !same(self.output[p], value) {
                        self.output[p] = value;
                        self.changed.push(p);
                    }
                },
            );
        }
    }
}

struct DirtyPixels {
    seen: Vec<bool>,
    pixels: Vec<usize>,
}

impl DirtyPixels {
    fn new(len: usize) -> Self {
        Self {
            seen: vec![false; len],
            pixels: Vec::new(),
        }
    }

    fn clear(&mut self) {
        for &p in &self.pixels {
            self.seen[p] = false;
        }
        self.pixels.clear();
    }

    fn insert(&mut self, p: usize) {
        if !self.seen[p] {
            self.seen[p] = true;
            self.pixels.push(p);
        }
    }
}

pub(crate) struct GeometryCache {
    #[cfg(not(target_arch = "wasm32"))]
    profile: super::stage_profile::Profile,
    smooth: CachedBlur,
    coarse: CachedBlur,
    edges: Vec<Edge>,
    moments: Vec<V3>,
    curvature: CachedBlur,
    dispersion: Vec<f32>,
    // Mass, linear/squared dispersion, missing/extra edges, coarse colour error.
    terms: Vec<[f64; 6]>,
    active_blocks: Vec<u16>,
    sums: SumCache<6>,
    fine_sums: SumCache<2>,
    dirty: DirtyPixels,
    missing_dirty: DirtyPixels,
    scaled_changed: Vec<usize>,
    edge_changed: Vec<usize>,
}

impl GeometryCache {
    pub(crate) fn new(reference: &PreparedReference, output: &OutputCache) -> Self {
        let (width, height) = (reference.width, reference.height);
        let smooth = CachedBlur::new(&output.scaled, width, height, 1);
        let edges = edges(&smooth.output, width, height);
        let moments: Vec<_> = edges
            .iter()
            .map(|e| {
                [
                    e.strength,
                    e.strength * e.direction[0],
                    e.strength * e.direction[1],
                ]
            })
            .collect();
        let curvature = CachedBlur::new(
            &moments,
            width,
            height,
            (reference.cell_scale / 4).clamp(1, 4),
        );
        let dispersion = curvature
            .output
            .iter()
            .copied()
            .map(moment_dispersion)
            .collect();
        let coarse = CachedBlur::new(&output.scaled, width, height, reference.cell_scale.max(1));
        let len = width * height;
        let mut cache = Self {
            #[cfg(not(target_arch = "wasm32"))]
            profile: Default::default(),
            smooth,
            coarse,
            edges,
            moments,
            curvature,
            dispersion,
            terms: vec![[0.0; 6]; len],
            active_blocks: vec![0; len.div_ceil(REDUCTION_BLOCK)],
            sums: SumCache::new(len),
            fine_sums: SumCache::new(reference.fine_width * reference.fine_height),
            dirty: DirtyPixels::new(len),
            missing_dirty: DirtyPixels::new(len),
            scaled_changed: Vec::new(),
            edge_changed: Vec::new(),
        };
        for p in 0..len {
            cache.refresh_roughness(p);
            cache.refresh_missing(reference, p);
            cache.refresh_extra(reference, p);
            cache.refresh_colour(reference, p);
        }
        cache
    }

    pub(crate) fn update(
        &mut self,
        reference: &PreparedReference,
        output: &OutputCache,
        rect: [usize; 4],
    ) {
        #[cfg(not(target_arch = "wasm32"))]
        let mut span = super::stage_profile::Span::new();
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.profile.updates += 1;
        }
        let (width, height, step) = (reference.width, reference.height, reference.sample_step);
        let [x0, y0, x1, y1] = rect;
        if x0 < x1 && y0 < y1 {
            // Invalidate on updates too: restoring a pair may update features
            // without asking for a score before another pair is considered.
            let fine_width = reference.fine_width;
            for y in y0.saturating_sub(1)..(y1 + 1).min(reference.fine_height) {
                self.fine_sums.invalidate_range(
                    y * fine_width + x0.saturating_sub(1),
                    y * fine_width + (x1 + 1).min(fine_width),
                );
            }
        }
        self.scaled_changed.clear();
        for y in y0 / step..y1.div_ceil(step) {
            for x in x0 / step..x1.div_ceil(step) {
                self.scaled_changed.push(y * width + x);
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        span.mark(&mut self.profile, 5);
        self.smooth
            .update(&output.scaled, width, height, &self.scaled_changed);
        #[cfg(not(target_arch = "wasm32"))]
        span.mark(&mut self.profile, 0);

        // Finite differences read the four adjacent pixels (or the clamped
        // centre at a boundary). Include the centre as a safe superset.
        self.dirty.clear();
        for &p in &self.smooth.changed {
            self.dirty.insert(p);
            if p % width > 0 {
                self.dirty.insert(p - 1);
            }
            if p % width + 1 < width {
                self.dirty.insert(p + 1);
            }
            if p >= width {
                self.dirty.insert(p - width);
            }
            if p + width < width * height {
                self.dirty.insert(p + width);
            }
        }
        self.edge_changed.clear();
        for &p in &self.dirty.pixels {
            let edge = edge_at(&self.smooth.output, width, height, p % width, p / width);
            let old = self.edges[p];
            if edge.strength.to_bits() != old.strength.to_bits()
                || edge.direction.map(f32::to_bits) != old.direction.map(f32::to_bits)
            {
                self.edges[p] = edge;
                self.moments[p] = [
                    edge.strength,
                    edge.strength * edge.direction[0],
                    edge.strength * edge.direction[1],
                ];
                self.edge_changed.push(p);
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        span.mark(&mut self.profile, 1);
        self.curvature
            .update(&self.moments, width, height, &self.edge_changed);
        self.dirty.clear();
        for &p in &self.curvature.changed {
            self.dispersion[p] = moment_dispersion(self.curvature.output[p]);
            self.dirty.insert(p);
        }
        for &p in &self.edge_changed {
            self.dirty.insert(p);
        }
        for i in 0..self.dirty.pixels.len() {
            self.refresh_roughness(self.dirty.pixels[i]);
        }

        // Missing source edges can match any changed output edge within the
        // tolerance. Extra edges match a fixed reference, so only their own
        // changed pixels need refreshing.
        #[cfg(not(target_arch = "wasm32"))]
        span.mark(&mut self.profile, 2);
        let tolerance = (reference.cell_scale / 4).clamp(1, 3);
        self.missing_dirty.clear();
        for i in 0..self.edge_changed.len() {
            let p = self.edge_changed[i];
            self.refresh_extra(reference, p);
            let (x, y) = (p % width, p / width);
            for yy in y.saturating_sub(tolerance)..=(y + tolerance).min(height - 1) {
                for xx in x.saturating_sub(tolerance)..=(x + tolerance).min(width - 1) {
                    self.missing_dirty.insert(yy * width + xx);
                }
            }
        }
        for i in 0..self.missing_dirty.pixels.len() {
            self.refresh_missing(reference, self.missing_dirty.pixels[i]);
        }
        #[cfg(not(target_arch = "wasm32"))]
        span.mark(&mut self.profile, 3);
        self.coarse
            .update(&output.scaled, width, height, &self.scaled_changed);
        for i in 0..self.coarse.changed.len() {
            self.refresh_colour(reference, self.coarse.changed[i]);
        }
        #[cfg(not(target_arch = "wasm32"))]
        span.mark(&mut self.profile, 4);
    }

    fn refresh_roughness(&mut self, p: usize) {
        let edge = self.edges[p];
        let value = self.dispersion[p] as f64;
        self.set_term(p, 0, edge.strength as f64);
        self.set_term(p, 1, edge.strength as f64 * value);
        self.set_term(p, 2, edge.strength as f64 * value.powi(2));
    }

    fn refresh_missing(&mut self, reference: &PreparedReference, p: usize) {
        let edge = reference.source_edges[p];
        let value = if edge.strength <= 1e-5 {
            0.0
        } else {
            (edge.strength
                * matching_error(
                    edge,
                    &self.edges,
                    reference.width,
                    reference.height,
                    p % reference.width,
                    p / reference.width,
                    (reference.cell_scale / 4).clamp(1, 3),
                )) as f64
        };
        self.set_term(p, 3, value);
    }

    fn refresh_extra(&mut self, reference: &PreparedReference, p: usize) {
        let edge = self.edges[p];
        let value = if edge.strength <= 1e-5 {
            0.0
        } else {
            (edge.strength
                * matching_error(
                    edge,
                    &reference.source_edges,
                    reference.width,
                    reference.height,
                    p % reference.width,
                    p / reference.width,
                    (reference.cell_scale / 4).clamp(1, 3),
                )) as f64
        };
        self.set_term(p, 4, value);
    }

    fn refresh_colour(&mut self, reference: &PreparedReference, p: usize) {
        let value = (0..3)
            .map(|c| {
                (reference.source_coarse[p][c] as f64 - self.coarse.output[p][c] as f64).powi(2)
            })
            .sum::<f64>();
        self.set_term(p, 5, value);
    }

    fn set_term(&mut self, p: usize, c: usize, value: f64) {
        if self.terms[p][c].to_bits() == value.to_bits() {
            return;
        }
        let count = &mut self.active_blocks[p / REDUCTION_BLOCK];
        *count = *count - u16::from(self.terms[p][c] != 0.0) + u16::from(value != 0.0);
        self.terms[p][c] = value;
        self.sums.invalidate(p);
    }

    pub(super) fn fine_totals(
        &mut self,
        reference: &PreparedReference,
        output: &OutputCache,
    ) -> (f64, f64) {
        #[cfg(not(target_arch = "wasm32"))]
        let mut span = super::stage_profile::Span::new();
        let sums = self.fine_sums.totals(|block, sums| {
            output.reduce_fine_block(&reference.fine_allowance, block, sums);
        });
        #[cfg(not(target_arch = "wasm32"))]
        span.mark(&mut self.profile, 7);
        (sums[0], 100.0 * sums[1] / reference.fine_denominator)
    }

    pub(super) fn totals(&mut self) -> [f64; 6] {
        #[cfg(not(target_arch = "wasm32"))]
        let mut span = super::stage_profile::Span::new();
        let terms = &self.terms;
        let active = &self.active_blocks;
        let result = self.sums.totals(|block, totals| {
            if active[block] == 0 {
                return;
            }
            let begin = block * REDUCTION_BLOCK;
            let end = (begin + REDUCTION_BLOCK).min(terms.len());
            for terms in &terms[begin..end] {
                // Keep the original order within each leaf. Tree ancestors
                // combine f64 totals; empty blocks contribute exact zeros.
                for c in 0..6 {
                    totals[c] += terms[c];
                }
            }
        });
        #[cfg(not(target_arch = "wasm32"))]
        span.mark(&mut self.profile, 6);
        result
    }
}
