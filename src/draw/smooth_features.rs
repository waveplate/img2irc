//! Experimental quadratic smoothing on features extracted from glyph rasters.
//!
//! For a rendered image I, minimize source squared error + 8 * ||L B I||²,
//! where B is a binomial blur with a small unblurred component and L is the
//! five-point Laplacian. Both terms
//! are averaged over their respective cell pixel counts. Linearity lets us
//! extract L B for every glyph once, then evaluate changes by dotting a small
//! glyph response against the current filtered image. No contour tracing,
//! clustering, patch allocation, or raster repaint is needed per candidate.
//! This is a curvature surrogate, not the legacy contour objective: thin
//! details can lose to smoother shapes, so source error has a fixed budget.

use super::*;
use crate::contour_score::{PREVIEW_CELL_HEIGHT as H, PREVIEW_CELL_WIDTH as W};
use std::sync::atomic::{AtomicBool, Ordering};

const R: usize = 2;
const FW: usize = W + 2 * R;
const FH: usize = H + 2 * R;
const N: usize = FW * FH;
const SWEEPS: usize = 4;
// Laplacian(16 * binomial blur + identity), normalized by 17. The identity
// term prevents alternating/checkerboard glyphs from disappearing in the
// blur's nullspace. Integer responses make incremental updates exact,
// including cancellation of contributions along shared seams.
const KERNEL: [[i16; 5]; 5] = [
    [0, 1, 2, 1, 0],
    [1, 0, -1, 0, 1],
    [2, -1, -12, -1, 2],
    [1, 0, -1, 0, 1],
    [0, 1, 2, 1, 0],
];
const FEATURE_SCALE: f64 = 8.0 / (W * H * 17 * 17) as f64;

type Response = [i16; N];

fn response(mask: u128) -> Response {
    let mut result = [0; N];
    for y in 0..H {
        for x in 0..W {
            if mask & (1 << (y * W + x)) == 0 {
                continue;
            }
            for (ky, row) in KERNEL.iter().enumerate() {
                for (kx, &weight) in row.iter().enumerate() {
                    result[(y + ky) * FW + x + kx] += weight;
                }
            }
        }
    }
    result
}

#[derive(Clone, Copy)]
struct Candidate {
    choice: (usize, bool),
    feature: usize,
    cost: u64,
}

// (index in the image feature field, index in the glyph response, in a neighbor)
type Sample = (usize, usize, bool);

fn samples(idx: usize, grid_w: usize, grid_h: usize) -> Vec<Sample> {
    let width = grid_w * W;
    let height = grid_h * H;
    let x0 = (idx % grid_w) * W;
    let y0 = (idx / grid_w) * H;
    let mut result = Vec::with_capacity(N);
    for fy in 0..FH {
        let y = y0 as isize + fy as isize - R as isize;
        // Only score complete filter stencils. Zero padding would invent a
        // black frame and encourage erasing glyphs along the image boundary.
        if y < R as isize || y >= (height - R) as isize {
            continue;
        }
        for fx in 0..FW {
            let x = x0 as isize + fx as isize - R as isize;
            if x < R as isize || x >= (width - R) as isize {
                continue;
            }
            let neighbor = x < x0 as isize
                || x >= (x0 + W) as isize
                || y < y0 as isize
                || y >= (y0 + H) as isize;
            result.push((y as usize * width + x as usize, fy * FW + fx, neighbor));
        }
    }
    result
}

fn colors(data: &BlockData) -> ([i32; 3], [i32; 3], i64) {
    let bg = data.bg.map(i32::from);
    let diff = std::array::from_fn(|c| data.fg[c] as i32 - bg[c]);
    let contrast = diff.iter().map(|&d| i64::from(d).pow(2)).sum();
    (bg, diff, contrast)
}

fn delta(
    current: &Response,
    candidate: &Response,
    projected: &[(usize, i64, bool)],
    contrast: i64,
) -> (i64, i64) {
    let mut total = 0;
    let mut neighbors = 0;
    for &(f, dot, neighbor) in projected {
        let d = i64::from(candidate[f] - current[f]);
        let change = d * (2 * dot + d * contrast);
        total += change;
        if neighbor {
            neighbors += change;
        }
    }
    (total, neighbors)
}

struct Trial {
    state: Vec<(usize, bool)>,
    energy: f64,
    changes: usize,
    evaluations: usize,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run(
    args: &RenderArgs,
    grid_state: &mut [(usize, bool)],
    grid_data: &[BlockData],
    glyphs: &[FastGlyph],
    costs: &[Vec<u64>],
    target_scores: &[f32],
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
    abort_flag: Option<&AtomicBool>,
) -> Result<(), ()> {
    let cancelled = || abort_flag.is_some_and(|flag| flag.load(Ordering::Relaxed));
    let mut classes = HashMap::new();
    let mut features = Vec::new();
    let mut feature_ids = Vec::with_capacity(glyphs.len() * 2);
    for glyph in glyphs {
        if cancelled() {
            return Err(());
        }
        let mask = score_fix_preview_glyph(glyph, gw, gh).mask;
        for mask in [mask, !mask] {
            let id = *classes.entry(mask).or_insert_with(|| {
                let id = features.len();
                features.push(response(mask));
                id
            });
            feature_ids.push(id);
        }
    }
    let candidate = |choice: (usize, bool), cost| Candidate {
        choice,
        feature: feature_ids[choice.0 * 2 + choice.1 as usize],
        cost,
    };
    let baseline: Vec<_> = grid_state
        .iter()
        .enumerate()
        .map(|(idx, &choice)| {
            candidate(
                choice,
                costs[idx]
                    .get(choice.0 * 2 + choice.1 as usize)
                    .copied()
                    .unwrap_or(0),
            )
        })
        .collect();
    let mut lists = vec![Vec::new(); grid_state.len()];
    let mut targets = Vec::new();
    let mut seen = vec![false; features.len()];
    let pixel_count = (gw * gh) as f64;
    for idx in 0..grid_state.len() {
        if cancelled() {
            return Err(());
        }
        if costs[idx].is_empty()
            || target_scores[idx] < args.discontinuity_threshold.max(0.0)
            || grid_data[idx].fg == grid_data[idx].bg
        {
            continue;
        }
        // Anchor to the INITIAL fit, so successive sweeps cannot gradually
        // spend the budget again. At most 1/16 of a full cell's fg/bg contrast
        // energy may be added to source error, independent of font size.
        let allowance = colors(&grid_data[idx]).2 as f64 * pixel_count / 16.0;
        let max_cost = baseline[idx].cost as f64 + allowance;
        seen.fill(false);
        lists[idx].push(baseline[idx]);
        seen[baseline[idx].feature] = true;
        for (g, inverted, cost) in best_candidate_pool_pre(&costs[idx], costs[idx].len()) {
            if cost as f64 > max_cost {
                break;
            }
            let next = candidate((g, inverted), cost);
            // The baseline stays available, including a cheaper source fit
            // with identical preview geometry at a different font resolution.
            if next.feature == baseline[idx].feature && cost < lists[idx][0].cost {
                lists[idx][0] = next;
            }
            if seen[next.feature] {
                continue;
            }
            seen[next.feature] = true;
            lists[idx].push(next);
            if lists[idx].len() > args.score_fix_candidates.max(1) as usize {
                break;
            }
        }
        if lists[idx].len() > 1 || lists[idx][0].cost < baseline[idx].cost {
            targets.push(idx);
        }
    }
    if targets.is_empty() {
        return Ok(());
    }

    let solid = response(u128::MAX);
    let mut initial_field = vec![[0i32; 3]; grid_w * W * grid_h * H];
    for idx in 0..grid_state.len() {
        if cancelled() {
            return Err(());
        }
        let (bg, diff, _) = colors(&grid_data[idx]);
        let shape = &features[baseline[idx].feature];
        for (p, f, _) in samples(idx, grid_w, grid_h) {
            for c in 0..3 {
                initial_field[p][c] += bg[c] * i32::from(solid[f]) + diff[c] * i32::from(shape[f]);
            }
        }
    }
    let initial_energy = baseline
        .iter()
        .map(|b| b.cost as f64 / pixel_count)
        .sum::<f64>()
        + FEATURE_SCALE
            * initial_field
                .iter()
                .flatten()
                .map(|&v| (v as f64).powi(2))
                .sum::<f64>();
    let spiral = score_fix_spiral_ranks(grid_w, grid_h);
    let orderings = &ScoreFixOrdering::ALL[..(args.score_fix_orderings as usize).clamp(1, 5)];
    let trials: Vec<Result<Trial, ()>> = orderings
        .par_iter()
        .map(|&ordering| {
            let mut field = initial_field.clone();
            let mut state = baseline.clone();
            let mut order = targets.clone();
            let mut energy = initial_energy;
            let mut changes = 0;
            let mut evaluations = 0;
            for sweep in 0..SWEEPS {
                order.sort_unstable_by(|&a, &b| {
                    score_fix_order_priority(ordering, &spiral, b, sweep, state.len())
                        .cmp(&score_fix_order_priority(
                            ordering,
                            &spiral,
                            a,
                            sweep,
                            state.len(),
                        ))
                        .then_with(|| target_scores[b].total_cmp(&target_scores[a]))
                        .then_with(|| a.cmp(&b))
                });
                if matches!(ordering, ScoreFixOrdering::WorstFirst) && sweep % 2 == 1 {
                    order.reverse();
                }
                let mut changed = false;
                for &idx in &order {
                    if cancelled() {
                        return Err(());
                    }
                    let (_, diff, contrast) = colors(&grid_data[idx]);
                    let points = samples(idx, grid_w, grid_h);
                    let projected: Vec<_> = points
                        .iter()
                        .map(|&(p, f, neighbor)| {
                            let dot = (0..3)
                                .map(|c| i64::from(diff[c]) * i64::from(field[p][c]))
                                .sum();
                            (f, dot, neighbor)
                        })
                        .collect();
                    let current = state[idx];
                    let mut best = current;
                    let mut best_delta = 0.0;
                    for &next in &lists[idx] {
                        if next.choice == current.choice {
                            continue;
                        }
                        evaluations += 1;
                        let (feature_delta, neighbor_delta) = delta(
                            &features[current.feature],
                            &features[next.feature],
                            &projected,
                            contrast,
                        );
                        if args.score_fix_neighborhood_guard && neighbor_delta > 0 {
                            continue;
                        }
                        let change = (next.cost as f64 - current.cost as f64) / pixel_count
                            + FEATURE_SCALE * feature_delta as f64;
                        if change < best_delta - 1e-9 {
                            best = next;
                            best_delta = change;
                        }
                    }
                    if best.choice == current.choice {
                        continue;
                    }
                    for (p, f, _) in points {
                        let d = i32::from(features[best.feature][f] - features[current.feature][f]);
                        for c in 0..3 {
                            field[p][c] += diff[c] * d;
                        }
                    }
                    state[idx] = best;
                    energy += best_delta;
                    changes += 1;
                    changed = true;
                }
                if !changed {
                    break;
                }
            }
            Ok(Trial {
                state: state.iter().map(|c| c.choice).collect(),
                energy,
                changes,
                evaluations,
            })
        })
        .collect();
    let mut best: Option<Trial> = None;
    for trial in trials {
        let trial = trial?;
        if best
            .as_ref()
            .is_none_or(|best| trial.energy < best.energy - 1e-9)
        {
            best = Some(trial);
        }
    }
    if let Some(best) = best {
        grid_state.copy_from_slice(&best.state);
        log::info!(
            "feature smoothing: {} glyph features, {} targets, {} orders; selected trial made {} changes from {} evaluations; energy {:.6} -> {:.6}",
            features.len(), targets.len(), orderings.len(), best.changes, best.evaluations,
            initial_energy / (3.0 * 255.0 * 255.0), best.energy / (3.0 * 255.0 * 255.0),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filtered(pixels: &[[i32; 3]], width: usize, height: usize) -> Vec<[i32; 3]> {
        let mut out = vec![[0; 3]; pixels.len()];
        for y in R..height - R {
            for x in R..width - R {
                for (ky, row) in KERNEL.iter().enumerate() {
                    for (kx, &weight) in row.iter().enumerate() {
                        for c in 0..3 {
                            out[y * width + x][c] +=
                                i32::from(weight) * pixels[(y + ky - R) * width + x + kx - R][c];
                        }
                    }
                }
            }
        }
        out
    }

    fn energy(field: &[[i32; 3]]) -> i64 {
        field.iter().flatten().map(|&v| i64::from(v).pow(2)).sum()
    }

    #[test]
    fn features_and_candidate_deltas_match_full_filter_at_edges_and_corners() {
        let (grid_w, grid_h) = (3, 3);
        let (width, height) = (grid_w * W, grid_h * H);
        let masks: Vec<_> = (0..9)
            .map(|i| {
                // Different, irregular glyphs exercise multiple boundaries and inversions.
                0x0f1e3c78f0e1c3870f1e3c78f0e1c387u128.rotate_left(i * 7)
            })
            .collect();
        let solid = response(u128::MAX);
        let mut pixels = vec![[0; 3]; width * height];
        let mut field = vec![[0; 3]; width * height];
        for idx in 0..9 {
            let bg = [20 + idx as i32 * 5, 70, 200];
            let diff = [100, -40, -130];
            for y in 0..H {
                for x in 0..W {
                    let bit = ((masks[idx] >> (y * W + x)) & 1) as i32;
                    let p = (idx / grid_w * H + y) * width + idx % grid_w * W + x;
                    pixels[p] = std::array::from_fn(|c| bg[c] + diff[c] * bit);
                }
            }
            let shape = response(masks[idx]);
            for (p, f, _) in samples(idx, grid_w, grid_h) {
                for c in 0..3 {
                    field[p][c] += bg[c] * i32::from(solid[f]) + diff[c] * i32::from(shape[f]);
                }
            }
        }
        assert_eq!(field, filtered(&pixels, width, height));
        for idx in 0..9 {
            let mut changed = pixels.clone();
            let diff = [100, -40, -130];
            for y in 0..H {
                for x in 0..W {
                    let bit = ((masks[idx] >> (y * W + x)) & 1) as i32;
                    let p = (idx / grid_w * H + y) * width + idx % grid_w * W + x;
                    for c in 0..3 {
                        changed[p][c] += diff[c] * (1 - 2 * bit);
                    }
                }
            }
            let projected: Vec<_> = samples(idx, grid_w, grid_h)
                .into_iter()
                .map(|(p, f, n)| {
                    (
                        f,
                        (0..3)
                            .map(|c| i64::from(diff[c]) * i64::from(field[p][c]))
                            .sum(),
                        n,
                    )
                })
                .collect();
            let contrast = diff.iter().map(|&v| i64::from(v).pow(2)).sum();
            let (change, _) = delta(
                &response(masks[idx]),
                &response(!masks[idx]),
                &projected,
                contrast,
            );
            assert_eq!(
                change,
                energy(&filtered(&changed, width, height)) - energy(&field)
            );
        }
    }

    #[test]
    fn uniform_image_has_no_artificial_outer_boundary() {
        for (w, h) in [(1, 1), (1, 3), (3, 1)] {
            let mut field = vec![[0; 3]; w * W * h * H];
            let solid = response(u128::MAX);
            for idx in 0..w * h {
                for (p, f, _) in samples(idx, w, h) {
                    for c in 0..3 {
                        field[p][c] += 255 * i32::from(solid[f]);
                    }
                }
            }
            assert_eq!(energy(&field), 0);
        }
    }

    #[test]
    fn continuous_diagonal_beats_zigzag_and_checkerboards_remain_visible() {
        let (width, height) = (32, 32);
        let make = |zigzag: bool, checker: bool| {
            (0..width * height)
                .map(|p| {
                    let (x, y) = (p % width, p / width);
                    let on = if checker {
                        (x + y) % 2 == 0
                    } else {
                        x < y / 2 + if zigzag { (y / 4 % 2) * 3 } else { 0 }
                    };
                    [if on { 255 } else { 0 }; 3]
                })
                .collect::<Vec<_>>()
        };
        let straight = energy(&filtered(&make(false, false), width, height));
        let zigzag = energy(&filtered(&make(true, false), width, height));
        assert!(straight < zigzag);
        assert!(energy(&filtered(&make(false, true), width, height)) > 0);
    }
}
