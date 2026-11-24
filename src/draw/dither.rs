use super::{
    encode_grid_with_overlays, linear_rgb_to_oklab, make_rgb_u32, srgb_to_linear, unpack_rgb,
    AnsiImage, BlockData, GeometryGlyphs, Oklab, RenderInstant, RenderResult, RenderedCell,
};
use crate::args::{Render, RenderArgs};
use crate::font::GlyphStore;
use crate::palette::{ANSI256, IRC99};
use once_cell::sync::Lazy;
use photon_rs::PhotonImage;
use rayon::prelude::*;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

type Lab = [f32; 3];

const LEVELS: u8 = 16;
const PROCEDURAL_BITMAP_SIDE: usize = 128;
type Rgb = [f32; 3];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Pair {
    fg: u8,
    bg: u8,
}

impl Pair {
    fn swapped(self) -> Self {
        Self {
            fg: self.bg,
            bg: self.fg,
        }
    }
}

fn lab(Oklab { l, a, b }: Oklab) -> Lab {
    [l, a, b]
}

static LINEAR: Lazy<[f32; 256]> =
    Lazy::new(|| std::array::from_fn(|i| srgb_to_linear(i as f32 / 255.0)));

fn linear(rgb: u32) -> Rgb {
    unpack_rgb(rgb).map(|channel| LINEAR[channel as usize])
}

fn mix(fg: Rgb, bg: Rgb, coverage: u8) -> Rgb {
    let alpha = f32::from(coverage) / f32::from(LEVELS);
    std::array::from_fn(|i| alpha * fg[i] + (1.0 - alpha) * bg[i])
}

fn distance(a: Lab, b: Lab) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

#[derive(Clone, Copy)]
struct Entry {
    point: Lab,
    pair: Pair,
}

fn build_tree(entries: &mut [Entry], axis: usize) {
    if entries.len() > 1 {
        let mid = entries.len() / 2;
        let (left, _, right) =
            entries.select_nth_unstable_by(mid, |a, b| a.point[axis].total_cmp(&b.point[axis]));
        build_tree(left, (axis + 1) % 3);
        build_tree(right, (axis + 1) % 3);
    }
}

fn nearest(tree: &[Entry], target: Lab, axis: usize, limit: usize, best: &mut Vec<(f32, Pair)>) {
    if tree.is_empty() {
        return;
    }
    let (left, rest) = tree.split_at(tree.len() / 2);
    let (node, right) = rest.split_first().unwrap();
    let error = distance(target, node.point);
    let position = best.partition_point(|&(score, _)| score <= error);
    if position < limit {
        if best.len() == limit {
            best.pop();
        }
        best.insert(position, (error, node.pair));
    }
    let delta = target[axis] - node.point[axis];
    let (near, far) = if delta < 0.0 {
        (left, right)
    } else {
        (right, left)
    };
    let next_axis = (axis + 1) % 3;
    nearest(near, target, next_axis, limit, best);
    if best.len() < limit || delta * delta < best[limit - 1].0 {
        nearest(far, target, next_axis, limit, best);
    }
}

struct Lookup {
    palette: &'static [u32],
    colours: Vec<Rgb>,
    labs: Vec<Lab>,
    indices: Vec<u8>,
    mixtures: Vec<[Lab; LEVELS as usize + 1]>,
    uniform: [once_cell::sync::OnceCell<Vec<Entry>>; LEVELS as usize / 2 + 1],
}

impl Lookup {
    fn new(palette: &'static [u32], first_index: usize) -> Self {
        let colours: Vec<_> = palette.iter().copied().map(linear).collect();
        let labs: Vec<_> = colours
            .iter()
            .map(|&rgb| lab(linear_rgb_to_oklab(rgb)))
            .collect();
        let mut seen = HashSet::new();
        let indices = (first_index..palette.len())
            .filter(|&i| seen.insert(palette[i]))
            .map(|i| i as u8)
            .collect();
        let mixtures = colours
            .iter()
            .flat_map(|&fg| {
                colours.iter().map(move |&bg| {
                    std::array::from_fn(|a| lab(linear_rgb_to_oklab(mix(fg, bg, a as u8))))
                })
            })
            .collect();
        Self {
            palette,
            colours,
            labs,
            indices,
            mixtures,
            uniform: Default::default(),
        }
    }

    fn mixed(&self, pair: Pair) -> &[Lab; LEVELS as usize + 1] {
        &self.mixtures[pair.fg as usize * self.palette.len() + pair.bg as usize]
    }

    fn tree(&self, coverage: u8) -> &[Entry] {
        self.uniform[coverage as usize].get_or_init(|| {
            let mut entries = Vec::new();
            for &fg in &self.indices {
                let solid = Pair { fg, bg: fg };
                entries.push(Entry {
                    point: self.labs[fg as usize],
                    pair: solid,
                });
                if coverage == 0 {
                    continue;
                }
                for &bg in &self.indices {
                    if fg == bg || coverage == LEVELS / 2 && fg > bg {
                        continue;
                    }
                    let pair = Pair { fg, bg };
                    entries.push(Entry {
                        point: self.mixed(pair)[coverage as usize],
                        pair,
                    });
                }
            }
            build_tree(&mut entries, 0);
            entries
        })
    }

    fn uniform_pair(&self, coverage: u8, target: Lab, scratch: &mut Vec<(f32, Pair)>) -> Pair {
        scratch.clear();
        nearest(
            self.tree(coverage.min(LEVELS - coverage)),
            target,
            0,
            1,
            scratch,
        );
        let pair = scratch[0].1;
        if coverage > LEVELS / 2 {
            pair.swapped()
        } else {
            pair
        }
    }
}

// Match the stable IRC colours used by the encoder, including its theme-independent black and white.
static IRC_LOOKUP: Lazy<Lookup> = Lazy::new(|| Lookup::new(&IRC99, 16));
static ANSI_LOOKUP: Lazy<Lookup> = Lazy::new(|| Lookup::new(&ANSI256, 0));

struct CoverageGlyph {
    ch: char,
    coverage: Vec<u8>,
    starts_with_ink: bool,
    mean: f32,
    variance: f32,
    uniform: Option<u8>,
}

impl CoverageGlyph {
    fn from_bitmap(ch: char, bitmap: &[Vec<u8>], side: usize) -> Self {
        let height = bitmap.len();
        let width = bitmap[0].len();
        let mut coverage = Vec::with_capacity(side * side);
        for y in 0..side {
            for x in 0..side {
                // Integrate every overlapping source pixel in units of 1/side.
                let (left, right) = (x * width, (x + 1) * width);
                let (top, bottom) = (y * height, (y + 1) * height);
                let mut ink_area = 0;
                for sy in top / side..bottom.div_ceil(side) {
                    let dy = bottom.min((sy + 1) * side) - top.max(sy * side);
                    for sx in left / side..right.div_ceil(side) {
                        let dx = right.min((sx + 1) * side) - left.max(sx * side);
                        if bitmap[sy][sx] != 0 {
                            ink_area += dx * dy;
                        }
                    }
                }
                coverage.push(
                    ((ink_area * LEVELS as usize + width * height / 2) / (width * height)) as u8,
                );
            }
        }
        let levels = f32::from(LEVELS);
        let mean =
            coverage.iter().map(|&a| f32::from(a)).sum::<f32>() / (levels * coverage.len() as f32);
        let variance = coverage
            .iter()
            .map(|&a| (f32::from(a) / levels - mean).powi(2))
            .sum();
        let uniform = coverage
            .iter()
            .all(|&a| a == coverage[0])
            .then_some(coverage[0]);
        Self {
            ch,
            coverage,
            starts_with_ink: bitmap[0][0] != 0,
            mean,
            variance,
            uniform,
        }
    }
}

fn load_coverage(args: &RenderArgs, store: &GlyphStore, side: usize) -> Vec<CoverageGlyph> {
    let explicit_only =
        args.blocks.is_empty() && (args.include.is_some() || !args.include_range.is_empty());
    let explicit = store.groups.get("explicit_includes");
    let mut glyphs: Vec<_> = store
        .glyphs
        .iter()
        .filter_map(|(ch, bitmap, origin)| {
            if !store.selected.contains(ch)
                || explicit_only && !explicit.is_some_and(|chars| chars.contains(ch))
            {
                return None;
            }
            if bitmap.is_empty() || bitmap[0].is_empty() {
                return None;
            }
            let procedural;
            let bitmap = if origin == "Procedural terminal geometry" {
                // Use one consistent bitmap for all detail settings. A finer
                // sample grid should not change the size of the printed stipple.
                procedural = crate::font::procedural_block_bitmap(
                    *ch,
                    PROCEDURAL_BITMAP_SIDE,
                    PROCEDURAL_BITMAP_SIDE,
                );
                procedural.as_deref().unwrap_or(bitmap)
            } else {
                bitmap
            };
            Some(CoverageGlyph::from_bitmap(*ch, bitmap, side))
        })
        .collect();
    // Order ties predictably and prefer spaces over ink for an exact solid.
    glyphs.sort_by_key(|glyph| (glyph.ch != ' ', glyph.ch));
    glyphs
}

#[derive(Clone, Copy)]
struct Sample {
    rgb: Rgb,
    lab: Lab,
}

#[derive(Clone, Copy)]
struct Match {
    glyph: usize,
    pair: Pair,
    error: f32,
}

struct MatchScratch {
    nearest: Vec<(f32, Pair)>,
    foregrounds: Vec<(f32, Pair)>,
    backgrounds: Vec<(f32, Pair)>,
    means: [Option<Pair>; LEVELS as usize + 1],
}

impl MatchScratch {
    fn new() -> Self {
        Self {
            nearest: Vec::with_capacity(1),
            foregrounds: Vec::with_capacity(8),
            backgrounds: Vec::with_capacity(8),
            means: [None; LEVELS as usize + 1],
        }
    }

    fn best_cell(
        &mut self,
        samples: &[Sample],
        glyphs: &[CoverageGlyph],
        lookup: &Lookup,
        limit: usize,
        last_fg: Option<u8>,
        abort: Option<&AtomicBool>,
    ) -> Option<Match> {
        let cancelled = || abort.is_some_and(|flag| flag.load(Ordering::Relaxed));
        self.means.fill(None);
        let mean_rgb: Rgb = std::array::from_fn(|c| {
            samples.iter().map(|p| p.rgb[c]).sum::<f32>() / samples.len() as f32
        });
        let mean_lab: Lab = std::array::from_fn(|c| {
            samples.iter().map(|p| p.lab[c]).sum::<f32>() / samples.len() as f32
        });
        let flat = samples.iter().all(|sample| sample.rgb == samples[0].rgb);
        let has_uniform = glyphs.iter().any(|glyph| glyph.uniform.is_some());
        let mut best = Match {
            glyph: 0,
            pair: Pair::default(),
            error: f32::INFINITY,
        };

        for (glyph_idx, glyph) in glyphs.iter().enumerate() {
            if cancelled() {
                return None;
            }
            if flat && has_uniform && glyph.uniform.is_none() {
                continue;
            }
            let mut consider = |pair: Pair| {
                let mixed = lookup.mixed(pair);
                let error: f32 = samples
                    .iter()
                    .zip(&glyph.coverage)
                    .map(|(sample, &a)| distance(sample.lab, mixed[a as usize]))
                    .sum();
                // Complementary masks can describe the same colours. A light
                // foreground is the default tie break, with matching the
                // already emitted foreground preferred for uniform half shades.
                let preference = |glyph: &CoverageGlyph, pair: Pair| {
                    let half_shade = glyph.uniform == Some(LEVELS / 2) && pair.fg != pair.bg;
                    let light_foreground =
                        lookup.labs[pair.fg as usize][0] >= lookup.labs[pair.bg as usize][0];
                    (
                        !half_shade || last_fg != Some(pair.fg),
                        // Preserve checkerboard phase when the colour order
                        // is changed to reuse the previous foreground.
                        half_shade && glyph.starts_with_ink != light_foreground,
                        !light_foreground,
                        pair.fg != pair.bg && glyph.uniform.is_none(),
                        glyph.ch,
                    )
                };
                if error < best.error - 1e-8
                    || ((error - best.error).abs() <= 1e-8
                        && preference(glyph, pair) < preference(&glyphs[best.glyph], best.pair))
                {
                    best = Match {
                        glyph: glyph_idx,
                        pair,
                        error,
                    };
                }
            };

            let mean_level = (glyph.mean * f32::from(LEVELS)).round() as u8;
            let mean_pair = *self.means[mean_level as usize].get_or_insert_with(|| {
                lookup.uniform_pair(mean_level, mean_lab, &mut self.nearest)
            });
            consider(mean_pair);
            if glyph.uniform == Some(LEVELS / 2) {
                consider(mean_pair.swapped());
            }
            if glyph.uniform.is_some() {
                continue;
            }

            // An unconstrained linear fit estimates both endpoints from the
            // coverage map. Compare nearby palette combinations in Oklab.
            let contrast: Rgb = std::array::from_fn(|c| {
                samples
                    .iter()
                    .zip(&glyph.coverage)
                    .map(|(sample, &a)| {
                        (f32::from(a) / f32::from(LEVELS) - glyph.mean)
                            * (sample.rgb[c] - mean_rgb[c])
                    })
                    .sum::<f32>()
                    / glyph.variance
            });
            let endpoint = |foreground: bool| {
                let offset = if foreground {
                    1.0 - glyph.mean
                } else {
                    -glyph.mean
                };
                lab(linear_rgb_to_oklab(std::array::from_fn(|c| {
                    (mean_rgb[c] + offset * contrast[c]).clamp(0.0, 1.0)
                })))
            };
            self.foregrounds.clear();
            self.backgrounds.clear();
            nearest(
                lookup.tree(0),
                endpoint(true),
                0,
                limit,
                &mut self.foregrounds,
            );
            nearest(
                lookup.tree(0),
                endpoint(false),
                0,
                limit,
                &mut self.backgrounds,
            );
            for &(_, fg) in &self.foregrounds {
                if cancelled() {
                    return None;
                }
                for &(_, bg) in &self.backgrounds {
                    consider(Pair {
                        fg: fg.fg,
                        bg: bg.bg,
                    });
                }
            }
        }
        Some(best)
    }
}

fn samples_into(image: &AnsiImage, row: usize, col: usize, side: usize, samples: &mut [Sample]) {
    let height = image.bitmap.len();
    let width = image.bitmap[0].len();
    for (i, sample) in samples.iter_mut().enumerate() {
        let x = (col * side + i % side) * 2;
        let y = (row * side + i / side) * 2;
        let mut rgb = [0.0; 3];
        for dy in 0..2 {
            for dx in 0..2 {
                let v = linear(image.bitmap[(y + dy).min(height - 1)][(x + dx).min(width - 1)]);
                for c in 0..3 {
                    rgb[c] += v[c] * 0.25;
                }
            }
        }
        *sample = Sample {
            rgb,
            lab: lab(linear_rgb_to_oklab(rgb)),
        };
    }
}

pub(crate) fn canvas(image: PhotonImage, glyphs: &GlyphStore) -> AnsiImage {
    let stride = image.get_width() as usize * 4;
    let mut bitmap = Vec::new();
    if stride > 0 {
        bitmap = image
            .get_raw_pixels()
            .chunks_exact(stride)
            .map(|line| {
                line.chunks_exact(4)
                    .map(|rgba| {
                        let opacity = rgba[3] as u16;
                        let rgb = [rgba[0], rgba[1], rgba[2]]
                            .map(|channel| ((channel as u16 * opacity + 127) / 255) as u8);
                        make_rgb_u32(&rgb)
                    })
                    .collect()
            })
            .collect();
    }
    AnsiImage {
        bitmap,
        block: Vec::new(),
        glyphs: glyphs.clone(),
    }
}

fn smooth(
    image: &AnsiImage,
    args: &RenderArgs,
    lookup: &Lookup,
    glyphs: &[CoverageGlyph],
    matches: &mut [Vec<Match>],
    side: usize,
    abort: Option<&AtomicBool>,
) -> Result<std::time::Duration, ()> {
    let grid_w = matches[0].len();
    let grid_h = matches.len();
    let width = grid_w * side;
    let mut grid_data = Vec::with_capacity(grid_w * grid_h);
    let mut grid_state = Vec::with_capacity(grid_w * grid_h);
    let mut raster = vec![[0; 3]; width * grid_h * side];
    let rgb8 = |rgb: Rgb| rgb.map(|v| (v * 255.0).round() as u8);
    for (idx, cell) in matches.iter().flatten().enumerate() {
        let fg = lookup.colours[cell.pair.fg as usize];
        let bg = lookup.colours[cell.pair.bg as usize];
        grid_data.push(BlockData {
            fg: rgb8(fg),
            bg: rgb8(bg),
            fg_col: None,
            bg_col: None,
            alt_fg_col: None,
            alt_bg_col: None,
        });
        grid_state.push((cell.glyph, false));
        let x = idx % grid_w * side;
        let y = idx / grid_w * side;
        for (i, &coverage) in glyphs[cell.glyph].coverage.iter().enumerate() {
            raster[(y + i / side) * width + x + i % side] = rgb8(mix(fg, bg, coverage));
        }
    }
    let targets = super::all_cell_discontinuity_scores(&raster, grid_w, grid_h, side, side);
    let targets = targets.as_slice();
    let row_costs: Vec<Vec<_>> = matches
        .par_iter()
        .enumerate()
        .map(|(row, cells)| {
            let mut samples = vec![
                Sample {
                    rgb: [0.0; 3],
                    lab: [0.0; 3],
                };
                side * side
            ];
            cells
                .iter()
                .enumerate()
                .map(move |(col, cell)| {
                    if abort.is_some_and(|flag| flag.load(Ordering::Relaxed))
                        || targets[row * grid_w + col] < args.discontinuity_threshold.max(0.0)
                        || cell.pair.fg == cell.pair.bg
                    {
                        return Vec::new();
                    }
                    samples_into(image, row, col, side, &mut samples);
                    let fg = lookup.colours[cell.pair.fg as usize];
                    let bg = lookup.colours[cell.pair.bg as usize];
                    let mixed: [Rgb; LEVELS as usize + 1] =
                        std::array::from_fn(|i| mix(fg, bg, i as u8));
                    glyphs
                        .iter()
                        .flat_map(|glyph| {
                            let cost = |inverted| {
                                let error: f32 = samples
                                    .iter()
                                    .zip(&glyph.coverage)
                                    .map(|(sample, &a)| {
                                        distance(
                                            sample.rgb,
                                            mixed[if inverted { LEVELS - a } else { a } as usize],
                                        )
                                    })
                                    .sum();
                                (error * 255.0 * 255.0).round() as u64
                            };
                            [cost(false), cost(true)]
                        })
                        .collect()
                })
                .collect()
        })
        .collect();
    let costs: Vec<_> = row_costs.into_iter().flatten().collect();
    let coverage = glyphs
        .iter()
        .map(|glyph| glyph.coverage.as_slice())
        .collect::<Vec<_>>();
    super::smooth_features::run_coverage(
        args,
        &mut grid_state,
        &grid_data,
        &coverage,
        LEVELS,
        &costs,
        &targets,
        grid_w,
        grid_h,
        side,
        abort,
    )?;
    let mut shape_time = std::time::Duration::ZERO;
    if args.smooth_shapes {
        let started = RenderInstant::now();
        let geometry = GeometryGlyphs::Coverage(&coverage, LEVELS);
        let mut source = vec![[0; 3]; raster.len()];
        let mut samples = vec![
            Sample {
                rgb: [0.0; 3],
                lab: [0.0; 3]
            };
            side * side
        ];
        for (idx, &choice) in grid_state.iter().enumerate() {
            if abort.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
                return Err(());
            }
            let (row, col) = (idx / grid_w, idx % grid_w);
            samples_into(image, row, col, side, &mut samples);
            for (i, sample) in samples.iter().enumerate() {
                source[(row * side + i / side) * width + col * side + i % side] = rgb8(sample.rgb);
            }
            geometry.paint(
                &mut raster,
                width,
                (col * side, row * side),
                (side, side),
                &grid_data[idx],
                choice,
                side,
                side,
            );
        }
        let targets = super::all_cell_discontinuity_scores(&raster, grid_w, grid_h, side, side);
        super::run_geometry_smooth(
            args,
            &mut raster,
            &mut grid_state,
            &grid_data,
            geometry,
            &costs,
            targets,
            &source,
            grid_w,
            grid_h,
            side,
            side,
            abort,
        )?;
        shape_time = started.elapsed();
    }
    for (cell, (glyph, inverted)) in matches.iter_mut().flatten().zip(grid_state) {
        cell.glyph = glyph;
        if inverted {
            cell.pair = cell.pair.swapped();
        }
    }
    Ok(shape_time)
}

pub(crate) fn render(
    canvas: &AnsiImage,
    args: &RenderArgs,
    abort: Option<&AtomicBool>,
) -> RenderResult {
    let started = RenderInstant::now();
    let height = canvas.bitmap.len();
    let width = canvas.bitmap.first().map_or(0, Vec::len);
    if width == 0 {
        return "".into();
    }
    if let Err(error) = crate::effects::check_dither_dimensions(args, width as u32, height as u32) {
        return error.into();
    }
    let lookup = match args.render {
        Render::Irc => &*IRC_LOOKUP,
        Render::Ansi => &*ANSI_LOOKUP,
        Render::Ansi24 => return "dither render mode requires the IRC99 or ANSI256 palette".into(),
    };
    let (w, h) = args.cell_pixels(canvas.glyphs.metrics);
    let side = w / 2;
    let glyphs = load_coverage(args, &canvas.glyphs, side);
    if glyphs.is_empty() {
        return "the selected characters produced no usable dither glyphs".into();
    }
    let available = lookup.indices.len();
    let limit = match args.dither_palette_candidates {
        0 => available,
        count => (count as usize).min(available),
    };
    let matches: Option<Vec<_>> = (0..height.div_ceil(h))
        .into_par_iter()
        .map(|row| {
            let mut cells = Vec::with_capacity(width.div_ceil(w));
            let mut last_fg = None;
            let mut scratch = MatchScratch::new();
            let mut samples = vec![
                Sample {
                    rgb: [0.0; 3],
                    lab: [0.0; 3],
                };
                side * side
            ];
            for col in 0..width.div_ceil(w) {
                if abort.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
                    return None;
                }
                samples_into(canvas, row, col, side, &mut samples);
                let cell = scratch.best_cell(&samples, &glyphs, lookup, limit, last_fg, abort)?;
                if glyphs[cell.glyph].ch != ' ' {
                    last_fg = Some(cell.pair.fg);
                }
                cells.push(cell);
            }
            Some(cells)
        })
        .collect();
    let Some(mut matches) = matches else {
        return "Cancelled".into();
    };
    let match_time = started.elapsed();
    let smoothing_started = RenderInstant::now();
    let (smoothing_time, shape_time) = if args.smooth {
        match smooth(canvas, args, lookup, &glyphs, &mut matches, side, abort) {
            Ok(shape_time) => (
                smoothing_started.elapsed().saturating_sub(shape_time),
                shape_time,
            ),
            Err(()) => return "Cancelled".into(),
        }
    } else {
        Default::default()
    };
    let mut base = RenderResult::from("");
    base.grid = matches
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|cell| RenderedCell {
                    char: glyphs[cell.glyph].ch,
                    fg: unpack_rgb(lookup.palette[cell.pair.fg as usize]),
                    bg: Some(unpack_rgb(lookup.palette[cell.pair.bg as usize])),
                    inverted: false,
                    bold: false,
                    italic: false,
                    underline: false,
                })
                .collect()
        })
        .collect();
    base.total_pixels = (width * height) as u64;
    base.profile.glyph_match = match_time;
    base.profile.smoothing_search = smoothing_time;
    base.profile.shape_refine = shape_time;
    let encode_started = RenderInstant::now();
    let mut result = encode_grid_with_overlays(
        &base,
        &args.overlays,
        args.render,
        args.as_preview,
        args.grayscale_tolerance,
        true,
    );
    result.profile.encode = encode_started.elapsed();
    result.initial_time = match_time;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complementary_half_shades_keep_the_light_pixel_in_phase() {
        let lookup = Lookup::new(&[0, 0xffffff], 0);
        let glyphs = ['▒', '🮐'].map(|ch| {
            let bitmap = crate::font::procedural_block_bitmap(ch, 32, 32).unwrap();
            CoverageGlyph::from_bitmap(ch, &bitmap, 4)
        });
        assert!(glyphs.iter().all(|glyph| glyph.uniform == Some(LEVELS / 2)));
        let rgb = [0.5; 3];
        let pixels = [Sample {
            rgb,
            lab: lab(linear_rgb_to_oklab(rgb)),
        }; 16];
        let mut scratch = MatchScratch::new();
        let normal = scratch
            .best_cell(&pixels, &glyphs, &lookup, 2, None, None)
            .unwrap();
        let inverse = scratch
            .best_cell(&pixels, &glyphs, &lookup, 2, Some(0), None)
            .unwrap();
        assert_eq!((normal.glyph, normal.pair.fg), (0, 1));
        assert_eq!((inverse.glyph, inverse.pair.fg), (1, 0));
    }
}
