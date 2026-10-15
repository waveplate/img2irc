//! Target suspected defects, exhaust neighbouring glyph-pair shortlists, then
//! retain full-image perceptual improvements. Colours remain fixed throughout.

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

fn cancelled(abort: Option<&AtomicBool>) -> bool {
    abort.is_some_and(|flag| flag.load(Ordering::Relaxed))
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
    grid_w: usize,
    grid_h: usize,
) -> Arc<GlyphDefectMap> {
    let targets: Vec<_> = priorities
        .iter()
        .enumerate()
        .map(|(cell, &score)| {
            data[cell].fg != data[cell].bg && (threshold <= 0.0 || score >= threshold)
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
) -> Arc<GlyphSearchPreview> {
    let mut result = RenderResult::from("");
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
    abort: Option<&AtomicBool>,
) -> Result<Vec<Vec<Choice>>, ()> {
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
            let mut ranked = Vec::with_capacity(glyphs.len() * 2);
            for (gi, glyph) in glyphs.iter().enumerate() {
                if gi & 63 == 0 && cancelled(abort) {
                    return Err(());
                }
                let dot: i64 = glyph.ones.iter().map(|&i| diff[i]).sum();
                for (inverted, cost) in [(false, base + dot), (true, base + total - dot)] {
                    let mask = masks[gi] ^ if inverted { u128::MAX } else { 0 };
                    let context_cost = cost as f64
                        + seam_error(
                            cell,
                            mask,
                            data,
                            preview_source,
                            preview_output,
                            grid_w,
                            grid_h,
                        ) * (gw * gh) as f64;
                    ranked.push((cost, (gi, inverted), context_cost));
                }
            }
            ranked.sort_unstable_by_key(|&(cost, state, _)| (cost, state));
            let mut seen = HashSet::new();
            seen.insert(masks[current.0] ^ if current.1 { u128::MAX } else { 0 });
            let mut choices = vec![current];
            let source_limit = limit.div_ceil(2);
            for &(_, choice, _) in &ranked {
                let mask = masks[choice.0] ^ if choice.1 { u128::MAX } else { 0 };
                if seen.insert(mask) {
                    choices.push(choice);
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
    reference: PreparedReference,
    output: Vec<[u8; 3]>,
    features: OutputCache,
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
        source: &[[u8; 3]],
        output: &[[u8; 3]],
        a: usize,
        b: usize,
        grid_w: usize,
        grid_h: usize,
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
        let reference = PreparedReference::new(&extract(source), width, height, CW)
            .expect("valid glyph search patch");
        let output = extract(output);
        let features = reference.prepare_output(&output);
        Self {
            reference,
            output,
            features,
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
}

impl TrialWorkspace {
    fn reset(&mut self, output: &Vec<[u8; 3]>, features: &OutputCache) {
        self.output.clone_from(output);
        self.features.copy_from(features);
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
    glyphs: &'a [FastGlyph],
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
        paint_preview_glyph_at(
            output,
            self.width,
            (cell % self.grid_w - self.cell_x) * CW,
            (cell / self.grid_w - self.cell_y) * CH,
            &self.data[cell],
            &self.glyphs[choice.0],
            choice.1,
            self.gw,
            self.gh,
        );
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
            self.features.update(&self.output, rect);
            self.geometry.update(reference, &self.features, rect);
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
    painter: &PairPainter<'_>,
    workers: &mut [TrialWorkspace],
    abort: Option<&AtomicBool>,
) -> Result<(), ()> {
    if trials.is_empty() {
        return Ok(());
    }
    let chunk_size = trials.len().div_ceil(workers.len());
    let rect = painter.rect();
    trials
        .par_chunks_mut(chunk_size)
        .zip(workers.par_iter_mut())
        .try_for_each(|(trials, work)| {
            work.reset(output, features);
            for trial in trials {
                if cancelled(abort) {
                    return Err(());
                }
                painter.paint(&mut work.output, trial.a, trial.b);
                work.features.update(&work.output, rect);
                trial.local = reference
                    .score_prepared(&work.features, &mut work.score)
                    .total;
            }
            Ok(())
        })
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
    source: &[[u8; 3]],
    data: &[BlockData],
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
    let mut best_estimate = ScoreEstimate::exact(best_score);
    let mut verification_raster = Vec::new();
    let mut precision_checks = 0usize;
    let mut features = reference.prepare_output(&output);
    let alternatives = args.glyph_search_candidates.clamp(1, 64) as usize;
    let finalists = args.glyph_search_finalists.min(4096) as usize;
    let full_worker_count = full_workers(finalists, output.len(), reference.geometry_pixels());
    let mut geometry = (full_worker_count == 1).then(|| GeometryCache::new(&reference, &features));
    let mut full_workspaces: Vec<Option<FullTrialWorkspace>> =
        (0..full_worker_count).map(|_| None).collect();
    let mut commits = Vec::new();
    let workers = cpu_workers();
    let mut local_workers = workspaces(workers);
    let mut score_cache: HashMap<ScoreKey, ScoreEstimate> = HashMap::new();
    let mut score_cache_hits = 0usize;
    status.score = best_score;
    let initial_score = best_score;
    let initial_states = states.to_vec();
    let masks: Vec<_> = glyphs
        .iter()
        .map(|glyph| smooth_preview_glyph(glyph, gw, gh).mask)
        .collect();
    let mut combinations = 0usize;
    let mut full_checks = 0usize;
    let mut moves = 0usize;
    let mut joint_moves = 0usize;
    let mut ranking_ms = 0.0;
    let mut checking_ms = 0.0;
    let threshold = args.discontinuity_threshold.max(0.0);
    let focused = threshold > 0.0;
    let mut contexts = HashMap::new();
    let mut skipped = 0usize;
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
        let mut progress = RenderInstant::now();
        for (group_index, &(a, b)) in groups.iter().enumerate() {
            if cancelled(abort) {
                return Err(());
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
                glyphs,
                gw,
                gh,
            };
            let rect = painter.rect();
            let mut selected = (old_a, old_b);
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
            let patch =
                (finalists != 0).then(|| Patch::new(&source, &output, a, b, grid_w, grid_h));
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
                        &local_painter,
                        &mut local_workers,
                        abort,
                    )?;
                }
                combinations += batch.len();
                status.combinations = combinations;
                status.pair_work_done += batch.len();
                publish_if_due(&mut status, &started, &mut last_status);
            }
            trials.retain(|trial| trial.local.is_finite());
            if finalists != 0 {
                trials.sort_unstable_by(|a, b| {
                    a.local
                        .total_cmp(&b.local)
                        .then_with(|| (a.a, a.b).cmp(&(b.a, b.b)))
                });
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
                                    work.features.update(&work.output, rect);
                                    *score = reference.score_incremental_estimate(
                                        &work.features,
                                        &mut work.geometry,
                                        rect,
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
                        let score = reference.score_incremental_estimate(
                            &features,
                            geometry.as_mut().expect("serial scoring cache"),
                            rect,
                        );
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
            features.update(&output, rect);
            if let Some(geometry) = &mut geometry {
                geometry.update(&reference, &features, rect);
            }
            checking_ms += checking_started.elapsed().as_secs_f64() * 1000.0;
            states[a] = selected.0;
            if a != b {
                states[b] = selected.1;
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
            break;
        }
    }
    if cancelled(abort) {
        return Err(());
    }
    status.stage = GlyphSearchStage::Verifying;
    status.publish(&started);
    // Rebuild through the actual renderer before handing the result back. Keep
    // the starting state if normalization or painting ever disagrees with it.
    let native = build_rendered_raster(data, states, glyphs, grid_w, grid_h, gw, gh);
    let (verified_output, _, _) =
        crate::contour_score::normalize_preview_raster(&native, grid_w, grid_h, gw, gh);
    let verified = reference.score(&verified_output).total;
    let rolled_back = !verified.is_finite() || verified > initial_score;
    if rolled_back {
        states.copy_from_slice(&initial_states);
    }
    log::info!("Glyph search finished: {} combinations, {} full checks, {} full scores reused, {} unchanged pairs skipped, {} moves ({} joint), perceptual {:.6} -> {:.6}, rollback={}, {:.1} ms", combinations, full_checks, score_cache_hits, skipped, moves, joint_moves, initial_score, if rolled_back { initial_score } else { verified }, rolled_back, started.elapsed().as_secs_f64() * 1000.0);
    log::info!(
        "Glyph search timing: local ranking {:.1} ms, full-image checks/restoration {:.1} ms",
        ranking_ms,
        checking_ms
    );
    log::info!(
        "Repair Edges: {} raster-order precision checks for borderline score decisions",
        precision_checks
    );
    status.stage = GlyphSearchStage::Finished;
    status.score = if rolled_back { initial_score } else { verified };
    status.publish(&started);
    Ok(initial_defects)
}
