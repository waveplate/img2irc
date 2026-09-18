use clap::Parser;
use img2irc_rs::draw::FastGlyph;
use img2irc_rs::font::{self, GlyphStore};
use std::collections::VecDeque;
use std::f32::consts::TAU;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Parser, Debug)]
#[command(
    author,
    version = env!("IMG2IRC_VERSION"),
    about = "Generate contiguous smooth shapes using the polygon glyph block"
)]
struct DemoArgs {
    #[arg(long, default_value_t = 80)]
    width: usize,

    #[arg(long, default_value_t = 32)]
    height: usize,

    #[arg(long)]
    seed: Option<u64>,

    #[arg(long, default_value_t = 0.18)]
    density: f32,

    #[arg(long, default_value_t = 0.70)]
    stroke_width_cells: f32,

    #[arg(long, default_value_t = 3.5)]
    min_radius_cells: f32,

    #[arg(long, default_value_t = 12)]
    branches: usize,

    #[arg(long, default_value_t = 2)]
    branch_depth: usize,

    #[arg(long, default_value_t = 3)]
    optimize_passes: usize,

    #[arg(long, default_value_t = 10)]
    candidate_pool: usize,

    #[arg(long, default_value_t = false)]
    no_color: bool,

    #[arg(long)]
    output: Option<PathBuf>,

    #[arg(
        long,
        default_value = concat!(env!("CARGO_MANIFEST_DIR"), "/static/CascadiaCode-Regular.ttf")
    )]
    font: String,

    #[arg(long, default_value_t = 16.0)]
    font_size: f32,
}

#[derive(Clone)]
struct DemoGlyph {
    fast: FastGlyph,
    complexity: u16,
}

#[derive(Clone, Copy)]
struct Walker {
    x: f32,
    y: f32,
    heading: f32,
    steps_left: usize,
    depth: usize,
}

#[derive(Clone, Copy)]
struct CellCandidate {
    glyph_idx: usize,
    ref_cost: u16,
}

struct Rng64 {
    state: u64,
}

impl Rng64 {
    fn new(seed: u64) -> Self {
        let state = if seed == 0 {
            0xA076_1D64_78BD_642F
        } else {
            seed
        };
        Self { state }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn next_f32(&mut self) -> f32 {
        let v = (self.next_u64() >> 40) as u32;
        v as f32 / ((1u32 << 24) as f32)
    }

    fn range_f32(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next_f32()
    }

    fn chance(&mut self, p: f32) -> bool {
        self.next_f32() < p
    }
}

fn default_seed() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15)
}

fn wrap_angle(mut angle: f32) -> f32 {
    while angle <= -std::f32::consts::PI {
        angle += TAU;
    }
    while angle > std::f32::consts::PI {
        angle -= TAU;
    }
    angle
}

fn shortest_angle_delta(from: f32, to: f32) -> f32 {
    wrap_angle(to - from)
}

fn compute_exit(edge: &[u8], target: u8) -> f32 {
    let count = edge.iter().filter(|&&b| b == target).count();
    if count == 0 || count == edge.len() {
        return -1.0;
    }
    let sum: f32 = edge
        .iter()
        .enumerate()
        .filter(|(_, b)| **b == target)
        .map(|(i, _)| i as f32)
        .sum();
    sum / count as f32
}

fn prepare_fast_glyphs(store: &GlyphStore) -> Vec<DemoGlyph> {
    let (gw, gh) = store.metrics;
    let bp = gw * gh;

    store
        .glyphs
        .iter()
        .filter(|(ch, _, _)| store.selected.contains(ch))
        .map(|(ch, bmp, _)| {
            let mut bitmap = Vec::with_capacity(bp);
            for row in bmp {
                bitmap.extend_from_slice(row);
            }

            let top = bitmap[0..gw].to_vec();
            let bottom = bitmap[bp - gw..bp].to_vec();
            let mut left = Vec::with_capacity(gh);
            let mut right = Vec::with_capacity(gh);
            for y in 0..gh {
                left.push(bitmap[y * gw]);
                right.push(bitmap[y * gw + gw - 1]);
            }

            let mut ones = Vec::new();
            let mut zeros = Vec::new();
            for (i, &bit) in bitmap.iter().enumerate() {
                if bit == 1 {
                    ones.push(i);
                } else {
                    zeros.push(i);
                }
            }

            let mut vertical_cuts = Vec::with_capacity(gh.saturating_mul(gw.saturating_sub(1)));
            if gw > 1 {
                for y in 0..gh {
                    for x in 0..gw - 1 {
                        vertical_cuts.push((bitmap[y * gw + x] != bitmap[y * gw + x + 1]) as u8);
                    }
                }
            }

            let mut horizontal_cuts = Vec::with_capacity(gw.saturating_mul(gh.saturating_sub(1)));
            if gh > 1 {
                for y in 0..gh - 1 {
                    for x in 0..gw {
                        horizontal_cuts
                            .push((bitmap[y * gw + x] != bitmap[(y + 1) * gw + x]) as u8);
                    }
                }
            }

            let complexity = vertical_cuts
                .iter()
                .chain(horizontal_cuts.iter())
                .map(|&v| v as u16)
                .sum::<u16>();
            let coverage = ones.len() as f32 / bp as f32;
            let exit_top = compute_exit(&top, 1);
            let exit_bottom = compute_exit(&bottom, 1);
            let exit_left = compute_exit(&left, 1);
            let exit_right = compute_exit(&right, 1);
            let exit_top_inv = compute_exit(&top, 0);
            let exit_bottom_inv = compute_exit(&bottom, 0);
            let exit_left_inv = compute_exit(&left, 0);
            let exit_right_inv = compute_exit(&right, 0);

            DemoGlyph {
                complexity,
                fast: FastGlyph {
                    ch: *ch,
                    bitmap,
                    ones,
                    zeros,
                    vertical_cuts,
                    horizontal_cuts,
                    top,
                    bottom,
                    left,
                    right,
                    coverage,
                    exit_top,
                    exit_bottom,
                    exit_left,
                    exit_right,
                    exit_top_inv,
                    exit_bottom_inv,
                    exit_left_inv,
                    exit_right_inv,
                    mask: 0,
                },
            }
        })
        .collect()
}

fn clampf(value: f32, lo: f32, hi: f32) -> f32 {
    value.max(lo).min(hi)
}

fn stamp_disc(mask: &mut [u8], width: usize, height: usize, cx: f32, cy: f32, radius: f32) {
    let r2 = radius * radius;
    let min_x = clampf((cx - radius).floor(), 0.0, (width.saturating_sub(1)) as f32) as usize;
    let max_x = clampf((cx + radius).ceil(), 0.0, (width.saturating_sub(1)) as f32) as usize;
    let min_y = clampf(
        (cy - radius).floor(),
        0.0,
        (height.saturating_sub(1)) as f32,
    ) as usize;
    let max_y = clampf((cy + radius).ceil(), 0.0, (height.saturating_sub(1)) as f32) as usize;

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            if dx * dx + dy * dy <= r2 {
                mask[y * width + x] = 1;
            }
        }
    }
}

fn smooth_mask(mask: &mut [u8], width: usize, height: usize) {
    let original = mask.to_vec();
    for y in 0..height {
        for x in 0..width {
            let mut neighbors = 0u32;
            for ny in y.saturating_sub(1)..=(y + 1).min(height.saturating_sub(1)) {
                for nx in x.saturating_sub(1)..=(x + 1).min(width.saturating_sub(1)) {
                    neighbors += original[ny * width + nx] as u32;
                }
            }
            let idx = y * width + x;
            mask[idx] = if neighbors >= 5 || (neighbors >= 4 && original[idx] == 1) {
                1
            } else {
                0
            };
        }
    }
}

fn generate_target_mask(
    args: &DemoArgs,
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
    rng: &mut Rng64,
) -> Vec<u8> {
    let width = grid_w * gw;
    let height = grid_h * gh;
    let unit = gw.min(gh) as f32;
    let stroke_radius = (args.stroke_width_cells * unit * 0.5).max(1.5);
    let min_radius = (args.min_radius_cells * unit).max(stroke_radius * 2.0);
    let step_len = (stroke_radius * 0.7).max(1.0);
    let max_turn = (step_len / min_radius).clamp(0.02, 0.32);
    let target_ink = (args.density.clamp(0.02, 0.90) * (width * height) as f32) as usize;
    let area_per_step = (step_len * stroke_radius * 2.0).max(1.0);
    let root_steps = ((target_ink as f32 / area_per_step) * 1.5).ceil() as usize;
    let margin = stroke_radius + unit * 0.35;

    let mut mask = vec![0u8; width * height];
    let mut walkers = VecDeque::new();
    walkers.push_back(Walker {
        x: width as f32 * 0.5 + rng.range_f32(-unit, unit),
        y: height as f32 * 0.5 + rng.range_f32(-unit, unit),
        heading: rng.range_f32(0.0, TAU),
        steps_left: root_steps.max(1),
        depth: 0,
    });

    let mut branches_left = args.branches;
    while let Some(mut walker) = walkers.pop_front() {
        while walker.steps_left > 0 {
            stamp_disc(&mut mask, width, height, walker.x, walker.y, stroke_radius);

            if branches_left > 0
                && walker.depth < args.branch_depth
                && walker.steps_left > root_steps / 6
                && rng.chance(0.015 + 0.015 * args.density.clamp(0.0, 1.0))
            {
                let branch_heading = walker.heading + rng.range_f32(-1.15, 1.15);
                let branch_steps =
                    ((walker.steps_left as f32) * rng.range_f32(0.35, 0.75)) as usize;
                walkers.push_back(Walker {
                    x: walker.x,
                    y: walker.y,
                    heading: branch_heading,
                    steps_left: branch_steps.max(1),
                    depth: walker.depth + 1,
                });
                branches_left -= 1;
            }

            let center_x = width as f32 * 0.5;
            let center_y = height as f32 * 0.5;
            let mut inward_bias = 0.0;
            if walker.x < margin
                || walker.x > width as f32 - margin
                || walker.y < margin
                || walker.y > height as f32 - margin
            {
                let target = (center_y - walker.y).atan2(center_x - walker.x);
                inward_bias = shortest_angle_delta(walker.heading, target).clamp(-0.8, 0.8) * 0.45;
            }

            walker.heading =
                wrap_angle(walker.heading + rng.range_f32(-max_turn, max_turn) + inward_bias);

            let next_x = walker.x + walker.heading.cos() * step_len;
            let next_y = walker.y + walker.heading.sin() * step_len;
            if next_x < margin
                || next_x > width as f32 - margin
                || next_y < margin
                || next_y > height as f32 - margin
            {
                let target = (center_y - walker.y).atan2(center_x - walker.x);
                walker.heading = wrap_angle(
                    walker.heading + shortest_angle_delta(walker.heading, target) * 0.75,
                );
            }

            walker.x = clampf(
                walker.x + walker.heading.cos() * step_len,
                margin,
                width as f32 - margin,
            );
            walker.y = clampf(
                walker.y + walker.heading.sin() * step_len,
                margin,
                height as f32 - margin,
            );
            walker.steps_left -= 1;
        }
    }

    smooth_mask(&mut mask, width, height);
    smooth_mask(&mut mask, width, height);
    mask
}

fn extract_cell_masks(
    raster: &[u8],
    raster_w: usize,
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
) -> Vec<Vec<u8>> {
    let mut cells = Vec::with_capacity(grid_w * grid_h);
    for cell_y in 0..grid_h {
        for cell_x in 0..grid_w {
            let mut cell = Vec::with_capacity(gw * gh);
            for py in 0..gh {
                let src = (cell_y * gh + py) * raster_w + cell_x * gw;
                cell.extend_from_slice(&raster[src..src + gw]);
            }
            cells.push(cell);
        }
    }
    cells
}

fn hamming_cost(a: &[u8], b: &[u8]) -> u16 {
    a.iter()
        .zip(b.iter())
        .map(|(&lhs, &rhs)| (lhs != rhs) as u16)
        .sum()
}

fn best_pool(costs: &[u16], pool_size: usize) -> Vec<CellCandidate> {
    let keep = pool_size.max(1).min(costs.len());
    let mut indices: Vec<usize> = (0..costs.len()).collect();
    indices.sort_by_key(|&idx| costs[idx]);
    indices
        .into_iter()
        .take(keep)
        .map(|glyph_idx| CellCandidate {
            glyph_idx,
            ref_cost: costs[glyph_idx],
        })
        .collect()
}

fn edge_mismatch(a: &[u8], b: &[u8]) -> u16 {
    a.iter()
        .zip(b.iter())
        .map(|(&lhs, &rhs)| (lhs != rhs) as u16)
        .sum()
}

fn exit_penalty(a: f32, b: f32) -> u16 {
    match (a >= 0.0, b >= 0.0) {
        (false, false) => 0,
        (true, true) => (a - b).abs().round() as u16,
        _ => 4,
    }
}

fn candidate_score(
    cell_idx: usize,
    candidate_idx: usize,
    states: &[usize],
    ref_costs: &[Vec<u16>],
    glyphs: &[DemoGlyph],
    grid_w: usize,
    grid_h: usize,
) -> u32 {
    let x = cell_idx % grid_w;
    let y = cell_idx / grid_w;
    let candidate = &glyphs[candidate_idx].fast;
    let mut seam_penalty = 0u32;
    let mut flow_penalty = 0u32;

    if x > 0 {
        let left = &glyphs[states[cell_idx - 1]].fast;
        seam_penalty += edge_mismatch(&left.right, &candidate.left) as u32;
        flow_penalty += exit_penalty(left.exit_right, candidate.exit_left) as u32;
    }
    if x + 1 < grid_w {
        let right = &glyphs[states[cell_idx + 1]].fast;
        seam_penalty += edge_mismatch(&candidate.right, &right.left) as u32;
        flow_penalty += exit_penalty(candidate.exit_right, right.exit_left) as u32;
    }
    if y > 0 {
        let top = &glyphs[states[cell_idx - grid_w]].fast;
        seam_penalty += edge_mismatch(&top.bottom, &candidate.top) as u32;
        flow_penalty += exit_penalty(top.exit_bottom, candidate.exit_top) as u32;
    }
    if y + 1 < grid_h {
        let bottom = &glyphs[states[cell_idx + grid_w]].fast;
        seam_penalty += edge_mismatch(&candidate.bottom, &bottom.top) as u32;
        flow_penalty += exit_penalty(candidate.exit_bottom, bottom.exit_top) as u32;
    }

    ref_costs[cell_idx][candidate_idx] as u32 * 10
        + seam_penalty * 24
        + flow_penalty * 8
        + glyphs[candidate_idx].complexity as u32
}

fn optimize_assignments(
    candidate_pools: &[Vec<CellCandidate>],
    ref_costs: &[Vec<u16>],
    glyphs: &[DemoGlyph],
    grid_w: usize,
    grid_h: usize,
    passes: usize,
) -> Vec<usize> {
    let mut states: Vec<usize> = candidate_pools
        .iter()
        .map(|pool| pool[0].glyph_idx)
        .collect();

    for pass in 0..passes.max(1) {
        let forward = pass % 2 == 0;
        let iter: Box<dyn Iterator<Item = usize>> = if forward {
            Box::new(0..states.len())
        } else {
            Box::new((0..states.len()).rev())
        };

        let mut changed = 0usize;
        for idx in iter {
            let current = states[idx];
            let mut best = current;
            let mut best_score =
                candidate_score(idx, current, &states, ref_costs, glyphs, grid_w, grid_h);

            for candidate in &candidate_pools[idx] {
                if candidate.glyph_idx == current {
                    continue;
                }
                let score = candidate_score(
                    idx,
                    candidate.glyph_idx,
                    &states,
                    ref_costs,
                    glyphs,
                    grid_w,
                    grid_h,
                );
                if score < best_score
                    || (score == best_score && candidate.ref_cost < ref_costs[idx][best])
                {
                    best = candidate.glyph_idx;
                    best_score = score;
                }
            }

            if best != current {
                states[idx] = best;
                changed += 1;
            }
        }

        if changed == 0 {
            break;
        }
    }

    states
}

fn render_output(states: &[usize], glyphs: &[DemoGlyph], grid_w: usize, grid_h: usize) -> String {
    let mut out = String::with_capacity(grid_w * grid_h + grid_h);
    for y in 0..grid_h {
        for x in 0..grid_w {
            out.push(glyphs[states[y * grid_w + x]].fast.ch);
        }
        out.push('\n');
    }
    out
}

fn colorize_if_tty(text: &str, no_color: bool) -> String {
    if no_color || !atty::is(atty::Stream::Stdout) {
        text.to_string()
    } else {
        format!("\x1b[38;2;255;170;0m{}\x1b[0m", text)
    }
}

fn run() -> Result<(), String> {
    let args = DemoArgs::parse();
    if args.width == 0 || args.height == 0 {
        return Err("width and height must both be positive".into());
    }

    let seed = args.seed.unwrap_or_else(default_seed);
    let mut rng = Rng64::new(seed);
    let blocks = vec!["polygons".to_string(), "space".to_string()];
    let fonts = vec![args.font.clone()];
    let glyph_store = font::glyph_store(
        &blocks,
        &[],
        &[],
        &[],
        None,
        &fonts,
        args.font_size,
        false,
        None,
    );

    let (gw, gh) = glyph_store.metrics;
    if gw == 0 || gh == 0 {
        return Err("failed to resolve polygon glyph metrics".into());
    }

    let glyphs = prepare_fast_glyphs(&glyph_store);
    if glyphs.len() < 2 {
        return Err(
            "not enough polygon glyphs were loaded; try a font with Symbols for Legacy Computing"
                .into(),
        );
    }

    let target = generate_target_mask(&args, args.width, args.height, gw, gh, &mut rng);
    let cell_masks = extract_cell_masks(&target, args.width * gw, args.width, args.height, gw, gh);
    let ref_costs: Vec<Vec<u16>> = cell_masks
        .iter()
        .map(|mask| {
            glyphs
                .iter()
                .map(|glyph| hamming_cost(mask, &glyph.fast.bitmap))
                .collect()
        })
        .collect();
    let candidate_pools: Vec<Vec<CellCandidate>> = ref_costs
        .iter()
        .map(|costs| best_pool(costs, args.candidate_pool))
        .collect();
    let states = optimize_assignments(
        &candidate_pools,
        &ref_costs,
        &glyphs,
        args.width,
        args.height,
        args.optimize_passes,
    );

    let plain = render_output(&states, &glyphs, args.width, args.height);
    if let Some(path) = &args.output {
        fs::write(path, plain.as_bytes())
            .map_err(|err| format!("failed to write {}: {}", path.display(), err))?;
    }

    eprintln!(
        "polygon_demo seed={} font={} cell={}x{} glyphs={} density={:.3}",
        seed,
        args.font,
        gw,
        gh,
        glyphs.len(),
        args.density
    );
    print!("{}", colorize_if_tty(&plain, args.no_color));
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("polygon_demo: {}", err);
            ExitCode::FAILURE
        }
    }
}
