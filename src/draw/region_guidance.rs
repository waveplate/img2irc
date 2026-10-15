//! Connected, colour-aware regions guide the existing two-colour glyph fitter.
//! Region growth borrows SNIC's seed/priority-queue idea. Observed region colours
//! and glyphs are fitted together; mixed pixels never supply averaged colours.

use super::*;
use crate::contour_score::{PREVIEW_CELL_HEIGHT as CH, PREVIEW_CELL_WIDTH as CW};
use std::cmp::Ordering;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

const MAX_ANALYSIS_PIXELS: usize = 1_048_576;
const COLOUR_RADIUS: f32 = 0.08;

#[derive(Default)]
struct Region {
    count: usize,
    lab: [f64; 3],
    lab_sq: f64,
    x: f64,
    y: f64,
}

impl Region {
    fn add(&mut self, lab: Oklab, x: usize, y: usize) {
        self.count += 1;
        for (sum, value) in self.lab.iter_mut().zip([lab.l, lab.a, lab.b]) {
            *sum += value as f64;
            self.lab_sq += (value as f64).powi(2);
        }
        self.x += x as f64;
        self.y += y as f64;
    }

    fn mean_lab(&self) -> Oklab {
        let n = self.count.max(1) as f64;
        Oklab {
            l: (self.lab[0] / n) as f32,
            a: (self.lab[1] / n) as f32,
            b: (self.lab[2] / n) as f32,
        }
    }

    fn coherent(&self) -> bool {
        let n = self.count.max(1) as f64;
        let variance = self.lab_sq / n - self.lab.iter().map(|sum| (sum / n).powi(2)).sum::<f64>();
        variance <= 0.035_f64.powi(2)
    }

    fn distance(&self, lab: Oklab, x: usize, y: usize) -> f32 {
        let n = self.count.max(1) as f64;
        let dx = (x as f64 - self.x / n) / (2 * CW) as f64;
        let dy = (y as f64 - self.y / n) / (2 * CH) as f64;
        oklab_distance_squared(lab, self.mean_lab()) + 0.02_f32.powi(2) * (dx * dx + dy * dy) as f32
    }
}

#[derive(Clone, Copy)]
struct Visit {
    distance: f32,
    pixel: usize,
    region: usize,
}

impl PartialEq for Visit {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Visit {}
impl PartialOrd for Visit {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Visit {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .distance
            .total_cmp(&self.distance)
            .then_with(|| other.pixel.cmp(&self.pixel))
            .then_with(|| other.region.cmp(&self.region))
    }
}

fn cancelled(abort: Option<&AtomicBool>) -> bool {
    abort.is_some_and(|flag| flag.load(AtomicOrdering::Relaxed))
}

fn segment(
    pixels: &[[u8; 3]],
    width: usize,
    height: usize,
    abort: Option<&AtomicBool>,
) -> Option<(Vec<usize>, Vec<Oklab>, Vec<Region>)> {
    let labs: Vec<_> = pixels.iter().copied().map(rgb_to_oklab).collect();
    let mut labels = vec![usize::MAX; pixels.len()];
    let mut regions = Vec::new();
    let mut queue = BinaryHeap::new();
    // Reserve every seed before growing so a competing region cannot swallow it.
    for y in (CH.min(height - 1)..height).step_by(2 * CH) {
        for x in (CW.min(width - 1)..width).step_by(2 * CW) {
            let pixel = y * width + x;
            labels[pixel] = regions.len();
            let mut region = Region::default();
            region.add(labs[pixel], x, y);
            regions.push(region);
        }
    }
    let push_neighbors = |pixel: usize,
                          region: usize,
                          queue: &mut BinaryHeap<Visit>,
                          labels: &[usize],
                          regions: &[Region]| {
        let x = pixel % width;
        let y = pixel / width;
        let neighbors = [
            (x > 0).then(|| pixel - 1),
            (x + 1 < width).then(|| pixel + 1),
            (y > 0).then(|| pixel - width),
            (y + 1 < height).then(|| pixel + width),
        ];
        for next in neighbors.into_iter().flatten() {
            if labels[next] == usize::MAX {
                queue.push(Visit {
                    distance: regions[region].distance(labs[next], next % width, next / width),
                    pixel: next,
                    region,
                });
            }
        }
    };
    for (pixel, &region) in labels.iter().enumerate() {
        if region != usize::MAX {
            push_neighbors(pixel, region, &mut queue, &labels, &regions);
        }
    }
    let region_limit = regions.len().saturating_mul(4);
    let mut visits = 0;
    while let Some(visit) = queue.pop() {
        visits += 1;
        if visits & 1023 == 0 && cancelled(abort) {
            return None;
        }
        if labels[visit.pixel] != usize::MAX {
            continue;
        }
        // A thin, contrasting component may contain no regular seed. Give it
        // a seed instead of mixing it into the colour of the adjacent region.
        let region = if regions.len() < region_limit
            && oklab_distance_squared(labs[visit.pixel], regions[visit.region].mean_lab())
                > COLOUR_RADIUS.powi(2)
        {
            regions.push(Region::default());
            regions.len() - 1
        } else {
            visit.region
        };
        labels[visit.pixel] = region;
        regions[region].add(labs[visit.pixel], visit.pixel % width, visit.pixel / width);
        push_neighbors(visit.pixel, region, &mut queue, &labels, &regions);
    }
    Some((labels, labs, regions))
}

#[derive(Default)]
struct Population {
    count: u64,
    sum: [u64; 3],
    sq: u64,
}

impl Population {
    fn add(&mut self, rgb: [u8; 3]) {
        self.count += 1;
        for (sum, value) in self.sum.iter_mut().zip(rgb) {
            *sum += value as u64;
            self.sq += (value as u64).pow(2);
        }
    }
    fn error(&self, colour: [u8; 3]) -> u64 {
        let square: u64 = colour.iter().map(|&c| (c as u64).pow(2)).sum();
        let dot: u64 = self
            .sum
            .iter()
            .zip(colour)
            .map(|(&sum, c)| sum * c as u64)
            .sum();
        self.sq + self.count * square - 2 * dot
    }
}

fn rgb_error(a: [u8; 3], b: [u8; 3]) -> u64 {
    a.into_iter()
        .zip(b)
        .map(|(a, b)| (a as i64 - b as i64).pow(2) as u64)
        .sum()
}

fn quantize(rgb: [u8; 3], args: &RenderArgs) -> ([u8; 3], Colour) {
    if args.render == Render::Ansi24 {
        return (rgb, Colour::RGB(rgb));
    }
    let palette: &[u32] = if args.as_preview || args.render == Render::Irc {
        &IRC99
    } else {
        &ANSI256
    };
    let index = nearest_strict_partitioned_hex_colour(
        make_rgb_u32(&rgb),
        palette,
        args.grayscale_tolerance,
    );
    let rgb = unpack_rgb(palette[index as usize]);
    (
        rgb,
        if args.as_preview {
            Colour::RGB(rgb)
        } else {
            Colour::Index(index)
        },
    )
}

#[derive(Clone, Copy)]
pub(super) struct CellGuide {
    known: u128,
    foreground: u128,
    background: u128,
    boundary_limit: u32,
    source_limit: u64,
}

impl CellGuide {
    fn boundary_error(&self, mask: u128) -> u32 {
        // Unsupported colours count as misses, including a third region in a
        // two-colour cell. Equal fg/bg colours correctly cover either mask.
        self.known.count_ones()
            - ((self.foreground & mask) | (self.background & !mask)).count_ones()
    }
    pub(super) fn allows(&self, mask: u128, source_error: u64) -> bool {
        source_error <= self.source_limit && self.boundary_error(mask) <= self.boundary_limit
    }
}

pub(super) struct Guidance {
    pub(super) cells: Vec<Option<CellGuide>>,
    pub(super) masks: Vec<u128>,
}

/// Use a dominant observed colour rather than a mean contaminated by a border.
/// For palette output, count colours after the normal output quantization.
fn representatives(
    args: &RenderArgs,
    pixels: &[[u8; 3]],
    labels: &[usize],
    labs: &[Oklab],
    regions: &[Region],
    abort: Option<&AtomicBool>,
) -> Option<Vec<Option<[u8; 3]>>> {
    let mut counts = vec![HashMap::<u32, usize>::new(); regions.len()];
    let mut palette_cache = HashMap::new();
    for (pixel, &rgb) in pixels.iter().enumerate() {
        if pixel & 1023 == 0 && cancelled(abort) {
            return None;
        }
        let region = labels[pixel];
        if !regions[region].coherent()
            || oklab_distance_squared(labs[pixel], regions[region].mean_lab())
                > COLOUR_RADIUS.powi(2)
        {
            continue;
        }
        let colour = *palette_cache
            .entry(make_rgb_u32(&rgb))
            .or_insert_with(|| make_rgb_u32(&quantize(rgb, args).0));
        *counts[region].entry(colour).or_default() += 1;
    }
    let colours: Vec<_> = counts
        .into_iter()
        .zip(regions)
        .map(|(counts, region)| {
            let (colour, count) = counts
                .into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))?;
            (count >= 4 && count * 10 >= region.count * 7).then(|| unpack_rgb(colour))
        })
        .collect();
    Some(
        pixels
            .iter()
            .enumerate()
            .map(|(pixel, rgb)| {
                let region = labels[pixel];
                let colour = colours[region]?;
                // Mixed edge samples are not evidence for an interior region colour.
                let observed = palette_cache.get(&make_rgb_u32(rgb))?;
                (*observed == make_rgb_u32(&colour)
                    && oklab_distance_squared(labs[pixel], regions[region].mean_lab())
                        <= COLOUR_RADIUS.powi(2))
                .then_some(colour)
            })
            .collect(),
    )
}

fn guide_for_colours(
    targets: &[Option<[u8; 3]>; CW * CH],
    fg: [u8; 3],
    bg: [u8; 3],
    source_limit: u64,
) -> CellGuide {
    let mut guide = CellGuide {
        known: 0,
        foreground: 0,
        background: 0,
        boundary_limit: 0,
        source_limit,
    };
    for (pixel, &colour) in targets.iter().enumerate() {
        let Some(colour) = colour else {
            continue;
        };
        let bit = 1 << pixel;
        guide.known |= bit;
        if colour == fg {
            guide.foreground |= bit;
        }
        if colour == bg {
            guide.background |= bit;
        }
    }
    guide
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run(
    args: &RenderArgs,
    image: &AnsiImage,
    source: &[[u8; 3]],
    data: &mut [BlockData],
    states: &mut [(usize, bool)],
    glyphs: &[FastGlyph],
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
    abort: Option<&AtomicBool>,
) -> Option<Guidance> {
    let pixels = grid_w.checked_mul(grid_h)?.checked_mul(CW * CH)?;
    if pixels == 0 || glyphs.is_empty() {
        return None;
    }
    if pixels > MAX_ANALYSIS_PIXELS {
        log::warn!(
            "Region guidance skipped: normalized image has {} pixels (limit {})",
            pixels,
            MAX_ANALYSIS_PIXELS
        );
        return None;
    }
    let started = RenderInstant::now();
    let (preview, width, height) =
        crate::contour_score::normalize_preview_raster(source, grid_w, grid_h, gw, gh);
    let (labels, labs, regions) = segment(&preview, width, height, abort)?;
    let targets = representatives(args, &preview, &labels, &labs, &regions, abort)?;
    let masks: Vec<_> = glyphs
        .iter()
        .map(|glyph| smooth_preview_glyph(glyph, gw, gh).mask)
        .collect();
    let results: Vec<_> = data
        .par_iter_mut()
        .zip(states.par_iter_mut())
        .enumerate()
        .map(|(cell, (data, state))| {
            if cancelled(abort) {
                return (None, false, false);
            }
            let x = cell % grid_w;
            let y = cell / grid_w;
            let block = &image.block[y][x];
            let original_state = *state;
            let current_mask = masks[state.0] ^ if state.1 { u128::MAX } else { 0 };
            let cell_targets: [_; CW * CH] =
                std::array::from_fn(|i| targets[(y * CH + i / CW) * width + x * CW + i % CW]);
            if cell_targets.iter().flatten().count() < 8 {
                return (None, false, false);
            }
            let mut populations = [Population::default(), Population::default()];
            let mut total = Population::default();
            let mut native_pixels = Vec::with_capacity(gw * gh);
            for i in 0..gw * gh {
                let rgb = unpack_rgb(block.pixels[i / gw][i % gw].orig);
                let foreground = (glyphs[state.0].bitmap[i] != 0) ^ state.1;
                populations[foreground as usize].add(rgb);
                total.add(rgb);
                native_pixels.push(rgb);
            }
            let contrast = rgb_error(data.fg, data.bg);
            let initial_error = populations[1].error(data.fg) + populations[0].error(data.bg);
            let source_limit = initial_error + contrast * (gw * gh) as u64 / 64;
            let mut guide = guide_for_colours(&cell_targets, data.fg, data.bg, source_limit);
            let boundary_limit = guide.boundary_error(current_mask);
            guide.boundary_limit = boundary_limit;
            let boundary_weight = contrast as f64 * (gw * gh) as f64 / (CW * CH) as f64 * 0.5;
            let mut best = initial_error as f64 + boundary_weight * boundary_limit as f64;
            let mut best_colours = (data.fg, data.bg);

            let mut votes = HashMap::<[u8; 3], usize>::new();
            for &colour in cell_targets.iter().flatten() {
                *votes.entry(colour).or_default() += 1;
            }
            let mut observed: Vec<_> = votes.into_iter().filter(|(_, count)| *count >= 4).collect();
            observed.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            let mut colours = vec![data.fg];
            if data.bg != data.fg {
                colours.push(data.bg);
            }
            for (colour, _) in observed.into_iter().take(3) {
                if !colours.contains(&colour) {
                    colours.push(colour);
                }
            }
            // Jointly search colours AND geometry. Statistics make each new
            // pair cheap; no mean or blended shade is added to the palette.
            let mut ones_statistics = Vec::with_capacity(glyphs.len());
            for (gi, glyph) in glyphs.iter().enumerate() {
                if gi & 63 == 0 && cancelled(abort) {
                    return (None, false, false);
                }
                let mut population = Population::default();
                for &pixel in &glyph.ones {
                    population.add(native_pixels[pixel]);
                }
                ones_statistics.push(population);
            }
            for (fg_index, &fg) in colours.iter().enumerate() {
                for &bg in &colours[fg_index..] {
                    let mut candidate_guide =
                        guide_for_colours(&cell_targets, fg, bg, source_limit);
                    candidate_guide.boundary_limit = boundary_limit;
                    let base_fg = total.error(fg);
                    let base_bg = total.error(bg);
                    for (gi, ones) in ones_statistics.iter().enumerate() {
                        if gi & 63 == 0 && cancelled(abort) {
                            return (None, false, false);
                        }
                        let ones_fg = ones.error(fg);
                        let ones_bg = ones.error(bg);
                        for (inverted, error) in [
                            (false, base_bg + ones_fg - ones_bg),
                            (true, base_fg + ones_bg - ones_fg),
                        ] {
                            let mask = masks[gi] ^ if inverted { u128::MAX } else { 0 };
                            if !candidate_guide.allows(mask, error) {
                                continue;
                            }
                            let objective = error as f64
                                + boundary_weight * candidate_guide.boundary_error(mask) as f64;
                            if objective < best {
                                best = objective;
                                *state = (gi, inverted);
                                best_colours = (fg, bg);
                                guide = candidate_guide;
                            }
                        }
                    }
                }
            }
            let colour_changed = best_colours != (data.fg, data.bg);
            // Preserve the existing colour objects when the pair did not change.
            if colour_changed {
                let (fg, fg_col) = quantize(best_colours.0, args);
                let (bg, bg_col) = quantize(best_colours.1, args);
                data.fg = fg;
                data.bg = bg;
                data.fg_col = Some(fg_col.clone());
                data.bg_col = Some(bg_col.clone());
                data.alt_fg_col = args.as_preview.then_some(fg_col);
                data.alt_bg_col = args.as_preview.then_some(bg_col);
            }
            guide.boundary_limit =
                guide.boundary_error(masks[state.0] ^ if state.1 { u128::MAX } else { 0 });
            (Some(guide), colour_changed, *state != original_state)
        })
        .collect();
    if cancelled(abort) {
        return None;
    }
    log::info!(
        "Region guidance: {} connected regions, {} colour changes, {} shape changes, {:.1} ms",
        regions.len(),
        results.iter().filter(|r| r.1).count(),
        results.iter().filter(|r| r.2).count(),
        started.elapsed().as_secs_f64() * 1000.0
    );
    Some(Guidance {
        cells: results.into_iter().map(|r| r.0).collect(),
        masks,
    })
}
