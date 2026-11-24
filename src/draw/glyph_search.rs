//! Target suspected defects, exhaust neighbouring glyph-pair shortlists, then
//! retain full-image perceptual improvements. Trials keep colours fixed; final
//! source-colour fitting is included in verification and the finished score.

use super::search_status::{
    GlyphDefectMap, GlyphSearchPreview, GlyphSearchProgress, GlyphSearchStage,
};
use super::*;
use crate::contour_score::{PREVIEW_CELL_HEIGHT as CH, PREVIEW_CELL_WIDTH as CW};
use crate::perceptual_score::{
    GeometryCache, OutputCache, PreparedReference, ScoreEstimate, ScoreWorkspace,
};
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

type Choice = (usize, bool);
pub(super) mod rank_cache;
pub(super) mod temporal;
#[cfg(test)]
mod seam_costs;
mod source_cache;
mod connections;

fn cancelled(abort: Option<&AtomicBool>) -> bool {
    abort.is_some_and(|flag| flag.load(Ordering::Relaxed))
}

fn detailed_profiling() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("IMG2IRC_REPAIR_PROFILE").is_some())
}

fn squared_error(a: [u8; 3], b: [u8; 3]) -> i64 {
    a.into_iter()
        .zip(b)
        .map(|(a, b)| (a as i64 - b as i64).pow(2))
        .sum()
}

fn neighbors(cell: usize, grid_w: usize, grid_h: usize) -> Vec<usize> {
    let x = cell % grid_w;
    let y = cell / grid_w;
    let mut cells = Vec::with_capacity(8);
    for yy in y.saturating_sub(1)..=(y + 1).min(grid_h - 1) {
        for xx in x.saturating_sub(1)..=(x + 1).min(grid_w - 1) {
            let neighbor = yy * grid_w + xx;
            if neighbor != cell {
                cells.push(neighbor);
            }
        }
    }
    cells
}

fn defect_map(
    priorities: Vec<f32>,
    data: &[BlockData],
    threshold: f32,
    allowed: Option<&[bool]>,
    grid_w: usize,
    grid_h: usize,
) -> Arc<GlyphDefectMap> {
    let targets: Vec<_> = priorities
        .iter()
        .enumerate()
        .map(|(cell, &score)| {
            data[cell].fg != data[cell].bg
                && (threshold <= 0.0 || score >= threshold)
                && allowed.is_none_or(|allowed| allowed[cell])
        })
        .collect();
    let mut active = targets.clone();
    for (cell, &target) in targets.iter().enumerate() {
        if target {
            for neighbor in neighbors(cell, grid_w, grid_h) {
                active[neighbor] = true;
            }
        }
    }
    Arc::new(GlyphDefectMap {
        columns: grid_w,
        rows: grid_h,
        threshold,
        priorities,
        targets,
        active,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn inspect(
    args: &RenderArgs,
    source: &[[u8; 3]],
    data: &[BlockData],
    states: &[Choice],
    glyphs: &[FastGlyph],
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
) -> Option<Arc<GlyphDefectMap>> {
    let (source, width, height) =
        crate::contour_score::normalize_preview_raster(source, grid_w, grid_h, gw, gh);
    let native = build_rendered_raster(data, states, glyphs, grid_w, grid_h, gw, gh);
    let (output, _, _) =
        crate::contour_score::normalize_preview_raster(&native, grid_w, grid_h, gw, gh);
    let reference = crate::perceptual_score::PreparedReference::new(&source, width, height, CW)?;
    Some(defect_map(
        reference.defect_cells(&output, CW, CH),
        data,
        args.discontinuity_threshold.max(0.0),
        None,
        grid_w,
        grid_h,
    ))
}

fn publish_if_due(
    progress: &mut GlyphSearchProgress,
    started: &RenderInstant,
    last: &mut RenderInstant,
) {
    if last.elapsed().as_secs_f64() >= 0.1 {
        progress.publish(started);
        *last = RenderInstant::now();
    }
}

fn search_preview(
    map: Arc<GlyphDefectMap>,
    data: &[BlockData],
    states: &[Choice],
    glyphs: &[FastGlyph],
    score: f64,
    foreground_only: bool,
) -> Arc<GlyphSearchPreview> {
    let mut result = RenderResult::from("");
    result.foreground_only = foreground_only;
    result.grid = states
        .chunks(map.columns)
        .enumerate()
        .map(|(y, row)| {
            row.iter()
                .enumerate()
                .map(|(x, &(gi, inverted))| {
                    let cell = &data[y * map.columns + x];
                    super::search_status::preview_cell(glyphs[gi].ch, cell.fg, cell.bg, inverted)
                })
                .collect()
        })
        .collect();
    result.score = score;
    result.glyph_defects = Some(map.clone());
    result = super::encode_output_colours(&result, Render::Ansi24, true, 0);
    Arc::new(GlyphSearchPreview {
        defects: map,
        result,
    })
}

#[cfg(test)]
fn seam_error(
    cell: usize,
    mask: u128,
    data: &BlockData,
    source: &[[u8; 3]],
    output: &[[u8; 3]],
    grid_w: usize,
    grid_h: usize,
) -> f64 {
    let x0 = cell % grid_w * CW;
    let y0 = cell / grid_w * CH;
    let width = grid_w * CW;
    let mut error = 0i64;
    let mut count = 0;
    let mut compare = |x: usize, y: usize, other: usize| {
        let pixel = (y0 + y) * width + x0 + x;
        let colour = if mask & (1 << (y * CW + x)) != 0 {
            data.fg
        } else {
            data.bg
        };
        for c in 0..3 {
            let delta = colour[c] as i64
                - output[other][c] as i64
                - (source[pixel][c] as i64 - source[other][c] as i64);
            error += delta * delta;
        }
        count += 1;
    };
    for y in 0..CH {
        if x0 > 0 {
            compare(0, y, (y0 + y) * width + x0 - 1);
        }
        if x0 + CW < width {
            compare(CW - 1, y, (y0 + y) * width + x0 + CW);
        }
    }
    for x in 0..CW {
        if y0 > 0 {
            compare(x, 0, (y0 - 1) * width + x0 + x);
        }
        if y0 + CH < grid_h * CH {
            compare(x, CH - 1, (y0 + CH) * width + x0 + x);
        }
    }
    error as f64 / count.max(1) as f64
}

fn context_fingerprint(
    output: &[[u8; 3]],
    a: usize,
    b: usize,
    pools: &[Vec<Choice>],
    grid_w: usize,
    grid_h: usize,
) -> u64 {
    let x0 = (a % grid_w).min(b % grid_w).saturating_sub(2);
    let y0 = (a / grid_w).min(b / grid_w).saturating_sub(2);
    let x1 = ((a % grid_w).max(b % grid_w) + 3).min(grid_w);
    let y1 = ((a / grid_w).max(b / grid_w) + 3).min(grid_h);
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    pools[a].hash(&mut hash);
    pools[b].hash(&mut hash);
    for y in y0 * CH..y1 * CH {
        output[y * grid_w * CW + x0 * CW..y * grid_w * CW + x1 * CW].hash(&mut hash);
    }
    hash.finish()
}

#[allow(clippy::too_many_arguments)]
fn candidates(
    source: &[[u8; 3]],
    data: &[BlockData],
    states: &[Choice],
    glyphs: &[FastGlyph],
    masks: &[u128],
    preview_source: &[[u8; 3]],
    preview_output: &[[u8; 3]],
    active: &[bool],
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
    limit: usize,
    attempted: Option<&[HashSet<u128>]>,
    foreground_only: bool,
    abort: Option<&AtomicBool>,
) -> Result<Vec<Vec<Choice>>, ()> {
    let connections = connections::Connections::cached(masks);
    data.par_iter()
        .enumerate()
        .map(|(cell, data)| {
            if cancelled(abort) {
                return Err(());
            }
            let current = states[cell];
            if !active[cell] || data.fg == data.bg {
                return Ok(vec![current]);
            }
            let mut base = 0i64;
            let mut diff = Vec::with_capacity(gw * gh);
            for i in 0..gw * gh {
                let rgb = source
                    [((cell / grid_w) * gh + i / gw) * grid_w * gw + (cell % grid_w) * gw + i % gw];
                let bg = squared_error(rgb, data.bg);
                base += bg;
                diff.push(squared_error(rgb, data.fg) - bg);
            }
            let total: i64 = diff.iter().sum();
            let seams = connections.costs(
                cell, data, preview_source, preview_output, grid_w, grid_h,
            );
            let mut ranked = Vec::with_capacity(glyphs.len() * 2);
            for (gi, glyph) in glyphs.iter().enumerate() {
                if gi & 63 == 0 && cancelled(abort) {
                    return Err(());
                }
                let dot: i64 = glyph.ones.iter().map(|&i| diff[i]).sum();
                for (inverted, cost) in [(false, base + dot), (true, base + total - dot)] {
                    if foreground_only && inverted {
                        continue;
                    }
                    let context_cost = cost as f64
                        + connections.error(&seams, (gi, inverted)) * (gw * gh) as f64;
                    ranked.push((cost, (gi, inverted), context_cost));
                }
            }
            ranked.sort_unstable_by_key(|&(cost, state, _)| (cost, state));
            let mut seen = HashSet::new();
            seen.insert(masks[current.0] ^ if current.1 { u128::MAX } else { 0 });
            let mut choices = vec![current];
            let source_limit = limit.div_ceil(2);
            let mut source_seen = seen.clone();
            let mut best_source = true;
            for &(_, choice, _) in &ranked {
                let mask = masks[choice.0] ^ if choice.1 { u128::MAX } else { 0 };
                if source_seen.insert(mask) {
                    // Keep the strongest source fit. After a stalled pass,
                    // rotate the remaining source slots to other shapes.
                    if best_source || attempted.is_none_or(|history| !history[cell].contains(&mask)) {
                        seen.insert(mask);
                        choices.push(choice);
                    }
                    best_source = false;
                }
                if choices.len() > source_limit {
                    break;
                }
            }
            ranked.sort_unstable_by(|a, b| {
                a.2.total_cmp(&b.2)
                    .then_with(|| (a.0, a.1).cmp(&(b.0, b.1)))
            });
            for (_, choice, _) in ranked {
                if choices.len() > limit {
                    break;
                }
                let mask = masks[choice.0] ^ if choice.1 { u128::MAX } else { 0 };
                if seen.insert(mask) {
                    choices.push(choice);
                }
            }
            Ok(choices)
        })
        .collect()
}

struct Patch {
    reference: Arc<PreparedReference>,
    output: Vec<[u8; 3]>,
    features: OutputCache,
    geometry: Option<GeometryCache>,
    width: usize,
    cell_x: usize,
    cell_y: usize,
}

impl Patch {
    fn bounds(a: usize, b: usize, grid_w: usize, grid_h: usize) -> [usize; 4] {
        [
            (a % grid_w).min(b % grid_w).saturating_sub(2),
            (a / grid_w).min(b / grid_w).saturating_sub(2),
            ((a % grid_w).max(b % grid_w) + 3).min(grid_w),
            ((a / grid_w).max(b / grid_w) + 3).min(grid_h),
        ]
    }

    fn new(
        output: &[[u8; 3]],
        a: usize,
        b: usize,
        grid_w: usize,
        grid_h: usize,
        incremental: bool,
        references: &source_cache::SourceReferences<'_>,
    ) -> Self {
        // Context reduces clipping bias in the ranking. The full-image check
        // remains authoritative, including all untouched neighbouring cells.
        let [x0, y0, x1, y1] = Self::bounds(a, b, grid_w, grid_h);
        let width = (x1 - x0) * CW;
        let height = (y1 - y0) * CH;
        let extract = |raster: &[[u8; 3]]| {
            let mut pixels = Vec::with_capacity(width * height);
            for y in y0 * CH..y1 * CH {
                let begin = y * grid_w * CW + x0 * CW;
                pixels.extend_from_slice(&raster[begin..begin + width]);
            }
            pixels
        };
        let reference = references.get([x0, y0, x1, y1]);
        let output = extract(output);
        let features = reference.prepare_output(&output);
        let geometry = incremental.then(|| reference.prepare_geometry(&features));
        Self {
            reference,
            output,
            features,
            geometry,
            width,
            cell_x: x0,
            cell_y: y0,
        }
    }
}

/// Local-ranking buffers belong to a bounded set of workers and are reused
/// between pairs. Full-image scoring keeps one incremental cache instead.
#[derive(Default)]
struct TrialWorkspace {
    output: Vec<[u8; 3]>,
    features: OutputCache,
    score: ScoreWorkspace,
    geometry: Option<GeometryCache>,
    copy_ms: f64,
    patch_ms: f64,
    features_ms: f64,
    scoring_ms: f64,
}

impl TrialWorkspace {
    fn reset(
        &mut self,
        output: &Vec<[u8; 3]>,
        features: &OutputCache,
        geometry: Option<&GeometryCache>,
    ) {
        self.output.clone_from(output);
        self.features.copy_from(features);
        if let Some(geometry) = geometry {
            if let Some(cache) = &mut self.geometry {
                cache.copy_from(geometry);
            } else {
                self.geometry = Some(geometry.clone());
            }
        }
    }
}

#[derive(Clone, Copy)]
struct PairPainter<'a> {
    a: usize,
    b: usize,
    grid_w: usize,
    width: usize,
    cell_x: usize,
    cell_y: usize,
    data: &'a [BlockData],
    masks: &'a [u128],
    gw: usize,
    gh: usize,
}

impl PairPainter<'_> {
    fn rect(&self) -> [usize; 4] {
        let (ax, ay) = (self.a % self.grid_w, self.a / self.grid_w);
        let (bx, by) = (self.b % self.grid_w, self.b / self.grid_w);
        [
            (ax.min(bx) - self.cell_x) * CW,
            (ay.min(by) - self.cell_y) * CH,
            (ax.max(bx) + 1 - self.cell_x) * CW,
            (ay.max(by) + 1 - self.cell_y) * CH,
        ]
    }

    fn paint_cell(&self, output: &mut [[u8; 3]], cell: usize, choice: Choice) {
        // Normalized glyph geometry is invariant across all pair trials.
        // Reuse its mask instead of resampling the font bitmap per pixel.
        let mask = self.masks[choice.0] ^ if choice.1 { u128::MAX } else { 0 };
        let x0 = (cell % self.grid_w - self.cell_x) * CW;
        let y0 = (cell / self.grid_w - self.cell_y) * CH;
        let data = &self.data[cell];
        for y in 0..CH {
            let bits = (mask >> (y * CW)) as u8;
            let start = (y0 + y) * self.width + x0;
            for (x, pixel) in output[start..start + CW].iter_mut().enumerate() {
                *pixel = if bits & (1 << x) != 0 { data.fg } else { data.bg };
            }
        }
    }

    fn paint(&self, output: &mut [[u8; 3]], a: Choice, b: Choice) {
        self.paint_cell(output, self.a, a);
        if self.a != self.b {
            self.paint_cell(output, self.b, b);
        }
    }
}

#[derive(Clone, Copy)]
struct Trial {
    a: Choice,
    b: Choice,
    local: f64,
    local_lower: f64,
    local_upper: f64,
}

fn pair_trials(
    a: usize,
    b: usize,
    old_a: Choice,
    old_b: Choice,
    pools: &[Vec<Choice>],
) -> Vec<Trial> {
    let single = [old_b];
    let b_choices = if a == b { &single[..] } else { &pools[b] };
    let mut trials = Vec::with_capacity(pools[a].len() * b_choices.len());
    for &ca in &pools[a] {
        for &cb in b_choices {
            if ca != old_a || cb != old_b {
                trials.push(Trial {
                    a: ca,
                    b: cb,
                    local: 0.0,
                    local_lower: 0.0,
                    local_upper: 0.0,
                });
            }
        }
    }
    trials
}

const SCORE_BATCH: usize = 32;

fn ranking_batch_size() -> usize {
    // Native workers amortize setup across larger candidate batches. The
    // browser keeps smaller batches for more frequent progress updates.
    if cfg!(target_arch = "wasm32") {
        SCORE_BATCH
    } else {
        128
    }
}

#[derive(Clone, Copy)]
struct CommittedPair {
    a: usize,
    b: usize,
}

/// A full-image worker retains its own exact caches. Synchronize only the
/// accepted edits since its last job, then restore its previous trial. Scores
/// are independent, but their results are consumed in the original order.
struct FullTrialWorkspace {
    output: Vec<[u8; 3]>,
    features: OutputCache,
    geometry: GeometryCache,
    commit_cursor: usize,
    previous_trial: Option<CommittedPair>,
}

impl FullTrialWorkspace {
    fn new(
        reference: &PreparedReference,
        output: &Vec<[u8; 3]>,
        features: &OutputCache,
        cursor: usize,
    ) -> Self {
        let mut cached = OutputCache::default();
        cached.copy_from(features);
        Self {
            output: output.clone(),
            geometry: GeometryCache::new(reference, &cached),
            features: cached,
            commit_cursor: cursor,
            previous_trial: None,
        }
    }

    fn synchronize(
        &mut self,
        reference: &PreparedReference,
        painter: PairPainter<'_>,
        states: &[Choice],
        commits: &[CommittedPair],
    ) {
        let previous = self.previous_trial.take();
        for pair in previous
            .into_iter()
            .chain(commits[self.commit_cursor..].iter().copied())
        {
            let restore = PairPainter {
                a: pair.a,
                b: pair.b,
                ..painter
            };
            restore.paint(&mut self.output, states[pair.a], states[pair.b]);
            let rect = restore.rect();
            let changed = self.features.update(&self.output, rect);
            self.geometry.update(reference, &self.features, changed);
        }
        self.commit_cursor = commits.len();
    }
}

fn full_workers(finalists: usize, fine_pixels: usize, geometry_pixels: usize) -> usize {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (finalists, fine_pixels, geometry_pixels);
        1
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Budget additional persistent full-image caches, including vectors
        // that can grow to the entire raster after rounding propagation.
        const CACHE_BUDGET: usize = 256 * 1024 * 1024;
        let bytes = fine_pixels
            .saturating_mul(52)
            .saturating_add(geometry_pixels.saturating_mul(220));
        let requested = std::env::var("IMG2IRC_REPAIR_CHECK_THREADS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(4)
            .clamp(1, 4);
        let trials = if finalists == 0 {
            SCORE_BATCH
        } else {
            finalists
        };
        let count = requested.min(rayon::current_num_threads()).min(trials);
        // The original serial cache exists either way; only extra workers
        // count against this budget. Large images retain the serial path.
        count.min(1 + CACHE_BUDGET / bytes.max(1)).max(1)
    }
}

fn cpu_workers() -> usize {
    #[cfg(not(target_arch = "wasm32"))]
    {
        rayon::current_num_threads().min(SCORE_BATCH)
    }
    #[cfg(target_arch = "wasm32")]
    {
        1
    }
}

fn workspaces(count: usize) -> Vec<TrialWorkspace> {
    (0..count).map(|_| TrialWorkspace::default()).collect()
}

/// Trials write disjoint slots. Reductions happen within each worker using the
/// original arithmetic; the caller still ranks and commits in the old order.
#[allow(clippy::too_many_arguments)]
fn score_trials(
    trials: &mut [Trial],
    reference: &PreparedReference,
    output: &Vec<[u8; 3]>,
    features: &OutputCache,
    geometry: Option<&GeometryCache>,
    painter: &PairPainter<'_>,
    workers: &mut [TrialWorkspace],
    abort: Option<&AtomicBool>,
    profiling: bool,
) -> Result<(), ()> {
    if trials.is_empty() {
        return Ok(());
    }
    let chunk_size = trials.len().div_ceil(workers.len());
    let rect = painter.rect();
    let score_chunk = |trials: &mut [Trial], work: &mut TrialWorkspace| -> Result<(), ()> {
        let copy_started = profiling.then(RenderInstant::now);
        work.reset(output, features, geometry);
        if let Some(started) = copy_started {
            work.copy_ms += started.elapsed().as_secs_f64() * 1000.0;
        }
        for trial in trials {
            if cancelled(abort) {
                return Err(());
            }
            let features_started = profiling.then(RenderInstant::now);
            painter.paint(&mut work.output, trial.a, trial.b);
            let changed = work.features.update(&work.output, rect);
            if let Some(started) = features_started {
                work.features_ms += started.elapsed().as_secs_f64() * 1000.0;
            }
            let scoring_started = profiling.then(RenderInstant::now);
            let score = if geometry.is_some() {
                reference.score_incremental_estimate(
                    &work.features,
                    work.geometry.as_mut().unwrap(),
                    changed,
                )
            } else {
                ScoreEstimate::exact(
                    reference
                        .score_prepared(&work.features, &mut work.score)
                        .total,
                )
            };
            if let Some(started) = scoring_started {
                work.scoring_ms += started.elapsed().as_secs_f64() * 1000.0;
            }
            trial.local = score.value;
            trial.local_lower = score.lower;
            trial.local_upper = score.upper;
        }
        Ok(())
    };
    if workers.len() == 1 {
        return score_chunk(trials, &mut workers[0]);
    }
    trials
        .par_chunks_mut(chunk_size)
        .zip(workers.par_iter_mut())
        .try_for_each(|(trials, work)| score_chunk(trials, work))
}

// Incremental totals use a different f64 reduction order. Only overlapping
// intervals can alter the finalist order; settle those with the original scorer.
fn settle_local_order(
    trials: &mut [Trial],
    patch: &Patch,
    painter: PairPainter<'_>,
    work: &mut TrialWorkspace,
) {
    let mut order: Vec<_> = (0..trials.len()).collect();
    order.sort_unstable_by(|&a, &b| trials[a].local.total_cmp(&trials[b].local));
    let mut exact = vec![false; trials.len()];
    for pair in order.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if trials[a].local_upper >= trials[b].local_lower {
            exact[a] = true;
            exact[b] = true;
        }
    }
    if !exact.iter().any(|&needed| needed) {
        return;
    }
    let rect = painter.rect();
    let copying = detailed_profiling().then(RenderInstant::now);
    work.reset(&patch.output, &patch.features, None);
    if let Some(start) = copying { work.copy_ms += start.elapsed().as_secs_f64() * 1000.0; }
    for (trial, exact) in trials.iter_mut().zip(exact) {
        if !exact {
            continue;
        }
        let painting = detailed_profiling().then(RenderInstant::now);
        painter.paint(&mut work.output, trial.a, trial.b);
        work.features.update(&work.output, rect);
        if let Some(start) = painting { work.features_ms += start.elapsed().as_secs_f64() * 1000.0; }
        let scoring = detailed_profiling().then(RenderInstant::now);
        trial.local = patch
            .reference
            .score_prepared(&work.features, &mut work.score)
            .total;
        trial.local_lower = trial.local;
        trial.local_upper = trial.local;
        if let Some(start) = scoring { work.scoring_ms += start.elapsed().as_secs_f64() * 1000.0; }
    }
}

struct PreparedGroup {
    trials: Vec<Trial>,
    versions: Vec<(usize, u64)>,
    // A cache hit needs no raster/features/geometry preparation at all.
    patch: Option<Patch>,
}

struct RankContext<'a> {
    output: &'a [[u8; 3]],
    data: &'a [BlockData],
    masks: &'a [u128],
    pools: &'a [Vec<Choice>],
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
    finalists: usize,
    abort: Option<&'a AtomicBool>,
    references: &'a source_cache::SourceReferences<'a>,
}

impl RankContext<'_> {
    fn rank(
        &self,
        a: usize,
        b: usize,
        states: &[Choice],
        work: &mut TrialWorkspace,
    ) -> Result<(Vec<Trial>, Patch), ()> {
        if cancelled(self.abort) {
            return Err(());
        }
        let setup = detailed_profiling().then(RenderInstant::now);
        let patch = Patch::new(
            self.output,
            a,
            b,
            self.grid_w,
            self.grid_h,
            true,
            self.references,
        );
        if let Some(start) = setup { work.patch_ms += start.elapsed().as_secs_f64() * 1000.0; }
        let trials = self.rank_patch(a, b, states, std::slice::from_mut(work), &patch)?;
        Ok((trials, patch))
    }

    fn rank_patch(
        &self,
        a: usize,
        b: usize,
        states: &[Choice],
        workers: &mut [TrialWorkspace],
        patch: &Patch,
    ) -> Result<Vec<Trial>, ()> {
        debug_assert!(self.finalists > 0);
        let painter = PairPainter {
            a,
            b,
            grid_w: self.grid_w,
            width: patch.width,
            cell_x: patch.cell_x,
            cell_y: patch.cell_y,
            data: self.data,
            masks: self.masks,
            gw: self.gw,
            gh: self.gh,
        };
        let mut trials = pair_trials(a, b, states[a], states[b], self.pools);
        score_trials(
            &mut trials,
            &patch.reference,
            &patch.output,
            &patch.features,
            patch.geometry.as_ref(),
            &painter,
            workers,
            self.abort,
            detailed_profiling(),
        )?;
        settle_local_order(&mut trials, &patch, painter, &mut workers[0]);
        trials.retain(|trial| trial.local.is_finite());
        trials.sort_unstable_by(|a, b| {
            a.local
                .total_cmp(&b.local)
                .then_with(|| (a.a, a.b).cmp(&(b.a, b.b)))
        });
        Ok(trials)
    }

    fn refresh(
        &self,
        a: usize,
        b: usize,
        states: &[Choice],
        versions: &[u64],
        ready: &mut PreparedGroup,
        workers: &mut [TrialWorkspace],
    ) -> Result<Vec<Trial>, ()> {
        let setup = detailed_profiling().then(RenderInstant::now);
        let patch = ready.patch.get_or_insert_with(|| Patch::new(
            self.output, a, b, self.grid_w, self.grid_h, true, self.references,
        ));
        let painter = PairPainter {
            a,
            b,
            grid_w: self.grid_w,
            width: patch.width,
            cell_x: patch.cell_x,
            cell_y: patch.cell_y,
            data: self.data,
            masks: self.masks,
            gw: self.gw,
            gh: self.gh,
        };
        let mut dirty = [usize::MAX, usize::MAX, 0, 0];
        for &(cell, version) in &ready.versions {
            if versions[cell] == version {
                continue;
            }
            painter.paint_cell(&mut patch.output, cell, states[cell]);
            let (x, y) = (
                cell % self.grid_w - patch.cell_x,
                cell / self.grid_w - patch.cell_y,
            );
            dirty = [
                dirty[0].min(x * CW),
                dirty[1].min(y * CH),
                dirty[2].max((x + 1) * CW),
                dirty[3].max((y + 1) * CH),
            ];
        }
        if dirty[0] < dirty[2] {
            let changed = patch.features.update(&patch.output, dirty);
            patch
                .geometry
                .as_mut()
                .unwrap()
                .update(&patch.reference, &patch.features, changed);
        }
        if let Some(start) = setup { workers[0].patch_ms += start.elapsed().as_secs_f64() * 1000.0; }
        // Initial batches already parallelize pairs. A stale pair is refreshed
        // after those jobs finish, so parallelize its independent trials here.
        let trials = self.pools[a].len() * if a == b { 1 } else { self.pools[b].len() };
        let count = workers.len().min(4).min((trials / 12).max(1));
        self.rank_patch(a, b, states, &mut workers[..count], patch)
    }

    fn prepare(
        &self,
        groups: &[(usize, usize)],
        states: &[Choice],
        versions: &[u64],
        workers: &mut [TrialWorkspace],
    ) -> Result<Vec<Option<PreparedGroup>>, ()> {
        // Look up scores on the calling render thread, before dispatching any
        // speculative scoring. Worker threads must not consult thread-local
        // caches belonging to unrelated render sessions.
        let mut jobs: Vec<_> = groups.iter().map(|&(a, b)| {
            let painter = PairPainter {
                a, b, grid_w: self.grid_w, width: self.grid_w * CW,
                cell_x: 0, cell_y: 0, data: self.data, masks: self.masks,
                gw: self.gw, gh: self.gh,
            };
            let key = rank_cache::key(self.references.source(), self.output, painter,
                self.masks, self.pools, (states[a], states[b]), self.grid_h,
                self.finalists, workers.len());
            let cached = key.as_ref().and_then(rank_cache::get);
            (a, b, if cached.is_some() { None } else { key }, cached)
        }).collect();
        let chunk = jobs.len().div_ceil(workers.len()).max(1);
        let chunks: Result<Vec<Vec<_>>, ()> = jobs
            .par_chunks_mut(chunk)
            .zip(workers.par_iter_mut())
            .map(|(jobs, work)| {
                // Keep allocations across batches, including small batches
                // chosen to avoid speculative work invalidated by commits.
                jobs
                    .iter_mut()
                    .map(|(a, b, _, cached)| {
                        let (a, b) = (*a, *b);
                        if cancelled(self.abort) { return Err(()); }
                        let (trials, patch) = if let Some(trials) = cached.take() {
                            (trials, None)
                        } else {
                            let (trials, patch) = self.rank(a, b, states, work)?;
                            (trials, Some(patch))
                        };
                        let [x0, y0, x1, y1] = Patch::bounds(a, b, self.grid_w, self.grid_h);
                        let versions = (y0..y1)
                            .flat_map(|y| {
                                (x0..x1).map(move |x| {
                                    let cell = y * self.grid_w + x;
                                    (cell, versions[cell])
                                })
                            })
                            .collect();
                        Ok(Some(PreparedGroup {
                            trials,
                            versions,
                            patch,
                        }))
                    })
                    .collect::<Result<Vec<_>, ()>>()
            })
            .collect();
        let prepared: Vec<_> = chunks?.into_iter().flatten().collect();
        for (job, ready) in jobs.into_iter().zip(&prepared) {
            if let (Some(key), Some(ready)) = (job.2, ready) {
                rank_cache::insert(key, &ready.trials);
            }
        }
        Ok(prepared)
    }
}

/// Key the exact edits against the current committed image. Single-cell edits
/// proposed by different pairs then share a score. Clear whenever a move is
/// committed: a changed image elsewhere invalidates a full-image score.
type ScoreKey = (Option<(usize, Choice)>, Option<(usize, Choice)>);

fn score_key(a: usize, b: usize, old_a: Choice, old_b: Choice, trial: Trial) -> ScoreKey {
    let first = (trial.a != old_a).then_some((a, trial.a));
    let second = (a != b && trial.b != old_b).then_some((b, trial.b));
    match (first, second) {
        (None, second) => (second, None),
        edits => edits,
    }
}

fn verify_pair_score(
    reference: &PreparedReference,
    base: &Vec<[u8; 3]>,
    painter: PairPainter<'_>,
    choices: (Choice, Choice),
    scratch: &mut Vec<[u8; 3]>,
) -> ScoreEstimate {
    scratch.clone_from(base);
    painter.paint(scratch, choices.0, choices.1);
    ScoreEstimate::exact(reference.score(scratch).total)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run(
    args: &RenderArgs,
    image: &AnsiImage,
    render: Render,
    source: &[[u8; 3]],
    data: &mut [BlockData],
    states: &mut [Choice],
    glyphs: &[FastGlyph],
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
    abort: Option<&AtomicBool>,
) -> Result<Option<Arc<GlyphDefectMap>>, ()> {
    rank_cache::with_default_cache(|| run_search(args, image, render, source, data,
        states, glyphs, grid_w, grid_h, gw, gh, abort))
}

#[allow(clippy::too_many_arguments)]
fn run_search(
    args: &RenderArgs,
    image: &AnsiImage,
    render: Render,
    source: &[[u8; 3]],
    data: &mut [BlockData],
    states: &mut [Choice],
    glyphs: &[FastGlyph],
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
    abort: Option<&AtomicBool>,
) -> Result<Option<Arc<GlyphDefectMap>>, ()> {
    if states.is_empty() {
        return Ok(None);
    }
    let started = RenderInstant::now();
    let initial_cache_stats = rank_cache::stats();
    let passes = args.glyph_search_passes.clamp(1, 20);
    let mut status = GlyphSearchProgress::new(passes);
    status.publish(&started);
    let mut last_status = RenderInstant::now();
    let mut initial_defects = None;
    let native_source = source;
    let (source, width, height) =
        crate::contour_score::normalize_preview_raster(native_source, grid_w, grid_h, gw, gh);
    let native_output = build_rendered_raster(data, states, glyphs, grid_w, grid_h, gw, gh);
    let (mut output, _, _) =
        crate::contour_score::normalize_preview_raster(&native_output, grid_w, grid_h, gw, gh);
    let reference = crate::perceptual_score::PreparedReference::new(&source, width, height, CW)
        .expect("valid glyph search reference");
    let mut best_score = reference.score(&output).total;
    if !best_score.is_finite() {
        return Ok(None);
    }
    let initial_score = best_score;
    let initial_states = states.to_vec();
    let initial_colours = (!args.blend_colours && !args.foreground_only).then(|| data.to_vec());
    let temporal_key = temporal::key(args, glyphs, grid_w, grid_h, gw, gh);
    let mut temporal_targets = temporal_key
        .as_deref()
        .and_then(|key| {
            temporal::reuse(key, &source, data, states, grid_w, grid_h, !args.foreground_only)
        });
    if let Some(targets) = &temporal_targets {
        let warm = build_rendered_raster(data, states, glyphs, grid_w, grid_h, gw, gh);
        let (warm, _, _) =
            crate::contour_score::normalize_preview_raster(&warm, grid_w, grid_h, gw, gh);
        let score = reference.score(&warm).total;
        if score.is_finite() && score <= initial_score {
            temporal::record_reused(targets.iter().filter(|&&target| !target).count());
            best_score = score;
            output = warm;
        } else {
            states.copy_from_slice(&initial_states);
            temporal_targets = None;
        }
    }
    let mut best_estimate = ScoreEstimate::exact(best_score);
    let mut verification_raster = Vec::new();
    let mut precision_checks = 0usize;
    let mut features = reference.prepare_output(&output);
    let alternatives = args.glyph_search_candidates.clamp(1, 64) as usize;
    let finalists = args.glyph_search_finalists.min(4096) as usize;
    let full_worker_count = full_workers(finalists, output.len(), reference.geometry_pixels());
    let mut geometry = (full_worker_count == 1).then(|| GeometryCache::new(&reference, &features));
    let mut pending_geometry: Option<[usize; 4]> = None;
    let mut full_workspaces: Vec<Option<FullTrialWorkspace>> =
        (0..full_worker_count).map(|_| None).collect();
    let mut commits = Vec::new();
    let workers = cpu_workers();
    let mut local_workers = workspaces(workers);
    let mut score_cache: HashMap<ScoreKey, ScoreEstimate> = HashMap::new();
    let mut score_cache_hits = 0usize;
    status.score = best_score;
    let masks: Vec<_> = glyphs
        .iter()
        .map(|glyph| smooth_preview_glyph(glyph, gw, gh).mask)
        .collect();
    let unique_shapes = masks.iter().copied().flat_map(|mask| {
        [Some(mask), (!args.foreground_only).then_some(!mask)].into_iter().flatten()
    }).collect::<HashSet<_>>().len();
    let mut attempted: Option<Vec<HashSet<u128>>> = None;
    let mut combinations = 0usize;
    let mut full_checks = 0usize;
    let mut moves = 0usize;
    let mut joint_moves = 0usize;
    let mut ranking_ms = 0.0;
    let mut key_ms = 0.0;
    let mut patch_ms = 0.0;
    let mut dispatch_ms = 0.0;
    let mut batch_ms = 0.0;
    let profiling = std::env::var_os("IMG2IRC_REPAIR_PROFILE").is_some();
    let incremental_rank = std::env::var_os("IMG2IRC_REPAIR_INCREMENTAL_RANK").is_some();
    let mut checking_ms = 0.0;
    let threshold = args.discontinuity_threshold.max(0.0);
    let focused = threshold > 0.0;
    let mut contexts = HashMap::new();
    let mut skipped = 0usize;
    let mut versions = vec![0_u64; states.len()];
    let mut prepared_reused = 0usize;
    let mut prepared_stale = 0usize;
    let fixed_batch_size = std::env::var("IMG2IRC_REPAIR_BATCH_SIZE")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .map(|size| size.min(256));
    // Very small batches spend too much time dispatching workers. Preserve
    // enough independent pairs to amortize that cost even after a busy region
    // causes the speculative batch size to shrink.
    let minimum_batch = workers.clamp(8, 16);
    let mut batch_size = fixed_batch_size.unwrap_or(if cfg!(target_arch = "wasm32") { 0 } else { minimum_batch * 2 });
    let adaptive_batches = fixed_batch_size.is_none() && batch_size > 0;
    let mut smallest_batch = usize::MAX;
    let mut largest_batch = 0;
    let mut prepared: Vec<Option<PreparedGroup>> = Vec::new();
    let references = source_cache::SourceReferences::new(&source, width);
    log::info!(
        "Glyph search: starting perceptual={:.6}, {} alternatives/cell, {} passes, {}, CPU workers={} local / {} full-image, guarded incremental full-image scoring with cached block totals",
        initial_score,
        alternatives,
        passes,
        if finalists == 0 {
            "full score for every pair combination".to_string()
        } else {
            format!("{} full-score finalists/pair", finalists)
        },
        workers,
        full_worker_count,
    );

    for pass in 0..passes {
        if cancelled(abort) {
            return Err(());
        }
        status.stage = GlyphSearchStage::Analyzing;
        status.pass = pass + 1;
        status.pairs_done = 0;
        status.pairs_total = 0;
        status.pair_work_done = 0;
        status.pair_work_total = 0;
        status.publish(&started);
        let map = defect_map(
            reference.defect_cells(&output, CW, CH),
            data,
            threshold,
            temporal_targets.as_deref(),
            grid_w,
            grid_h,
        );
        if initial_defects.is_none() {
            initial_defects = Some(map.clone());
        }
        if super::search_status::wants_preview() {
            status.preview = Some(search_preview(
                map.clone(),
                data,
                states,
                glyphs,
                best_score,
                args.foreground_only,
            ));
        }
        let (priorities, targets, active) = (&map.priorities, &map.targets, &map.active);
        status.stage = GlyphSearchStage::Candidates;
        status.publish(&started);
        // Keep both strong source fits and seam-aware alternatives. A defect
        // can require changing a neighbouring cell that looked good by itself.
        let pools = candidates(
            native_source,
            data,
            states,
            glyphs,
            &masks,
            &source,
            &output,
            active,
            grid_w,
            grid_h,
            gw,
            gh,
            alternatives,
            attempted.as_deref(),
            args.foreground_only,
            abort,
        )?;
        let mut groups = HashSet::new();
        for cell in 0..states.len() {
            if !targets[cell] || pools[cell].len() <= 1 {
                continue;
            }
            for neighbor in neighbors(cell, grid_w, grid_h) {
                groups.insert((cell.min(neighbor), cell.max(neighbor)));
            }
            if states.len() == 1 {
                groups.insert((cell, cell));
            }
        }
        let mut groups: Vec<_> = groups.into_iter().collect();
        groups.sort_unstable_by(|a, b| {
            priorities[b.0]
                .max(priorities[b.1])
                .total_cmp(&priorities[a.0].max(priorities[a.1]))
                .then_with(|| if pass % 2 == 0 { a.cmp(b) } else { b.cmp(a) })
        });
        log::info!("Glyph search pass {}: detected {}/{} target cells (threshold {:.3}), prepared {} neighbour cells and {} horizontal/vertical/diagonal pairs", pass + 1, targets.iter().filter(|&&target| target).count(), states.len(), threshold, active.iter().filter(|&&active| active).count(), groups.len());
        status.stage = GlyphSearchStage::Searching;
        status.pairs_total = groups.len();
        status.publish(&started);
        let before_pass = best_score;
        let moves_before = moves;
        let skipped_before = skipped;
        let mut batch_start = 0;
        let mut batch_end = 0;
        let mut batch_reused = prepared_reused;
        let mut batch_stale = prepared_stale;
        let mut progress = RenderInstant::now();
        for (group_index, &(a, b)) in groups.iter().enumerate() {
            if cancelled(abort) {
                return Err(());
            }
            if batch_size > 0 && finalists != 0 && group_index == batch_end {
                if adaptive_batches && batch_end > 0 {
                    let reused = prepared_reused - batch_reused;
                    let stale = prepared_stale - batch_stale;
                    // Accepted edits can invalidate speculative rankings.
                    // Spend less work ahead when that is happening; quiet
                    // regions can amortize dispatch with larger batches.
                    if stale * 8 > reused + stale {
                        batch_size = (batch_size / 2).max(minimum_batch);
                    } else if stale == 0 && reused >= batch_size / 2 {
                        batch_size = (batch_size * 2).min(64);
                    }
                }
                batch_reused = prepared_reused;
                batch_stale = prepared_stale;
                batch_start = group_index;
                batch_end = (group_index + batch_size).min(groups.len());
                smallest_batch = smallest_batch.min(batch_size);
                largest_batch = largest_batch.max(batch_size);
                let started = RenderInstant::now();
                let context = RankContext {
                    output: &output,
                    data,
                    masks: &masks,
                    pools: &pools,
                    grid_w,
                    grid_h,
                    gw,
                    gh,
                    finalists,
                    abort,
                    references: &references,
                };
                prepared = context.prepare(
                    &groups[batch_start..batch_end],
                    states,
                    &versions,
                    &mut local_workers,
                )?;
                let elapsed = started.elapsed().as_secs_f64() * 1000.0;
                ranking_ms += elapsed;
                batch_ms += elapsed;
            }
            let fingerprint = context_fingerprint(&output, a, b, &pools, grid_w, grid_h);
            if focused && contexts.get(&(a, b)) == Some(&fingerprint) {
                skipped += 1;
                status.skipped = skipped;
                status.pairs_done = group_index + 1;
                status.pair_work_done = 0;
                status.pair_work_total = 0;
                publish_if_due(&mut status, &started, &mut last_status);
                continue;
            }
            contexts.insert((a, b), fingerprint);
            let ranking_started = RenderInstant::now();
            let old_a = states[a];
            let old_b = states[b];
            let painter = PairPainter {
                a,
                b,
                grid_w,
                width,
                cell_x: 0,
                cell_y: 0,
                data,
                masks: &masks,
                gw,
                gh,
            };
            let rect = painter.rect();
            let mut selected = (old_a, old_b);
            let mut last_raster_trial = None;
            let before_group = best_score;
            let single = [old_b];
            let b_choices = if a == b { &single[..] } else { &pools[b][..] };
            let trial_count = pools[a].len() * b_choices.len() - 1;
            status.pairs_done = group_index;
            status.pair_work_done = 0;
            status.pair_work_total = trial_count
                + if finalists == 0 {
                    trial_count
                } else {
                    trial_count.min(finalists)
                };
            let mut trials = pair_trials(a, b, old_a, old_b, &pools);
            let ready = if batch_size > 0 && finalists != 0 {
                prepared[group_index - batch_start].take()
            } else {
                None
            };
            let ready_valid = ready.as_ref().is_some_and(|ready| ready
                .versions.iter().all(|&(cell, version)| versions[cell] == version));
            if ready.is_some() {
                if ready_valid { prepared_reused += 1; } else { prepared_stale += 1; }
            }
            let key_started = RenderInstant::now();
            // Valid prepared rankings were already looked up before dispatch.
            // Only a changed neighbourhood needs another key and lookup.
            let cache_key = (finalists != 0 && !ready_valid)
                .then(|| {
                    rank_cache::key(
                        &source,
                        &output,
                        painter,
                        &masks,
                        &pools,
                        (old_a, old_b),
                        grid_h,
                        finalists,
                        workers,
                    )
                })
                .flatten();
            let cached = cache_key.as_ref().and_then(rank_cache::get);
            key_ms += key_started.elapsed().as_secs_f64() * 1000.0;
            if let Some(cached) = cached {
                trials = cached;
                combinations += trial_count;
                status.combinations = combinations;
                status.pair_work_done += trial_count;
            } else if let Some(mut ready) = ready {
                if ready_valid {
                    trials = ready.trials;
                } else {
                    let context = RankContext {
                        output: &output,
                        data,
                        masks: &masks,
                        pools: &pools,
                        grid_w,
                        grid_h,
                        gw,
                        gh,
                        finalists,
                        abort,
                        references: &references,
                    };
                    trials = context.refresh(
                        a,
                        b,
                        states,
                        &versions,
                        &mut ready,
                        &mut local_workers,
                    )?;
                }
                combinations += trial_count;
                status.combinations = combinations;
                status.pair_work_done += trial_count;
                if let Some(key) = cache_key {
                    rank_cache::insert(key, &trials);
                }
            } else {
                let patch_started = RenderInstant::now();
                let patch = (finalists != 0)
                    .then(|| Patch::new(&output, a, b, grid_w, grid_h, incremental_rank, &references));
                patch_ms += patch_started.elapsed().as_secs_f64() * 1000.0;
                let dispatch_started = RenderInstant::now();
                for batch in trials.chunks_mut(ranking_batch_size()) {
                    if cancelled(abort) {
                        return Err(());
                    }
                    if let Some(patch) = &patch {
                        let local_painter = PairPainter {
                            width: patch.width,
                            cell_x: patch.cell_x,
                            cell_y: patch.cell_y,
                            ..painter
                        };
                        score_trials(
                            batch,
                            &patch.reference,
                            &patch.output,
                            &patch.features,
                            patch.geometry.as_ref(),
                            &local_painter,
                            &mut local_workers,
                            abort,
                            profiling,
                        )?;
                    }
                    combinations += batch.len();
                    status.combinations = combinations;
                    status.pair_work_done += batch.len();
                    publish_if_due(&mut status, &started, &mut last_status);
                }
                dispatch_ms += dispatch_started.elapsed().as_secs_f64() * 1000.0;
                if incremental_rank {
                    if let Some(patch) = &patch {
                        let painter = PairPainter {
                            width: patch.width,
                            cell_x: patch.cell_x,
                            cell_y: patch.cell_y,
                            ..painter
                        };
                        settle_local_order(&mut trials, patch, painter, &mut local_workers[0]);
                    }
                }
                trials.retain(|trial| trial.local.is_finite());
                if finalists != 0 {
                    trials.sort_unstable_by(|a, b| {
                        a.local
                            .total_cmp(&b.local)
                            .then_with(|| (a.a, a.b).cmp(&(b.a, b.b)))
                    });
                }
                if let Some(key) = cache_key {
                    rank_cache::insert(key, &trials);
                }
            }
            if finalists != 0 {
                trials.truncate(finalists);
            }
            ranking_ms += ranking_started.elapsed().as_secs_f64() * 1000.0;
            let checking_started = RenderInstant::now();
            for checking_batch in trials.chunks(SCORE_BATCH) {
                if full_worker_count > 1 {
                    if score_cache.len() > 16_384 {
                        score_cache.clear();
                    }
                    let mut missing: Vec<_> = checking_batch
                        .iter()
                        .copied()
                        .filter_map(|trial| {
                            let key = score_key(a, b, old_a, old_b, trial);
                            (!score_cache.contains_key(&key)).then_some((
                                trial,
                                key,
                                ScoreEstimate::exact(f64::NAN),
                            ))
                        })
                        .collect();
                    score_cache_hits += checking_batch.len() - missing.len();
                    if !missing.is_empty() {
                        let chunk = missing.len().div_ceil(full_workspaces.len());
                        missing
                            .par_chunks_mut(chunk)
                            .zip(full_workspaces.par_iter_mut())
                            .try_for_each(|(jobs, slot)| -> Result<(), ()> {
                                if cancelled(abort) {
                                    return Err(());
                                }
                                let work = slot.get_or_insert_with(|| {
                                    FullTrialWorkspace::new(
                                        &reference,
                                        &output,
                                        &features,
                                        commits.len(),
                                    )
                                });
                                work.synchronize(&reference, painter, states, &commits);
                                for (trial, _, score) in jobs {
                                    if cancelled(abort) {
                                        return Err(());
                                    }
                                    painter.paint(&mut work.output, trial.a, trial.b);
                                    let changed = work.features.update(&work.output, rect);
                                    *score = reference.score_incremental_estimate(
                                        &work.features,
                                        &mut work.geometry,
                                        changed,
                                    );
                                    work.previous_trial = Some(CommittedPair { a, b });
                                }
                                Ok(())
                            })?;
                        full_checks += missing.len();
                        for (_, key, score) in missing {
                            score_cache.insert(key, score);
                        }
                    }
                }
                for &trial in checking_batch {
                    if cancelled(abort) {
                        return Err(());
                    }
                    if full_worker_count == 1 && score_cache.len() > 16_384 {
                        score_cache.clear();
                    }
                    let key = score_key(a, b, old_a, old_b, trial);
                    let mut score = if let Some(&score) = score_cache.get(&key) {
                        if full_worker_count == 1 {
                            score_cache_hits += 1;
                        }
                        score
                    } else {
                        painter.paint(&mut output, trial.a, trial.b);
                        features.update(&output, rect);
                        let mut changed = rect;
                        if let Some(pending) = pending_geometry.take() {
                            let union = [
                                pending[0].min(rect[0]),
                                pending[1].min(rect[1]),
                                pending[2].max(rect[2]),
                                pending[3].max(rect[3]),
                            ];
                            let area = |r: [usize; 4]| (r[2] - r[0]) * (r[3] - r[1]);
                            if area(union) <= area(pending) + 2 * area(rect) {
                                changed = union;
                            } else {
                                geometry
                                    .as_mut()
                                    .unwrap()
                                    .update(&reference, &features, pending);
                            }
                        }
                        let score = reference.score_incremental_estimate(
                            &features,
                            geometry.as_mut().expect("serial scoring cache"),
                            changed,
                        );
                        last_raster_trial = Some((trial.a, trial.b));
                        full_checks += 1;
                        score_cache.insert(key, score);
                        score
                    };
                    // Non-overlapping intervals prove the raster-order
                    // decision. Resolve borderline improvements with the
                    // original scorer, including the currently selected pair.
                    if score.value.is_finite()
                        && score.lower + 1e-7 < best_estimate.upper
                        && score.upper + 1e-7 >= best_estimate.lower
                    {
                        score = verify_pair_score(
                            &reference,
                            &output,
                            painter,
                            (trial.a, trial.b),
                            &mut verification_raster,
                        );
                        score_cache.insert(key, score);
                        precision_checks += 1;
                        if best_estimate.lower != best_estimate.upper {
                            best_estimate = verify_pair_score(
                                &reference,
                                &output,
                                painter,
                                selected,
                                &mut verification_raster,
                            );
                            best_score = best_estimate.value;
                            precision_checks += 1;
                        }
                    }
                    // Preserve the original trial order and improvement epsilon.
                    if score.value.is_finite() && score.value + 1e-7 < best_score {
                        best_score = score.value;
                        best_estimate = score;
                        selected = (trial.a, trial.b);
                    }
                    status.full_checks = full_checks;
                    status.score = best_score;
                    status.pair_work_done += 1;
                    publish_if_due(&mut status, &started, &mut last_status);
                }
            }
            // Restore the best tested combination, including the starting pair.
            paint_global_preview_cell(
                &mut output,
                width,
                a,
                grid_w,
                &data[a],
                &glyphs[selected.0 .0],
                selected.0 .1,
                gw,
                gh,
            );
            if a != b {
                paint_global_preview_cell(
                    &mut output,
                    width,
                    b,
                    grid_w,
                    &data[b],
                    &glyphs[selected.1 .0],
                    selected.1 .1,
                    gw,
                    gh,
                );
            }
            // The last scored trial may differ from the chosen pair, including
            // when all scores came from the memo table. Restore its features too.
            if last_raster_trial.is_some() || selected != (old_a, old_b) {
                features.update(&output, rect);
            }
            if geometry.is_some() {
                if let Some(last) = last_raster_trial {
                    pending_geometry = (last != selected).then_some(rect);
                } else if selected != (old_a, old_b) {
                    pending_geometry = Some(if let Some(p) = pending_geometry {
                        [
                            p[0].min(rect[0]),
                            p[1].min(rect[1]),
                            p[2].max(rect[2]),
                            p[3].max(rect[3]),
                        ]
                    } else {
                        rect
                    });
                }
            }
            checking_ms += checking_started.elapsed().as_secs_f64() * 1000.0;
            states[a] = selected.0;
            if a != b {
                states[b] = selected.1;
            }
            if selected.0 != old_a {
                versions[a] += 1;
            }
            if a != b && selected.1 != old_b {
                versions[b] += 1;
            }
            if selected != (old_a, old_b) {
                commits.push(CommittedPair { a, b });
                score_cache.clear();
                moves += 1;
                joint_moves += usize::from(a != b && selected.0 != old_a && selected.1 != old_b);
                log::debug!(
                    "Glyph search pair ({},{}) ({},{}): perceptual {:.6} -> {:.6}",
                    a % grid_w,
                    a / grid_w,
                    b % grid_w,
                    b / grid_w,
                    before_group,
                    best_score
                );
            }
            status.moves = moves;
            status.pairs_done = group_index + 1;
            status.pair_work_done = 0;
            status.pair_work_total = 0;
            publish_if_due(&mut status, &started, &mut last_status);
            if progress.elapsed().as_secs_f64() >= 2.0 {
                if super::search_status::wants_preview() {
                    status.preview = Some(search_preview(
                        map.clone(),
                        data,
                        states,
                        glyphs,
                        best_score,
                        args.foreground_only,
                    ));
                    status.publish(&started);
                    last_status = RenderInstant::now();
                }
                log::info!("Glyph search pass {}: {}/{} pairs, {} combinations, {} full checks, {} full scores reused, {} unchanged pairs skipped, {} moves ({} joint), perceptual {:.6}", pass + 1, group_index + 1, groups.len(), combinations, full_checks, score_cache_hits, skipped, moves, joint_moves, best_score);
                progress = RenderInstant::now();
            }
        }
        log::info!(
            "Glyph search pass {}: {} pairs, {} unchanged pairs skipped, {} moves, perceptual {:.6} -> {:.6}",
            pass + 1,
            groups.len(),
            skipped - skipped_before,
            moves - moves_before,
            before_pass,
            best_score
        );
        status.publish(&started);
        if moves == moves_before {
            let step = alternatives.div_ceil(2).saturating_sub(1);
            if pass + 1 >= passes || step == 0 {
                break;
            }
            let history = attempted.get_or_insert_with(|| vec![HashSet::new(); states.len()]);
            let mut unexplored = false;
            for (cell, pool) in pools.iter().enumerate() {
                history[cell].extend(pool.iter().map(|&(gi, inverted)| masks[gi] ^ if inverted { u128::MAX } else { 0 }));
                unexplored |= active[cell] && data[cell].fg != data[cell].bg && history[cell].len() < unique_shapes;
            }
            if !unexplored { break; }
            log::info!("Glyph search stalled: use the remaining pass budget to try untested source-fit alternatives");
        } else {
            // Successful changes provide new neighbourhoods. Previously tried
            // glyphs may be useful again in that context.
            attempted = None;
        }
    }
    if cancelled(abort) {
        return Err(());
    }
    status.stage = GlyphSearchStage::Verifying;
    status.publish(&started);
    // Without blending, colours must come from the final glyph's source masks.
    // Verify those actual output colours before reporting completion: fitting
    // them later would change both the published score and the rollback check.
    let colours_changed = source_colours::refit(image, args, render, data, states, glyphs, grid_w);
    // Rebuild through the actual renderer before handing the result back. Keep
    // the starting state if normalization or painting ever disagrees with it.
    let native = build_rendered_raster(data, states, glyphs, grid_w, grid_h, gw, gh);
    let (verified_output, _, _) =
        crate::contour_score::normalize_preview_raster(&native, grid_w, grid_h, gw, gh);
    let verified = reference.score(&verified_output).total;
    let rolled_back = !verified.is_finite() || verified > initial_score;
    if rolled_back {
        states.copy_from_slice(&initial_states);
        if let Some(initial_colours) = initial_colours {
            data.clone_from_slice(&initial_colours);
        }
    }
    if colours_changed && args.show_glyph_defects {
        initial_defects = inspect(args, native_source, data, states, glyphs, grid_w, grid_h, gw, gh);
    }
    if let Some(key) = temporal_key {
        temporal::remember(key, &source, data, states);
    }
    log::info!("Glyph search finished: {} combinations, {} full checks, {} full scores reused, {} unchanged pairs skipped, {} moves ({} joint), perceptual {:.6} -> {:.6}, rollback={}, {:.1} ms", combinations, full_checks, score_cache_hits, skipped, moves, joint_moves, initial_score, if rolled_back { initial_score } else { verified }, rolled_back, started.elapsed().as_secs_f64() * 1000.0);
    log::info!(
        "Glyph search timing: local ranking {:.1} ms, full-image checks/restoration {:.1} ms",
        ranking_ms,
        checking_ms
    );
    if profiling {
        log::info!("Local ranking wall time: cache keys outside batches {:.1}ms, sequential patch setup {:.1}ms, sequential scoring/dispatch {:.1}ms, batched lookup/preparation/ranking {:.1}ms", key_ms, patch_ms, dispatch_ms, batch_ms);
        log::info!("Local ranking summed worker time (includes batched jobs and precision checks): patch setup {:.1}ms, copying {:.1}ms, painting/features {:.1}ms, perceptual score {:.1}ms", local_workers.iter().map(|w| w.patch_ms).sum::<f64>(), local_workers.iter().map(|w| w.copy_ms).sum::<f64>(), local_workers.iter().map(|w| w.features_ms).sum::<f64>(), local_workers.iter().map(|w| w.scoring_ms).sum::<f64>());
    }
    log::info!(
        "Repair Edges: {} raster-order precision checks for borderline score decisions",
        precision_checks
    );
    log::info!(
        "Parallel pair rankings: {} reused unchanged, {} recalculated after neighbouring commits",
        prepared_reused,
        prepared_stale
    );
    if adaptive_batches && largest_batch > 0 {
        log::info!("Adaptive ranking batches: {}..{} pairs based on observed invalidation", smallest_batch, largest_batch);
    }
    references.report();
    let cache_stats = rank_cache::stats();
    log::info!("Local ranking cache: {} hits, {} misses during this search",
        cache_stats.0 - initial_cache_stats.0, cache_stats.1 - initial_cache_stats.1);
    status.stage = GlyphSearchStage::Finished;
    status.score = if rolled_back { initial_score } else { verified };
    if super::search_status::wants_preview() {
        if let Some(map) = &initial_defects {
            status.preview = Some(search_preview(map.clone(), data, states, glyphs,
                status.score, args.foreground_only));
        }
    }
    status.publish(&started);
    Ok(initial_defects)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn cached_glyph_masks_paint_identical_pixels_at_different_font_metrics() {
        let args = crate::generator::default_options();
        for (gw, gh) in [(3, 7), (8, 16), (11, 29)] {
            let store = GlyphStore {
                glyphs: vec![('X', (0..gh).map(|y| (0..gw)
                    .map(|x| ((x * 3 + y) % 7 < 3) as u8).collect()).collect(), "test".into())],
                groups: HashMap::new(), selected: HashSet::from(['X']),
                metrics: (gw, gh), float_metrics: (gw as f32, gh as f32),
            };
            let glyphs = prepare_glyphs(&store, &args);
            let masks = [smooth_preview_glyph(&glyphs[0], gw, gh).mask];
            let data: Vec<_> = (0..4).map(|i| BlockData {
                fg: [220, 30 + i, 50], bg: [20, 30, 180], fg_col: None,
                bg_col: None, alt_fg_col: None, alt_bg_col: None,
            }).collect();
            let painter = PairPainter { a: 0, b: 1, grid_w: 2, width: 2 * CW,
                cell_x: 0, cell_y: 0, data: &data, masks: &masks, gw, gh };
            for cell in 0..4 {
                for inverted in [false, true] {
                    let mut cached = vec![[100; 3]; 4 * CW * CH];
                    let mut original = cached.clone();
                    painter.paint_cell(&mut cached, cell, (0, inverted));
                    paint_preview_glyph_at(&mut original, 2 * CW, cell % 2 * CW,
                        cell / 2 * CH, &data[cell], &glyphs[0], inverted, gw, gh);
                    assert_eq!(cached, original);
                }
            }
        }
    }

    #[test]
    fn foreground_only_search_excludes_inverted_candidates() {
        let args = crate::args_to_render_args(&crate::args::Args::parse_from(["test"]));
        let store = crate::font::glyph_store(
            &["half".into()], &[], &[], &[], None, &[], 16.0, false, None,
        );
        let glyphs = prepare_glyphs(&store, &args);
        let (gw, gh) = store.metrics;
        let source: Vec<_> = glyphs[0].bitmap.iter()
            .map(|&bit| if bit == 0 { [255; 3] } else { [0; 3] }).collect();
        let data = [BlockData {
            fg: [255; 3], bg: [0; 3], fg_col: None, bg_col: None,
            alt_fg_col: None, alt_bg_col: None,
        }];
        let states = [(0, false)];
        let native = build_rendered_raster(&data, &states, &glyphs, 1, 1, gw, gh);
        let (preview_source, _, _) = crate::contour_score::normalize_preview_raster(&source, 1, 1, gw, gh);
        let (preview_output, _, _) = crate::contour_score::normalize_preview_raster(&native, 1, 1, gw, gh);
        let masks: Vec<_> = glyphs.iter().map(|g| smooth_preview_glyph(g, gw, gh).mask).collect();
        for foreground_only in [false, true] {
            let pools = candidates(
                &source, &data, &states, &glyphs, &masks, &preview_source, &preview_output,
                &[true], 1, 1, gw, gh, 64, None, foreground_only, None,
            ).unwrap();
            assert!(pools[0].len() > 1);
            assert_eq!(pools[0].iter().any(|&(_, inverted)| inverted), !foreground_only);
        }
    }

    #[test]
    fn stalled_source_slots_rotate_while_preserving_the_incumbent_and_best_fit() {
        let args = crate::generator::default_options();
        let store = crate::font::glyph_store(
            &["large".into(), "space".into()], &[], &[], &[], None, &[], 16.0, false, None,
        );
        let glyphs = prepare_glyphs(&store, &args);
        let (gw, gh) = store.metrics;
        let source: Vec<_> = glyphs[0].bitmap.iter().map(|&bit| if bit == 0 { [0; 3] } else { [255; 3] }).collect();
        let data = [BlockData { fg: [255; 3], bg: [0; 3], fg_col: None, bg_col: None,
            alt_fg_col: None, alt_bg_col: None }];
        let states = [(0, false)];
        let (preview, _, _) = crate::contour_score::normalize_preview_raster(&source, 1, 1, gw, gh);
        let masks: Vec<_> = glyphs.iter().map(|g| smooth_preview_glyph(g, gw, gh).mask).collect();
        for foreground_only in [false, true] {
            let first = candidates(
                &source, &data, &states, &glyphs, &masks, &preview, &preview,
                &[true], 1, 1, gw, gh, 6, None, foreground_only, None,
            ).unwrap().remove(0);
            let attempted = vec![first.iter().map(|&(gi, inverted)| masks[gi] ^ if inverted { u128::MAX } else { 0 }).collect()];
            let second = candidates(
                &source, &data, &states, &glyphs, &masks, &preview, &preview,
                &[true], 1, 1, gw, gh, 6, Some(&attempted), foreground_only, None,
            ).unwrap().remove(0);
            let pools = [first, second];
            assert_eq!(&pools[0][..2], &pools[1][..2]);
            assert_ne!(pools[0][2], pools[1][2]);
            assert!(!attempted[0].contains(&(masks[pools[1][2].0] ^ if pools[1][2].1 { u128::MAX } else { 0 })));
            assert_eq!(pools[0].len(), pools[1].len());
            if foreground_only { assert!(pools.iter().flatten().all(|c| !c.1)); }
        }
    }

    #[test]
    fn batched_rankings_match_original_scores_and_refresh_after_neighbour_changes() {
        let mut session = rank_cache::Session::new(8 * 1024 * 1024);
        session.run(check_cached_rankings);
    }

    fn check_cached_rankings() {
        let args = crate::args_to_render_args(&crate::args::Args::parse_from(["test"]));
        let store = crate::font::glyph_store(
            &["half".into()],
            &[],
            &[],
            &[],
            None,
            &[],
            16.0,
            false,
            None,
        );
        let glyphs = prepare_glyphs(&store, &args);
        assert!(glyphs.len() > 1);
        let (gw, gh) = store.metrics;
        let (grid_w, grid_h) = (7, 6);
        let source: Vec<_> = (0..grid_w * CW * grid_h * CH)
            .map(|p| {
                [
                    ((p * 17) % 256) as u8,
                    ((p * 11) % 256) as u8,
                    ((p / 5 * 23) % 256) as u8,
                ]
            })
            .collect();
        let data: Vec<_> = (0..grid_w * grid_h)
            .map(|cell| BlockData {
                fg: [220, 30 + cell as u8, 50],
                bg: [20, 30, 180],
                fg_col: None,
                bg_col: None,
                alt_fg_col: None,
                alt_bg_col: None,
            })
            .collect();
        let mut states = vec![(0, false); data.len()];
        let mut output = vec![[0; 3]; source.len()];
        let masks: Vec<_> = glyphs.iter().map(|g| smooth_preview_glyph(g, gw, gh).mask).collect();
        let painter = PairPainter {
            a: 0,
            b: 0,
            grid_w,
            width: grid_w * CW,
            cell_x: 0,
            cell_y: 0,
            data: &data,
            masks: &masks,
            gw,
            gh,
        };
        for (cell, &state) in states.iter().enumerate() {
            painter.paint_cell(&mut output, cell, state);
        }
        let choices: Vec<_> = (0..glyphs.len().min(3))
            .flat_map(|g| [(g, false), (g, true)])
            .collect();
        let pools = vec![choices; states.len()];
        let groups = [
            (0, 1),
            (3 * grid_w + 3, 3 * grid_w + 4),
            (states.len() - 2, states.len() - 1),
        ];
        let mut versions = vec![0; states.len()];
        let references = source_cache::SourceReferences::new(&source, grid_w * CW);
        let context = RankContext {
            output: &output,
            data: &data,
            masks: &masks,
            pools: &pools,
            grid_w,
            grid_h,
            gw,
            gh,
            finalists: 3,
            abort: None,
            references: &references,
        };
        let mut workers = workspaces(4);
        let mut ready = context.prepare(&groups, &states, &versions, &mut workers).unwrap();
        let mut cache_workers = workspaces(4);
        let mut repeated = context.prepare(&groups, &states, &versions, &mut cache_workers).unwrap();
        assert!(cache_workers.iter().all(|worker| worker.output.is_empty()),
            "cache hits must skip scoring and workspace preparation");
        assert!(repeated.iter().all(|ready| ready.as_ref().unwrap().patch.is_none()));
        for (first, second) in ready.iter().zip(&repeated) {
            let trials = |group: &Option<PreparedGroup>| group.as_ref().unwrap().trials.iter()
                .map(|t| (t.a, t.b, t.local.to_bits())).collect::<Vec<_>>();
            assert_eq!(trials(first), trials(second));
        }
        for (&(a, b), ready) in groups.iter().zip(&ready) {
            let ready = ready.as_ref().unwrap();
            let patch = ready.patch.as_ref().unwrap();
            let painter = PairPainter {
                a,
                b,
                width: patch.width,
                cell_x: patch.cell_x,
                cell_y: patch.cell_y,
                ..painter
            };
            let mut original = pair_trials(a, b, states[a], states[b], &pools);
            score_trials(
                &mut original,
                &patch.reference,
                &patch.output,
                &patch.features,
                None,
                &painter,
                &mut workspaces(1),
                None,
                false,
            )
            .unwrap();
            original.sort_unstable_by(|a, b| {
                a.local
                    .total_cmp(&b.local)
                    .then_with(|| (a.a, a.b).cmp(&(b.a, b.b)))
            });
            assert_eq!(
                ready.trials.iter().map(|t| (t.a, t.b)).collect::<Vec<_>>(),
                original.iter().map(|t| (t.a, t.b)).collect::<Vec<_>>()
            );
            for (fast, exact) in ready.trials.iter().zip(original) {
                assert!((fast.local - exact.local).abs() < 1e-9);
            }
        }
        let cell = 3 * grid_w + 2;
        states[cell] = (1, true);
        versions[cell] += 1;
        painter.paint_cell(&mut output, cell, states[cell]);
        let cell = groups[1].0;
        states[cell] = (1, false);
        versions[cell] += 1;
        painter.paint_cell(&mut output, cell, states[cell]);
        let context = RankContext {
            output: &output,
            data: &data,
            masks: &masks,
            pools: &pools,
            grid_w,
            grid_h,
            gw,
            gh,
            finalists: 3,
            abort: None,
            references: &references,
        };
        let (a, b) = groups[1];
        let refreshed = context
            .refresh(
                a,
                b,
                &states,
                &versions,
                ready[1].as_mut().unwrap(),
                &mut workspaces(4),
            )
            .unwrap();
        let (fresh, _) = context
            .rank(a, b, &states, &mut TrialWorkspace::default())
            .unwrap();
        assert_eq!(
            refreshed.iter().map(|t| (t.a, t.b)).collect::<Vec<_>>(),
            fresh.iter().map(|t| (t.a, t.b)).collect::<Vec<_>>()
        );
        for (a, b) in refreshed.iter().zip(fresh) {
            assert!((a.local - b.local).abs() < 1e-9);
        }
        let cached_refresh = context.refresh(a, b, &states, &versions,
            repeated[1].as_mut().unwrap(), &mut cache_workers).unwrap();
        for (a, b) in cached_refresh.iter().zip(&refreshed) {
            assert_eq!((a.a, a.b), (b.a, b.b));
            assert!((a.local - b.local).abs() < 1e-9);
        }
        // A changed neighbourhood must miss instead of reusing the original
        // cached scores. The untouched corner still uses its cached ranking.
        let changed = context.prepare(&groups, &states, &versions, &mut workers).unwrap();
        assert!(changed[0].as_ref().unwrap().patch.is_none());
        assert!(changed[1].as_ref().unwrap().patch.is_some());
        let mut different_source = source.clone();
        different_source[0][0] ^= 1;
        let different_references = source_cache::SourceReferences::new(&different_source, grid_w * CW);
        let different_context = RankContext { references: &different_references, ..context };
        let changed = different_context.prepare(&groups[..1], &states, &versions, &mut workers).unwrap();
        assert!(changed[0].as_ref().unwrap().patch.is_some());
        let mut different_pools = pools.clone();
        different_pools[0].pop();
        let different_context = RankContext { pools: &different_pools, ..context };
        let changed = different_context.prepare(&groups[..1], &states, &versions, &mut workers).unwrap();
        assert!(changed[0].as_ref().unwrap().patch.is_some());
    }
}
