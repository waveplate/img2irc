use crate::args::{Render, RenderArgs};
use crate::font::GlyphStore;
use crate::palette::{ANSI256, IRC99};
use image::{DynamicImage, Rgb, RgbImage};
use once_cell::sync::Lazy;
use photon_rs::PhotonImage;
use rayon::prelude::*;
use std::collections::{BinaryHeap, HashMap};
use std::f32::consts::PI;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(not(target_arch = "wasm32"))]
pub(crate) type RenderInstant = std::time::Instant;

#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub(crate) struct RenderInstant(f64);

#[cfg(all(target_arch = "wasm32", feature = "web"))]
impl RenderInstant {
    pub(crate) fn now() -> Self {
        Self(js_sys::Date::now())
    }

    pub(crate) fn elapsed(&self) -> std::time::Duration {
        std::time::Duration::from_secs_f64(((js_sys::Date::now() - self.0) / 1000.0).max(0.0))
    }
}

#[cfg(all(target_arch = "wasm32", not(feature = "web")))]
pub(crate) struct RenderInstant;

#[cfg(all(target_arch = "wasm32", not(feature = "web")))]
impl RenderInstant {
    pub(crate) fn now() -> Self {
        Self
    }

    pub(crate) fn elapsed(&self) -> std::time::Duration {
        std::time::Duration::ZERO
    }
}

#[derive(Clone)]
pub struct FastGlyph {
    pub ch: char,
    pub bitmap: Vec<u8>,
    pub ones: Vec<usize>,
    pub zeros: Vec<usize>,
    pub vertical_cuts: Vec<u8>,
    pub horizontal_cuts: Vec<u8>,
    pub top: Vec<u8>,
    pub bottom: Vec<u8>,
    pub left: Vec<u8>,
    pub right: Vec<u8>,
    pub coverage: f32,
    // Exit positions: average boundary crossing point per edge (-1.0 = no boundary)
    pub exit_top: f32,
    pub exit_bottom: f32,
    pub exit_left: f32,
    pub exit_right: f32,
    // Inverted exit positions: average position of ZERO pixels (which become FG when inverted)
    pub exit_top_inv: f32,
    pub exit_bottom_inv: f32,
    pub exit_left_inv: f32,
    pub exit_right_inv: f32,
    pub mask: u128,
}

#[derive(Clone, Copy, Debug)]
pub struct PrePixel {
    pub r: u64,
    pub g: u64,
    pub b: u64,
    pub sq: u64,
    pub ansi_part: u8,
    pub irc_part: u8,
}

pub fn prepare_glyphs(glyphs: &GlyphStore, args: &RenderArgs) -> Vec<FastGlyph> {
    if glyphs.glyphs.is_empty() {
        return Vec::new();
    }
    let (gw, gh) = glyphs.metrics;
    let bp = gh * gw;

    // We consider "filtering" active if the user provided specific blocks or inclusions.
    let is_filtering =
        !args.blocks.is_empty() || args.include.is_some() || !args.include_range.is_empty();

    let mut filtered_glyphs: Vec<_> = if is_filtering {
        // Only include selected characters, no extra spaces
        glyphs
            .glyphs
            .iter()
            .filter(|(ch, _, _)| {
                glyphs.selected.contains(ch) && !crate::font::is_emoji_render_candidate(*ch)
            })
            .collect()
    } else {
        // If not filtering, include everything
        glyphs
            .glyphs
            .iter()
            .filter(|(ch, _, _)| !crate::font::is_emoji_render_candidate(*ch))
            .collect()
    };

    // Fallback only when no glyph filters are active. With filters active, an empty
    // set can be intentional, for example after disabling every glyph in a group.
    if filtered_glyphs.is_empty() && !is_filtering {
        filtered_glyphs = glyphs
            .glyphs
            .iter()
            .filter(|(ch, _, _)| !crate::font::is_emoji_render_candidate(*ch))
            .collect();
    }

    filtered_glyphs
        .into_iter()
        .map(|(ch, bmp, _font)| {
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
                right.push(bitmap[y * gw + (gw - 1)]);
            }

            let mut ones_indices = Vec::with_capacity(bp);
            let mut zeros_indices = Vec::with_capacity(bp);
            for (i, &b) in bitmap.iter().enumerate() {
                if b == 1 {
                    ones_indices.push(i);
                } else {
                    zeros_indices.push(i);
                }
            }

            let ones = ones_indices.len();
            let coverage = ones as f32 / bp as f32;

            let mut vertical_cuts = Vec::with_capacity(gh.saturating_mul(gw.saturating_sub(1)));
            if gw > 1 {
                for y in 0..gh {
                    for x in 0..(gw - 1) {
                        vertical_cuts.push((bitmap[y * gw + x] != bitmap[y * gw + x + 1]) as u8);
                    }
                }
            }

            let mut horizontal_cuts = Vec::with_capacity(gw.saturating_mul(gh.saturating_sub(1)));
            if gh > 1 {
                for y in 0..(gh - 1) {
                    for x in 0..gw {
                        horizontal_cuts
                            .push((bitmap[y * gw + x] != bitmap[(y + 1) * gw + x]) as u8);
                    }
                }
            }

            // Compute exit positions: average position of FG pixels along each edge
            // -1.0 means uniform edge (all FG or all BG, so no boundary to track)
            let compute_exit = |edge: &[u8], target: u8| -> f32 {
                let count: usize = edge.iter().filter(|&&b| b == target).count();
                if count == 0 || count == edge.len() {
                    return -1.0; // uniform edge, no boundary
                }
                let sum: f32 = edge
                    .iter()
                    .enumerate()
                    .filter(|(_, &b)| b == target)
                    .map(|(i, _)| i as f32)
                    .sum();
                sum / count as f32
            };

            // Normal exits: average position of 1-pixels (FG)
            let exit_top = compute_exit(&top, 1);
            let exit_bottom = compute_exit(&bottom, 1);
            let exit_left = compute_exit(&left, 1);
            let exit_right = compute_exit(&right, 1);

            // Inverted exits: average position of 0-pixels (which become FG when inverted)
            let exit_top_inv = compute_exit(&top, 0);
            let exit_bottom_inv = compute_exit(&bottom, 0);
            let exit_left_inv = compute_exit(&left, 0);
            let exit_right_inv = compute_exit(&right, 0);

            let mut mask = 0u128;
            for &idx in &ones_indices {
                if idx < 128 {
                    mask |= 1u128 << idx;
                }
            }

            FastGlyph {
                ch: *ch,
                bitmap,
                ones: ones_indices,
                zeros: zeros_indices,
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
                mask,
            }
        })
        .collect()
}

const POSITIONS: [(usize, usize, u32); 8] = [
    (0, 0, 0x01),
    (0, 1, 0x02),
    (0, 2, 0x04),
    (1, 0, 0x08),
    (1, 1, 0x10),
    (1, 2, 0x20),
    (0, 3, 0x40),
    (1, 3, 0x80),
];

static COLOR_CACHE: Lazy<dashmap::DashMap<u32, (u8, u8, u8, u8, u8, u8)>> =
    Lazy::new(|| dashmap::DashMap::with_capacity(8_192));

struct PaletteInfo {
    components: Vec<[i32; 3]>,
    is_chromatic: Vec<bool>,
}

static ANSI256_INFO: Lazy<PaletteInfo> = Lazy::new(|| PaletteInfo {
    components: ANSI256
        .iter()
        .map(|&p| {
            [
                ((p >> 16) & 0xFF) as i32,
                ((p >> 8) & 0xFF) as i32,
                (p & 0xFF) as i32,
            ]
        })
        .collect(),
    is_chromatic: ANSI256.iter().map(|&p| !is_near_grayscale(p, 0)).collect(),
});

static IRC99_INFO: Lazy<PaletteInfo> = Lazy::new(|| PaletteInfo {
    components: IRC99
        .iter()
        .map(|&p| {
            [
                ((p >> 16) & 0xFF) as i32,
                ((p >> 8) & 0xFF) as i32,
                (p & 0xFF) as i32,
            ]
        })
        .collect(),
    is_chromatic: IRC99.iter().map(|&p| !is_near_grayscale(p, 0)).collect(),
});

#[derive(Clone, Copy, Debug)]
pub struct Oklab {
    pub l: f32,
    pub a: f32,
    pub b: f32,
}

#[inline]
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

pub fn rgb_to_oklab(rgb: [u8; 3]) -> Oklab {
    let r = srgb_to_linear(rgb[0] as f32 / 255.0);
    let g = srgb_to_linear(rgb[1] as f32 / 255.0);
    let b = srgb_to_linear(rgb[2] as f32 / 255.0);

    let l = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b;
    let m = 0.2119034982 * r + 0.6806995451 * g + 0.1073969617 * b;
    let s = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b;

    let l_ = l.cbrt();
    let m_ = m.cbrt();
    let s_ = s.cbrt();

    Oklab {
        l: 0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_,
        a: 1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_,
        b: 0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_,
    }
}

#[inline]
pub fn oklab_distance(c1: Oklab, c2: Oklab) -> f32 {
    let dl = c1.l - c2.l;
    let da = c1.a - c2.a;
    let db = c1.b - c2.b;
    (dl * dl + da * da + db * db).sqrt()
}

#[inline]
pub fn oklab_distance_squared(c1: Oklab, c2: Oklab) -> f32 {
    let dl = c1.l - c2.l;
    let da = c1.a - c2.a;
    let db = c1.b - c2.b;
    dl * dl + da * da + db * db
}

struct OklabPalette {
    entries: Vec<Oklab>,
    is_chromatic: Vec<bool>,
}

impl OklabPalette {
    fn new(palette: &[u32], is_chromatic: &[bool]) -> Self {
        let entries = palette
            .iter()
            .map(|&p| {
                let [r, g, b] = unpack_rgb(p);
                rgb_to_oklab([r, g, b])
            })
            .collect();
        OklabPalette {
            entries,
            is_chromatic: is_chromatic.to_vec(),
        }
    }
}

static ANSI256_OKLAB: Lazy<OklabPalette> =
    Lazy::new(|| OklabPalette::new(&ANSI256, &ANSI256_INFO.is_chromatic));

static IRC99_OKLAB: Lazy<OklabPalette> =
    Lazy::new(|| OklabPalette::new(&IRC99, &IRC99_INFO.is_chromatic));

#[inline(always)]
fn should_skip_irc99_match_index(palette_len: usize, idx: usize) -> bool {
    palette_len == IRC99.len() && idx < 16
}

#[inline(always)]
pub fn nearest_hex_colour_fast(col: u32, palette: &[u32]) -> u8 {
    let pr = ((col >> 16) & 0xFF) as i32;
    let pg = ((col >> 8) & 0xFF) as i32;
    let pb = (col & 0xFF) as i32;

    let mut best_i = 0u8;
    let mut best_d = u32::MAX;
    if palette.is_empty() {
        return 0;
    }

    let info = if palette.len() == 256 {
        Some(&ANSI256_INFO)
    } else if palette.len() == 99 {
        Some(&IRC99_INFO)
    } else {
        None
    };
    if let Some(inf) = info {
        for (i, [r, g, b]) in inf.components.iter().enumerate() {
            if should_skip_irc99_match_index(palette.len(), i) {
                continue;
            }

            let dr = pr - r;
            let dg = pg - g;
            let db = pb - b;
            let d = (dr * dr + dg * dg + db * db) as u32;
            if d < best_d {
                best_d = d;
                best_i = i as u8;
                if d == 0 {
                    break;
                }
            }
        }
    } else {
        for (i, &p) in palette.iter().enumerate() {
            if should_skip_irc99_match_index(palette.len(), i) {
                continue;
            }

            let dr = pr - (((p >> 16) & 0xFF) as i32);
            let dg = pg - (((p >> 8) & 0xFF) as i32);
            let db = pb - ((p & 0xFF) as i32);
            let d = (dr * dr + dg * dg + db * db) as u32;
            if d < best_d {
                best_d = d;
                best_i = i as u8;
                if d == 0 {
                    break;
                }
            }
        }
    }
    best_i
}

#[inline(always)]
pub fn nearest_distinctly_chromatic_hex_colour(
    col: u32,
    palette: &[u32],
    tolerance: u8,
) -> Option<u8> {
    let pr = ((col >> 16) & 0xFF) as i32;
    let pg = ((col >> 8) & 0xFF) as i32;
    let pb = (col & 0xFF) as i32;

    let mut best_i: Option<u8> = None;
    let mut best_d = u32::MAX;

    let info = if palette.len() == 256 {
        Some(&ANSI256_INFO)
    } else if palette.len() == 99 {
        Some(&IRC99_INFO)
    } else {
        None
    };

    if let Some(inf) = info {
        for (i, [r, g, b]) in inf.components.iter().enumerate() {
            if should_skip_irc99_match_index(palette.len(), i) {
                continue;
            }
            if is_near_grayscale_rgb(*r as u8, *g as u8, *b as u8, tolerance) {
                continue;
            }
            let dr = pr - r;
            let dg = pg - g;
            let db = pb - b;
            let d = (dr * dr + dg * dg + db * db) as u32;
            if d < best_d {
                best_d = d;
                best_i = Some(i as u8);
                if d == 0 {
                    break;
                }
            }
        }
    } else {
        for (i, &p) in palette.iter().enumerate() {
            if should_skip_irc99_match_index(palette.len(), i) {
                continue;
            }
            if is_near_grayscale(p, tolerance) {
                continue;
            }
            let dr = pr - (((p >> 16) & 0xFF) as i32);
            let dg = pg - (((p >> 8) & 0xFF) as i32);
            let db = pb - ((p & 0xFF) as i32);
            let d = (dr * dr + dg * dg + db * db) as u32;
            if d < best_d {
                best_d = d;
                best_i = Some(i as u8);
                if d == 0 {
                    break;
                }
            }
        }
    }
    best_i
}

#[inline(always)]
pub fn nearest_strict_partitioned_hex_colour(target: u32, palette: &[u32], tol: u8) -> u8 {
    let target_is_chromatic = !is_near_grayscale(target, tol);
    let mut best_i = 0u8;
    let mut best_d = u32::MAX;

    let pr = ((target >> 16) & 0xFF) as i32;
    let pg = ((target >> 8) & 0xFF) as i32;
    let pb = (target & 0xFF) as i32;

    let info = if palette.len() == 256 {
        Some(&ANSI256_INFO)
    } else if palette.len() == 99 {
        Some(&IRC99_INFO)
    } else {
        None
    };

    if let Some(inf) = info {
        for (i, [r, g, b]) in inf.components.iter().enumerate() {
            if should_skip_irc99_match_index(palette.len(), i) {
                continue;
            }

            let p_is_chromatic = inf.is_chromatic[i];

            if target_is_chromatic == p_is_chromatic {
                let d = (pr - r).pow(2) + (pg - g).pow(2) + (pb - b).pow(2);
                let d = d as u32;
                if d < best_d {
                    best_d = d;
                    best_i = i as u8;
                    if d == 0 {
                        break;
                    }
                }
            }
        }
    } else {
        for (i, &p) in palette.iter().enumerate() {
            if should_skip_irc99_match_index(palette.len(), i) {
                continue;
            }

            let p_is_chromatic = !is_near_grayscale(p, 0); // Strict check for palette

            if target_is_chromatic == p_is_chromatic {
                let r = ((p >> 16) & 0xFF) as i32;
                let g = ((p >> 8) & 0xFF) as i32;
                let b = (p & 0xFF) as i32;
                let d = (pr - r).pow(2) + (pg - g).pow(2) + (pb - b).pow(2);
                let d = d as u32;
                if d < best_d {
                    best_d = d;
                    best_i = i as u8;
                    if d == 0 {
                        break;
                    }
                }
            }
        }
    }

    if best_d == u32::MAX {
        nearest_hex_colour_fast(target, palette)
    } else {
        best_i
    }
}

#[inline]
pub fn is_near_grayscale_rgb(r: u8, g: u8, b: u8, tolerance: u8) -> bool {
    let min_val = r.min(g.min(b));
    let max_val = r.max(g.max(b));
    max_val.saturating_sub(min_val) <= tolerance
}

#[inline]
pub fn make_rgb_u32(px: &[u8]) -> u32 {
    if px.len() < 3 {
        return 0;
    }
    ((px[0] as u32) << 16) | ((px[1] as u32) << 8) | (px[2] as u32)
}
#[inline]
pub fn unpack_rgb(rgb: u32) -> [u8; 3] {
    [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8]
}

#[inline]
pub fn is_near_grayscale(col: u32, tolerance: u8) -> bool {
    let [r, g, b] = unpack_rgb(col);
    let min_val = r.min(g.min(b));
    let max_val = r.max(g.max(b));
    max_val.saturating_sub(min_val) <= tolerance
}

fn find_nearest_gray_and_color_oklab(
    pixel_oklab: Oklab,
    oklab_palette: &OklabPalette,
    palette_len: usize,
) -> (f32, f32) {
    let mut min_gray_dist_sq = f32::MAX;
    let mut min_color_dist_sq = f32::MAX;

    for (i, &entry_oklab) in oklab_palette.entries.iter().enumerate() {
        if should_skip_irc99_match_index(palette_len, i) {
            continue;
        }
        let dist_sq = oklab_distance_squared(pixel_oklab, entry_oklab);
        if oklab_palette.is_chromatic[i] {
            if dist_sq < min_color_dist_sq {
                min_color_dist_sq = dist_sq;
            }
        } else {
            if dist_sq < min_gray_dist_sq {
                min_gray_dist_sq = dist_sq;
            }
        }
    }
    (min_gray_dist_sq.sqrt(), min_color_dist_sq.sqrt())
}

fn compute_spatial_grayscale_classification(
    src: &Vec<Vec<u32>>,
    oklab_palette: &OklabPalette,
    palette_len: usize,
    tolerance: u8,
) -> Vec<Vec<bool>> {
    let height = src.len();
    if height == 0 {
        return Vec::new();
    }
    let width = src[0].len();
    if width == 0 {
        return vec![Vec::new(); height];
    }

    let high_threshold = (tolerance as f32 / 32.0) * 0.03;

    let row_data: Vec<(Vec<Oklab>, Vec<f32>, Vec<f32>, Vec<Option<bool>>)> = (0..height)
        .into_par_iter()
        .map(|y| {
            let mut oklabs = Vec::with_capacity(width);
            let mut dg_row = Vec::with_capacity(width);
            let mut dc_row = Vec::with_capacity(width);
            let mut class_row = Vec::with_capacity(width);

            for x in 0..width {
                let [r, g, b] = unpack_rgb(src[y][x]);
                let oklab = rgb_to_oklab([r, g, b]);
                let (dg, dc) = find_nearest_gray_and_color_oklab(oklab, oklab_palette, palette_len);

                let class = if dc - dg > high_threshold {
                    Some(false)
                } else if dg - dc > high_threshold {
                    Some(true)
                } else {
                    None
                };

                oklabs.push(oklab);
                dg_row.push(dg);
                dc_row.push(dc);
                class_row.push(class);
            }
            (oklabs, dg_row, dc_row, class_row)
        })
        .collect();

    let mut oklab_grid = Vec::with_capacity(height);
    let mut d_gray = Vec::with_capacity(height);
    let mut d_color = Vec::with_capacity(height);
    let mut classification = Vec::with_capacity(height);

    for (oklabs, dg, dc, class) in row_data {
        oklab_grid.push(oklabs);
        d_gray.push(dg);
        d_color.push(dc);
        classification.push(class);
    }

    let mut visited = vec![vec![false; width]; height];
    let similarity_threshold_sq = 0.0004f32; // 0.02 * 0.02
    let mut result = vec![vec![false; width]; height];

    for y in 0..height {
        for x in 0..width {
            if let Some(is_chromatic) = classification[y][x] {
                result[y][x] = is_chromatic;
                continue;
            }

            if visited[y][x] {
                continue;
            }

            let mut component = Vec::new();
            let mut queue = std::collections::VecDeque::new();
            queue.push_back((y, x));
            visited[y][x] = true;

            while let Some((cy, cx)) = queue.pop_front() {
                component.push((cy, cx));

                let neighbors = [
                    (cy.wrapping_sub(1), cx),
                    (cy + 1, cx),
                    (cy, cx.wrapping_sub(1)),
                    (cy, cx + 1),
                ];

                for &(ny, nx) in &neighbors {
                    if ny < height && nx < width {
                        if classification[ny][nx].is_none() && !visited[ny][nx] {
                            if oklab_distance_squared(oklab_grid[cy][cx], oklab_grid[ny][nx])
                                < similarity_threshold_sq
                            {
                                visited[ny][nx] = true;
                                queue.push_back((ny, nx));
                            }
                        }
                    }
                }
            }

            let mut sum_d_gray = 0.0f32;
            let mut sum_d_color = 0.0f32;
            for &(cy, cx) in &component {
                sum_d_gray += d_gray[cy][cx];
                sum_d_color += d_color[cy][cx];
            }

            let is_chromatic = sum_d_color < sum_d_gray;
            for &(cy, cx) in &component {
                result[cy][cx] = is_chromatic;
            }
        }
    }

    result
}

#[derive(Debug, Clone)]
pub struct AnsiImage {
    pub bitmap: Vec<Vec<u32>>,
    pub block: Vec<Vec<AnsiPixelBlock>>,
    pub glyphs: GlyphStore,
}

#[derive(Debug, Clone, Copy)]
pub struct AnsiPixel {
    pub orig: u32,
    pub ansi_std: u8,
    pub irc_std: u8,
    pub ansi_ng: u8,
    pub irc_ng: u8,
    pub ansi_part: u8,
    pub irc_part: u8,
    pub ansi_distinct: bool,
    pub irc_distinct: bool,
}

#[derive(Debug, Clone)]
pub struct AnsiPixelBlock {
    pub pixels: Vec<Vec<AnsiPixel>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Colour {
    Index(u8),
    RGB([u8; 3]),
}

impl AnsiPixel {
    #[inline]
    pub fn new(px: &u32, tolerance: u8) -> Self {
        let distinct = !is_near_grayscale(*px, tolerance);
        Self::new_with_distinct(px, tolerance, distinct, distinct)
    }

    #[inline]
    pub fn new_with_distinct(
        px: &u32,
        tolerance: u8,
        ansi_distinct: bool,
        irc_distinct: bool,
    ) -> Self {
        if let Some(entry) = COLOR_CACHE.get(px) {
            let (a, i, an, in_, ap, ip) = *entry;
            return AnsiPixel {
                orig: *px,
                ansi_std: a,
                irc_std: i,
                ansi_ng: an,
                irc_ng: in_,
                ansi_part: ap,
                irc_part: ip,
                ansi_distinct,
                irc_distinct,
            };
        }

        let ansi_std_val = nearest_hex_colour_fast(*px, &ANSI256);
        let irc_std_val = nearest_hex_colour_fast(*px, &IRC99);

        let mut ansi_ng_val = ansi_std_val;
        let mut irc_ng_val = irc_std_val;

        if let Some(idx) = nearest_distinctly_chromatic_hex_colour(*px, &ANSI256, tolerance) {
            ansi_ng_val = idx;
        }
        if let Some(idx) = nearest_distinctly_chromatic_hex_colour(*px, &IRC99, tolerance) {
            irc_ng_val = idx;
        }

        let ansi_part_val = nearest_strict_partitioned_hex_colour(*px, &ANSI256, tolerance);
        let irc_part_val = nearest_strict_partitioned_hex_colour(*px, &IRC99, tolerance);

        COLOR_CACHE.insert(
            *px,
            (
                ansi_std_val,
                irc_std_val,
                ansi_ng_val,
                irc_ng_val,
                ansi_part_val,
                irc_part_val,
            ),
        );
        AnsiPixel {
            orig: *px,
            ansi_std: ansi_std_val,
            irc_std: irc_std_val,
            ansi_ng: ansi_ng_val,
            irc_ng: irc_ng_val,
            ansi_part: ansi_part_val,
            irc_part: irc_part_val,
            ansi_distinct,
            irc_distinct,
        }
    }
}

static LAST_TOLERANCE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

impl AnsiImage {
    pub fn new(img: PhotonImage, glyphs: GlyphStore, tolerance: u8, nograyscale: bool) -> Self {
        let last_tol = LAST_TOLERANCE.swap(tolerance, std::sync::atomic::Ordering::Relaxed);
        if last_tol != tolerance {
            COLOR_CACHE.clear();
        }
        let w = img.get_width() as usize;
        let raw = img.get_raw_pixels();
        let flat: Vec<u32> = raw.par_chunks(4).map(make_rgb_u32).collect();
        let mut bitmap: Vec<Vec<u32>> = flat.chunks(w).map(|r| r.to_vec()).collect();

        if !bitmap.len().is_multiple_of(2) {
            bitmap.push(vec![0; w]);
        }
        for row in &mut bitmap {
            if row.len() % 2 != 0 {
                row.push(0);
            }
        }

        let block = block_bitmap(&bitmap, &glyphs, tolerance, nograyscale);
        AnsiImage {
            bitmap,
            block,
            glyphs,
        }
    }
}

fn block_bitmap(
    src: &Vec<Vec<u32>>,
    glyphs: &GlyphStore,
    tolerance: u8,
    nograyscale: bool,
) -> Vec<Vec<AnsiPixelBlock>> {
    if glyphs.glyphs.is_empty() {
        return Vec::new();
    }
    let (gw, gh) = glyphs.metrics;
    if gh == 0 || gw == 0 {
        return Vec::new();
    }

    let (ansi_distinct_map, irc_distinct_map) = if nograyscale {
        let a =
            compute_spatial_grayscale_classification(src, &ANSI256_OKLAB, ANSI256.len(), tolerance);
        let i = compute_spatial_grayscale_classification(src, &IRC99_OKLAB, IRC99.len(), tolerance);
        (a, i)
    } else {
        (Vec::new(), Vec::new())
    };

    // Find all unique pixels in the src image to minimize map lookup overhead
    let mut flat_pixels: Vec<u32> = src.iter().flat_map(|row| row.iter().copied()).collect();
    flat_pixels.sort_unstable();
    flat_pixels.dedup();

    // Map unique pixels to their cached colors
    let unique_mappings: std::collections::HashMap<u32, (u8, u8, u8, u8, u8, u8)> = flat_pixels
        .par_iter()
        .map(|&px| {
            if let Some(entry) = COLOR_CACHE.get(&px) {
                (px, *entry)
            } else {
                let ansi_std_val = nearest_hex_colour_fast(px, &ANSI256);
                let irc_std_val = nearest_hex_colour_fast(px, &IRC99);

                let mut ansi_ng_val = ansi_std_val;
                let mut irc_ng_val = irc_std_val;

                if let Some(idx) = nearest_distinctly_chromatic_hex_colour(px, &ANSI256, tolerance)
                {
                    ansi_ng_val = idx;
                }
                if let Some(idx) = nearest_distinctly_chromatic_hex_colour(px, &IRC99, tolerance) {
                    irc_ng_val = idx;
                }

                let ansi_part_val = nearest_strict_partitioned_hex_colour(px, &ANSI256, tolerance);
                let irc_part_val = nearest_strict_partitioned_hex_colour(px, &IRC99, tolerance);

                COLOR_CACHE.insert(
                    px,
                    (
                        ansi_std_val,
                        irc_std_val,
                        ansi_ng_val,
                        irc_ng_val,
                        ansi_part_val,
                        irc_part_val,
                    ),
                );
                (
                    px,
                    (
                        ansi_std_val,
                        irc_std_val,
                        ansi_ng_val,
                        irc_ng_val,
                        ansi_part_val,
                        irc_part_val,
                    ),
                )
            }
        })
        .collect();

    let mut px_rows: Vec<Vec<AnsiPixel>> = src
        .par_iter()
        .enumerate()
        .map(|(y, row)| {
            row.iter()
                .enumerate()
                .map(|(x, px)| {
                    let (ansi_distinct, irc_distinct) = if nograyscale {
                        (ansi_distinct_map[y][x], irc_distinct_map[y][x])
                    } else {
                        (false, false)
                    };
                    let (a, i, an, in_, ap, ip) = unique_mappings.get(px).copied().unwrap();
                    AnsiPixel {
                        orig: *px,
                        ansi_std: a,
                        irc_std: i,
                        ansi_ng: an,
                        irc_ng: in_,
                        ansi_part: ap,
                        irc_part: ip,
                        ansi_distinct,
                        irc_distinct,
                    }
                })
                .collect()
        })
        .collect();

    let ph = px_rows.len().div_ceil(gh) * gh;
    let pw = if px_rows.is_empty() {
        0
    } else {
        px_rows[0].len().div_ceil(gw) * gw
    };

    // Pad px_rows if necessary to avoid bounds checks in the tight loop
    for r in &mut px_rows {
        if r.len() < pw {
            r.extend(std::iter::repeat_n(
                AnsiPixel::new(&0, tolerance),
                pw - r.len(),
            ));
        }
    }
    if px_rows.len() < ph {
        let blank = vec![AnsiPixel::new(&0, tolerance); pw];
        px_rows.extend(std::iter::repeat_n(blank, ph - px_rows.len()));
    }

    let mut out = Vec::with_capacity(ph / gh);
    out.extend(
        (0..ph)
            .into_par_iter()
            .step_by(gh)
            .map(|y| {
                let mut row = Vec::with_capacity(pw / gw);
                for x in (0..pw).step_by(gw) {
                    let mut block_pixels = Vec::with_capacity(gh);
                    for j in 0..gh {
                        let mut row_pixels = Vec::with_capacity(gw);
                        for i in 0..gw {
                            row_pixels.push(px_rows[y + j][x + i]);
                        }
                        block_pixels.push(row_pixels);
                    }
                    row.push(AnsiPixelBlock {
                        pixels: block_pixels,
                    });
                }
                row
            })
            .collect::<Vec<_>>(),
    );
    out
}

pub fn emit_colourized(
    out: &mut String,
    render: Render,
    as_preview: bool,
    fg: Colour,
    bg: Option<Colour>,
    glyph: char,
    first: &mut bool,
    last_fg: &mut Option<Colour>,
    last_bg: &mut Option<Colour>,
    bold: bool,
    italic: bool,
    underline: bool,
    last_bold: &mut bool,
    last_italic: &mut bool,
    last_underline: &mut bool,
    lookahead_fg: Option<&Colour>,
) {
    let is_space = glyph == ' ' || glyph == '\u{2800}';

    // Determine what actually changed
    let fg_changed = *first || last_fg.as_ref() != Some(&fg);
    let bg_changed = *first || last_bg.as_ref() != bg.as_ref();
    let bold_changed = *first || *last_bold != bold;
    let italic_changed = *first || *last_italic != italic;
    let underline_changed = *first || *last_underline != underline;

    // For spaces, FG is invisible (no glyph to color), so we never need to emit FG for them.
    // We also don't update last_fg for spaces, so when a real glyph appears next,
    // we compare against the most recent *visible* FG.
    let need_fg = fg_changed && !is_space;

    // BG always matters (it fills the cell), so we need it when changed
    let need_bg = bg_changed;
    let need_style = bold_changed || italic_changed || underline_changed;

    if need_fg || need_bg || need_style {
        if render == Render::Irc && !as_preview {
            // IRC Path: Styles are toggled, colors are absolute.
            // Newlines reset style/color state in most IRC clients.
            if need_style {
                if *first {
                    // Start of line: assume all styles are OFF.
                    // Only toggle if we want them ON.
                    if bold {
                        out.push('\x02');
                    }
                    if italic {
                        out.push('\x1D');
                    }
                    if underline {
                        out.push('\x1F');
                    }
                } else {
                    // Middle of line: toggle if state changed.
                    if bold_changed {
                        out.push('\x02');
                    }
                    if italic_changed {
                        out.push('\x1D');
                    }
                    if underline_changed {
                        out.push('\x1F');
                    }
                }
                *last_bold = bold;
                *last_italic = italic;
                *last_underline = underline;
            }

            if need_fg || need_bg {
                let effective_fg = if is_space {
                    lookahead_fg.or(last_fg.as_ref()).unwrap_or(&fg)
                } else {
                    &fg
                };

                let grayscale_tolerance = 5;

                let fg_idx = match effective_fg {
                    Colour::Index(i) => *i as usize,
                    Colour::RGB(rgb) => {
                        let col = make_rgb_u32(rgb);
                        if !is_near_grayscale(col, grayscale_tolerance) {
                            nearest_distinctly_chromatic_hex_colour(
                                col,
                                &IRC99,
                                grayscale_tolerance,
                            )
                            .unwrap_or_else(|| nearest_hex_colour_fast(col, &IRC99))
                                as usize
                        } else {
                            nearest_hex_colour_fast(col, &IRC99) as usize
                        }
                    }
                };

                if *first || need_bg {
                    // If it's the first char or BG changed, we MUST specify BG if it exists.
                    if let Some(bg_c) = &bg {
                        let bg_idx = match bg_c {
                            Colour::Index(i) => *i as usize,
                            Colour::RGB(rgb) => {
                                let col = make_rgb_u32(rgb);
                                nearest_hex_colour_fast(col, &IRC99) as usize
                            }
                        };
                        write!(out, "\x03{},{}", fg_idx, bg_idx).unwrap();
                    } else {
                        write!(out, "\x03{}", fg_idx).unwrap();
                    }
                } else if need_fg {
                    // If only FG changed, and BG is the same as before,
                    // we only emit FG. (Note: Some clients might reset BG,
                    // but the user explicitly requested this optimization).
                    write!(out, "\x03{}", fg_idx).unwrap();
                }

                *last_fg = Some(effective_fg.clone());
                *last_bg = bg.clone();
            }
        } else {
            // ANSI / Preview Path: Consolidated escape sequences

            // Determine changes relative to the reset/default state on 'first'
            let fg_changed = if *first {
                true
            } else {
                last_fg.as_ref() != Some(&fg)
            };
            let bg_changed = if *first {
                bg.is_some()
            } else {
                last_bg.as_ref() != bg.as_ref()
            };
            let bold_changed = if *first { bold } else { *last_bold != bold };
            let italic_changed = if *first {
                italic
            } else {
                *last_italic != italic
            };
            let underline_changed = if *first {
                underline
            } else {
                *last_underline != underline
            };

            let need_fg = fg_changed && !is_space;
            let need_bg = bg_changed;
            let need_style = bold_changed || italic_changed || underline_changed;

            if need_fg || need_bg || need_style {
                out.push_str("\x1b[");
                let mut parts = Vec::new();

                if bold_changed {
                    parts.push(if bold {
                        "1".to_string()
                    } else {
                        "22".to_string()
                    });
                }
                if italic_changed {
                    parts.push(if italic {
                        "3".to_string()
                    } else {
                        "23".to_string()
                    });
                }
                if underline_changed {
                    parts.push(if underline {
                        "4".to_string()
                    } else {
                        "24".to_string()
                    });
                }

                if need_fg {
                    match fg {
                        Colour::RGB(rgb) => {
                            if as_preview || render == Render::Ansi24 {
                                parts.push(format!("38;2;{};{};{}", rgb[0], rgb[1], rgb[2]));
                            } else {
                                let col = make_rgb_u32(&rgb);
                                let idx = if !is_near_grayscale(col, 5) {
                                    nearest_distinctly_chromatic_hex_colour(col, &ANSI256, 5)
                                        .unwrap_or_else(|| nearest_hex_colour_fast(col, &ANSI256))
                                } else {
                                    nearest_hex_colour_fast(col, &ANSI256)
                                };
                                parts.push(format!("38;5;{}", idx));
                            }
                        }
                        Colour::Index(i) => {
                            if as_preview || render == Render::Ansi24 {
                                let pal: &[u32] = if render == Render::Irc {
                                    &IRC99
                                } else {
                                    &ANSI256
                                };
                                let [r, g, b] = unpack_rgb(pal[i as usize]);
                                parts.push(format!("38;2;{};{};{}", r, g, b));
                            } else {
                                parts.push(format!("38;5;{}", i));
                            }
                        }
                    }
                    *last_fg = Some(fg.clone());
                }

                if need_bg {
                    if let Some(bg_c) = &bg {
                        match bg_c {
                            Colour::RGB(rgb) => {
                                if as_preview || render == Render::Ansi24 {
                                    parts.push(format!("48;2;{};{};{}", rgb[0], rgb[1], rgb[2]));
                                } else {
                                    let idx = nearest_hex_colour_fast(make_rgb_u32(rgb), &ANSI256);
                                    parts.push(format!("48;5;{}", idx));
                                }
                            }
                            Colour::Index(i) => {
                                if as_preview || render == Render::Ansi24 {
                                    let pal: &[u32] = if render == Render::Irc {
                                        &IRC99
                                    } else {
                                        &ANSI256
                                    };
                                    let [r, g, b] = unpack_rgb(pal[*i as usize]);
                                    parts.push(format!("48;2;{};{};{}", r, g, b));
                                } else {
                                    parts.push(format!("48;5;{}", i));
                                }
                            }
                        }
                    } else {
                        parts.push("49".to_string());
                    }
                    *last_bg = bg.clone();
                }

                write!(out, "{}m", parts.join(";")).unwrap();
            }

            *last_bold = bold;
            *last_italic = italic;
            *last_underline = underline;
        }
    }

    *first = false;
    out.push(glyph);
}

#[derive(Clone, Debug)]
pub struct RenderedCell {
    pub char: char,
    pub fg: [u8; 3],
    pub bg: Option<[u8; 3]>,
    pub inverted: bool,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

#[derive(Clone)]
pub struct RenderProfile {
    pub prepare_canvas: std::time::Duration,
    pub glyph_match: std::time::Duration,
    pub smoothing_prepare: std::time::Duration,
    pub smoothing_search: std::time::Duration,
    pub shape_refine: std::time::Duration,
    pub final_score: std::time::Duration,
    pub encode: std::time::Duration,
    pub contour_cache_hits: u64,
    pub contour_cache_misses: u64,
}

impl Default for RenderProfile {
    fn default() -> Self {
        Self {
            prepare_canvas: std::time::Duration::ZERO,
            glyph_match: std::time::Duration::ZERO,
            smoothing_prepare: std::time::Duration::ZERO,
            smoothing_search: std::time::Duration::ZERO,
            shape_refine: std::time::Duration::ZERO,
            final_score: std::time::Duration::ZERO,
            encode: std::time::Duration::ZERO,
            contour_cache_hits: 0,
            contour_cache_misses: 0,
        }
    }
}

impl RenderProfile {
    pub fn summary(&self) -> String {
        let millis = |duration: std::time::Duration| duration.as_secs_f64() * 1000.0;
        format!(
            "prepare={:.1}ms match={:.1}ms smooth-prep={:.1}ms smooth={:.1}ms shapes={:.1}ms score={:.1}ms encode={:.1}ms contour-cache={}/{}",
            millis(self.prepare_canvas),
            millis(self.glyph_match),
            millis(self.smoothing_prepare),
            millis(self.smoothing_search),
            millis(self.shape_refine),
            millis(self.final_score),
            millis(self.encode),
            self.contour_cache_hits,
            self.contour_cache_hits + self.contour_cache_misses,
        )
    }
}

#[derive(Clone)]
pub struct RenderResult {
    pub content: String,
    pub save_content: Option<String>,
    pub error_count: u64,
    pub total_pixels: u64,
    pub dynamic_scaling_count: u64,
    pub grid: Vec<Vec<RenderedCell>>,
    pub score: f64,
    pub longest_line_bytes: usize,
    pub initial_time: std::time::Duration,
    pub profile: RenderProfile,
}

impl From<&str> for RenderResult {
    fn from(s: &str) -> Self {
        RenderResult {
            content: s.to_string(),
            save_content: None,
            error_count: 0,
            total_pixels: 0,
            dynamic_scaling_count: 0,
            grid: Vec::new(),
            score: 0.0,
            longest_line_bytes: 0,
            initial_time: std::time::Duration::ZERO,
            profile: RenderProfile::default(),
        }
    }
}

/// Re-emit content/save_content from the cached grid with overlays applied.
/// This is O(grid_cells) and never re-runs image processing — safe to call on every keystroke.
pub fn apply_overlays_to_result(
    base: &RenderResult,
    overlays: &[crate::args::TextOverlay],
    render: Render,
    as_preview: bool,
    grayscale_tolerance: u8,
) -> RenderResult {
    if base.grid.is_empty() || overlays.is_empty() {
        // Nothing to apply; just return a clone with corrected content
        return base.clone();
    }

    let mut content = String::new();
    let mut alt_out = String::new();
    let mut grid = base.grid.clone();
    let prepared_overlays: Vec<_> = overlays.iter().map(PreparedTextOverlay::new).collect();

    for (y, row) in base.grid.iter().enumerate() {
        let mut last_fg: Option<Colour> = None;
        let mut last_bg: Option<Colour> = None;
        let mut last_bold = false;
        let mut last_italic = false;
        let mut last_underline = false;
        let mut first = true;
        let mut alt_last_fg: Option<Colour> = None;
        let mut alt_last_bg: Option<Colour> = None;
        let mut alt_last_bold = false;
        let mut alt_last_italic = false;
        let mut alt_last_underline = false;
        let mut alt_first = true;

        if (render != Render::Irc || as_preview) && y == 0 {
            content.push_str("\x1b[0m");
        }
        // IRC newlines reset formatting implicitly, no need for \x0f at start of line.
        // alt_out (IRC) also doesn't need a leading reset.

        for (x, cell) in row.iter().enumerate() {
            let cx = x as i32;
            let cy = y as i32;

            // Check if any overlay covers this cell
            let mut ov_char: Option<char> = None;
            let mut ov_fg: Option<[u8; 3]> = None;
            let mut ov_bg: Option<[u8; 3]> = None;
            let mut ov_bold = false;
            let mut ov_italic = false;
            let mut ov_underline = false;

            for prepared in &prepared_overlays {
                let ov = prepared.overlay;
                if let Some(opt_c) = prepared.at(cx, cy) {
                    if (ov.transparent_spaces || ov.figlet_enabled())
                        && opt_c.is_none_or(|character| character.is_whitespace())
                    {
                        continue;
                    }
                    if opt_c.is_none() && ov.bg.is_none() {
                        continue;
                    }
                    let ch = opt_c.unwrap_or(' ');
                    ov_char = Some(ch);
                    ov_bold = ov.bold;
                    ov_italic = ov.italic;
                    ov_underline = ov.underline;
                    ov_fg = ov.fg.as_ref().map(|spec| match spec {
                        crate::args::ColorSpec::Rgb(rgb) => *rgb,
                        crate::args::ColorSpec::Index(i) => {
                            let palette = match render {
                                Render::Irc => &IRC99[..],
                                _ => &ANSI256[..],
                            };
                            let hex = if (*i as usize) < palette.len() {
                                palette[*i as usize]
                            } else {
                                0xFFFFFF
                            };
                            [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8]
                        }
                    });
                    ov_bg = ov.bg.as_ref().map(|spec| match spec {
                        crate::args::ColorSpec::Rgb(rgb) => *rgb,
                        crate::args::ColorSpec::Index(i) => {
                            let palette = match render {
                                Render::Irc => &IRC99[..],
                                _ => &ANSI256[..],
                            };
                            let hex = if (*i as usize) < palette.len() {
                                palette[*i as usize]
                            } else {
                                0x000000
                            };
                            [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8]
                        }
                    });
                    break;
                }
            }

            let overlay_applied = ov_char.is_some();
            let (draw_fg_raw, draw_bg_raw, draw_char, draw_bold, draw_italic, draw_underline) =
                if let Some(ch) = ov_char {
                    let fg_raw = ov_fg.unwrap_or(cell.fg);
                    let bg_raw = ov_bg.or(Some(cell.bg.unwrap_or([0, 0, 0])));
                    (fg_raw, bg_raw, ch, ov_bold, ov_italic, ov_underline)
                } else {
                    (
                        cell.fg,
                        cell.bg,
                        cell.char,
                        cell.bold,
                        cell.italic,
                        cell.underline,
                    )
                };

            if overlay_applied {
                grid[y][x] = RenderedCell {
                    char: draw_char,
                    fg: draw_fg_raw,
                    bg: draw_bg_raw,
                    inverted: false,
                    bold: draw_bold,
                    italic: draw_italic,
                    underline: draw_underline,
                };
            }

            let fg_colour = Colour::RGB(draw_fg_raw);
            let bg_colour = draw_bg_raw.map(Colour::RGB);

            emit_colourized(
                &mut content,
                render,
                as_preview,
                fg_colour.clone(),
                bg_colour.clone(),
                draw_char,
                &mut first,
                &mut last_fg,
                &mut last_bg,
                draw_bold,
                draw_italic,
                draw_underline,
                &mut last_bold,
                &mut last_italic,
                &mut last_underline,
                None,
            );

            if as_preview && render == Render::Irc {
                // IRC alt output: quantize RGB → IRC99
                let irc_colour_from_rgb = |rgb: [u8; 3]| -> Colour {
                    let col = make_rgb_u32(&rgb);
                    let idx = if !is_near_grayscale(col, grayscale_tolerance) {
                        nearest_distinctly_chromatic_hex_colour(col, &IRC99, grayscale_tolerance)
                            .unwrap_or_else(|| nearest_hex_colour_fast(col, &IRC99))
                    } else {
                        nearest_hex_colour_fast(col, &IRC99)
                    };
                    Colour::Index(idx)
                };
                let alt_fg = irc_colour_from_rgb(draw_fg_raw);
                let alt_bg = draw_bg_raw.map(|rgb| irc_colour_from_rgb(rgb));
                emit_colourized(
                    &mut alt_out,
                    Render::Irc,
                    false,
                    alt_fg,
                    alt_bg,
                    draw_char,
                    &mut alt_first,
                    &mut alt_last_fg,
                    &mut alt_last_bg,
                    draw_bold,
                    draw_italic,
                    draw_underline,
                    &mut alt_last_bold,
                    &mut alt_last_italic,
                    &mut alt_last_underline,
                    None,
                );
            }
        }

        let end_seq = match render {
            Render::Ansi | Render::Ansi24 => "\x1b[0m\n",
            Render::Irc => "\x0f\n",
        };
        content.push_str(end_seq);
        if as_preview && render == Render::Irc {
            alt_out.push_str("\x0f\n");
        }
    }

    let content = content.trim_end_matches('\n').to_string();
    let save_content = if as_preview && render == Render::Irc {
        Some(alt_out.trim_end_matches('\n').to_string())
    } else {
        None
    };

    let longest_line_bytes = if let Some(ref s) = save_content {
        s.lines().map(|l| l.len()).max().unwrap_or(0)
    } else {
        content.lines().map(|l| l.len()).max().unwrap_or(0)
    };

    RenderResult {
        content,
        save_content,
        grid,
        error_count: base.error_count,
        total_pixels: base.total_pixels,
        dynamic_scaling_count: base.dynamic_scaling_count,
        score: base.score,
        longest_line_bytes,
        initial_time: base.initial_time,
        profile: base.profile.clone(),
    }
}

#[derive(Clone)]
struct BlockData {
    fg: [u8; 3],
    bg: [u8; 3],
    fg_col: Option<Colour>,
    bg_col: Option<Colour>,
    alt_fg_col: Option<Colour>,
    alt_bg_col: Option<Colour>,
}

#[derive(Clone, Copy, Default)]
struct Complex32 {
    re: f32,
    im: f32,
}

const MAX_RGB_DISTANCE_SQUARED: f32 = (255 * 255 * 3) as f32;
const NEIGHBORHOOD_RADIUS: usize = 1;

#[inline(always)]
fn normalized_rgb_distance(a: [u8; 3], b: [u8; 3]) -> f32 {
    let dr = a[0] as f32 - b[0] as f32;
    let dg = a[1] as f32 - b[1] as f32;
    let db = a[2] as f32 - b[2] as f32;
    ((dr * dr + dg * dg + db * db) / MAX_RGB_DISTANCE_SQUARED).sqrt()
}

#[inline(always)]
fn complex_mul(a: Complex32, b: Complex32) -> Complex32 {
    Complex32 {
        re: a.re * b.re - a.im * b.im,
        im: a.re * b.im + a.im * b.re,
    }
}

fn fft_in_place(values: &mut [Complex32]) {
    let n = values.len();
    if n <= 1 {
        return;
    }
    debug_assert!(n.is_power_of_two());

    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            values.swap(i, j);
        }
    }

    let mut len = 2usize;
    while len <= n {
        let angle = -2.0 * PI / len as f32;
        let wlen = Complex32 {
            re: angle.cos(),
            im: angle.sin(),
        };

        for start in (0..n).step_by(len) {
            let mut w = Complex32 { re: 1.0, im: 0.0 };
            for i in 0..(len / 2) {
                let even = values[start + i];
                let odd = complex_mul(values[start + i + len / 2], w);
                values[start + i] = Complex32 {
                    re: even.re + odd.re,
                    im: even.im + odd.im,
                };
                values[start + i + len / 2] = Complex32 {
                    re: even.re - odd.re,
                    im: even.im - odd.im,
                };
                w = complex_mul(w, wlen);
            }
        }

        len <<= 1;
    }
}

#[inline(always)]
fn hann_weight(i: usize, len: usize) -> f32 {
    if len <= 1 {
        1.0
    } else {
        0.5 - 0.5 * ((2.0 * PI * i as f32) / (len - 1) as f32).cos()
    }
}

fn spectral_high_frequency_score(raster: &[[u8; 3]], width: usize, height: usize) -> f32 {
    if width == 0 || height == 0 {
        return 0.0;
    }

    let fft_w = width.next_power_of_two().max(1);
    let fft_h = height.next_power_of_two().max(1);
    let half_w = (fft_w / 2).max(1) as f32;
    let half_h = (fft_h / 2).max(1) as f32;
    let mut total_energy = 0.0f64;
    let mut high_energy = 0.0f64;

    for channel in 0..3 {
        let mean =
            raster.iter().map(|px| px[channel] as f32).sum::<f32>() / (width * height) as f32;
        let mut spectrum = vec![Complex32::default(); fft_w * fft_h];

        for y in 0..height {
            let wy = hann_weight(y, height);
            for x in 0..width {
                let wx = hann_weight(x, width);
                let sample = (raster[y * width + x][channel] as f32 - mean) / 255.0;
                spectrum[y * fft_w + x].re = sample * wx * wy;
            }
        }

        for row in 0..fft_h {
            let start = row * fft_w;
            fft_in_place(&mut spectrum[start..start + fft_w]);
        }

        let mut column = vec![Complex32::default(); fft_h];
        for x in 0..fft_w {
            for y in 0..fft_h {
                column[y] = spectrum[y * fft_w + x];
            }
            fft_in_place(&mut column);
            for y in 0..fft_h {
                spectrum[y * fft_w + x] = column[y];
            }
        }

        for y in 0..fft_h {
            let fy = y.min(fft_h - y) as f32 / half_h;
            for x in 0..fft_w {
                if x == 0 && y == 0 {
                    continue;
                }
                let fx = x.min(fft_w - x) as f32 / half_w;
                let radius = ((fx * fx + fy * fy).sqrt() / 2f32.sqrt()).min(1.0);
                let weight = radius * radius;
                let v = spectrum[y * fft_w + x];
                let energy = (v.re * v.re + v.im * v.im) as f64;
                total_energy += energy;
                high_energy += energy * weight as f64;
            }
        }
    }

    if total_energy <= 1e-9 {
        0.0
    } else {
        (100.0 * (high_energy / total_energy) as f32).min(100.0)
    }
}

#[derive(Clone)]
struct LocalPatch {
    pixels: Vec<[u8; 3]>,
    width: usize,
    height: usize,
    cell_offset_x: usize,
    cell_offset_y: usize,
}

#[derive(Clone, Copy)]
struct BoundarySegment {
    start: (usize, usize),
    end: (usize, usize),
}

fn extract_local_patch(
    raster: &[[u8; 3]],
    cell_x: usize,
    cell_y: usize,
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
) -> LocalPatch {
    let patch_cells = NEIGHBORHOOD_RADIUS * 2 + 1;
    let width = patch_cells * gw;
    let height = patch_cells * gh;
    let raster_width = grid_w * gw;
    let mut pixels = vec![[0u8; 3]; width * height];

    for patch_cell_y in 0..patch_cells {
        let src_cell_y = cell_y
            .saturating_add(patch_cell_y)
            .saturating_sub(NEIGHBORHOOD_RADIUS)
            .min(grid_h - 1);

        for local_y in 0..gh {
            let dst_row = (patch_cell_y * gh + local_y) * width;
            let src_row = (src_cell_y * gh + local_y) * raster_width;

            for patch_cell_x in 0..patch_cells {
                let src_cell_x = cell_x
                    .saturating_add(patch_cell_x)
                    .saturating_sub(NEIGHBORHOOD_RADIUS)
                    .min(grid_w - 1);
                let dst_idx = dst_row + patch_cell_x * gw;
                let src_idx = src_row + src_cell_x * gw;
                pixels[dst_idx..dst_idx + gw].copy_from_slice(&raster[src_idx..src_idx + gw]);
            }
        }
    }

    LocalPatch {
        pixels,
        width,
        height,
        cell_offset_x: NEIGHBORHOOD_RADIUS * gw,
        cell_offset_y: NEIGHBORHOOD_RADIUS * gh,
    }
}

fn extract_center_cell_pixels(
    raster: &[[u8; 3]],
    cell_x: usize,
    cell_y: usize,
    grid_w: usize,
    gw: usize,
    gh: usize,
) -> Vec<[u8; 3]> {
    let raster_width = grid_w * gw;
    let mut pixels = vec![[0u8; 3]; gw * gh];

    for py in 0..gh {
        let src_y = cell_y * gh + py;
        let src_x = cell_x * gw;
        let src_idx = src_y * raster_width + src_x;
        let dst_idx = py * gw;
        pixels[dst_idx..dst_idx + gw].copy_from_slice(&raster[src_idx..src_idx + gw]);
    }

    pixels
}

fn paint_cell_pixels(
    pixels: &mut [[u8; 3]],
    width: usize,
    offset_x: usize,
    offset_y: usize,
    data: &BlockData,
    glyph: &FastGlyph,
    inv: bool,
    gw: usize,
    gh: usize,
) {
    let inv_u8 = inv as u8;
    for py in 0..gh {
        for px in 0..gw {
            let bit = glyph.bitmap[py * gw + px];
            pixels[(offset_y + py) * width + offset_x + px] = if (bit ^ inv_u8) == 1 {
                data.fg
            } else {
                data.bg
            }
        }
    }
}

fn paint_preview_glyph_at(
    pixels: &mut [[u8; 3]],
    width: usize,
    offset_x: usize,
    offset_y: usize,
    data: &BlockData,
    glyph: &FastGlyph,
    inverted: bool,
    gw: usize,
    gh: usize,
) {
    let cell_width = crate::contour_score::PREVIEW_CELL_WIDTH;
    let cell_height = crate::contour_score::PREVIEW_CELL_HEIGHT;
    for y in 0..cell_height {
        let source_y = y * gh / cell_height;
        for x in 0..cell_width {
            let source_x = x * gw / cell_width;
            let bit = glyph.bitmap[source_y * gw + source_x] ^ inverted as u8;
            pixels[(offset_y + y) * width + offset_x + x] =
                if bit == 1 { data.fg } else { data.bg };
        }
    }
}

fn paint_preview_center_glyph(
    pixels: &mut [[u8; 3]],
    width: usize,
    data: &BlockData,
    glyph: &FastGlyph,
    inverted: bool,
    gw: usize,
    gh: usize,
) {
    paint_preview_glyph_at(
        pixels,
        width,
        NEIGHBORHOOD_RADIUS * crate::contour_score::PREVIEW_CELL_WIDTH,
        NEIGHBORHOOD_RADIUS * crate::contour_score::PREVIEW_CELL_HEIGHT,
        data,
        glyph,
        inverted,
        gw,
        gh,
    );
}

fn paint_global_preview_cell(
    pixels: &mut [[u8; 3]],
    width: usize,
    cell_idx: usize,
    grid_w: usize,
    data: &BlockData,
    glyph: &FastGlyph,
    inverted: bool,
    gw: usize,
    gh: usize,
) {
    let cell_x = cell_idx % grid_w;
    let cell_y = cell_idx / grid_w;
    paint_preview_glyph_at(
        pixels,
        width,
        cell_x * crate::contour_score::PREVIEW_CELL_WIDTH,
        cell_y * crate::contour_score::PREVIEW_CELL_HEIGHT,
        data,
        glyph,
        inverted,
        gw,
        gh,
    );
}

fn local_patch_frequency_score(patch_pixels: &[[u8; 3]], width: usize, height: usize) -> f32 {
    spectral_high_frequency_score(patch_pixels, width, height)
}

fn segment_intersects_center_cell(
    segment: &BoundarySegment,
    patch: &LocalPatch,
    gw: usize,
    gh: usize,
) -> bool {
    let x0 = patch.cell_offset_x;
    let y0 = patch.cell_offset_y;
    let x1 = x0 + gw;
    let y1 = y0 + gh;

    let min_x = segment.start.0.min(segment.end.0);
    let max_x = segment.start.0.max(segment.end.0);
    let min_y = segment.start.1.min(segment.end.1);
    let max_y = segment.start.1.max(segment.end.1);

    max_x >= x0 && min_x <= x1 && max_y >= y0 && min_y <= y1
}

fn component_endpoint_span(vertices: &[(usize, usize)]) -> f32 {
    let mut best = 0usize;
    for (i, &(ax, ay)) in vertices.iter().enumerate() {
        for &(bx, by) in &vertices[i + 1..] {
            best = best.max(ax.abs_diff(bx) + ay.abs_diff(by));
        }
    }
    best as f32
}

fn build_boundary_segments_into(
    pixels: &[[u8; 3]],
    width: usize,
    height: usize,
    segments: &mut Vec<BoundarySegment>,
) {
    if width >= 2 {
        for y in 0..height {
            let row = y * width;
            for x in 1..width {
                if raster_boundary_strength(pixels[row + x - 1], pixels[row + x])
                    >= DISCONTINUITY_EDGE_THRESHOLD
                {
                    segments.push(BoundarySegment {
                        start: (x, y),
                        end: (x, y + 1),
                    });
                }
            }
        }
    }

    if height >= 2 {
        for y in 1..height {
            for x in 0..width {
                if raster_boundary_strength(pixels[(y - 1) * width + x], pixels[y * width + x])
                    >= DISCONTINUITY_EDGE_THRESHOLD
                {
                    segments.push(BoundarySegment {
                        start: (x, y),
                        end: (x + 1, y),
                    });
                }
            }
        }
    }
}

struct DiscontinuityScratch {
    segments: Vec<BoundarySegment>,
    edges: Vec<[u16; 4]>,
    degrees: Vec<u8>,
    modified_vertices: Vec<usize>,
    visited: Vec<bool>,
    queue: std::collections::VecDeque<usize>,
    component: Vec<usize>,
    vertices: Vec<(usize, usize)>,
}

thread_local! {
    static DIS_SCRATCH: std::cell::RefCell<DiscontinuityScratch> = std::cell::RefCell::new(DiscontinuityScratch {
        segments: Vec::with_capacity(128),
        edges: Vec::new(),
        degrees: Vec::new(),
        modified_vertices: Vec::with_capacity(128),
        visited: Vec::with_capacity(128),
        queue: std::collections::VecDeque::with_capacity(64),
        component: Vec::with_capacity(64),
        vertices: Vec::with_capacity(64),
    });
}

fn local_patch_boundary_complexity_score(
    raster: &[[u8; 3]],
    cell_x: usize,
    cell_y: usize,
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
) -> f32 {
    let patch = extract_local_patch(raster, cell_x, cell_y, grid_w, grid_h, gw, gh);

    let result = DIS_SCRATCH.with(|scratch_cell| {
        let mut scratch = scratch_cell.borrow_mut();

        scratch.segments.clear();
        build_boundary_segments_into(
            &patch.pixels,
            patch.width,
            patch.height,
            &mut scratch.segments,
        );
        if scratch.segments.is_empty() {
            return 0.0;
        }

        let num_vertices = (patch.height + 2) * (patch.width + 2);
        if scratch.edges.len() < num_vertices {
            scratch.edges.resize(num_vertices, [u16::MAX; 4]);
            scratch.degrees.resize(num_vertices, 0);
        }

        scratch.modified_vertices.clear();
        let width_stride = patch.width + 2;

        let temp_segments = scratch.segments.clone();
        for (idx, segment) in temp_segments.iter().enumerate() {
            let seg_idx = idx as u16;

            let v1 = segment.start.1 * width_stride + segment.start.0;
            let deg1 = scratch.degrees[v1] as usize;
            if deg1 < 4 {
                scratch.edges[v1][deg1] = seg_idx;
                scratch.degrees[v1] += 1;
                if deg1 == 0 {
                    scratch.modified_vertices.push(v1);
                }
            }

            let v2 = segment.end.1 * width_stride + segment.end.0;
            let deg2 = scratch.degrees[v2] as usize;
            if deg2 < 4 {
                scratch.edges[v2][deg2] = seg_idx;
                scratch.degrees[v2] += 1;
                if deg2 == 0 {
                    scratch.modified_vertices.push(v2);
                }
            }
        }

        let segments_len = scratch.segments.len();
        scratch.visited.clear();
        scratch.visited.resize(segments_len, false);

        let mut total_penalty = 0.0f32;
        let mut counted_components = 0u32;

        for start_idx in 0..segments_len {
            if scratch.visited[start_idx] {
                continue;
            }

            scratch.queue.clear();
            scratch.queue.push_back(start_idx);

            scratch.component.clear();
            let mut relevant = false;

            while let Some(seg_idx) = scratch.queue.pop_front() {
                if scratch.visited[seg_idx] {
                    continue;
                }
                scratch.visited[seg_idx] = true;
                let segment = scratch.segments[seg_idx];
                relevant |= segment_intersects_center_cell(&segment, &patch, gw, gh);
                scratch.component.push(seg_idx);

                let v1 = segment.start.1 * width_stride + segment.start.0;
                let deg1 = scratch.degrees[v1] as usize;
                for i in 0..deg1 {
                    let next_seg = scratch.edges[v1][i] as usize;
                    if !scratch.visited[next_seg] {
                        scratch.queue.push_back(next_seg);
                    }
                }

                let v2 = segment.end.1 * width_stride + segment.end.0;
                let deg2 = scratch.degrees[v2] as usize;
                for i in 0..deg2 {
                    let next_seg = scratch.edges[v2][i] as usize;
                    if !scratch.visited[next_seg] {
                        scratch.queue.push_back(next_seg);
                    }
                }
            }

            if relevant {
                let mut min_x = usize::MAX;
                let mut min_y = usize::MAX;
                let mut max_x = 0usize;
                let mut max_y = 0usize;

                scratch.vertices.clear();

                let temp_component = scratch.component.clone();
                for &seg_idx in &temp_component {
                    let segment = scratch.segments[seg_idx];

                    min_x = min_x.min(segment.start.0).min(segment.end.0);
                    min_y = min_y.min(segment.start.1).min(segment.end.1);
                    max_x = max_x.max(segment.start.0).max(segment.end.0);
                    max_y = max_y.max(segment.start.1).max(segment.end.1);

                    let v1 = segment.start.1 * width_stride + segment.start.0;
                    if scratch.degrees[v1] == 1 {
                        scratch.vertices.push(segment.start);
                    }
                    let v2 = segment.end.1 * width_stride + segment.end.0;
                    if scratch.degrees[v2] == 1 {
                        scratch.vertices.push(segment.end);
                    }
                }

                scratch.vertices.sort_unstable();
                scratch.vertices.dedup();

                let length = scratch.component.len() as f32;
                let baseline = if scratch.vertices.len() >= 2 {
                    component_endpoint_span(&scratch.vertices).max(1.0)
                } else {
                    (max_x.saturating_sub(min_x) + max_y.saturating_sub(min_y)).max(1) as f32
                };

                total_penalty += (length - baseline).max(0.0);
                counted_components += 1;
            }
        }

        let temp_modified_vertices = scratch.modified_vertices.clone();
        for &v in &temp_modified_vertices {
            scratch.edges[v] = [u16::MAX; 4];
            scratch.degrees[v] = 0;
        }

        if counted_components == 0 {
            0.0
        } else {
            (100.0 * total_penalty / (counted_components as f32 * (gw + gh).max(1) as f32))
                .min(100.0)
        }
    });

    result
}

fn save_rgb_pixels_as_png(
    path: &Path,
    pixels: &[[u8; 3]],
    width: usize,
    height: usize,
) -> std::io::Result<()> {
    let mut image = RgbImage::new(width as u32, height as u32);
    for y in 0..height {
        for x in 0..width {
            image.put_pixel(x as u32, y as u32, Rgb(pixels[y * width + x]));
        }
    }
    image.save(path).map_err(std::io::Error::other)
}

fn with_grid_overlay(
    pixels: &[[u8; 3]],
    width: usize,
    height: usize,
    gw: usize,
    gh: usize,
) -> Vec<[u8; 3]> {
    let mut out = pixels.to_vec();
    let grid_color = [255, 0, 255];

    if gw > 0 {
        for x in (0..width).step_by(gw) {
            for y in 0..height {
                out[y * width + x] = grid_color;
            }
        }
    }

    if gh > 0 {
        for y in (0..height).step_by(gh) {
            let row = y * width;
            for x in 0..width {
                out[row + x] = grid_color;
            }
        }
    }

    out
}

fn with_center_outline(patch: &LocalPatch, gw: usize, gh: usize) -> Vec<[u8; 3]> {
    let mut out = patch.pixels.clone();
    if out.is_empty() || patch.width == 0 || patch.height == 0 || gw == 0 || gh == 0 {
        return out;
    }

    let outline = [0, 255, 255];
    let x0 = patch.cell_offset_x.min(patch.width - 1);
    let y0 = patch.cell_offset_y.min(patch.height - 1);
    let x1 = (patch.cell_offset_x + gw)
        .saturating_sub(1)
        .min(patch.width - 1);
    let y1 = (patch.cell_offset_y + gh)
        .saturating_sub(1)
        .min(patch.height - 1);

    for x in x0..=x1 {
        out[y0 * patch.width + x] = outline;
        out[y1 * patch.width + x] = outline;
    }

    for y in y0..=y1 {
        out[y * patch.width + x0] = outline;
        out[y * patch.width + x1] = outline;
    }

    out
}

fn ensure_fft_debug_dir(path: &str) -> Option<PathBuf> {
    let root = PathBuf::from(path);
    match fs::create_dir_all(&root) {
        Ok(_) => Some(root),
        Err(err) => {
            log::warn!(
                "failed to create FFT debug dir '{}': {}",
                root.display(),
                err
            );
            None
        }
    }
}

fn dump_fft_stage_inputs(
    root: &Path,
    stage: &str,
    raster: &[[u8; 3]],
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
) -> std::io::Result<()> {
    let stage_dir = root.join(stage);
    let cells_dir = stage_dir.join("center_cells_raw");
    let raw_dir = stage_dir.join("patches_raw");
    let marked_dir = stage_dir.join("patches_marked");
    fs::create_dir_all(&cells_dir)?;
    fs::create_dir_all(&raw_dir)?;
    fs::create_dir_all(&marked_dir)?;

    let raster_width = grid_w * gw;
    let raster_height = grid_h * gh;
    save_rgb_pixels_as_png(
        &stage_dir.join("rendered_raster.png"),
        raster,
        raster_width,
        raster_height,
    )?;
    let overlaid = with_grid_overlay(raster, raster_width, raster_height, gw, gh);
    save_rgb_pixels_as_png(
        &stage_dir.join("rendered_raster_grid.png"),
        &overlaid,
        raster_width,
        raster_height,
    )?;

    let mut manifest = String::from(
        "cell_x,cell_y,score,patch_width,patch_height,center_offset_x,center_offset_y,center_cell_file,raw_file,marked_file\n",
    );
    for cell_y in 0..grid_h {
        for cell_x in 0..grid_w {
            let patch = extract_local_patch(raster, cell_x, cell_y, grid_w, grid_h, gw, gh);
            let score = local_patch_frequency_score(&patch.pixels, patch.width, patch.height);
            let name = format!("y{:04}_x{:04}.png", cell_y, cell_x);
            let center_pixels = extract_center_cell_pixels(raster, cell_x, cell_y, grid_w, gw, gh);
            save_rgb_pixels_as_png(&cells_dir.join(&name), &center_pixels, gw, gh)?;
            save_rgb_pixels_as_png(
                &raw_dir.join(&name),
                &patch.pixels,
                patch.width,
                patch.height,
            )?;
            let marked = with_center_outline(&patch, gw, gh);
            save_rgb_pixels_as_png(&marked_dir.join(&name), &marked, patch.width, patch.height)?;
            writeln!(
                manifest,
                "{},{},{:.4},{},{},{},{},center_cells_raw/{},patches_raw/{},patches_marked/{}",
                cell_x,
                cell_y,
                score,
                patch.width,
                patch.height,
                patch.cell_offset_x,
                patch.cell_offset_y,
                name,
                name,
                name
            )
            .map_err(std::io::Error::other)?;
        }
    }

    fs::write(stage_dir.join("manifest.csv"), manifest)?;
    Ok(())
}

fn build_rendered_raster(
    grid_data: &[BlockData],
    grid_state: &[(usize, bool)],
    glyphs: &[FastGlyph],
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
) -> Vec<[u8; 3]> {
    let width = grid_w * gw;
    let height = grid_h * gh;
    let mut raster = vec![[0u8; 3]; width * height];

    for cy in 0..grid_h {
        for cx in 0..grid_w {
            let idx = cy * grid_w + cx;
            let (gi, inv) = grid_state[idx];
            paint_cell_pixels(
                &mut raster,
                width,
                cx * gw,
                cy * gh,
                &grid_data[idx],
                &glyphs[gi],
                inv,
                gw,
                gh,
            );
        }
    }

    raster
}

fn build_source_raster(
    image: &AnsiImage,
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
) -> Vec<[u8; 3]> {
    let width = grid_w * gw;
    let mut raster = vec![[0u8; 3]; width * grid_h * gh];
    for cell_y in 0..grid_h {
        for cell_x in 0..grid_w {
            let block = &image.block[cell_y][cell_x];
            for y in 0..gh {
                for x in 0..gw {
                    raster[(cell_y * gh + y) * width + cell_x * gw + x] =
                        unpack_rgb(block.pixels[y][x].orig);
                }
            }
        }
    }
    raster
}

fn preview_contour_score(
    source: &[[u8; 3]],
    output: &[[u8; 3]],
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
    weights: &crate::contour_score::ContourScoreWeights,
) -> crate::contour_score::ReferenceContourScore {
    let width = grid_w * crate::contour_score::PREVIEW_CELL_WIDTH;
    let height = grid_h * crate::contour_score::PREVIEW_CELL_HEIGHT;
    let reference = if weights.fidelity > 0.0 || weights.boundary_balance > 0.0 {
        let (source, _, _) =
            crate::contour_score::normalize_preview_raster(source, grid_w, grid_h, gw, gh);
        crate::contour_score::prepare_reference_with_weights(
            &source,
            width,
            height,
            crate::contour_score::PREVIEW_CELL_WIDTH,
            weights,
        )
    } else {
        crate::contour_score::prepare_reference_with_weights(
            &[],
            width,
            height,
            crate::contour_score::PREVIEW_CELL_WIDTH,
            weights,
        )
    };
    let (output, _, _) =
        crate::contour_score::normalize_preview_raster(output, grid_w, grid_h, gw, gh);
    crate::contour_score::score_rgb_against_prepared_with_weights(
        &reference,
        &output,
        crate::contour_score::PREVIEW_CELL_WIDTH,
        weights,
    )
}

fn contour_score_weights(args: &RenderArgs) -> crate::contour_score::ContourScoreWeights {
    crate::contour_score::ContourScoreWeights {
        bending: args.contour_bending_weight.max(0.0) as f64,
        endpoints: args.contour_endpoint_weight.max(0.0) as f64,
        junctions: args.contour_junction_weight.max(0.0) as f64,
        fragments: args.contour_fragment_weight.max(0.0) as f64,
        fidelity: args.contour_fidelity_weight.max(0.0) as f64,
        boundary_balance: args.contour_boundary_weight.max(0.0) as f64,
        peak_sensitivity: args.contour_peak_weight.clamp(0.0, 1.0) as f64,
    }
}

fn prepare_local_preview_reference(
    source: &[[u8; 3]],
    cell_idx: usize,
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
    weights: &crate::contour_score::ContourScoreWeights,
) -> crate::contour_score::PreparedReference {
    let patch_cells = NEIGHBORHOOD_RADIUS * 2 + 1;
    let width = patch_cells * crate::contour_score::PREVIEW_CELL_WIDTH;
    let height = patch_cells * crate::contour_score::PREVIEW_CELL_HEIGHT;
    if weights.fidelity <= 0.0 && weights.boundary_balance <= 0.0 {
        return crate::contour_score::prepare_reference_with_weights(
            &[],
            width,
            height,
            crate::contour_score::PREVIEW_CELL_WIDTH,
            weights,
        );
    }
    let cell_x = cell_idx % grid_w;
    let cell_y = cell_idx / grid_w;
    let source_patch = extract_local_patch(source, cell_x, cell_y, grid_w, grid_h, gw, gh);
    let (source_preview, _, _) = crate::contour_score::normalize_preview_raster(
        &source_patch.pixels,
        patch_cells,
        patch_cells,
        gw,
        gh,
    );
    crate::contour_score::prepare_reference_with_weights(
        &source_preview,
        width,
        height,
        crate::contour_score::PREVIEW_CELL_WIDTH,
        weights,
    )
}

fn score_local_preview_output(
    reference: &crate::contour_score::PreparedReference,
    output: &[[u8; 3]],
    cell_idx: usize,
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
    weights: &crate::contour_score::ContourScoreWeights,
) -> f64 {
    let cell_x = cell_idx % grid_w;
    let cell_y = cell_idx / grid_w;
    let output_patch = extract_local_patch(output, cell_x, cell_y, grid_w, grid_h, gw, gh);
    let patch_cells = NEIGHBORHOOD_RADIUS * 2 + 1;
    let (output_preview, _, _) = crate::contour_score::normalize_preview_raster(
        &output_patch.pixels,
        patch_cells,
        patch_cells,
        gw,
        gh,
    );
    crate::contour_score::score_rgb_against_prepared_with_weights(
        reference,
        &output_preview,
        crate::contour_score::PREVIEW_CELL_WIDTH,
        weights,
    )
    .total
}

fn score_local_normalized_preview_output(
    reference: &crate::contour_score::PreparedReference,
    output_preview: &[[u8; 3]],
    cell_idx: usize,
    grid_w: usize,
    grid_h: usize,
    weights: &crate::contour_score::ContourScoreWeights,
) -> f64 {
    let cell_x = cell_idx % grid_w;
    let cell_y = cell_idx / grid_w;
    let output_patch = extract_local_patch(
        output_preview,
        cell_x,
        cell_y,
        grid_w,
        grid_h,
        crate::contour_score::PREVIEW_CELL_WIDTH,
        crate::contour_score::PREVIEW_CELL_HEIGHT,
    );
    crate::contour_score::score_rgb_against_prepared_with_weights(
        reference,
        &output_patch.pixels,
        crate::contour_score::PREVIEW_CELL_WIDTH,
        weights,
    )
    .total
}

fn score_local_normalized_preview_details(
    reference: &crate::contour_score::PreparedReference,
    output_preview: &[[u8; 3]],
    cell_idx: usize,
    grid_w: usize,
    grid_h: usize,
    weights: &crate::contour_score::ContourScoreWeights,
) -> crate::contour_score::ReferenceContourScore {
    let cell_x = cell_idx % grid_w;
    let cell_y = cell_idx / grid_w;
    let output_patch = extract_local_patch(
        output_preview,
        cell_x,
        cell_y,
        grid_w,
        grid_h,
        crate::contour_score::PREVIEW_CELL_WIDTH,
        crate::contour_score::PREVIEW_CELL_HEIGHT,
    );
    crate::contour_score::score_rgb_against_prepared_with_weights(
        reference,
        &output_patch.pixels,
        crate::contour_score::PREVIEW_CELL_WIDTH,
        weights,
    )
}

fn best_candidate_pool_pre(costs: &[u64], pool_size: usize) -> Vec<(usize, bool, u64)> {
    if pool_size >= costs.len() {
        let mut pool: Vec<(usize, bool, u64)> = costs
            .iter()
            .enumerate()
            .map(|(idx, &cost)| (idx / 2, idx % 2 == 1, cost))
            .collect();
        pool.sort_by_key(|&(_, _, cost)| cost);
        return pool;
    }

    let mut pool = Vec::with_capacity(pool_size + 1);

    for (idx, &cost) in costs.iter().enumerate() {
        let candidate = (idx / 2, idx % 2 == 1, cost);
        let insert_at = pool
            .iter()
            .position(|&(_, _, existing)| cost < existing)
            .unwrap_or(pool.len());

        if insert_at < pool_size {
            pool.insert(insert_at, candidate);
            if pool.len() > pool_size {
                pool.pop();
            }
        } else if pool.len() < pool_size {
            pool.push(candidate);
        }
    }

    pool
}

fn glyph_states_render_identically(
    a: &FastGlyph,
    a_inverted: bool,
    b: &FastGlyph,
    b_inverted: bool,
) -> bool {
    let inversion = (a_inverted ^ b_inverted) as u8;
    a.bitmap
        .iter()
        .zip(&b.bitmap)
        .all(|(&a_pixel, &b_pixel)| a_pixel == (b_pixel ^ inversion))
}

fn deduplicate_rendered_candidates(
    candidates: Vec<(usize, bool, u64)>,
    glyphs: &[FastGlyph],
    current: (usize, bool),
) -> Vec<(usize, bool, u64)> {
    let mut unique = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if glyph_states_render_identically(
            &glyphs[candidate.0],
            candidate.1,
            &glyphs[current.0],
            current.1,
        ) || unique.iter().any(|&(glyph_idx, inverted, _)| {
            glyph_states_render_identically(
                &glyphs[candidate.0],
                candidate.1,
                &glyphs[glyph_idx],
                inverted,
            )
        }) {
            continue;
        }
        unique.push(candidate);
    }
    unique
}

fn apply_choice_to_raster(
    raster: &mut [[u8; 3]],
    grid_data: &[BlockData],
    glyphs: &[FastGlyph],
    grid_w: usize,
    gw: usize,
    gh: usize,
    cell_idx: usize,
    choice: (usize, bool),
) {
    let cell_x = cell_idx % grid_w;
    let cell_y = cell_idx / grid_w;
    paint_cell_pixels(
        raster,
        grid_w * gw,
        cell_x * gw,
        cell_y * gh,
        &grid_data[cell_idx],
        &glyphs[choice.0],
        choice.1,
        gw,
        gh,
    );
}

const DISCONTINUITY_EDGE_THRESHOLD: f32 = 0.05;
const DISCONTINUITY_ALLOWED_STEP: f32 = 2.0;
const DISCONTINUITY_APPEAR_PENALTY: f32 = 1.5;

#[inline(always)]
fn raster_boundary_strength(a: [u8; 3], b: [u8; 3]) -> f32 {
    normalized_rgb_distance(a, b)
}

fn vertical_boundary_positions(
    raster: &[[u8; 3]],
    width: usize,
    seam_x: usize,
    y: usize,
    search_radius: usize,
) -> Vec<f32> {
    if width < 2 {
        return Vec::new();
    }

    let start = seam_x.saturating_sub(search_radius).max(1);
    let end = (seam_x + search_radius).min(width - 1);
    if start > end {
        return Vec::new();
    }

    let row = y * width;
    let mut positions = Vec::new();

    for x in start..=end {
        let strength = raster_boundary_strength(raster[row + x - 1], raster[row + x]);
        if strength >= DISCONTINUITY_EDGE_THRESHOLD {
            positions.push(x as f32 - 0.5);
        }
    }

    positions
}

fn horizontal_boundary_positions(
    raster: &[[u8; 3]],
    width: usize,
    height: usize,
    x: usize,
    seam_y: usize,
    search_radius: usize,
) -> Vec<f32> {
    if height < 2 {
        return Vec::new();
    }

    let start = seam_y.saturating_sub(search_radius).max(1);
    let end = (seam_y + search_radius).min(height - 1);
    if start > end {
        return Vec::new();
    }

    let mut positions = Vec::new();

    for y in start..=end {
        let strength = raster_boundary_strength(raster[(y - 1) * width + x], raster[y * width + x]);
        if strength >= DISCONTINUITY_EDGE_THRESHOLD {
            positions.push(y as f32 - 0.5);
        }
    }

    positions
}

fn seam_discontinuity_score(rows: &[Vec<f32>], allow_step: f32) -> f32 {
    if rows.len() < 2 {
        return 0.0;
    }

    let mut penalty = 0.0f32;
    let mut had_any_boundary = false;
    let mut comparisons = 0u32;

    for i in 1..rows.len() {
        let prev = &rows[i - 1];
        let curr = &rows[i];
        if prev.is_empty() && curr.is_empty() {
            continue;
        }

        had_any_boundary = true;
        comparisons += 1;

        let at_edge = i == 1 || i + 1 == rows.len();
        if prev.len() != curr.len() && !at_edge {
            penalty += (prev.len().abs_diff(curr.len()) as f32) * DISCONTINUITY_APPEAR_PENALTY;
        }

        let paired = prev.len().min(curr.len());
        for j in 0..paired {
            let jump = (curr[j] - prev[j]).abs();
            if jump > allow_step {
                penalty += jump - allow_step;
            }
        }
    }

    if !had_any_boundary || comparisons == 0 {
        return 0.0;
    }

    let normalized = penalty / (comparisons as f32 * DISCONTINUITY_APPEAR_PENALTY.max(allow_step));
    (normalized * 100.0).min(100.0)
}

fn vertical_seam_discontinuity_score(
    raster: &[[u8; 3]],
    width: usize,
    seam_x: usize,
    y0: usize,
    gh: usize,
    search_radius: usize,
) -> f32 {
    let positions: Vec<Vec<f32>> = (0..gh)
        .map(|dy| vertical_boundary_positions(raster, width, seam_x, y0 + dy, search_radius))
        .collect();
    seam_discontinuity_score(&positions, DISCONTINUITY_ALLOWED_STEP)
}

fn horizontal_seam_discontinuity_score(
    raster: &[[u8; 3]],
    width: usize,
    height: usize,
    x0: usize,
    seam_y: usize,
    gw: usize,
    search_radius: usize,
) -> f32 {
    let positions: Vec<Vec<f32>> = (0..gw)
        .map(|dx| {
            horizontal_boundary_positions(raster, width, height, x0 + dx, seam_y, search_radius)
        })
        .collect();
    seam_discontinuity_score(&positions, DISCONTINUITY_ALLOWED_STEP)
}

fn cell_discontinuity_score(
    curr_idx: usize,
    raster: &[[u8; 3]],
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
) -> f32 {
    let width = grid_w * gw;
    let height = grid_h * gh;
    let cx = curr_idx % grid_w;
    let cy = curr_idx / grid_w;
    let x0 = cx * gw;
    let y0 = cy * gh;
    let search_radius = gw.min(gh).clamp(1, 3);

    let mut total = 0.0f32;
    let mut count = 0u32;

    if cx > 0 {
        total += vertical_seam_discontinuity_score(raster, width, x0, y0, gh, search_radius);
        count += 1;
    }
    if cx + 1 < grid_w {
        total += vertical_seam_discontinuity_score(raster, width, x0 + gw, y0, gh, search_radius);
        count += 1;
    }
    if cy > 0 {
        total +=
            horizontal_seam_discontinuity_score(raster, width, height, x0, y0, gw, search_radius);
        count += 1;
    }
    if cy + 1 < grid_h {
        total += horizontal_seam_discontinuity_score(
            raster,
            width,
            height,
            x0,
            y0 + gh,
            gw,
            search_radius,
        );
        count += 1;
    }

    let seam_score = if count == 0 {
        0.0
    } else {
        total / count as f32
    };

    let perimeter_score =
        local_patch_boundary_complexity_score(raster, cx, cy, grid_w, grid_h, gw, gh);

    seam_score.max(perimeter_score)
}

fn all_cell_discontinuity_scores(
    raster: &[[u8; 3]],
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
) -> Vec<f32> {
    (0..grid_w * grid_h)
        .into_par_iter()
        .map(|idx| cell_discontinuity_score(idx, raster, grid_w, grid_h, gw, gh))
        .collect()
}

#[derive(Clone, Copy)]
struct AffectedCellIndices {
    indices: [usize; 9],
    len: usize,
}

impl std::ops::Deref for AffectedCellIndices {
    type Target = [usize];

    fn deref(&self) -> &Self::Target {
        &self.indices[..self.len]
    }
}

impl IntoIterator for AffectedCellIndices {
    type Item = usize;
    type IntoIter = std::iter::Take<std::array::IntoIter<usize, 9>>;

    fn into_iter(self) -> Self::IntoIter {
        self.indices.into_iter().take(self.len)
    }
}

impl<'a> IntoIterator for &'a AffectedCellIndices {
    type Item = &'a usize;
    type IntoIter = std::slice::Iter<'a, usize>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

fn affected_discontinuity_cell_indices(
    curr_idx: usize,
    grid_w: usize,
    grid_h: usize,
) -> AffectedCellIndices {
    let cx = curr_idx % grid_w;
    let cy = curr_idx / grid_w;
    let x0 = cx.saturating_sub(NEIGHBORHOOD_RADIUS);
    let y0 = cy.saturating_sub(NEIGHBORHOOD_RADIUS);
    let x1 = (cx + NEIGHBORHOOD_RADIUS).min(grid_w - 1);
    let y1 = (cy + NEIGHBORHOOD_RADIUS).min(grid_h - 1);

    let mut indices = [0usize; 9];
    let mut len = 0usize;
    for y in y0..=y1 {
        for x in x0..=x1 {
            indices[len] = y * grid_w + x;
            len += 1;
        }
    }
    AffectedCellIndices { indices, len }
}

/// Lay out overlay text into character cells. When a width is supplied, use
/// Unicode line-break opportunities (spaces, hyphens, CJK boundaries, etc.)
/// and only split a word when it cannot fit on an otherwise empty line.
pub(crate) fn layout_overlay_text(text: &str, wrap_width: Option<usize>) -> Vec<Vec<char>> {
    let Some(width) = wrap_width.filter(|width| *width > 0) else {
        return text
            .split('\n')
            .map(|line| line.trim_end_matches('\r').chars().collect())
            .collect();
    };

    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let paragraph = paragraph.trim_end_matches('\r');
        let chars: Vec<char> = paragraph.chars().collect();
        if chars.is_empty() {
            lines.push(Vec::new());
            continue;
        }
        if chars.iter().all(|ch| ch.is_whitespace()) {
            lines.push(Vec::new());
            continue;
        }

        let byte_to_char = |byte_idx: usize| paragraph[..byte_idx].chars().count();
        let break_points: Vec<usize> = unicode_linebreak::linebreaks(paragraph)
            .map(|(byte_idx, _)| byte_to_char(byte_idx))
            .collect();

        let mut start = 0;
        while start < chars.len() {
            while start < chars.len() && chars[start].is_whitespace() {
                start += 1;
            }
            if start >= chars.len() {
                break;
            }

            let hard_end = (start + width).min(chars.len());
            let end = if hard_end == chars.len() {
                hard_end
            } else {
                break_points
                    .iter()
                    .copied()
                    .filter(|point| {
                        if *point <= start {
                            return false;
                        }
                        let mut visible_end = *point;
                        while visible_end > start && chars[visible_end - 1].is_whitespace() {
                            visible_end -= 1;
                        }
                        visible_end - start <= width
                    })
                    .max()
                    .unwrap_or(hard_end)
            };

            let mut visible_end = end;
            while visible_end > start && chars[visible_end - 1].is_whitespace() {
                visible_end -= 1;
            }
            lines.push(chars[start..visible_end].to_vec());
            start = end;
        }
    }

    if lines.is_empty() {
        lines.push(Vec::new());
    }
    lines
}

struct PreparedTextOverlay<'a> {
    overlay: &'a crate::args::TextOverlay,
    lines: Vec<Vec<char>>,
}

impl<'a> PreparedTextOverlay<'a> {
    fn new(overlay: &'a crate::args::TextOverlay) -> Self {
        let wrap_width = (overlay.wrap && overlay.w > 0).then_some(overlay.w as usize);
        Self {
            overlay,
            lines: layout_overlay_text(&overlay.text, wrap_width),
        }
    }

    fn at(&self, cx: i32, cy: i32) -> Option<Option<char>> {
        let ov = self.overlay;
        if cy < ov.y {
            return None;
        }
        let dy = (cy - ov.y) as usize;

        if ov.w > 0 && ov.h > 0 {
            if cx < ov.x || cx >= ov.x + ov.w || cy >= ov.y + ov.h {
                return None;
            }

            let dx = (cx - ov.x) as usize;
            return Some(self.lines.get(dy).and_then(|line| line.get(dx)).copied());
        }

        if cx < ov.x {
            return None;
        }
        let dx = (cx - ov.x) as usize;
        self.lines
            .get(dy)
            .and_then(|line| line.get(dx))
            .copied()
            .map(Some)
    }
}

struct ScoreFixPrefixStats {
    target_count: usize,
    candidate_trials: usize,
    duplicate_candidates_skipped: usize,
    changes: usize,
    rejected: usize,
    prefix_count: usize,
    best_prefix: usize,
    initial_score: f64,
    best_score: f64,
    priority_refreshes: usize,
    used_band_fast_path: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct ScoreFixGlyphPreview {
    #[allow(dead_code)]
    mask: u128,
    internal_boundary_length: usize,
    left_edge: u16,
    right_edge: u16,
    top_edge: u8,
    bottom_edge: u8,
}

struct ScoreFixBandCache {
    glyph_previews: Vec<ScoreFixGlyphPreview>,
    reference_patch_lengths: Vec<usize>,
    output_patch_lengths: Vec<Option<usize>>,
    global_reference_length: usize,
    global_output_length: usize,
}

#[derive(Clone, Copy, Debug)]
struct ScoreFixBandCandidate {
    current_score: f64,
    candidate_score: f64,
    global_boundary_delta: isize,
}

#[inline]
fn score_fix_band_only(weights: &crate::contour_score::ContourScoreWeights) -> bool {
    weights.boundary_balance > 0.0
        && weights.bending <= 0.0
        && weights.endpoints <= 0.0
        && weights.junctions <= 0.0
        && weights.fragments <= 0.0
        && weights.fidelity <= 0.0
}

fn score_fix_has_at_most_eight_colours<'a>(pixels: impl IntoIterator<Item = &'a [u8; 3]>) -> bool {
    let mut colours = Vec::with_capacity(9);
    for &pixel in pixels {
        if !colours.contains(&pixel) {
            colours.push(pixel);
            if colours.len() > 8 {
                return false;
            }
        }
    }
    true
}

fn score_fix_preview_glyph(glyph: &FastGlyph, gw: usize, gh: usize) -> ScoreFixGlyphPreview {
    let mut mask = 0u128;
    for y in 0..crate::contour_score::PREVIEW_CELL_HEIGHT {
        let source_y = y * gh / crate::contour_score::PREVIEW_CELL_HEIGHT;
        for x in 0..crate::contour_score::PREVIEW_CELL_WIDTH {
            let source_x = x * gw / crate::contour_score::PREVIEW_CELL_WIDTH;
            if glyph.bitmap[source_y * gw + source_x] != 0 {
                mask |= 1u128 << (y * crate::contour_score::PREVIEW_CELL_WIDTH + x);
            }
        }
    }

    // Compare all 112 horizontal and 120 vertical pixel pairs in parallel.
    // The masks exclude row wrapping and the final row respectively.
    let not_right_edge = u128::from_le_bytes([0x7f; 16]);
    let first_fifteen_rows = (1u128 << 120) - 1;
    let horizontal = ((mask ^ (mask >> 1)) & not_right_edge).count_ones() as usize;
    let vertical = ((mask ^ (mask >> 8)) & first_fifteen_rows).count_ones() as usize;
    let mut left_edge = 0u16;
    let mut right_edge = 0u16;
    for y in 0..crate::contour_score::PREVIEW_CELL_HEIGHT {
        left_edge |= (((mask >> (y * crate::contour_score::PREVIEW_CELL_WIDTH)) & 1) as u16)
            << y;
        right_edge |= (((mask
            >> (y * crate::contour_score::PREVIEW_CELL_WIDTH
                + crate::contour_score::PREVIEW_CELL_WIDTH
                - 1))
            & 1) as u16)
            << y;
    }
    ScoreFixGlyphPreview {
        mask,
        internal_boundary_length: horizontal + vertical,
        left_edge,
        right_edge,
        top_edge: mask as u8,
        bottom_edge: (mask >> 120) as u8,
    }
}

#[inline]
fn score_fix_edge_mismatches(
    a_data: &BlockData,
    a_mask: u16,
    a_inverted: bool,
    b_data: &BlockData,
    b_mask: u16,
    b_inverted: bool,
    valid_mask: u16,
) -> usize {
    let a_fg = (a_mask ^ if a_inverted { valid_mask } else { 0 }) & valid_mask;
    let b_fg = (b_mask ^ if b_inverted { valid_mask } else { 0 }) & valid_mask;
    let a_bg = !a_fg & valid_mask;
    let b_bg = !b_fg & valid_mask;
    let mut matches = 0u32;
    if a_data.fg == b_data.fg {
        matches += (a_fg & b_fg).count_ones();
    }
    if a_data.fg == b_data.bg {
        matches += (a_fg & b_bg).count_ones();
    }
    if a_data.bg == b_data.fg {
        matches += (a_bg & b_fg).count_ones();
    }
    if a_data.bg == b_data.bg {
        matches += (a_bg & b_bg).count_ones();
    }
    valid_mask.count_ones() as usize - matches as usize
}

#[cfg(test)]
#[inline]
fn score_fix_preview_pixel(
    data: &BlockData,
    glyph: ScoreFixGlyphPreview,
    inverted: bool,
    x: usize,
    y: usize,
) -> [u8; 3] {
    let bit = ((glyph.mask >> (y * crate::contour_score::PREVIEW_CELL_WIDTH + x)) & 1) as u8;
    if (bit ^ inverted as u8) != 0 {
        data.fg
    } else {
        data.bg
    }
}

fn score_fix_preview_internal_lengths(
    preview: &[[u8; 3]],
    grid_w: usize,
    grid_h: usize,
) -> Vec<usize> {
    let cell_w = crate::contour_score::PREVIEW_CELL_WIDTH;
    let cell_h = crate::contour_score::PREVIEW_CELL_HEIGHT;
    let width = grid_w * cell_w;
    (0..grid_w * grid_h)
        .into_par_iter()
        .map(|idx| {
            let cell_x = idx % grid_w;
            let cell_y = idx / grid_w;
            let x0 = cell_x * cell_w;
            let y0 = cell_y * cell_h;
            let mut length = 0usize;
            for y in 0..cell_h {
                let row = (y0 + y) * width + x0;
                for x in 1..cell_w {
                    length += (preview[row + x - 1] != preview[row + x]) as usize;
                }
            }
            for y in 1..cell_h {
                let upper = (y0 + y - 1) * width + x0;
                let lower = upper + width;
                for x in 0..cell_w {
                    length += (preview[upper + x] != preview[lower + x]) as usize;
                }
            }
            length
        })
        .collect()
}

#[inline]
fn score_fix_preview_at(
    preview: &[[u8; 3]],
    preview_width: usize,
    cell_idx: usize,
    grid_w: usize,
    x: usize,
    y: usize,
) -> [u8; 3] {
    let cell_x = cell_idx % grid_w;
    let cell_y = cell_idx / grid_w;
    preview[(cell_y * crate::contour_score::PREVIEW_CELL_HEIGHT + y) * preview_width
        + cell_x * crate::contour_score::PREVIEW_CELL_WIDTH
        + x]
}

fn score_fix_patch_boundary_length(
    preview: &[[u8; 3]],
    internal_lengths: &[usize],
    center_idx: usize,
    grid_w: usize,
    grid_h: usize,
) -> usize {
    let center_x = center_idx % grid_w;
    let center_y = center_idx / grid_w;
    let preview_width = grid_w * crate::contour_score::PREVIEW_CELL_WIDTH;
    let mut cells = [[0usize; 3]; 3];
    let mut length = 0usize;
    for (patch_y, row) in cells.iter_mut().enumerate() {
        let cell_y = center_y
            .saturating_add(patch_y)
            .saturating_sub(1)
            .min(grid_h - 1);
        for (patch_x, idx) in row.iter_mut().enumerate() {
            let cell_x = center_x
                .saturating_add(patch_x)
                .saturating_sub(1)
                .min(grid_w - 1);
            *idx = cell_y * grid_w + cell_x;
            length += internal_lengths[*idx];
        }
    }

    let cell_w = crate::contour_score::PREVIEW_CELL_WIDTH;
    let cell_h = crate::contour_score::PREVIEW_CELL_HEIGHT;
    for row in cells {
        for pair in row.windows(2) {
            for y in 0..cell_h {
                length += (score_fix_preview_at(
                    preview,
                    preview_width,
                    pair[0],
                    grid_w,
                    cell_w - 1,
                    y,
                ) != score_fix_preview_at(preview, preview_width, pair[1], grid_w, 0, y))
                    as usize;
            }
        }
    }
    for rows in cells.windows(2) {
        for x in 0..3 {
            for pixel_x in 0..cell_w {
                length += (score_fix_preview_at(
                    preview,
                    preview_width,
                    rows[0][x],
                    grid_w,
                    pixel_x,
                    cell_h - 1,
                ) != score_fix_preview_at(
                    preview,
                    preview_width,
                    rows[1][x],
                    grid_w,
                    pixel_x,
                    0,
                )) as usize;
            }
        }
    }
    length
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn score_fix_patch_boundary_length_with_candidate(
    preview: &[[u8; 3]],
    internal_lengths: &[usize],
    center_idx: usize,
    changed_idx: usize,
    data: &BlockData,
    glyph: ScoreFixGlyphPreview,
    inverted: bool,
    candidate_internal_length: usize,
    grid_w: usize,
    grid_h: usize,
) -> usize {
    let center_x = center_idx % grid_w;
    let center_y = center_idx / grid_w;
    let preview_width = grid_w * crate::contour_score::PREVIEW_CELL_WIDTH;
    let mut cells = [[0usize; 3]; 3];
    let mut length = 0usize;
    for (patch_y, row) in cells.iter_mut().enumerate() {
        let cell_y = center_y
            .saturating_add(patch_y)
            .saturating_sub(1)
            .min(grid_h - 1);
        for (patch_x, idx) in row.iter_mut().enumerate() {
            let cell_x = center_x
                .saturating_add(patch_x)
                .saturating_sub(1)
                .min(grid_w - 1);
            *idx = cell_y * grid_w + cell_x;
            length += if *idx == changed_idx {
                candidate_internal_length
            } else {
                internal_lengths[*idx]
            };
        }
    }

    let pixel = |idx, x, y| {
        if idx == changed_idx {
            score_fix_preview_pixel(data, glyph, inverted, x, y)
        } else {
            score_fix_preview_at(preview, preview_width, idx, grid_w, x, y)
        }
    };
    let cell_w = crate::contour_score::PREVIEW_CELL_WIDTH;
    let cell_h = crate::contour_score::PREVIEW_CELL_HEIGHT;
    for row in cells {
        for pair in row.windows(2) {
            for y in 0..cell_h {
                length += (pixel(pair[0], cell_w - 1, y) != pixel(pair[1], 0, y)) as usize;
            }
        }
    }
    for rows in cells.windows(2) {
        for x in 0..3 {
            for pixel_x in 0..cell_w {
                length += (pixel(rows[0][x], pixel_x, cell_h - 1)
                    != pixel(rows[1][x], pixel_x, 0)) as usize;
            }
        }
    }
    length
}

fn score_fix_patch_boundary_length_from_cells(
    grid_state: &[(usize, bool)],
    grid_data: &[BlockData],
    glyph_previews: &[ScoreFixGlyphPreview],
    center_idx: usize,
    changed: Option<(usize, (usize, bool))>,
    grid_w: usize,
    grid_h: usize,
) -> usize {
    let center_x = center_idx % grid_w;
    let center_y = center_idx / grid_w;
    let mut cells = [[0usize; 3]; 3];
    for (patch_y, row) in cells.iter_mut().enumerate() {
        let cell_y = center_y
            .saturating_add(patch_y)
            .saturating_sub(1)
            .min(grid_h - 1);
        for (patch_x, idx) in row.iter_mut().enumerate() {
            let cell_x = center_x
                .saturating_add(patch_x)
                .saturating_sub(1)
                .min(grid_w - 1);
            *idx = cell_y * grid_w + cell_x;
        }
    }
    let choice_at = |idx: usize| match changed {
        Some((changed_idx, choice)) if idx == changed_idx => choice,
        _ => grid_state[idx],
    };
    let mut length = cells
        .iter()
        .flatten()
        .map(|&idx| {
            let choice = choice_at(idx);
            if grid_data[idx].fg == grid_data[idx].bg {
                0
            } else {
                glyph_previews[choice.0].internal_boundary_length
            }
        })
        .sum::<usize>();

    for row in cells {
        for pair in row.windows(2) {
            let left = pair[0];
            let right = pair[1];
            let left_choice = choice_at(left);
            let right_choice = choice_at(right);
            length += score_fix_edge_mismatches(
                &grid_data[left],
                glyph_previews[left_choice.0].right_edge,
                left_choice.1,
                &grid_data[right],
                glyph_previews[right_choice.0].left_edge,
                right_choice.1,
                u16::MAX,
            );
        }
    }
    for rows in cells.windows(2) {
        for x in 0..3 {
            let top = rows[0][x];
            let bottom = rows[1][x];
            let top_choice = choice_at(top);
            let bottom_choice = choice_at(bottom);
            length += score_fix_edge_mismatches(
                &grid_data[top],
                glyph_previews[top_choice.0].bottom_edge as u16,
                top_choice.1,
                &grid_data[bottom],
                glyph_previews[bottom_choice.0].top_edge as u16,
                bottom_choice.1,
                u8::MAX as u16,
            );
        }
    }
    length
}

fn score_fix_grid_boundary_length(
    preview: &[[u8; 3]],
    internal_lengths: &[usize],
    grid_w: usize,
    grid_h: usize,
) -> usize {
    let preview_width = grid_w * crate::contour_score::PREVIEW_CELL_WIDTH;
    let cell_w = crate::contour_score::PREVIEW_CELL_WIDTH;
    let cell_h = crate::contour_score::PREVIEW_CELL_HEIGHT;
    let mut length = internal_lengths.iter().sum::<usize>();
    for cell_y in 0..grid_h {
        for cell_x in 1..grid_w {
            let left = cell_y * grid_w + cell_x - 1;
            let right = left + 1;
            for y in 0..cell_h {
                length += (score_fix_preview_at(
                    preview,
                    preview_width,
                    left,
                    grid_w,
                    cell_w - 1,
                    y,
                ) != score_fix_preview_at(preview, preview_width, right, grid_w, 0, y))
                    as usize;
            }
        }
    }
    for cell_y in 1..grid_h {
        for cell_x in 0..grid_w {
            let top = (cell_y - 1) * grid_w + cell_x;
            let bottom = top + grid_w;
            for x in 0..cell_w {
                length += (score_fix_preview_at(
                    preview,
                    preview_width,
                    top,
                    grid_w,
                    x,
                    cell_h - 1,
                ) != score_fix_preview_at(preview, preview_width, bottom, grid_w, x, 0))
                    as usize;
            }
        }
    }
    length
}

#[allow(clippy::too_many_arguments)]
fn score_fix_center_boundary_contribution(
    grid_state: &[(usize, bool)],
    grid_data: &[BlockData],
    glyph_previews: &[ScoreFixGlyphPreview],
    cell_idx: usize,
    grid_w: usize,
    grid_h: usize,
    candidate: Option<(usize, bool)>,
    replicate_edges: bool,
) -> usize {
    let cell_x = cell_idx % grid_w;
    let cell_y = cell_idx / grid_w;
    let center_choice = candidate.unwrap_or(grid_state[cell_idx]);
    let center_data = &grid_data[cell_idx];
    let center_glyph = glyph_previews[center_choice.0];
    let mut length = if center_data.fg == center_data.bg {
        0
    } else {
        center_glyph.internal_boundary_length
    };

    let mut add_vertical_seam = |neighbor_idx: usize, center_mask: u16, neighbor_mask: u16| {
        let neighbor_choice = grid_state[neighbor_idx];
        length += score_fix_edge_mismatches(
            center_data,
            center_mask,
            center_choice.1,
            &grid_data[neighbor_idx],
            neighbor_mask,
            neighbor_choice.1,
            u16::MAX,
        );
    };

    if cell_x > 0 || replicate_edges {
        let neighbor = cell_y * grid_w + cell_x.saturating_sub(1);
        let neighbor_glyph = glyph_previews[grid_state[neighbor].0];
        add_vertical_seam(neighbor, center_glyph.left_edge, neighbor_glyph.right_edge);
    }
    if cell_x + 1 < grid_w || replicate_edges {
        let neighbor = cell_y * grid_w + (cell_x + 1).min(grid_w - 1);
        let neighbor_glyph = glyph_previews[grid_state[neighbor].0];
        add_vertical_seam(neighbor, center_glyph.right_edge, neighbor_glyph.left_edge);
    }

    let mut add_horizontal_seam = |neighbor_idx: usize, center_mask: u8, neighbor_mask: u8| {
        let neighbor_choice = grid_state[neighbor_idx];
        length += score_fix_edge_mismatches(
            center_data,
            center_mask as u16,
            center_choice.1,
            &grid_data[neighbor_idx],
            neighbor_mask as u16,
            neighbor_choice.1,
            u8::MAX as u16,
        );
    };
    if cell_y > 0 || replicate_edges {
        let neighbor = cell_y.saturating_sub(1) * grid_w + cell_x;
        let neighbor_glyph = glyph_previews[grid_state[neighbor].0];
        add_horizontal_seam(neighbor, center_glyph.top_edge, neighbor_glyph.bottom_edge);
    }
    if cell_y + 1 < grid_h || replicate_edges {
        let neighbor = (cell_y + 1).min(grid_h - 1) * grid_w + cell_x;
        let neighbor_glyph = glyph_previews[grid_state[neighbor].0];
        add_horizontal_seam(neighbor, center_glyph.bottom_edge, neighbor_glyph.top_edge);
    }
    length
}

#[inline]
fn score_fix_band_score(reference_length: usize, output_length: usize, weight: f64) -> f64 {
    weight.max(0.0)
        * 100.0
        * (((output_length as f64 + 1.0) / (reference_length as f64 + 1.0))
            .ln()
            .abs())
}

#[allow(clippy::too_many_arguments)]
fn prepare_score_fix_band_cache(
    source_preview: &[[u8; 3]],
    output_preview: &[[u8; 3]],
    grid_data: &[BlockData],
    glyphs: &[FastGlyph],
    weights: &crate::contour_score::ContourScoreWeights,
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
) -> Option<ScoreFixBandCache> {
    if !score_fix_band_only(weights)
        || grid_w == 0
        || grid_h == 0
        || source_preview.len()
            != grid_w
                * grid_h
                * crate::contour_score::PREVIEW_CELL_WIDTH
                * crate::contour_score::PREVIEW_CELL_HEIGHT
        || !score_fix_has_at_most_eight_colours(source_preview)
        || !score_fix_has_at_most_eight_colours(
            grid_data.iter().flat_map(|data| [&data.fg, &data.bg]),
        )
    {
        return None;
    }

    let glyph_previews = glyphs
        .par_iter()
        .map(|glyph| score_fix_preview_glyph(glyph, gw, gh))
        .collect::<Vec<_>>();
    let reference_internal_lengths =
        score_fix_preview_internal_lengths(source_preview, grid_w, grid_h);
    let output_internal_lengths =
        score_fix_preview_internal_lengths(output_preview, grid_w, grid_h);
    let reference_patch_lengths = (0..grid_w * grid_h)
        .into_par_iter()
        .map(|idx| {
            score_fix_patch_boundary_length(
                source_preview,
                &reference_internal_lengths,
                idx,
                grid_w,
                grid_h,
            )
        })
        .collect::<Vec<_>>();
    let global_reference_length = score_fix_grid_boundary_length(
        source_preview,
        &reference_internal_lengths,
        grid_w,
        grid_h,
    );
    let global_output_length = score_fix_grid_boundary_length(
        output_preview,
        &output_internal_lengths,
        grid_w,
        grid_h,
    );

    Some(ScoreFixBandCache {
        glyph_previews,
        reference_patch_lengths,
        output_patch_lengths: vec![None; grid_w * grid_h],
        global_reference_length,
        global_output_length,
    })
}

#[allow(clippy::too_many_arguments)]
fn evaluate_score_fix_band_candidate(
    cache: &mut ScoreFixBandCache,
    grid_state: &[(usize, bool)],
    grid_data: &[BlockData],
    curr_idx: usize,
    candidate: (usize, bool),
    boundary_weight: f64,
    grid_w: usize,
    grid_h: usize,
) -> ScoreFixBandCandidate {
    let current_patch_length = cache.output_patch_lengths[curr_idx].unwrap_or_else(|| {
        let length = score_fix_patch_boundary_length_from_cells(
            grid_state,
            grid_data,
            &cache.glyph_previews,
            curr_idx,
            None,
            grid_w,
            grid_h,
        );
        cache.output_patch_lengths[curr_idx] = Some(length);
        length
    });
    let current_local_contribution = score_fix_center_boundary_contribution(
        grid_state,
        grid_data,
        &cache.glyph_previews,
        curr_idx,
        grid_w,
        grid_h,
        None,
        true,
    );
    let candidate_local_contribution = score_fix_center_boundary_contribution(
        grid_state,
        grid_data,
        &cache.glyph_previews,
        curr_idx,
        grid_w,
        grid_h,
        Some(candidate),
        true,
    );
    let candidate_patch_length = current_patch_length - current_local_contribution
        + candidate_local_contribution;
    let cell_x = curr_idx % grid_w;
    let cell_y = curr_idx / grid_w;
    let global_boundary_delta = if cell_x > 0
        && cell_x + 1 < grid_w
        && cell_y > 0
        && cell_y + 1 < grid_h
    {
        candidate_local_contribution as isize - current_local_contribution as isize
    } else {
        let current_global_contribution = score_fix_center_boundary_contribution(
            grid_state,
            grid_data,
            &cache.glyph_previews,
            curr_idx,
            grid_w,
            grid_h,
            None,
            false,
        );
        let candidate_global_contribution = score_fix_center_boundary_contribution(
            grid_state,
            grid_data,
            &cache.glyph_previews,
            curr_idx,
            grid_w,
            grid_h,
            Some(candidate),
            false,
        );
        candidate_global_contribution as isize - current_global_contribution as isize
    };
    let reference_length = cache.reference_patch_lengths[curr_idx];
    ScoreFixBandCandidate {
        current_score: score_fix_band_score(
            reference_length,
            current_patch_length,
            boundary_weight,
        ),
        candidate_score: score_fix_band_score(
            reference_length,
            candidate_patch_length,
            boundary_weight,
        ),
        global_boundary_delta,
    }
}

fn apply_score_fix_band_commit(
    cache: &mut ScoreFixBandCache,
    curr_idx: usize,
    candidate: ScoreFixBandCandidate,
    grid_w: usize,
    grid_h: usize,
) {
    cache.global_output_length = if candidate.global_boundary_delta >= 0 {
        cache
            .global_output_length
            .checked_add(candidate.global_boundary_delta as usize)
            .expect("smoothing global boundary length overflowed")
    } else {
        cache
            .global_output_length
            .checked_sub((-candidate.global_boundary_delta) as usize)
            .expect("smoothing global boundary length became negative")
    };
    for idx in affected_discontinuity_cell_indices(curr_idx, grid_w, grid_h) {
        cache.output_patch_lengths[idx] = None;
    }
}

#[derive(Clone, Copy, Debug)]
enum ScoreFixOrdering {
    WorstFirst,
    SpiralIn,
    SpiralOut,
    AlternatingForward,
    AlternatingReverse,
}

impl ScoreFixOrdering {
    // Keep worst-first first and only append strategies: SF Orders=k is a
    // strict superset of SF Orders=k-1, so increasing it cannot discard the
    // previously selected result.
    const ALL: [Self; 5] = [
        Self::WorstFirst,
        Self::SpiralIn,
        Self::SpiralOut,
        Self::AlternatingForward,
        Self::AlternatingReverse,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::WorstFirst => "worst-first",
            Self::SpiralIn => "spiral-in",
            Self::SpiralOut => "spiral-out",
            Self::AlternatingForward => "alternating-forward",
            Self::AlternatingReverse => "alternating-reverse",
        }
    }
}

fn score_fix_spiral_ranks(grid_w: usize, grid_h: usize) -> Vec<usize> {
    let mut ranks = vec![0usize; grid_w * grid_h];
    let mut rank = 0usize;
    let (mut left, mut top) = (0usize, 0usize);
    let (mut right, mut bottom) = (grid_w - 1, grid_h - 1);
    while left <= right && top <= bottom {
        for x in left..=right {
            ranks[top * grid_w + x] = rank;
            rank += 1;
        }
        if top == bottom {
            break;
        }
        for y in top + 1..=bottom {
            ranks[y * grid_w + right] = rank;
            rank += 1;
        }
        if left == right {
            break;
        }
        for x in (left..right).rev() {
            ranks[bottom * grid_w + x] = rank;
            rank += 1;
        }
        for y in (top + 1..bottom).rev() {
            ranks[y * grid_w + left] = rank;
            rank += 1;
        }
        left += 1;
        right -= 1;
        top += 1;
        bottom -= 1;
    }
    ranks
}

fn score_fix_order_priority(
    ordering: ScoreFixOrdering,
    spiral_ranks: &[usize],
    cell_idx: usize,
    prefix_index: usize,
    cell_count: usize,
) -> usize {
    match ordering {
        ScoreFixOrdering::WorstFirst => 0,
        ScoreFixOrdering::SpiralIn => cell_count - spiral_ranks[cell_idx],
        ScoreFixOrdering::SpiralOut => spiral_ranks[cell_idx] + 1,
        ScoreFixOrdering::AlternatingForward => {
            if prefix_index % 2 == 0 {
                cell_count - cell_idx
            } else {
                cell_idx + 1
            }
        }
        ScoreFixOrdering::AlternatingReverse => {
            if prefix_index % 2 == 0 {
                cell_idx + 1
            } else {
                cell_count - cell_idx
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ScoreFixPriorityEntry {
    order_priority: usize,
    score: f32,
    cell_idx: usize,
    version: u32,
}

impl PartialEq for ScoreFixPriorityEntry {
    fn eq(&self, other: &Self) -> bool {
        self.order_priority == other.order_priority
            && self.score.total_cmp(&other.score).is_eq()
            && self.cell_idx == other.cell_idx
            && self.version == other.version
    }
}

impl Eq for ScoreFixPriorityEntry {}

impl PartialOrd for ScoreFixPriorityEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScoreFixPriorityEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.order_priority
            .cmp(&other.order_priority)
            .then_with(|| self.score.total_cmp(&other.score))
            // Resolve equal scores in stable row-major order.
            .then_with(|| other.cell_idx.cmp(&self.cell_idx))
            .then_with(|| self.version.cmp(&other.version))
    }
}

const GEOMETRY_FLOW_DISTANCE_CAP: u8 = 4;
const GEOMETRY_FLOW_STRENGTH: f64 = 3.0;

/// One categorical median pass. Unlike an RGB blur, this never invents a
/// colour: it evolves the existing colour regions by one discrete
/// mean-curvature step, shaving protrusions and filling matching indentations.
fn categorical_median_pass(
    labels: &[u32],
    width: usize,
    height: usize,
) -> Vec<u32> {
    if width == 0 || height == 0 || labels.len() != width * height {
        return Vec::new();
    }
    let mut output = labels.to_vec();
    for y in 0..height {
        for x in 0..width {
            let center = labels[y * width + x];
            let mut colours = [0u32; 9];
            let mut counts = [0u8; 9];
            let mut colour_count = 0usize;
            for ny in y.saturating_sub(1)..=(y + 1).min(height - 1) {
                for nx in x.saturating_sub(1)..=(x + 1).min(width - 1) {
                    let colour = labels[ny * width + nx];
                    let mut slot = 0usize;
                    while slot < colour_count && colours[slot] != colour {
                        slot += 1;
                    }
                    if slot == colour_count {
                        colours[slot] = colour;
                        colour_count += 1;
                    }
                    counts[slot] += 1;
                }
            }
            let mut best = 0usize;
            for slot in 1..colour_count {
                if counts[slot] > counts[best]
                    || (counts[slot] == counts[best]
                        && colours[slot] == center
                        && colours[best] != center)
                {
                    best = slot;
                }
            }
            output[y * width + x] = colours[best];
        }
    }
    output
}

fn categorical_boundary_mask(
    labels: &[u32],
    width: usize,
    height: usize,
) -> Vec<bool> {
    let mut boundary = vec![false; width * height];
    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            let colour = labels[idx];
            boundary[idx] = (x > 0 && labels[idx - 1] != colour)
                || (x + 1 < width && labels[idx + 1] != colour)
                || (y > 0 && labels[idx - width] != colour)
                || (y + 1 < height && labels[idx + width] != colour);
        }
    }
    boundary
}

fn truncated_boundary_distances(boundary: &[bool], width: usize, height: usize) -> Vec<u8> {
    let cap = GEOMETRY_FLOW_DISTANCE_CAP;
    let mut distance = boundary
        .iter()
        .map(|&is_boundary| if is_boundary { 0 } else { cap })
        .collect::<Vec<_>>();
    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            if x > 0 {
                distance[idx] = distance[idx].min(distance[idx - 1].saturating_add(1));
            }
            if y > 0 {
                distance[idx] = distance[idx].min(distance[idx - width].saturating_add(1));
            }
        }
    }
    for y in (0..height).rev() {
        for x in (0..width).rev() {
            let idx = y * width + x;
            if x + 1 < width {
                distance[idx] = distance[idx].min(distance[idx + 1].saturating_add(1));
            }
            if y + 1 < height {
                distance[idx] = distance[idx].min(distance[idx + width].saturating_add(1));
            }
        }
    }
    distance
}

struct PreparedCategoricalBoundary {
    mask: Vec<bool>,
    distance: Vec<u8>,
    count: usize,
}

fn prepare_categorical_boundary(
    labels: &[u32],
    width: usize,
    height: usize,
) -> PreparedCategoricalBoundary {
    let mask = categorical_boundary_mask(labels, width, height);
    let count = mask.iter().filter(|&&value| value).count();
    let distance = truncated_boundary_distances(&mask, width, height);
    PreparedCategoricalBoundary {
        mask,
        distance,
        count,
    }
}

fn symmetric_prepared_boundary_distance(
    a: &PreparedCategoricalBoundary,
    b: &PreparedCategoricalBoundary,
) -> f64 {
    if a.count == 0 && b.count == 0 {
        return 0.0;
    }
    if a.count == 0 || b.count == 0 {
        return 100.0;
    }
    let a_to_b = a
        .mask
        .iter()
        .zip(&b.distance)
        .filter_map(|(&on_boundary, &distance)| on_boundary.then_some(distance as f64))
        .sum::<f64>()
        / a.count as f64;
    let b_to_a = b
        .mask
        .iter()
        .zip(&a.distance)
        .filter_map(|(&on_boundary, &distance)| on_boundary.then_some(distance as f64))
        .sum::<f64>()
        / b.count as f64;
    50.0 * (a_to_b + b_to_a) / GEOMETRY_FLOW_DISTANCE_CAP as f64
}

/// Intrinsic stability under one and two median-flow steps. Straight regions
/// remain unchanged, while teeth, spikes, and abrupt pixel-scale turns move.
fn curvature_flow_regularity(pixels: &[[u8; 3]], width: usize, height: usize) -> f64 {
    let labels = pixels
        .iter()
        .map(|pixel| {
            ((pixel[0] as u32) << 16) | ((pixel[1] as u32) << 8) | pixel[2] as u32
        })
        .collect::<Vec<_>>();
    let once = categorical_median_pass(&labels, width, height);
    if once == labels {
        return 0.0;
    }
    let twice = categorical_median_pass(&once, width, height);
    let original_boundary = prepare_categorical_boundary(&labels, width, height);
    let once_boundary = prepare_categorical_boundary(&once, width, height);
    if twice == once {
        let distance =
            symmetric_prepared_boundary_distance(&original_boundary, &once_boundary);
        return 0.7 * distance + 0.3 * distance;
    }
    let twice_boundary = prepare_categorical_boundary(&twice, width, height);
    let one_step = symmetric_prepared_boundary_distance(&original_boundary, &once_boundary);
    let two_step = symmetric_prepared_boundary_distance(&original_boundary, &twice_boundary);
    0.7 * one_step + 0.3 * two_step
}

#[derive(Clone, Copy, Debug, Default)]
struct GeometryScoreFixStats {
    target_count: usize,
    distinct_states: usize,
    candidate_evaluations: usize,
    flow_candidate_evaluations: usize,
    changes: usize,
    initial_score: f64,
    final_score: f64,
}

struct GeometryCandidateEvaluation {
    choice: (usize, bool),
    affected_scores: Vec<crate::contour_score::ReferenceContourScore>,
    affected_flow_scores: Vec<f64>,
    energy_sum: f64,
    total_sum: f64,
    endpoint_count_sum: usize,
    junction_count_sum: usize,
    component_count_sum: usize,
    fidelity_sum: f64,
}

fn geometry_score_fix_weights(args: &RenderArgs) -> crate::contour_score::ContourScoreWeights {
    // Keep raw topology and fidelity measurements available when their UI
    // weights are zero. This epsilon is below the acceptance tolerance, so it
    // only enables the anti-artifact measurements used below.
    const MEASURE_ONLY: f64 = 1e-12;
    crate::contour_score::ContourScoreWeights {
        bending: (args.contour_bending_weight.max(0.0) as f64).max(MEASURE_ONLY),
        endpoints: (args.contour_endpoint_weight.max(0.0) as f64).max(MEASURE_ONLY),
        junctions: (args.contour_junction_weight.max(0.0) as f64).max(MEASURE_ONLY),
        fragments: (args.contour_fragment_weight.max(0.0) as f64).max(MEASURE_ONLY),
        fidelity: (args.contour_fidelity_weight.max(0.0) as f64).max(MEASURE_ONLY),
        boundary_balance: args.contour_boundary_weight.max(0.0) as f64,
        peak_sensitivity: args.contour_peak_weight.clamp(0.0, 1.0) as f64,
    }
}

fn geometry_score_fix_fidelity_allowance() -> f64 {
    let patch_width = (NEIGHBORHOOD_RADIUS * 2 + 1)
        * crate::contour_score::PREVIEW_CELL_WIDTH;
    let patch_height = (NEIGHBORHOOD_RADIUS * 2 + 1)
        * crate::contour_score::PREVIEW_CELL_HEIGHT;
    let one_preview_edge = crate::contour_score::PREVIEW_CELL_WIDTH
        .max(crate::contour_score::PREVIEW_CELL_HEIGHT);
    100.0 * one_preview_edge as f64 / (patch_width * patch_height) as f64
}

#[cfg(test)]
fn distinct_nonuniform_glyph_states(glyphs: &[FastGlyph]) -> Vec<(usize, bool)> {
    let mut states = Vec::with_capacity(glyphs.len() * 2);
    for (glyph_idx, glyph) in glyphs.iter().enumerate() {
        if glyph.bitmap.is_empty()
            || glyph
                .bitmap
                .iter()
                .all(|&pixel| pixel == glyph.bitmap[0])
        {
            continue;
        }
        for inverted in [false, true] {
            if states.iter().any(|&(existing_idx, existing_inverted)| {
                glyph_states_render_identically(
                    glyph,
                    inverted,
                    &glyphs[existing_idx],
                    existing_inverted,
                )
            }) {
                continue;
            }
            states.push((glyph_idx, inverted));
        }
    }
    states
}

struct GeometryRenderedStateCatalog {
    class_ids: Vec<usize>,
    uniform: Vec<bool>,
}

fn geometry_rendered_state_catalog(glyphs: &[FastGlyph]) -> GeometryRenderedStateCatalog {
    let mut classes = std::collections::HashMap::<Vec<u8>, usize>::new();
    let mut class_ids = Vec::with_capacity(glyphs.len() * 2);
    let mut uniform = Vec::with_capacity(glyphs.len() * 2);
    for glyph in glyphs {
        for inverted in [false, true] {
            let rendered = glyph
                .bitmap
                .iter()
                .map(|&pixel| pixel ^ inverted as u8)
                .collect::<Vec<_>>();
            let is_uniform = rendered
                .first()
                .is_none_or(|&first| rendered.iter().all(|&pixel| pixel == first));
            let next_id = classes.len();
            let class_id = *classes.entry(rendered).or_insert(next_id);
            class_ids.push(class_id);
            uniform.push(is_uniform);
        }
    }
    GeometryRenderedStateCatalog { class_ids, uniform }
}

fn top_unique_geometry_choices(
    costs: &[u64],
    current: (usize, bool),
    limit: usize,
    catalog: &GeometryRenderedStateCatalog,
) -> Vec<(usize, bool)> {
    let current_state = current.0 * 2 + current.1 as usize;
    let Some(&current_class) = catalog.class_ids.get(current_state) else {
        return vec![current];
    };
    let mut ranked = (0..costs.len().min(catalog.class_ids.len())).collect::<Vec<_>>();
    ranked.sort_unstable_by_key(|&state| (costs[state], state));
    let mut seen_classes = std::collections::HashSet::with_capacity(limit);
    let mut choices = Vec::with_capacity(limit + 1);
    for state in ranked {
        let class_id = catalog.class_ids[state];
        if catalog.uniform[state]
            || class_id == current_class
            || !seen_classes.insert(class_id)
        {
            continue;
        }
        choices.push((state / 2, state % 2 == 1));
        if choices.len() == limit {
            break;
        }
    }
    choices.push(current);
    choices
}

#[allow(clippy::too_many_arguments)]
fn paint_geometry_candidate_in_patch(
    patch: &mut LocalPatch,
    patch_center_idx: usize,
    changed_idx: usize,
    grid_data: &[BlockData],
    glyphs: &[FastGlyph],
    candidate: (usize, bool),
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
) {
    let center_x = patch_center_idx % grid_w;
    let center_y = patch_center_idx / grid_w;
    let changed_x = changed_idx % grid_w;
    let changed_y = changed_idx / grid_w;
    let patch_cells = NEIGHBORHOOD_RADIUS * 2 + 1;
    for patch_y in 0..patch_cells {
        let source_y = center_y
            .saturating_add(patch_y)
            .saturating_sub(NEIGHBORHOOD_RADIUS)
            .min(grid_h - 1);
        for patch_x in 0..patch_cells {
            let source_x = center_x
                .saturating_add(patch_x)
                .saturating_sub(NEIGHBORHOOD_RADIUS)
                .min(grid_w - 1);
            if source_x == changed_x && source_y == changed_y {
                paint_preview_glyph_at(
                    &mut patch.pixels,
                    patch.width,
                    patch_x * crate::contour_score::PREVIEW_CELL_WIDTH,
                    patch_y * crate::contour_score::PREVIEW_CELL_HEIGHT,
                    &grid_data[changed_idx],
                    &glyphs[candidate.0],
                    candidate.1,
                    gw,
                    gh,
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_geometry_score_fix(
    args: &RenderArgs,
    raster: &mut Vec<[u8; 3]>,
    grid_state: &mut Vec<(usize, bool)>,
    grid_data: &[BlockData],
    glyphs: &[FastGlyph],
    block_glyph_costs: &[Vec<u64>],
    target_scores: Vec<f32>,
    source_raster: &[[u8; 3]],
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
    abort_flag: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Result<GeometryScoreFixStats, ()> {
    let weights = geometry_score_fix_weights(args);
    let flow_weight = args.contour_bending_weight.max(0.0) as f64
        * GEOMETRY_FLOW_STRENGTH;
    let fidelity_allowance = geometry_score_fix_fidelity_allowance();
    let threshold = args.discontinuity_threshold.max(0.0);
    let cell_count = grid_w * grid_h;
    let mut target_mask = vec![false; cell_count];
    let mut priority_scores = vec![f32::NEG_INFINITY; cell_count];
    let mut queue = BinaryHeap::new();
    for (cell_idx, score) in target_scores.into_iter().enumerate() {
        if score >= threshold && grid_data[cell_idx].fg != grid_data[cell_idx].bg {
            target_mask[cell_idx] = true;
            priority_scores[cell_idx] = score;
            queue.push(ScoreFixPriorityEntry {
                order_priority: 0,
                score,
                cell_idx,
                version: 0,
            });
        }
    }
    let target_count = target_mask.iter().filter(|&&target| target).count();
    let mut active_mask = vec![false; cell_count];
    for (cell_idx, &is_target) in target_mask.iter().enumerate() {
        if is_target {
            for affected_idx in affected_discontinuity_cell_indices(cell_idx, grid_w, grid_h) {
                active_mask[affected_idx] = true;
            }
        }
    }

    // Geometry used to ignore Score Fix N and exhaustively try every enabled
    // glyph/inversion state. Freeze the same source-ranked, render-deduplicated
    // top-N alternatives used by Score Fix. Keep the initial state as a free
    // rollback choice if neighboring edits later make it useful again.
    let candidate_limit = args.score_fix_candidates.max(1) as usize;
    let baseline_state = grid_state.clone();
    let state_catalog = geometry_rendered_state_catalog(glyphs);
    let candidate_lists = (0..cell_count)
        .into_par_iter()
        .map(|cell_idx| {
            if !target_mask[cell_idx] {
                return Vec::new();
            }
            let current = baseline_state[cell_idx];
            block_glyph_costs
                .get(cell_idx)
                .map(|costs| {
                    top_unique_geometry_choices(costs, current, candidate_limit, &state_catalog)
                })
                .unwrap_or_else(|| vec![current])
        })
        .collect::<Vec<_>>();
    let distinct_states = candidate_lists
        .iter()
        .flatten()
        .copied()
        .collect::<std::collections::HashSet<_>>()
        .len();
    let preview_width = grid_w * crate::contour_score::PREVIEW_CELL_WIDTH;
    let (mut output_preview, _, _) = crate::contour_score::normalize_preview_raster(
        raster,
        grid_w,
        grid_h,
        gw,
        gh,
    );

    let references = (0..cell_count)
        .into_par_iter()
        .map(|cell_idx| {
            active_mask[cell_idx].then(|| {
                prepare_local_preview_reference(
                    source_raster,
                    cell_idx,
                    grid_w,
                    grid_h,
                    gw,
                    gh,
                    &weights,
                )
            })
        })
        .collect::<Vec<_>>();
    let baseline_scores = (0..cell_count)
        .into_par_iter()
        .map(|cell_idx| {
            references[cell_idx].as_ref().map(|reference| {
                score_local_normalized_preview_details(
                    reference,
                    &output_preview,
                    cell_idx,
                    grid_w,
                    grid_h,
                    &weights,
                )
            })
        })
        .collect::<Vec<_>>();
    let mut current_flow_scores = if flow_weight > 0.0 {
        (0..cell_count)
            .into_par_iter()
            .map(|cell_idx| {
                if !active_mask[cell_idx] {
                    return 0.0;
                }
                let patch = extract_local_patch(
                    &output_preview,
                    cell_idx % grid_w,
                    cell_idx / grid_w,
                    grid_w,
                    grid_h,
                    crate::contour_score::PREVIEW_CELL_WIDTH,
                    crate::contour_score::PREVIEW_CELL_HEIGHT,
                );
                curvature_flow_regularity(&patch.pixels, patch.width, patch.height)
            })
            .collect::<Vec<_>>()
    } else {
        vec![0.0; cell_count]
    };
    let mut current_scores = baseline_scores.clone();
    let initial_score = current_scores
        .iter()
        .filter_map(|score| score.as_ref().map(|score| score.total))
        .sum::<f64>()
        + flow_weight * current_flow_scores.iter().sum::<f64>();
    let mut global_score = initial_score;
    let mut versions = vec![0u32; cell_count];
    let mut candidate_evaluations = 0usize;
    let flow_candidate_evaluations = std::sync::atomic::AtomicUsize::new(0);
    let mut changes = 0usize;

    while let Some(entry) = queue.pop() {
        let curr_idx = entry.cell_idx;
        if entry.version != versions[curr_idx]
            || !target_mask[curr_idx]
            || entry.score < threshold
        {
            continue;
        }
        if abort_flag.is_some_and(|flag| {
            flag.load(std::sync::atomic::Ordering::Relaxed)
        }) {
            return Err(());
        }

        let current_choice = grid_state[curr_idx];
        let affected = affected_discontinuity_cell_indices(curr_idx, grid_w, grid_h);
        let current_total_sum = affected
            .iter()
            .map(|&idx| {
                current_scores[idx]
                    .as_ref()
                    .expect("geometry score should exist in a target halo")
                    .total
            })
            .sum::<f64>();
        let current_flow_sum = affected
            .iter()
            .map(|&idx| current_flow_scores[idx])
            .sum::<f64>();
        let current_energy_sum = current_total_sum + flow_weight * current_flow_sum;
        let current_endpoint_count_sum = affected
            .iter()
            .map(|&idx| {
                current_scores[idx]
                    .as_ref()
                    .expect("geometry score should exist in a target halo")
                    .contour
                    .endpoint_count
            })
            .sum::<usize>();
        let current_junction_count_sum = affected
            .iter()
            .map(|&idx| {
                current_scores[idx]
                    .as_ref()
                    .expect("geometry score should exist in a target halo")
                    .contour
                    .junction_count
            })
            .sum::<usize>();
        let current_component_count_sum = affected
            .iter()
            .map(|&idx| {
                current_scores[idx]
                    .as_ref()
                    .expect("geometry score should exist in a target halo")
                    .contour
                    .component_count
            })
            .sum::<usize>();
        let baseline_fidelity_limit = affected
            .iter()
            .map(|&idx| {
                baseline_scores[idx]
                    .as_ref()
                    .expect("geometry baseline should exist in a target halo")
                    .fidelity
                    + fidelity_allowance
            })
            .sum::<f64>();
        let patches = affected
            .iter()
            .map(|&idx| {
                (
                    idx,
                    extract_local_patch(
                        &output_preview,
                        idx % grid_w,
                        idx / grid_w,
                        grid_w,
                        grid_h,
                        crate::contour_score::PREVIEW_CELL_WIDTH,
                        crate::contour_score::PREVIEW_CELL_HEIGHT,
                    ),
                )
            })
            .collect::<Vec<_>>();

        let evaluate_candidate = |&candidate: &(usize, bool)| {
                if glyph_states_render_identically(
                    &glyphs[candidate.0],
                    candidate.1,
                    &glyphs[current_choice.0],
                    current_choice.1,
                ) {
                    return None;
                }
                let mut affected_scores = Vec::with_capacity(affected.len());
                let mut candidate_patches = Vec::with_capacity(affected.len());
                let mut total_sum = 0.0;
                let mut endpoint_count_sum = 0usize;
                let mut junction_count_sum = 0usize;
                let mut component_count_sum = 0usize;
                let mut fidelity_sum = 0.0;
                for (affected_idx, baseline_patch) in &patches {
                    let mut candidate_patch = baseline_patch.clone();
                    paint_geometry_candidate_in_patch(
                        &mut candidate_patch,
                        *affected_idx,
                        curr_idx,
                        grid_data,
                        glyphs,
                        candidate,
                        grid_w,
                        grid_h,
                        gw,
                        gh,
                    );
                    let score = crate::contour_score::score_rgb_against_prepared_with_weights(
                        references[*affected_idx]
                            .as_ref()
                            .expect("geometry reference should exist in a target halo"),
                        &candidate_patch.pixels,
                        crate::contour_score::PREVIEW_CELL_WIDTH,
                        &weights,
                    );
                    total_sum += score.total;
                    endpoint_count_sum += score.contour.endpoint_count;
                    junction_count_sum += score.contour.junction_count;
                    component_count_sum += score.contour.component_count;
                    fidelity_sum += score.fidelity;
                    affected_scores.push(score);
                    candidate_patches.push(candidate_patch);
                    // Flow is non-negative, as are all remaining contour and
                    // fidelity terms. Reject impossible candidates before the
                    // expensive median passes without changing the result.
                    if total_sum + 1e-9 >= current_energy_sum
                        || fidelity_sum > baseline_fidelity_limit + 1e-9
                    {
                        return None;
                    }
                }
                // Only these overlapping terms can change. Geometry may move
                // between adjacent windows, but it may not create additional
                // broken ends, junctions, or components overall. The fixed
                // baseline fidelity limit prevents cumulative drift.
                if endpoint_count_sum > current_endpoint_count_sum
                    || junction_count_sum > current_junction_count_sum
                    || component_count_sum > current_component_count_sum
                {
                    return None;
                }
                let affected_flow_scores = if flow_weight > 0.0 {
                    flow_candidate_evaluations.fetch_add(
                        1,
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    candidate_patches
                        .iter()
                        .map(|candidate_patch| {
                            curvature_flow_regularity(
                                &candidate_patch.pixels,
                                candidate_patch.width,
                                candidate_patch.height,
                            )
                        })
                        .collect::<Vec<_>>()
                } else {
                    vec![0.0; affected.len()]
                };
                let flow_sum = affected_flow_scores.iter().sum::<f64>();
                let energy_sum = total_sum + flow_weight * flow_sum;
                if energy_sum + 1e-9 >= current_energy_sum {
                    return None;
                }
                Some(GeometryCandidateEvaluation {
                    choice: candidate,
                    affected_scores,
                    affected_flow_scores,
                    energy_sum,
                    total_sum,
                    endpoint_count_sum,
                    junction_count_sum,
                    component_count_sum,
                    fidelity_sum,
                })
            };
        let candidates = &candidate_lists[curr_idx];
        candidate_evaluations += candidates
            .iter()
            .filter(|&&candidate| {
                !glyph_states_render_identically(
                    &glyphs[candidate.0],
                    candidate.1,
                    &glyphs[current_choice.0],
                    current_choice.1,
                )
            })
            .count();
        let evaluations = if candidates.len() >= 8 {
            candidates
                .par_iter()
                .filter_map(evaluate_candidate)
                .collect::<Vec<_>>()
        } else {
            candidates
                .iter()
                .filter_map(evaluate_candidate)
                .collect::<Vec<_>>()
        };
        let best = evaluations.into_iter().min_by(|a, b| {
            a.energy_sum
                .total_cmp(&b.energy_sum)
                .then_with(|| a.total_sum.total_cmp(&b.total_sum))
                .then_with(|| a.endpoint_count_sum.cmp(&b.endpoint_count_sum))
                .then_with(|| a.junction_count_sum.cmp(&b.junction_count_sum))
                .then_with(|| a.component_count_sum.cmp(&b.component_count_sum))
                .then_with(|| a.fidelity_sum.total_cmp(&b.fidelity_sum))
                .then_with(|| a.choice.cmp(&b.choice))
        });
        let Some(best) = best else {
            continue;
        };

        global_score += best.energy_sum - current_energy_sum;
        grid_state[curr_idx] = best.choice;
        apply_choice_to_raster(
            raster,
            grid_data,
            glyphs,
            grid_w,
            gw,
            gh,
            curr_idx,
            best.choice,
        );
        paint_global_preview_cell(
            &mut output_preview,
            preview_width,
            curr_idx,
            grid_w,
            &grid_data[curr_idx],
            &glyphs[best.choice.0],
            best.choice.1,
            gw,
            gh,
        );
        for ((&affected_idx, score), flow_score) in affected
            .iter()
            .zip(best.affected_scores)
            .zip(best.affected_flow_scores)
        {
            current_scores[affected_idx] = Some(score);
            current_flow_scores[affected_idx] = flow_score;
        }
        changes += 1;

        for affected_idx in affected {
            if !target_mask[affected_idx] {
                continue;
            }
            let score = cell_discontinuity_score(
                affected_idx,
                raster,
                grid_w,
                grid_h,
                gw,
                gh,
            );
            priority_scores[affected_idx] = score;
            versions[affected_idx] = versions[affected_idx].wrapping_add(1);
            if score >= threshold {
                queue.push(ScoreFixPriorityEntry {
                    order_priority: 0,
                    score,
                    cell_idx: affected_idx,
                    version: versions[affected_idx],
                });
            }
        }
    }

    Ok(GeometryScoreFixStats {
        target_count,
        distinct_states,
        candidate_evaluations,
        flow_candidate_evaluations: flow_candidate_evaluations
            .load(std::sync::atomic::Ordering::Relaxed),
        changes,
        initial_score,
        final_score: global_score,
    })
}

#[allow(clippy::too_many_arguments)]
fn refresh_score_fix_priorities(
    changed_idx: usize,
    raster: &[[u8; 3]],
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
    threshold: f32,
    prefix_index: usize,
    candidate_lists: &[Vec<(usize, bool, u64)>],
    processed: &[bool],
    priority_scores: &mut [f32],
    priority_versions: &mut [u32],
    queue: &mut BinaryHeap<ScoreFixPriorityEntry>,
    ordering: ScoreFixOrdering,
    spiral_ranks: &[usize],
) -> usize {
    let mut refreshed = 0usize;
    for idx in affected_discontinuity_cell_indices(changed_idx, grid_w, grid_h) {
        if candidate_lists[idx].get(prefix_index).is_none() {
            continue;
        }
        let score = cell_discontinuity_score(idx, raster, grid_w, grid_h, gw, gh);
        priority_scores[idx] = score;
        priority_versions[idx] = priority_versions[idx].wrapping_add(1);
        refreshed += 1;
        if !processed[idx] && score >= threshold {
            queue.push(ScoreFixPriorityEntry {
                order_priority: score_fix_order_priority(
                    ordering,
                    spiral_ranks,
                    idx,
                    prefix_index,
                    grid_w * grid_h,
                ),
                score,
                cell_idx: idx,
                version: priority_versions[idx],
            });
        }
    }
    refreshed
}

#[allow(clippy::too_many_arguments)]
fn run_score_fix_prefix_search(
    args: &RenderArgs,
    ordering: ScoreFixOrdering,
    raster: &mut Vec<[u8; 3]>,
    grid_state: &mut Vec<(usize, bool)>,
    grid_data: &[BlockData],
    glyphs: &[FastGlyph],
    block_glyph_costs: &[Vec<u64>],
    target_scores: Vec<f32>,
    source_raster: &[[u8; 3]],
    contour_weights: &crate::contour_score::ContourScoreWeights,
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
    abort_flag: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Result<ScoreFixPrefixStats, ()> {
    // Preserve the established N=1 behaviour for the primary search: Score
    // Fix being enabled must still produce a visible alternative even when
    // that first alternative does not beat N=0.  Extra ordering trials are
    // exploratory, however.  Forcing their first candidate into every cell
    // makes the completed N=1 grid independent of visit order, and that grid
    // commonly wins every trial before ordering can have any effect.
    let force_first_prefix = matches!(ordering, ScoreFixOrdering::WorstFirst);
    let spiral_ranks = score_fix_spiral_ranks(grid_w, grid_h);
    let threshold = args.discontinuity_threshold.max(0.0);
    let mut targets = target_scores
        .into_iter()
        .enumerate()
        .filter(|(_, score)| *score >= threshold)
        .collect::<Vec<_>>();
    targets.sort_by(|a, b| b.1.total_cmp(&a.1));
    let target_count = targets.len();
    let mut priority_scores = vec![f32::NEG_INFINITY; grid_w * grid_h];
    for &(idx, score) in &targets {
        priority_scores[idx] = score;
    }

    let candidate_count = args.score_fix_candidates.max(1) as usize;
    let baseline_state = grid_state.clone();
    let mut candidate_lists = vec![Vec::new(); grid_w * grid_h];
    let mut candidate_trials = 0usize;
    let mut duplicate_candidates_skipped = 0usize;

    // Freeze the target set and complete candidate rankings before changing
    // the raster. Candidate k must mean the same thing in an N=k and N>k run;
    // the processing order within each prefix remains dynamic.
    // Deduplicate before truncating so N counts actual rendered alternatives,
    // not the unchanged glyph or duplicate glyph/inversion states.
    let prepared_candidates = targets
        .par_iter()
        .map(|&(curr_idx, _)| {
            if grid_data[curr_idx].fg == grid_data[curr_idx].bg {
                return (curr_idx, Vec::new(), 0usize);
            }
            let current_choice = baseline_state[curr_idx];
            let Some(costs) = block_glyph_costs.get(curr_idx) else {
                return (curr_idx, Vec::new(), 0usize);
            };
            let candidates_with_duplicates = best_candidate_pool_pre(costs, costs.len());
            let original_count = candidates_with_duplicates.len();
            let mut candidates = deduplicate_rendered_candidates(
                candidates_with_duplicates,
                glyphs,
                current_choice,
            );
            let skipped = original_count.saturating_sub(candidates.len());
            candidates.truncate(candidate_count.min(candidates.len()));
            (curr_idx, candidates, skipped)
        })
        .collect::<Vec<_>>();
    for (curr_idx, candidates, skipped) in prepared_candidates {
        candidate_trials += candidates.len();
        duplicate_candidates_skipped += skipped;
        candidate_lists[curr_idx] = candidates;
    }

    let prefix_count = candidate_lists
        .iter()
        .map(Vec::len)
        .max()
        .unwrap_or(0);
    let preview_width = grid_w * crate::contour_score::PREVIEW_CELL_WIDTH;
    let preview_height = grid_h * crate::contour_score::PREVIEW_CELL_HEIGHT;
    let source_preview = if contour_weights.fidelity > 0.0
        || contour_weights.boundary_balance > 0.0
    {
        crate::contour_score::normalize_preview_raster(
            source_raster,
            grid_w,
            grid_h,
            gw,
            gh,
        )
        .0
    } else {
        Vec::new()
    };
    let (mut global_output_preview, _, _) = crate::contour_score::normalize_preview_raster(
        raster,
        grid_w,
        grid_h,
        gw,
        gh,
    );
    let mut band_cache = prepare_score_fix_band_cache(
        &source_preview,
        &global_output_preview,
        grid_data,
        glyphs,
        contour_weights,
        grid_w,
        grid_h,
        gw,
        gh,
    );
    let global_reference = band_cache.is_none().then(|| {
        crate::contour_score::prepare_reference_with_weights(
            &source_preview,
            preview_width,
            preview_height,
            crate::contour_score::PREVIEW_CELL_WIDTH,
            contour_weights,
        )
    });
    let initial_score = if let Some(cache) = band_cache.as_ref() {
        score_fix_band_score(
            cache.global_reference_length,
            cache.global_output_length,
            contour_weights.boundary_balance,
        )
    } else {
        crate::contour_score::score_rgb_against_prepared_with_weights(
            global_reference
                .as_ref()
                .expect("general smoothing reference should be prepared"),
            &global_output_preview,
            crate::contour_score::PREVIEW_CELL_WIDTH,
            contour_weights,
        )
        .total
    };
    if prefix_count == 0 {
        return Ok(ScoreFixPrefixStats {
            target_count,
            candidate_trials,
            duplicate_candidates_skipped,
            changes: 0,
            rejected: 0,
            prefix_count: 0,
            best_prefix: 0,
            initial_score,
            best_score: initial_score,
            priority_refreshes: 0,
            used_band_fast_path: band_cache.is_some(),
        });
    }

    let mut reference_needed = vec![false; grid_w * grid_h];
    for &(target_idx, _) in &targets {
        if args.score_fix_neighborhood_guard {
            for affected_idx in affected_discontinuity_cell_indices(target_idx, grid_w, grid_h) {
                reference_needed[affected_idx] = true;
            }
        } else {
            reference_needed[target_idx] = true;
        }
    }
    let mut cached_references = if band_cache.is_some() {
        (0..grid_w * grid_h).map(|_| None).collect::<Vec<_>>()
    } else {
        reference_needed
            .into_par_iter()
            .enumerate()
            .map(|(idx, needed)| {
                needed.then(|| {
                    prepare_local_preview_reference(
                        source_raster,
                        idx,
                        grid_w,
                        grid_h,
                        gw,
                        gh,
                        contour_weights,
                    )
                })
            })
            .collect::<Vec<_>>()
    };
    let mut cached_cell_scores = vec![None::<f64>; grid_w * grid_h];
    let mut best_prefix_raster = raster.clone();
    let mut best_prefix_state = grid_state.clone();
    let mut best_prefix_score = f64::INFINITY;
    let mut best_prefix = 0usize;
    let mut changes = 0usize;
    let mut rejected = 0usize;
    let mut priority_refreshes = 0usize;
    let mut processed = vec![false; grid_w * grid_h];
    let mut priority_versions = vec![0u32; grid_w * grid_h];
    let mut priority_queue = BinaryHeap::with_capacity(target_count);

    for prefix_index in 0..prefix_count {
        let mut prefix_changed = false;
        processed.fill(false);
        priority_versions.fill(0);
        priority_queue.clear();
        for &(cell_idx, _) in &targets {
            let score = priority_scores[cell_idx];
            if candidate_lists[cell_idx].get(prefix_index).is_some() && score >= threshold {
                priority_queue.push(ScoreFixPriorityEntry {
                    order_priority: score_fix_order_priority(
                        ordering,
                        &spiral_ranks,
                        cell_idx,
                        prefix_index,
                        grid_w * grid_h,
                    ),
                    score,
                    cell_idx,
                    version: 0,
                });
            }
        }

        while let Some(entry) = priority_queue.pop() {
            let curr_idx = entry.cell_idx;
            if processed[curr_idx] || entry.version != priority_versions[curr_idx] {
                continue;
            }
            processed[curr_idx] = true;
            if abort_flag.is_some_and(|flag| {
                flag.load(std::sync::atomic::Ordering::Relaxed)
            }) {
                return Err(());
            }
            let Some(&(glyph_idx, inverted, _)) = candidate_lists[curr_idx].get(prefix_index)
            else {
                continue;
            };
            let candidate = (glyph_idx, inverted);
            let current_choice = grid_state[curr_idx];
            if candidate == current_choice {
                continue;
            }
            let band_candidate = band_cache.as_mut().map(|cache| {
                evaluate_score_fix_band_candidate(
                    cache,
                    grid_state,
                    grid_data,
                    curr_idx,
                    candidate,
                    contour_weights.boundary_balance,
                    grid_w,
                    grid_h,
                )
            });

            // N=1 remains the enabled baseline for the primary worst-first
            // search. Additional ordering trials accept their first moves by
            // the configured score, so changing the visit order can actually
            // lead them into different neighbourhood-dependent states.
            if prefix_index == 0 && force_first_prefix {
                apply_choice_to_raster(
                    raster,
                    grid_data,
                    glyphs,
                    grid_w,
                    gw,
                    gh,
                    curr_idx,
                    candidate,
                );
                grid_state[curr_idx] = candidate;
                paint_global_preview_cell(
                    &mut global_output_preview,
                    preview_width,
                    curr_idx,
                    grid_w,
                    &grid_data[curr_idx],
                    &glyphs[candidate.0],
                    candidate.1,
                    gw,
                    gh,
                );
                if let (Some(cache), Some(evaluation)) =
                    (band_cache.as_mut(), band_candidate)
                {
                    apply_score_fix_band_commit(cache, curr_idx, evaluation, grid_w, grid_h);
                }
                changes += 1;
                prefix_changed = true;
                priority_refreshes += refresh_score_fix_priorities(
                    curr_idx,
                    raster,
                    grid_w,
                    grid_h,
                    gw,
                    gh,
                    threshold,
                    prefix_index,
                    &candidate_lists,
                    &processed,
                    &mut priority_scores,
                    &mut priority_versions,
                    &mut priority_queue,
                    ordering,
                    &spiral_ranks,
                );
                continue;
            }

            let (current_score, candidate_score) = if let Some(evaluation) = band_candidate {
                (evaluation.current_score, evaluation.candidate_score)
            } else {
                let cell_x = curr_idx % grid_w;
                let cell_y = curr_idx / grid_w;
                let output_patch = extract_local_patch(
                    &global_output_preview,
                    cell_x,
                    cell_y,
                    grid_w,
                    grid_h,
                    crate::contour_score::PREVIEW_CELL_WIDTH,
                    crate::contour_score::PREVIEW_CELL_HEIGHT,
                );
                let mut output_preview = output_patch.pixels;
                let local_preview_width = output_patch.width;
                let prepared_reference = cached_references[curr_idx]
                    .as_ref()
                    .expect("smoothing reference should be cached");
                if contour_weights.fidelity > 0.0 {
                    let baseline_preview = output_preview.clone();
                    let (current_score_details, prepared_output) =
                        crate::contour_score::score_rgb_and_prepare_output_with_weights(
                            prepared_reference,
                            &baseline_preview,
                            crate::contour_score::PREVIEW_CELL_WIDTH,
                            contour_weights,
                        );
                    paint_preview_center_glyph(
                        &mut output_preview,
                        local_preview_width,
                        &grid_data[curr_idx],
                        &glyphs[glyph_idx],
                        inverted,
                        gw,
                        gh,
                    );
                    let candidate_score =
                        crate::contour_score::score_rgb_candidate_against_prepared_with_weights(
                            prepared_reference,
                            &prepared_output,
                            &baseline_preview,
                            &output_preview,
                            NEIGHBORHOOD_RADIUS * crate::contour_score::PREVIEW_CELL_WIDTH,
                            NEIGHBORHOOD_RADIUS * crate::contour_score::PREVIEW_CELL_HEIGHT,
                            crate::contour_score::PREVIEW_CELL_WIDTH,
                            crate::contour_score::PREVIEW_CELL_HEIGHT,
                            crate::contour_score::PREVIEW_CELL_WIDTH,
                            contour_weights,
                        )
                        .total;
                    (current_score_details.total, candidate_score)
                } else {
                    let current_score = cached_cell_scores[curr_idx].unwrap_or_else(|| {
                        crate::contour_score::score_rgb_against_prepared_with_weights(
                            prepared_reference,
                            &output_preview,
                            crate::contour_score::PREVIEW_CELL_WIDTH,
                            contour_weights,
                        )
                        .total
                    });
                    paint_preview_center_glyph(
                        &mut output_preview,
                        local_preview_width,
                        &grid_data[curr_idx],
                        &glyphs[glyph_idx],
                        inverted,
                        gw,
                        gh,
                    );
                    let candidate_score =
                        crate::contour_score::score_rgb_against_prepared_with_weights(
                            prepared_reference,
                            &output_preview,
                            crate::contour_score::PREVIEW_CELL_WIDTH,
                            contour_weights,
                        )
                        .total;
                    (current_score, candidate_score)
                }
            };
            cached_cell_scores[curr_idx] = Some(current_score);
            if candidate_score + 1e-9 >= current_score {
                continue;
            }

            if !args.score_fix_neighborhood_guard {
                apply_choice_to_raster(
                    raster,
                    grid_data,
                    glyphs,
                    grid_w,
                    gw,
                    gh,
                    curr_idx,
                    candidate,
                );
                grid_state[curr_idx] = candidate;
                paint_global_preview_cell(
                    &mut global_output_preview,
                    preview_width,
                    curr_idx,
                    grid_w,
                    &grid_data[curr_idx],
                    &glyphs[candidate.0],
                    candidate.1,
                    gw,
                    gh,
                );
                if let (Some(cache), Some(evaluation)) =
                    (band_cache.as_mut(), band_candidate)
                {
                    apply_score_fix_band_commit(cache, curr_idx, evaluation, grid_w, grid_h);
                }
                for idx in affected_discontinuity_cell_indices(curr_idx, grid_w, grid_h) {
                    cached_cell_scores[idx] = None;
                }
                cached_cell_scores[curr_idx] = Some(if let Some(cache) = band_cache.as_mut() {
                    let length = score_fix_patch_boundary_length_from_cells(
                        grid_state,
                        grid_data,
                        &cache.glyph_previews,
                        curr_idx,
                        None,
                        grid_w,
                        grid_h,
                    );
                    cache.output_patch_lengths[curr_idx] = Some(length);
                    score_fix_band_score(
                        cache.reference_patch_lengths[curr_idx],
                        length,
                        contour_weights.boundary_balance,
                    )
                } else {
                    candidate_score
                });
                changes += 1;
                prefix_changed = true;
                priority_refreshes += refresh_score_fix_priorities(
                    curr_idx,
                    raster,
                    grid_w,
                    grid_h,
                    gw,
                    gh,
                    threshold,
                    prefix_index,
                    &candidate_lists,
                    &processed,
                    &mut priority_scores,
                    &mut priority_versions,
                    &mut priority_queue,
                    ordering,
                    &spiral_ranks,
                );
                continue;
            }

            let affected = affected_discontinuity_cell_indices(curr_idx, grid_w, grid_h);
            let mut before_sum = 0.0;
            for &idx in &affected {
                if cached_cell_scores[idx].is_none() {
                    cached_cell_scores[idx] = Some(if let Some(cache) = band_cache.as_mut() {
                        let length = cache.output_patch_lengths[idx].unwrap_or_else(|| {
                            let length = score_fix_patch_boundary_length_from_cells(
                                grid_state,
                                grid_data,
                                &cache.glyph_previews,
                                idx,
                                None,
                                grid_w,
                                grid_h,
                            );
                            cache.output_patch_lengths[idx] = Some(length);
                            length
                        });
                        score_fix_band_score(
                            cache.reference_patch_lengths[idx],
                            length,
                            contour_weights.boundary_balance,
                        )
                    } else {
                        if cached_references[idx].is_none() {
                            cached_references[idx] = Some(prepare_local_preview_reference(
                                source_raster,
                                idx,
                                grid_w,
                                grid_h,
                                gw,
                                gh,
                                contour_weights,
                            ));
                        }
                        score_local_normalized_preview_output(
                            cached_references[idx]
                                .as_ref()
                                .expect("smoothing reference should be cached"),
                            &global_output_preview,
                            idx,
                            grid_w,
                            grid_h,
                            contour_weights,
                        )
                    });
                }
                before_sum +=
                    cached_cell_scores[idx].expect("smoothing cell score should be cached");
            }
            let mut band_patch_lengths = [0usize; 9];
            let mut new_scores = [0.0f64; 9];
            if let (Some(cache), Some(_evaluation)) =
                (band_cache.as_ref(), band_candidate)
            {
                for (slot, &idx) in affected.iter().enumerate() {
                    let length = score_fix_patch_boundary_length_from_cells(
                        grid_state,
                        grid_data,
                        &cache.glyph_previews,
                        idx,
                        Some((curr_idx, candidate)),
                        grid_w,
                        grid_h,
                    );
                    band_patch_lengths[slot] = length;
                    new_scores[slot] = score_fix_band_score(
                        cache.reference_patch_lengths[idx],
                        length,
                        contour_weights.boundary_balance,
                    );
                }
            } else {
                apply_choice_to_raster(
                    raster,
                    grid_data,
                    glyphs,
                    grid_w,
                    gw,
                    gh,
                    curr_idx,
                    candidate,
                );
                paint_global_preview_cell(
                    &mut global_output_preview,
                    preview_width,
                    curr_idx,
                    grid_w,
                    &grid_data[curr_idx],
                    &glyphs[candidate.0],
                    candidate.1,
                    gw,
                    gh,
                );
                for (slot, &idx) in affected.iter().enumerate() {
                    new_scores[slot] = score_local_normalized_preview_output(
                        cached_references[idx]
                            .as_ref()
                            .expect("smoothing reference should be cached"),
                        &global_output_preview,
                        idx,
                        grid_w,
                        grid_h,
                        contour_weights,
                    );
                }
            }
            let after_sum = new_scores[..affected.len()].iter().sum::<f64>();
            if after_sum + 1e-9 < before_sum {
                if band_candidate.is_some() {
                    apply_choice_to_raster(
                        raster,
                        grid_data,
                        glyphs,
                        grid_w,
                        gw,
                        gh,
                        curr_idx,
                        candidate,
                    );
                    paint_global_preview_cell(
                        &mut global_output_preview,
                        preview_width,
                        curr_idx,
                        grid_w,
                        &grid_data[curr_idx],
                        &glyphs[candidate.0],
                        candidate.1,
                        gw,
                        gh,
                    );
                }
                grid_state[curr_idx] = candidate;
                if let (Some(cache), Some(evaluation)) =
                    (band_cache.as_mut(), band_candidate)
                {
                    apply_score_fix_band_commit(
                        cache,
                        curr_idx,
                        evaluation,
                        grid_w,
                        grid_h,
                    );
                    for (&idx, &length) in affected
                        .iter()
                        .zip(&band_patch_lengths[..affected.len()])
                    {
                        cache.output_patch_lengths[idx] = Some(length);
                    }
                }
                for (&idx, &score) in affected
                    .iter()
                    .zip(&new_scores[..affected.len()])
                {
                    cached_cell_scores[idx] = Some(score);
                }
                changes += 1;
                prefix_changed = true;
                priority_refreshes += refresh_score_fix_priorities(
                    curr_idx,
                    raster,
                    grid_w,
                    grid_h,
                    gw,
                    gh,
                    threshold,
                    prefix_index,
                    &candidate_lists,
                    &processed,
                    &mut priority_scores,
                    &mut priority_versions,
                    &mut priority_queue,
                    ordering,
                    &spiral_ranks,
                );
            } else {
                if band_candidate.is_none() {
                    apply_choice_to_raster(
                        raster,
                        grid_data,
                        glyphs,
                        grid_w,
                        gw,
                        gh,
                        curr_idx,
                        current_choice,
                    );
                    paint_global_preview_cell(
                        &mut global_output_preview,
                        preview_width,
                        curr_idx,
                        grid_w,
                        &grid_data[curr_idx],
                        &glyphs[current_choice.0],
                        current_choice.1,
                        gw,
                        gh,
                    );
                }
                rejected += 1;
            }
        }

        // No accepted move means this complete prefix is byte-for-byte the
        // same as the preceding one, so its already-known global score wins or
        // loses identically. Avoid an expensive full-image contour trace.
        if !prefix_changed {
            continue;
        }
        let prefix_score = if let Some(cache) = band_cache.as_ref() {
            score_fix_band_score(
                cache.global_reference_length,
                cache.global_output_length,
                contour_weights.boundary_balance,
            )
        } else {
            crate::contour_score::score_rgb_against_prepared_with_weights(
                global_reference
                    .as_ref()
                    .expect("general smoothing reference should be prepared"),
                &global_output_preview,
                crate::contour_score::PREVIEW_CELL_WIDTH,
                contour_weights,
            )
            .total
        };
        if prefix_index == 0 || prefix_score + 1e-9 < best_prefix_score {
            best_prefix_score = prefix_score;
            best_prefix = prefix_index + 1;
            best_prefix_raster.copy_from_slice(raster);
            best_prefix_state.copy_from_slice(grid_state);
        }
    }

    raster.copy_from_slice(&best_prefix_raster);
    grid_state.copy_from_slice(&best_prefix_state);
    Ok(ScoreFixPrefixStats {
        target_count,
        candidate_trials,
        duplicate_candidates_skipped,
        changes,
        rejected,
        prefix_count,
        best_prefix,
        initial_score,
        best_score: best_prefix_score,
        priority_refreshes,
        used_band_fast_path: band_cache.is_some(),
    })
}

pub fn render_blocks(
    image: &AnsiImage,
    args: &RenderArgs,
    render: Render,
    glyphs: &[FastGlyph],
    abort_flag: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> RenderResult {
    // Redirect to smooth renderer but with smoothing features effectively disabled by the 'args.smooth' flag check inside.
    // However, if args.smooth is false, we need to ensure render_blocks_smooth behaves correctly (it does, assuming I fixed the loop issue).
    // The user wants "quantized the same as is donen when --smooth is used".
    // This implies using the exact same pipeline.
    render_blocks_smooth(image, args, render, glyphs, abort_flag)
}

#[inline(always)]
fn luma(rgb: &[u8; 3]) -> u8 {
    (0.299 * rgb[0] as f32 + 0.587 * rgb[1] as f32 + 0.114 * rgb[2] as f32).round() as u8
}

#[inline(always)]
fn last_max_index_u16(counts: &[u16; 256]) -> (usize, u16) {
    let mut best_idx = 0usize;
    let mut best_count = 0u16;
    for (idx, &count) in counts.iter().enumerate() {
        if count >= best_count {
            best_count = count;
            best_idx = idx;
        }
    }
    (best_idx, best_count)
}

#[inline(always)]
fn canonical_shape_hash(indices: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut remap = [u8::MAX; 256];
    let mut next = 0u8;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();

    indices.len().hash(&mut hasher);
    for &idx in indices {
        let slot = &mut remap[idx as usize];
        if *slot == u8::MAX {
            *slot = next;
            next = next.wrapping_add(1);
        }
        slot.hash(&mut hasher);
    }

    hasher.finish()
}

fn glyph_set_cache_hash(glyphs: &[FastGlyph], gw: usize, gh: usize) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    gw.hash(&mut hasher);
    gh.hash(&mut hasher);
    glyphs.len().hash(&mut hasher);
    for glyph in glyphs {
        glyph.ch.hash(&mut hasher);
        glyph.bitmap.hash(&mut hasher);
    }
    hasher.finish()
}

thread_local! {
    static THREAD_SHAPE_CACHE: std::cell::RefCell<std::collections::HashMap<(u64, u64), (usize, bool)>> =
        std::cell::RefCell::new(std::collections::HashMap::with_capacity(8192));
}

pub fn render_blocks_smooth(
    image: &AnsiImage,
    args: &RenderArgs,
    render: Render,
    glyphs: &[FastGlyph],
    abort_flag: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> RenderResult {
    let process_start_time = RenderInstant::now();
    let contour_cache_start = crate::contour_score::cache_stats();

    if glyphs.is_empty() {
        return "Error: no glyphs available".into();
    }
    let (gw, gh) = image.glyphs.metrics;
    let bp = gh * gw;
    let glyph_cache_hash = glyph_set_cache_hash(glyphs, gw, gh);

    // Grid Dimensions
    let grid_h = image.block.len();
    let grid_w = if grid_h > 0 { image.block[0].len() } else { 0 };
    if grid_w == 0 {
        return "".into();
    }

    let needs_frequency_metric = true;

    let scales = vec![(1.0, 1.0)];

    // Parallel CPU rendering
    let rows_results: Vec<(Vec<BlockData>, Vec<(usize, bool)>)> = image
        .block
        .par_iter()
        .map(|block_row| {
                if let Some(flag) = &abort_flag {
                    if flag.load(std::sync::atomic::Ordering::Relaxed) {
                        return (Vec::new(), Vec::new());
                    }
                }
                let mut row_data = Vec::with_capacity(block_row.len());
                let mut row_state = Vec::with_capacity(block_row.len());
                let mut working_pixels_buffer = Vec::with_capacity(bp);
                let mut original_pixels = Vec::with_capacity(bp);
                let mut indices = Vec::with_capacity(bp);

                for blk in block_row {
                    if let Some(flag) = &abort_flag {
                        if flag.load(std::sync::atomic::Ordering::Relaxed) {
                            return (Vec::new(), Vec::new());
                        }
                    }

                    // 1. Capture Base Pixels
                    original_pixels.clear();
                    for y in 0..gh {
                        for x in 0..gw {
                            let px = &blk.pixels[y][x];
                            let rgb = unpack_rgb(px.orig);
                            let p = PrePixel {
                                r: rgb[0] as u64,
                                g: rgb[1] as u64,
                                b: rgb[2] as u64,
                                sq: (rgb[0] as u64).pow(2)
                                    + (rgb[1] as u64).pow(2)
                                    + (rgb[2] as u64).pow(2),
                                ansi_part: px.ansi_part,
                                irc_part: px.irc_part,
                            };
                            original_pixels.push(p);
                        }
                    }

                    let mut best_overall_glyph = 0;
                    let mut best_overall_inv = false;
                    let mut best_overall_fg = [0u8; 3];
                    let mut best_overall_bg = [0u8; 3];

                    for (sx, sy) in &scales {
                        if *sx == 1.0 && *sy == 1.0 {
                            working_pixels_buffer.clear();
                            working_pixels_buffer.extend_from_slice(&original_pixels);
                        } else {
                            get_transformed_block_pixels_into(
                                &original_pixels,
                                gw,
                                gh,
                                *sx,
                                *sy,
                                &mut working_pixels_buffer,
                            );
                        }
                        let pixels = &working_pixels_buffer;

                        let is_ansi = if args.as_preview {
                            false
                        } else {
                            matches!(render, Render::Ansi | Render::Ansi24)
                        };
                        let palette: &[u32] = if is_ansi { &ANSI256 } else { &IRC99 };
                        let mut base_cnt = [0u16; 256];
                        indices.clear();

                        for p in pixels {
                            let idx = if is_ansi { p.ansi_part } else { p.irc_part };
                            base_cnt[idx as usize] += 1;
                            indices.push(idx);
                        }

                        let shape_hash = canonical_shape_hash(&indices);
                        let shape_cache_key = (glyph_cache_hash, shape_hash);

                        let cached = THREAD_SHAPE_CACHE
                            .with(|cache_cell| cache_cell.borrow().get(&shape_cache_key).copied());

                        let (scale_best_glyph, scale_best_inv) =
                            if let Some(c) = cached.filter(|&(gi, _)| gi < glyphs.len()) {
                                c
                            } else {
                                let mut unique_block_colors = [0u8; 32];
                                let mut num_unique_colors = 0usize;
                                for idx in 0..256 {
                                    if base_cnt[idx] > 0 {
                                        if num_unique_colors < 32 {
                                            unique_block_colors[num_unique_colors] = idx as u8;
                                            num_unique_colors += 1;
                                        }
                                    }
                                }
                                let active_colors = &unique_block_colors[0..num_unique_colors];

                                let mut best_cost = u64::MAX;
                                let mut best_glyph = 0;
                                let mut best_inv = false;

                                if num_unique_colors <= 1 {
                                    best_glyph = 0;
                                    best_inv = false;
                                } else if bp <= 128 {
                                    let mut color_masks = [0u128; 256];
                                    for (i, &color_idx) in indices.iter().enumerate() {
                                        if i < 128 {
                                            color_masks[color_idx as usize] |= 1u128 << i;
                                        }
                                    }

                                    for (gi, fglyph) in glyphs.iter().enumerate() {
                                        if gi & 0x3f == 0 {
                                            if let Some(flag) = &abort_flag {
                                                if flag.load(std::sync::atomic::Ordering::Relaxed) {
                                                    return (Vec::new(), Vec::new());
                                                }
                                            }
                                        }

                                        let mut ones_mode_count = 0u16;
                                        let mut zeros_mode_count = 0u16;
                                        for &idx in active_colors {
                                            let c_ones = (color_masks[idx as usize] & fglyph.mask)
                                                .count_ones()
                                                as u16;
                                            if c_ones > ones_mode_count {
                                                ones_mode_count = c_ones;
                                            }
                                            let c_zeros = base_cnt[idx as usize] - c_ones;
                                            if c_zeros > zeros_mode_count {
                                                zeros_mode_count = c_zeros;
                                            }
                                        }

                                        let ones_len = fglyph.ones.len() as u64;
                                        let zeros_len = (bp - fglyph.ones.len()) as u64;

                                        let cost_normal = (ones_len - ones_mode_count as u64)
                                            + (zeros_len - zeros_mode_count as u64);
                                        if cost_normal < best_cost {
                                            best_cost = cost_normal;
                                            best_glyph = gi;
                                            best_inv = false;
                                        }

                                        let cost_inverted = (zeros_len - zeros_mode_count as u64)
                                            + (ones_len - ones_mode_count as u64);
                                        if cost_inverted < best_cost {
                                            best_cost = cost_inverted;
                                            best_glyph = gi;
                                            best_inv = true;
                                        }

                                        if best_cost == 0 {
                                            break;
                                        }
                                    }
                                } else {
                                    let mut ones_cnt = [0u16; 256];
                                    for (gi, fglyph) in glyphs.iter().enumerate() {
                                        if gi & 0x3f == 0 {
                                            if let Some(flag) = &abort_flag {
                                                if flag.load(std::sync::atomic::Ordering::Relaxed) {
                                                    return (Vec::new(), Vec::new());
                                                }
                                            }
                                        }

                                        for &i in &fglyph.ones {
                                            ones_cnt[indices[i] as usize] += 1;
                                        }

                                        let mut ones_mode_count = 0u16;
                                        let mut zeros_mode_count = 0u16;
                                        for &idx in active_colors {
                                            let count = ones_cnt[idx as usize];
                                            if count > ones_mode_count {
                                                ones_mode_count = count;
                                            }
                                            let count_zeros = base_cnt[idx as usize] - count;
                                            if count_zeros > zeros_mode_count {
                                                zeros_mode_count = count_zeros;
                                            }
                                        }

                                        let ones_len = fglyph.ones.len() as u64;
                                        let zeros_len = (bp - fglyph.ones.len()) as u64;

                                        let cost_normal = (ones_len - ones_mode_count as u64)
                                            + (zeros_len - zeros_mode_count as u64);
                                        if cost_normal < best_cost {
                                            best_cost = cost_normal;
                                            best_glyph = gi;
                                            best_inv = false;
                                        }

                                        let cost_inverted = (zeros_len - zeros_mode_count as u64)
                                            + (ones_len - ones_mode_count as u64);
                                        if cost_inverted < best_cost {
                                            best_cost = cost_inverted;
                                            best_glyph = gi;
                                            best_inv = true;
                                        }

                                        for &idx in active_colors {
                                            ones_cnt[idx as usize] = 0;
                                        }

                                        if best_cost == 0 {
                                            break;
                                        }
                                    }
                                }

                                THREAD_SHAPE_CACHE.with(|cache_cell| {
                                    cache_cell
                                        .borrow_mut()
                                        .insert(shape_cache_key, (best_glyph, best_inv));
                                });
                                (best_glyph, best_inv)
                            };

                        let scale_best_fg;
                        let scale_best_bg;

                        let fglyph = &glyphs[scale_best_glyph];
                        let mut ones_cnt = [0u16; 256];
                        for &i in &fglyph.ones {
                            ones_cnt[indices[i] as usize] += 1;
                        }
                        let (ones_mode_idx, _) = last_max_index_u16(&ones_cnt);
                        let mut zeros_mode_idx = 0usize;
                        let mut zeros_mode_count = 0u16;
                        for idx in 0..256 {
                            let count = base_cnt[idx] - ones_cnt[idx];
                            if count >= zeros_mode_count {
                                zeros_mode_count = count;
                                zeros_mode_idx = idx;
                            }
                        }

                        let ones_len = fglyph.ones.len();
                        let zeros_len = bp - ones_len;
                        let (fgi_raw, bgi_raw, fg_len, bg_len) = if scale_best_inv {
                            (zeros_mode_idx, ones_mode_idx, zeros_len, ones_len)
                        } else {
                            (ones_mode_idx, zeros_mode_idx, ones_len, zeros_len)
                        };
                        let fgi = if fg_len == 0 { bgi_raw } else { fgi_raw };
                        let bgi = if bg_len == 0 { fgi_raw } else { bgi_raw };

                        if render != Render::Ansi24 {
                            scale_best_fg = unpack_rgb(palette[fgi.min(palette.len() - 1)]);
                            scale_best_bg = unpack_rgb(palette[bgi.min(palette.len() - 1)]);
                        } else {
                            let mut ones_rgb = None;
                            for i in 0..bp {
                                let is_under_ones = fglyph.ones.contains(&i);
                                let is_target = if !scale_best_inv {
                                    is_under_ones
                                } else {
                                    !is_under_ones
                                };
                                if is_target && indices[i] as usize == fgi {
                                    let px = unsafe { pixels.get_unchecked(i) };
                                    ones_rgb = Some([px.r as u8, px.g as u8, px.b as u8]);
                                    break;
                                }
                            }
                            scale_best_fg = ones_rgb
                                .unwrap_or_else(|| unpack_rgb(palette[fgi.min(palette.len() - 1)]));

                            let mut zeros_rgb = None;
                            for i in 0..bp {
                                let is_under_ones = fglyph.ones.contains(&i);
                                let is_target = if !scale_best_inv {
                                    !is_under_ones
                                } else {
                                    is_under_ones
                                };
                                if is_target && indices[i] as usize == bgi {
                                    let px = unsafe { pixels.get_unchecked(i) };
                                    zeros_rgb = Some([px.r as u8, px.g as u8, px.b as u8]);
                                    break;
                                }
                            }
                            scale_best_bg = zeros_rgb
                                .unwrap_or_else(|| unpack_rgb(palette[bgi.min(palette.len() - 1)]));
                        }

                        best_overall_glyph = scale_best_glyph;
                        best_overall_inv = scale_best_inv;
                        best_overall_fg = scale_best_fg;
                        best_overall_bg = scale_best_bg;
                    }
                    let best_fg_rgb = best_overall_fg;
                    let best_bg_rgb = best_overall_bg;
                    let best_glyph_idx = best_overall_glyph;
                    let best_inv = best_overall_inv;

                    let (final_fg, final_bg, fg_obj, bg_obj, alt_fg_obj, alt_bg_obj) =
                        if render == Render::Ansi24 {
                            (
                                best_fg_rgb,
                                best_bg_rgb,
                                Some(Colour::RGB(best_fg_rgb)),
                                Some(Colour::RGB(best_bg_rgb)),
                                None,
                                None,
                            )
                        } else if args.as_preview {
                            let col_hi = make_rgb_u32(&best_fg_rgb);
                            let col_lo = make_rgb_u32(&best_bg_rgb);

                            let hi_irc = nearest_strict_partitioned_hex_colour(
                                col_hi,
                                &IRC99,
                                args.grayscale_tolerance,
                            );
                            let lo_irc = nearest_strict_partitioned_hex_colour(
                                col_lo,
                                &IRC99,
                                args.grayscale_tolerance,
                            );

                            let hi_rgb = unpack_rgb(IRC99[hi_irc as usize]);
                            let lo_rgb = unpack_rgb(IRC99[lo_irc as usize]);
                            (
                                hi_rgb,
                                lo_rgb,
                                Some(Colour::RGB(hi_rgb)),
                                Some(Colour::RGB(lo_rgb)),
                                Some(Colour::RGB(hi_rgb)),
                                Some(Colour::RGB(lo_rgb)),
                            )
                        } else if render == Render::Irc {
                            let col_hi = make_rgb_u32(&best_fg_rgb);
                            let col_lo = make_rgb_u32(&best_bg_rgb);
                            let hi_idx = nearest_strict_partitioned_hex_colour(
                                col_hi,
                                &IRC99,
                                args.grayscale_tolerance,
                            );
                            let lo_idx = nearest_strict_partitioned_hex_colour(
                                col_lo,
                                &IRC99,
                                args.grayscale_tolerance,
                            );
                            let hi_rgb = unpack_rgb(IRC99[hi_idx as usize]);
                            let lo_rgb = unpack_rgb(IRC99[lo_idx as usize]);
                            (
                                hi_rgb,
                                lo_rgb,
                                Some(Colour::Index(hi_idx)),
                                Some(Colour::Index(lo_idx)),
                                None,
                                None,
                            )
                        } else {
                            let palette = &ANSI256[..];
                            let col_hi = make_rgb_u32(&best_fg_rgb);
                            let col_lo = make_rgb_u32(&best_bg_rgb);

                            let hi_idx = nearest_strict_partitioned_hex_colour(
                                col_hi,
                                palette,
                                args.grayscale_tolerance,
                            );
                            let lo_idx = nearest_strict_partitioned_hex_colour(
                                col_lo,
                                palette,
                                args.grayscale_tolerance,
                            );

                            let hi_rgb = unpack_rgb(palette[hi_idx as usize]);
                            let lo_rgb = unpack_rgb(palette[lo_idx as usize]);
                            (
                                hi_rgb,
                                lo_rgb,
                                Some(Colour::Index(hi_idx)),
                                Some(Colour::Index(lo_idx)),
                                None,
                                None,
                            )
                        };

                    row_data.push(BlockData {
                        fg: final_fg,
                        bg: final_bg,
                        fg_col: fg_obj,
                        bg_col: bg_obj,
                        alt_fg_col: alt_fg_obj,
                        alt_bg_col: alt_bg_obj,
                    });
                    row_state.push((best_glyph_idx, best_inv));
                }
                (row_data, row_state)
        })
        .collect();

    let mut grid_data = Vec::with_capacity(grid_h * grid_w);
    let mut grid_state = Vec::with_capacity(grid_h * grid_w);
    for (r_d, r_s) in rows_results {
        if r_d.is_empty() && grid_h > 0 && grid_w > 0 {
            return "Cancelled".into();
        }
        grid_data.extend(r_d);
        grid_state.extend(r_s);
    }

    if grid_data.len() != grid_h * grid_w {
        log::error!(
            "Incomplete grid data: expected {}, got {}. Returning Cancelled.",
            grid_h * grid_w,
            grid_data.len()
        );
        return "Cancelled".into();
    }

    let glyph_match_time = process_start_time.elapsed();
    let smoothing_prepare_started = RenderInstant::now();

    let fft_debug_root = args.fft_debug_dir.as_deref().and_then(ensure_fft_debug_dir);

    let mut rendered_raster = if needs_frequency_metric {
        Some(build_rendered_raster(
            &grid_data,
            &grid_state,
            glyphs,
            grid_w,
            grid_h,
            gw,
            gh,
        ))
    } else {
        None
    };
    let source_raster = build_source_raster(image, grid_w, grid_h, gw, gh);
    let contour_weights = contour_score_weights(args);

    if let (Some(root), Some(raster)) = (fft_debug_root.as_ref(), rendered_raster.as_ref()) {
        if let Err(err) = dump_fft_stage_inputs(root, "initial", raster, grid_w, grid_h, gw, gh) {
            log::warn!("failed to dump initial FFT inputs: {}", err);
        }
    }

    // Score Fix can identify its target cells before building candidate
    // costs. This avoids doing glyph work for the usually much larger set of
    // cells below the discontinuity threshold.
    let mut score_fix_initial_disc_scores = if args.score_fix {
        Some(all_cell_discontinuity_scores(
            rendered_raster
                .as_ref()
                .expect("rendered raster should exist when smoothing is enabled"),
            grid_w,
            grid_h,
            gw,
            gh,
        ))
    } else {
        None
    };

    let block_glyph_costs: Vec<Vec<u64>> = if args.score_fix {
        grid_data
            .par_iter()
            .enumerate()
            .map(|(curr_idx, data)| {
                if score_fix_initial_disc_scores
                    .as_ref()
                    .is_some_and(|scores| scores[curr_idx] < args.discontinuity_threshold.max(0.0))
                {
                    return Vec::new();
                }

                let x = curr_idx % grid_w;
                let y = curr_idx / grid_w;
                let blk = &image.block[y][x];

                if let Some(flag) = &abort_flag {
                    if flag.load(std::sync::atomic::Ordering::Relaxed) {
                        return Vec::new();
                    }
                }
                let fg_p = PrePixel {
                    r: data.fg[0] as u64,
                    g: data.fg[1] as u64,
                    b: data.fg[2] as u64,
                    sq: (data.fg[0] as u64).pow(2)
                        + (data.fg[1] as u64).pow(2)
                        + (data.fg[2] as u64).pow(2),
                    ansi_part: 0,
                    irc_part: 0,
                };
                let bg_p = PrePixel {
                    r: data.bg[0] as u64,
                    g: data.bg[1] as u64,
                    b: data.bg[2] as u64,
                    sq: (data.bg[0] as u64).pow(2)
                        + (data.bg[1] as u64).pow(2)
                        + (data.bg[2] as u64).pow(2),
                    ansi_part: 0,
                    irc_part: 0,
                };

                let mut base_cost = 0u64;
                let mut diff = vec![0i64; bp];

                for i in 0..bp {
                    let lx = i % gw;
                    let ly = i / gw;
                    let rgb = unpack_rgb(blk.pixels[ly][lx].orig);
                    let p_r = rgb[0] as u64;
                    let p_g = rgb[1] as u64;
                    let p_b = rgb[2] as u64;
                    let p_sq = p_r * p_r + p_g * p_g + p_b * p_b;

                    let c_fg = p_sq + fg_p.sq - 2 * (p_r * fg_p.r + p_g * fg_p.g + p_b * fg_p.b);
                    let c_bg = p_sq + bg_p.sq - 2 * (p_r * bg_p.r + p_g * bg_p.g + p_b * bg_p.b);
                    let bg_wins = if c_bg <= c_fg { 0i64 } else { 1i64 };
                    base_cost += bg_wins as u64;
                    let fg_wins = if c_fg <= c_bg { 0i64 } else { 1i64 };
                    diff[i] = fg_wins - bg_wins;
                }

                let total_diff: i64 = diff.iter().sum();
                let mut costs = Vec::with_capacity(glyphs.len() * 2);

                for (gi, fglyph) in glyphs.iter().enumerate() {
                    if gi & 0x3f == 0 {
                        if let Some(flag) = &abort_flag {
                            if flag.load(std::sync::atomic::Ordering::Relaxed) {
                                return Vec::new();
                            }
                        }
                    }

                    let mut dot = 0i64;
                    for &i in &fglyph.ones {
                        dot += unsafe { *diff.get_unchecked(i) };
                    }

                    let c_total_f = (base_cost as i64 + dot).max(0) as u64;
                    let c_total_t = (base_cost as i64 + total_diff - dot).max(0) as u64;

                    costs.push(c_total_f);
                    costs.push(c_total_t);
                }

                costs
            })
            .collect()
    } else {
        Vec::new()
    };

    let smoothing_prepare_time = smoothing_prepare_started.elapsed();
    let initial_time = process_start_time.elapsed();
    let mut smoothing_search_time = std::time::Duration::ZERO;
    let mut shape_refine_time = std::time::Duration::ZERO;

    if args.score_fix {
        let raster = rendered_raster
            .as_mut()
            .expect("rendered raster should exist when smoothing is enabled");
        let started = RenderInstant::now();
        let target_scores = score_fix_initial_disc_scores
            .take()
            .unwrap_or_else(|| all_cell_discontinuity_scores(raster, grid_w, grid_h, gw, gh));
        let baseline_raster = raster.clone();
        let baseline_state = grid_state.clone();
        let ordering_count = (args.score_fix_orderings as usize).clamp(1, ScoreFixOrdering::ALL.len());
        let trial_results = ScoreFixOrdering::ALL[..ordering_count]
            .par_iter()
            .map(|&ordering| {
                let mut trial_raster = baseline_raster.clone();
                let mut trial_state = baseline_state.clone();
                let stats = run_score_fix_prefix_search(
                    args,
                    ordering,
                    &mut trial_raster,
                    &mut trial_state,
                    &grid_data,
                    glyphs,
                    &block_glyph_costs,
                    target_scores.clone(),
                    &source_raster,
                    &contour_weights,
                    grid_w,
                    grid_h,
                    gw,
                    gh,
                    abort_flag.as_ref(),
                )?;
                Ok((ordering, stats, trial_raster, trial_state))
            })
            .collect::<Vec<Result<_, ()>>>();
        let mut trials = Vec::with_capacity(trial_results.len());
        for result in trial_results {
            match result {
                Ok(trial) => trials.push(trial),
                Err(()) => return "Cancelled".into(),
            }
        }
        let mut best_trial_idx = 0usize;
        for idx in 1..trials.len() {
            if trials[idx].1.best_score + 1e-9 < trials[best_trial_idx].1.best_score {
                best_trial_idx = idx;
            }
        }
        let order_scores = trials
            .iter()
            .map(|(ordering, stats, _, _)| {
                format!("{}={:.6}", ordering.name(), stats.best_score)
            })
            .collect::<Vec<_>>()
            .join(", ");
        let selected_ordering = trials[best_trial_idx].0;
        let (_, stats, best_raster, best_state) = trials.swap_remove(best_trial_idx);
        raster.copy_from_slice(&best_raster);
        grid_state.copy_from_slice(&best_state);
        smoothing_search_time = started.elapsed();
        log::info!(
            "smoothing order search [{}] selected {}; selected {} cells, tried {} frozen alternatives (skipped {} duplicate/current rasters), made {} progressive changes, refreshed {} affected priorities, and rejected {} neighbor regressions; evaluated N=1..{}, retained N={} with contour {:.6} (N=0 was {:.6}) using {} in {:?}",
            order_scores,
            selected_ordering.name(),
            stats.target_count,
            stats.candidate_trials,
            stats.duplicate_candidates_skipped,
            stats.changes,
            stats.priority_refreshes,
            stats.rejected,
            stats.prefix_count,
            stats.best_prefix,
            stats.best_score,
            stats.initial_score,
            if stats.used_band_fast_path {
                "incremental Band scorer"
            } else {
                "general contour scorer"
            },
            smoothing_search_time,
        );
    }

    // Start from normal Score Fix, then exhaustively search every distinct
    // rendered glyph state using the overlapping local geometry energy.
    if args.score_fix && args.score_fix_geometry_first {
        let raster = rendered_raster
            .as_mut()
            .expect("rendered raster should exist when smoothing is enabled");
        let started = RenderInstant::now();
        let target_scores = all_cell_discontinuity_scores(raster, grid_w, grid_h, gw, gh);
        let stats = match run_geometry_score_fix(
            args,
            raster,
            &mut grid_state,
            &grid_data,
            glyphs,
            &block_glyph_costs,
            target_scores,
            &source_raster,
            grid_w,
            grid_h,
            gw,
            gh,
            abort_flag.as_ref(),
        ) {
            Ok(stats) => stats,
            Err(()) => return "Cancelled".into(),
        };
        shape_refine_time = started.elapsed();
        log::info!(
            "smoothing shape refinement selected {} cells, used {} distinct states across source-ranked top-N candidate lists ({} cheap candidate evaluations, {} median-flow evaluations), made {} monotonic topology-preserving median-flow changes, and reduced overlapping-patch energy from {:.6} to {:.6} in {:?}",
            stats.target_count,
            stats.distinct_states,
            stats.candidate_evaluations,
            stats.flow_candidate_evaluations,
            stats.changes,
            stats.initial_score,
            stats.final_score,
            shape_refine_time,
        );
    }

    // Kept temporarily as a compile-checked reference for the recovered
    // single-pass implementation. Prefix search above is the active path.
    if false && args.score_fix {
        let raster = rendered_raster
            .as_mut()
            .expect("rendered raster should exist when smoothing is enabled");
        let started = RenderInstant::now();
        let pre_score_fix_raster = raster.clone();
        let pre_score_fix_grid_state = grid_state.clone();
        let before_global = preview_contour_score(
            &source_raster,
            raster,
            grid_w,
            grid_h,
            gw,
            gh,
            &contour_weights,
        )
        .total;

        let threshold = args.discontinuity_threshold.max(0.0);
        let target_scores = score_fix_initial_disc_scores
            .take()
            .unwrap_or_else(|| all_cell_discontinuity_scores(raster, grid_w, grid_h, gw, gh));
        let mut targets = target_scores
            .into_iter()
            .enumerate()
            .filter(|(_, score)| *score >= threshold)
            .collect::<Vec<_>>();
        targets.sort_by(|a, b| b.1.total_cmp(&a.1));
        let target_count = targets.len();

        let candidate_count = args.score_fix_candidates.max(1) as usize;
        let mut changes = 0usize;
        let mut rejected = 0usize;
        let mut candidate_trials = 0usize;
        let mut duplicate_candidates_skipped = 0usize;
        let mut local_improvement = 0.0f64;
        let mut cached_cell_scores = vec![None::<f64>; grid_w * grid_h];
        let mut reference_needed = vec![false; grid_w * grid_h];
        for &(target_idx, _) in &targets {
            if args.score_fix_neighborhood_guard {
                for affected_idx in
                    affected_discontinuity_cell_indices(target_idx, grid_w, grid_h)
                {
                    reference_needed[affected_idx] = true;
                }
            } else {
                reference_needed[target_idx] = true;
            }
        }
        let mut cached_references = reference_needed
            .into_par_iter()
            .enumerate()
            .map(|(idx, needed)| {
                needed.then(|| {
                    prepare_local_preview_reference(
                        &source_raster,
                        idx,
                        grid_w,
                        grid_h,
                        gw,
                        gh,
                        &contour_weights,
                    )
                })
            })
            .collect::<Vec<_>>();
        for (curr_idx, _) in targets {
            if let Some(flag) = &abort_flag {
                if flag.load(std::sync::atomic::Ordering::Relaxed) {
                    return "Cancelled".into();
                }
            }
            if grid_data[curr_idx].fg == grid_data[curr_idx].bg {
                continue;
            }

            let current_choice = grid_state[curr_idx];
            let cell_x = curr_idx % grid_w;
            let cell_y = curr_idx / grid_w;
            let output_patch = extract_local_patch(raster, cell_x, cell_y, grid_w, grid_h, gw, gh);
            let patch_cells = NEIGHBORHOOD_RADIUS * 2 + 1;
            let (mut output_preview, preview_width, _) =
                crate::contour_score::normalize_preview_raster(
                    &output_patch.pixels,
                    patch_cells,
                    patch_cells,
                    gw,
                    gh,
                );
            if cached_references[curr_idx].is_none() {
                cached_references[curr_idx] = Some(prepare_local_preview_reference(
                    &source_raster,
                    curr_idx,
                    grid_w,
                    grid_h,
                    gw,
                    gh,
                    &contour_weights,
                ));
            }
            let prepared_reference = cached_references[curr_idx]
                .as_ref()
                .expect("smoothing reference should be cached");
            let baseline_preview = output_preview.clone();
            let (current_score_details, prepared_output) =
                crate::contour_score::score_rgb_and_prepare_output_with_weights(
                    prepared_reference,
                    &baseline_preview,
                    crate::contour_score::PREVIEW_CELL_WIDTH,
                    &contour_weights,
                );
            let current_score = current_score_details.total;
            let mut best_score = current_score;
            let mut best_choice = current_choice;
            let candidates_with_duplicates = block_glyph_costs
                .get(curr_idx)
                .map(|costs| best_candidate_pool_pre(costs, candidate_count))
                .unwrap_or_default();
            let original_candidate_count = candidates_with_duplicates.len();
            let candidates = deduplicate_rendered_candidates(
                candidates_with_duplicates,
                glyphs,
                current_choice,
            );
            candidate_trials += candidates.len();
            duplicate_candidates_skipped += original_candidate_count - candidates.len();

            if candidates.len() >= 8 {
                let scored = candidates
                    .par_iter()
                    .map(|&(glyph_idx, inverted, _)| {
                        let candidate = (glyph_idx, inverted);
                        if candidate == current_choice {
                            return None;
                        }
                        let mut candidate_preview = output_preview.clone();
                        paint_preview_center_glyph(
                            &mut candidate_preview,
                            preview_width,
                            &grid_data[curr_idx],
                            &glyphs[glyph_idx],
                            inverted,
                            gw,
                            gh,
                        );
                        let score = crate::contour_score::score_rgb_candidate_against_prepared_with_weights(
                            prepared_reference,
                            &prepared_output,
                            &baseline_preview,
                            &candidate_preview,
                            NEIGHBORHOOD_RADIUS * crate::contour_score::PREVIEW_CELL_WIDTH,
                            NEIGHBORHOOD_RADIUS * crate::contour_score::PREVIEW_CELL_HEIGHT,
                            crate::contour_score::PREVIEW_CELL_WIDTH,
                            crate::contour_score::PREVIEW_CELL_HEIGHT,
                            crate::contour_score::PREVIEW_CELL_WIDTH,
                            &contour_weights,
                        )
                        .total;
                        Some((candidate, score))
                    })
                    .collect::<Vec<_>>();
                for (candidate, score) in scored.into_iter().flatten() {
                    if score + 1e-9 < best_score {
                        best_score = score;
                        best_choice = candidate;
                    }
                }
            } else {
                for (glyph_idx, inverted, _) in candidates {
                    let candidate = (glyph_idx, inverted);
                    if candidate == current_choice {
                        continue;
                    }
                    paint_preview_center_glyph(
                        &mut output_preview,
                        preview_width,
                        &grid_data[curr_idx],
                        &glyphs[glyph_idx],
                        inverted,
                        gw,
                        gh,
                    );
                    let score = crate::contour_score::score_rgb_candidate_against_prepared_with_weights(
                        prepared_reference,
                        &prepared_output,
                        &baseline_preview,
                        &output_preview,
                        NEIGHBORHOOD_RADIUS * crate::contour_score::PREVIEW_CELL_WIDTH,
                        NEIGHBORHOOD_RADIUS * crate::contour_score::PREVIEW_CELL_HEIGHT,
                        crate::contour_score::PREVIEW_CELL_WIDTH,
                        crate::contour_score::PREVIEW_CELL_HEIGHT,
                        crate::contour_score::PREVIEW_CELL_WIDTH,
                        &contour_weights,
                    )
                    .total;
                    paint_preview_center_glyph(
                        &mut output_preview,
                        preview_width,
                        &grid_data[curr_idx],
                        &glyphs[current_choice.0],
                        current_choice.1,
                        gw,
                        gh,
                    );
                    if score + 1e-9 < best_score {
                        best_score = score;
                        best_choice = candidate;
                    }
                }
            }

            if best_choice != current_choice {
                if !args.score_fix_neighborhood_guard {
                    apply_choice_to_raster(
                        raster,
                        &grid_data,
                        glyphs,
                        grid_w,
                        gw,
                        gh,
                        curr_idx,
                        best_choice,
                    );
                    grid_state[curr_idx] = best_choice;
                    local_improvement += current_score - best_score;
                    changes += 1;
                    continue;
                }

                cached_cell_scores[curr_idx] = Some(current_score);
                let affected = affected_discontinuity_cell_indices(curr_idx, grid_w, grid_h);
                let mut before_sum = 0.0;
                for &idx in &affected {
                    if cached_references[idx].is_none() {
                        cached_references[idx] = Some(prepare_local_preview_reference(
                            &source_raster,
                            idx,
                            grid_w,
                            grid_h,
                            gw,
                            gh,
                            &contour_weights,
                        ));
                    }
                    if cached_cell_scores[idx].is_none() {
                        let score = score_local_preview_output(
                            cached_references[idx]
                                .as_ref()
                                .expect("smoothing reference should be cached"),
                            raster,
                            idx,
                            grid_w,
                            grid_h,
                            gw,
                            gh,
                            &contour_weights,
                        );
                        cached_cell_scores[idx] = Some(score);
                    }
                    before_sum +=
                        cached_cell_scores[idx].expect("smoothing cell score should be cached");
                }
                apply_choice_to_raster(
                    raster,
                    &grid_data,
                    glyphs,
                    grid_w,
                    gw,
                    gh,
                    curr_idx,
                    best_choice,
                );
                let mut new_scores = Vec::with_capacity(affected.len());
                for &idx in &affected {
                    new_scores.push(score_local_preview_output(
                        cached_references[idx]
                            .as_ref()
                            .expect("smoothing reference should be cached"),
                        raster,
                        idx,
                        grid_w,
                        grid_h,
                        gw,
                        gh,
                        &contour_weights,
                    ));
                }
                let after_sum = new_scores.iter().sum::<f64>();
                if after_sum + 1e-9 < before_sum {
                    grid_state[curr_idx] = best_choice;
                    for (&idx, score) in affected.iter().zip(new_scores) {
                        cached_cell_scores[idx] = Some(score);
                    }
                    local_improvement += before_sum - after_sum;
                    changes += 1;
                } else {
                    apply_choice_to_raster(
                        raster,
                        &grid_data,
                        glyphs,
                        grid_w,
                        gw,
                        gh,
                        curr_idx,
                        current_choice,
                    );
                    rejected += 1;
                }
            }
        }

        let mut after_global = preview_contour_score(
            &source_raster,
            raster,
            grid_w,
            grid_h,
            gw,
            gh,
            &contour_weights,
        )
        .total;
        let rolled_back = after_global > before_global + 1e-9;
        if rolled_back {
            raster.copy_from_slice(&pre_score_fix_raster);
            grid_state.copy_from_slice(&pre_score_fix_grid_state);
            after_global = before_global;
        }
        log::info!(
            "smoothing selected {} cells at threshold {:.1}; mode={}; tried {} unique candidates (skipped {} duplicate rasters), kept {} regional improvements, rejected {} neighbor regressions: local -{:.6}, global {:.6} -> {:.6}, full rollback={} in {:?}",
            target_count,
            threshold,
            if args.score_fix_neighborhood_guard { "overlap-guard" } else { "center-3x3" },
            candidate_trials,
            duplicate_candidates_skipped,
            changes,
            rejected,
            local_improvement,
            before_global,
            after_global,
            rolled_back,
            started.elapsed(),
        );
    }

    let disc_threshold = args.discontinuity_threshold.max(0.0);
    let t_disc_score = RenderInstant::now();
    let cell_disc_scores = all_cell_discontinuity_scores(
        rendered_raster
            .as_ref()
            .expect("rendered raster should exist when scoring discontinuities"),
        grid_w,
        grid_h,
        gw,
        gh,
    );
    let relative_disc_score = if cell_disc_scores.is_empty() {
        0.0
    } else {
        cell_disc_scores
            .iter()
            .copied()
            .map(|score| (score - disc_threshold).max(0.0))
            .sum::<f32>() as f64
            / cell_disc_scores.len() as f64
    };
    let contour_score = preview_contour_score(
        &source_raster,
        rendered_raster
            .as_ref()
            .expect("rendered raster should exist when scoring contours"),
        grid_w,
        grid_h,
        gw,
        gh,
        &contour_weights,
    );
    let disc_score_time = t_disc_score.elapsed();
    log::info!(
        "Contour Score: total={:.4}, bending={:.4}, endpoints={:.4}, junctions={:.4}, fragments={:.4}, boundary_length={}",
        contour_score.total,
        contour_score.contour.bending,
        contour_score.contour.endpoints,
        contour_score.contour.junctions,
        contour_score.contour.fragments,
        contour_score.contour.boundary_length,
    );
    log::info!(
        "Contour Reference: fidelity={:.4}, boundary_balance={:.4}",
        contour_score.fidelity,
        contour_score.boundary_balance,
    );

    if let (Some(root), Some(raster)) = (fft_debug_root.as_ref(), rendered_raster.as_ref()) {
        if let Err(err) = dump_fft_stage_inputs(root, "final", raster, grid_w, grid_h, gw, gh) {
            log::warn!("failed to dump final FFT inputs: {}", err);
        }
    }

    let disc_overlay: Vec<Option<([u8; 3], [u8; 3])>> = if args.show_discontinuities {
        let max_score = cell_disc_scores
            .iter()
            .copied()
            .fold(0.0f32, f32::max)
            .max(disc_threshold + 1.0);
        cell_disc_scores
            .into_iter()
            .map(|score| {
                if score >= disc_threshold {
                    let severity =
                        ((score - disc_threshold) / (max_score - disc_threshold)).min(1.0);
                    let red = (80.0 + 175.0 * severity).min(255.0) as u8;
                    let bg_int = (red / 2).max(30);
                    Some(([red, 20, 20], [bg_int, 10, 10]))
                } else {
                    None
                }
            })
            .collect()
    } else {
        Vec::new()
    };

    log::info!(
        "Discontinuity scoring completed in {:?} (threshold {:.2})",
        disc_score_time,
        disc_threshold
    );
    log::info!("Image Discontinuity Score: {:.2}", relative_disc_score);

    let encode_started = RenderInstant::now();

    // Precompute cheapest glyph for uniform cells (FG==BG, glyph invisible).
    // Space/braille blank allow skipping the FG color code (~3 byte saving),
    // so they get a bonus. Among remaining glyphs, prefer smallest UTF-8 encoding.
    let cheapest_uniform_glyph: Option<char> = if glyphs.is_empty() {
        None
    } else {
        let mut best_ch: Option<char> = None;
        let mut best_cost = usize::MAX;
        for g in glyphs.iter() {
            let is_blank = g.ch == ' ' || g.ch == '\u{2800}';
            let is_block = g.ch >= '\u{2580}' && g.ch <= '\u{259F}';
            if !is_blank && !is_block {
                continue;
            }
            // Effective cost: glyph bytes + estimated FG overhead (0 for blanks, ~3 for others)
            let cost = g.ch.len_utf8() + if is_blank { 0 } else { 3 };
            if cost < best_cost {
                best_cost = cost;
                best_ch = Some(g.ch);
            }
        }
        best_ch
    };
    let mut out = String::new();
    let mut alt_out = String::new();
    let mut grid_out = Vec::new();
    let total_error = 0u64;
    let prepared_overlays: Vec<_> = args.overlays.iter().map(PreparedTextOverlay::new).collect();

    for y in 0..grid_h {
        let mut row_vec = Vec::new();
        let mut first = true;
        let mut last_fg: Option<Colour> = None;
        let mut last_bg: Option<Colour> = None;
        let mut last_bold = false;
        let mut last_italic = false;
        let mut last_underline = false;

        // Pass 1: Collect processed cell data for the row
        let mut row_cells: Vec<(Colour, Option<Colour>, char, bool, bool, bool)> =
            Vec::with_capacity(grid_w);
        let mut alt_row_cells: Vec<(Colour, Option<Colour>, char, bool, bool, bool)> =
            Vec::with_capacity(grid_w);

        if (render != Render::Irc || args.as_preview) && y == 0 {
            out.push_str("\x1b[0m");
        }

        for x in 0..grid_w {
            let idx = y * grid_w + x;
            let data = &grid_data[idx];
            let (gi, inv) = grid_state[idx];
            let glyph = &glyphs[gi];
            let ch = glyph.ch;

            let mut text_overlay: Option<&crate::args::TextOverlay> = None;
            let mut overlay_char = None;
            for prepared in &prepared_overlays {
                let ov = prepared.overlay;
                if let Some(opt_c) = prepared.at(x as i32, y as i32) {
                    if opt_c.is_none() && ov.bg.is_none() {
                        continue;
                    }
                    text_overlay = Some(ov);
                    overlay_char = opt_c;
                    break;
                }
            }

            // Check for discontinuity overlay on this cell
            let overlay = if !disc_overlay.is_empty() {
                disc_overlay[idx]
            } else {
                None
            };

            let (draw_fg, draw_bg, draw_fg_raw, draw_bg_raw) =
                if let Some((ovr_fg, ovr_bg)) = overlay {
                    // Overlay: use red highlighting colors, keep glyph unchanged
                    (
                        Colour::RGB(ovr_fg),
                        Some(Colour::RGB(ovr_bg)),
                        ovr_fg,
                        Some(ovr_bg),
                    )
                } else if inv {
                    (
                        data.bg_col.clone().unwrap_or(Colour::Index(0)),
                        Some(data.fg_col.clone().unwrap()),
                        data.bg,
                        Some(data.fg),
                    )
                } else {
                    (
                        data.fg_col.clone().unwrap(),
                        data.bg_col.clone(),
                        data.fg,
                        Some(data.bg),
                    )
                };

            // Ensure we respect chromatic preference before emitting
            let draw_fg = if let Colour::RGB(rgb) = draw_fg {
                if render != Render::Ansi24 && overlay.is_none() {
                    let col = make_rgb_u32(&rgb);
                    let palette = if render == Render::Irc {
                        &IRC99[..]
                    } else {
                        &ANSI256[..]
                    };
                    let idx = if !is_near_grayscale(col, args.grayscale_tolerance) {
                        nearest_distinctly_chromatic_hex_colour(
                            col,
                            palette,
                            args.grayscale_tolerance,
                        )
                        .unwrap_or_else(|| nearest_hex_colour_fast(col, palette))
                    } else {
                        nearest_hex_colour_fast(col, palette)
                    };
                    Colour::Index(idx)
                } else {
                    Colour::RGB(rgb)
                }
            } else {
                draw_fg
            };

            let draw_bg = if let Some(Colour::RGB(rgb)) = draw_bg {
                if render != Render::Ansi24 && overlay.is_none() {
                    let col = make_rgb_u32(&rgb);
                    let palette = if render == Render::Irc {
                        &IRC99[..]
                    } else {
                        &ANSI256[..]
                    };
                    let idx = if !is_near_grayscale(col, args.grayscale_tolerance) {
                        nearest_distinctly_chromatic_hex_colour(
                            col,
                            palette,
                            args.grayscale_tolerance,
                        )
                        .unwrap_or_else(|| nearest_hex_colour_fast(col, palette))
                    } else {
                        nearest_hex_colour_fast(col, palette)
                    };
                    Some(Colour::Index(idx))
                } else {
                    Some(Colour::RGB(rgb))
                }
            } else {
                draw_bg
            };

            let mut final_char = ch;
            if overlay.is_none() && text_overlay.is_none() {
                if let Some(uniform_ch) = cheapest_uniform_glyph {
                    if let Some(bg_c) = &draw_bg {
                        if &draw_fg == bg_c {
                            final_char = uniform_ch;
                        }
                    }
                }
            }

            let mut final_draw_fg = draw_fg.clone();
            let mut final_draw_bg = draw_bg.clone();
            let mut final_draw_fg_raw = draw_fg_raw;
            let mut final_draw_bg_raw = draw_bg_raw;

            if let Some(ov) = text_overlay {
                if let Some(fg_param) = &ov.fg {
                    final_draw_fg = match fg_param {
                        crate::args::ColorSpec::Index(i) => Colour::Index(*i),
                        crate::args::ColorSpec::Rgb(rgb) => Colour::RGB(*rgb),
                    };
                    final_draw_fg_raw = match fg_param {
                        crate::args::ColorSpec::Index(i) => {
                            let is_irc = args.render == crate::args::Render::Irc || args.as_preview;
                            let idx = (*i as usize).min(if is_irc { 98 } else { 255 });
                            if is_irc {
                                unpack_rgb(IRC99[idx])
                            } else {
                                unpack_rgb(ANSI256[idx])
                            }
                        }
                        crate::args::ColorSpec::Rgb(rgb) => *rgb,
                    };
                }
                if let Some(bg_param) = &ov.bg {
                    final_draw_bg = Some(match bg_param {
                        crate::args::ColorSpec::Index(i) => Colour::Index(*i),
                        crate::args::ColorSpec::Rgb(rgb) => Colour::RGB(*rgb),
                    });
                    final_draw_bg_raw = Some(match bg_param {
                        crate::args::ColorSpec::Index(i) => {
                            let is_irc = args.render == crate::args::Render::Irc || args.as_preview;
                            let idx = (*i as usize).min(if is_irc { 98 } else { 255 });
                            if is_irc {
                                unpack_rgb(IRC99[idx])
                            } else {
                                unpack_rgb(ANSI256[idx])
                            }
                        }
                        crate::args::ColorSpec::Rgb(rgb) => *rgb,
                    });
                }
                if let Some(c) = overlay_char {
                    final_char = c;
                } else if ov.bg.is_some() {
                    final_char = ' ';
                }
            }

            row_vec.push(RenderedCell {
                char: final_char,
                fg: final_draw_fg_raw,
                bg: final_draw_bg_raw,
                // `final_draw_*` already account for an inverted glyph state.
                // Export final display colours so structured consumers do not
                // need to repeat the renderer's internal inversion step.
                inverted: false,
                bold: text_overlay.map(|ov| ov.bold).unwrap_or(false),
                italic: text_overlay.map(|ov| ov.italic).unwrap_or(false),
                underline: text_overlay.map(|ov| ov.underline).unwrap_or(false),
            });

            row_cells.push((
                final_draw_fg.clone(),
                final_draw_bg.clone(),
                final_char,
                text_overlay.map(|ov| ov.bold).unwrap_or(false),
                text_overlay.map(|ov| ov.italic).unwrap_or(false),
                text_overlay.map(|ov| ov.underline).unwrap_or(false),
            ));

            if args.as_preview {
                // Helper: derive IRC99 index from raw RGB, respecting chromatic preference
                let irc_colour_from_rgb = |rgb: [u8; 3]| -> Colour {
                    let col = make_rgb_u32(&rgb);
                    let idx = if !is_near_grayscale(col, args.grayscale_tolerance) {
                        nearest_distinctly_chromatic_hex_colour(
                            col,
                            &IRC99,
                            args.grayscale_tolerance,
                        )
                        .unwrap_or_else(|| nearest_hex_colour_fast(col, &IRC99))
                    } else {
                        nearest_hex_colour_fast(col, &IRC99)
                    };
                    Colour::Index(idx)
                };

                let (alt_fg, alt_bg) = if overlay.is_some() {
                    (draw_fg.clone(), draw_bg.clone())
                } else if inv {
                    (
                        data.alt_bg_col
                            .clone()
                            .unwrap_or_else(|| irc_colour_from_rgb(data.bg)),
                        Some(
                            data.alt_fg_col
                                .clone()
                                .unwrap_or_else(|| irc_colour_from_rgb(data.fg)),
                        ),
                    )
                } else {
                    (
                        data.alt_fg_col
                            .clone()
                            .unwrap_or_else(|| irc_colour_from_rgb(data.fg)),
                        Some(
                            data.alt_bg_col
                                .clone()
                                .unwrap_or_else(|| irc_colour_from_rgb(data.bg)),
                        ),
                    )
                };
                // Compute uniform-glyph replacement for the IRC alt output
                // using the IRC99 colors, not the ANSI256-based final_char.
                let mut alt_final_char = ch;
                if overlay.is_none() && text_overlay.is_none() {
                    if let Some(uniform_ch) = cheapest_uniform_glyph {
                        if let Some(bg_c) = &alt_bg {
                            if &alt_fg == bg_c {
                                alt_final_char = uniform_ch;
                            }
                        }
                    }
                }

                let mut final_alt_fg = alt_fg;
                let mut final_alt_bg = alt_bg;
                if let Some(ov) = text_overlay {
                    if let Some(fg_param) = &ov.fg {
                        final_alt_fg = match fg_param {
                            crate::args::ColorSpec::Index(i) => Colour::Index(*i),
                            crate::args::ColorSpec::Rgb(rgb) => Colour::RGB(*rgb),
                        };
                    }
                    if let Some(bg_param) = &ov.bg {
                        final_alt_bg = Some(match bg_param {
                            crate::args::ColorSpec::Index(i) => Colour::Index(*i),
                            crate::args::ColorSpec::Rgb(rgb) => Colour::RGB(*rgb),
                        });
                    }
                    if let Some(c) = overlay_char {
                        alt_final_char = c;
                    } else if ov.bg.is_some() {
                        alt_final_char = ' ';
                    }
                }

                alt_row_cells.push((
                    final_alt_fg,
                    final_alt_bg,
                    alt_final_char,
                    text_overlay.map(|ov| ov.bold).unwrap_or(false),
                    text_overlay.map(|ov| ov.italic).unwrap_or(false),
                    text_overlay.map(|ov| ov.underline).unwrap_or(false),
                ));
            }
        }

        // Pass 2: Emit with lookahead for FG optimization
        for (i, (fg, bg, ch, bold, italic, underline)) in row_cells.iter().enumerate() {
            // Find next non-space cell's FG for lookahead
            let lookahead = if render == Render::Irc || (args.as_preview) {
                row_cells[i + 1..].iter().find_map(|(f, _, c, _, _, _)| {
                    let is_sp = *c == ' ' || *c == '\u{2800}';
                    if !is_sp {
                        Some(f)
                    } else {
                        None
                    }
                })
            } else {
                None
            };
            emit_colourized(
                &mut out,
                render,
                args.as_preview,
                fg.clone(),
                bg.clone(),
                *ch,
                &mut first,
                &mut last_fg,
                &mut last_bg,
                *bold,
                *italic,
                *underline,
                &mut last_bold,
                &mut last_italic,
                &mut last_underline,
                lookahead,
            );
        }

        if args.as_preview || render == Render::Irc {
            let mut alt_first = true;
            let mut alt_last_fg: Option<Colour> = None;
            let mut alt_last_bg: Option<Colour> = None;
            let mut alt_last_bold = false;
            let mut alt_last_italic = false;
            let mut alt_last_underline = false;

            for (i, (fg, bg, ch, bold, italic, underline)) in alt_row_cells.iter().enumerate() {
                // Find next non-space cell's FG for lookahead
                let lookahead = alt_row_cells[i + 1..]
                    .iter()
                    .find_map(|(f, _, c, _, _, _)| {
                        let is_sp = *c == ' ' || *c == '\u{2800}';
                        if !is_sp {
                            Some(f)
                        } else {
                            None
                        }
                    });

                emit_colourized(
                    &mut alt_out,
                    Render::Irc,
                    false,
                    fg.clone(),
                    bg.clone(),
                    *ch,
                    &mut alt_first,
                    &mut alt_last_fg,
                    &mut alt_last_bg,
                    *bold,
                    *italic,
                    *underline,
                    &mut alt_last_bold,
                    &mut alt_last_italic,
                    &mut alt_last_underline,
                    lookahead,
                );
            }
        }

        let end_seq = match render {
            Render::Ansi | Render::Ansi24 => "\x1b[0m\n",
            Render::Irc => "\x0f\n",
        };
        out.push_str(end_seq);
        if args.as_preview {
            alt_out.push_str("\x0f\n");
        }
        grid_out.push(row_vec);
    }

    let longest_line_bytes = if args.as_preview && render == Render::Irc {
        alt_out.lines().map(|l| l.len()).max().unwrap_or(0)
    } else {
        out.lines().map(|l| l.len()).max().unwrap_or(0)
    };
    let encode_time = encode_started.elapsed();
    let contour_cache_end = crate::contour_score::cache_stats();

    RenderResult {
        content: out.trim_end_matches('\n').into(),
        save_content: if args.as_preview && render == Render::Irc {
            Some(alt_out.trim_end_matches('\n').into())
        } else {
            None
        },
        error_count: total_error,
        total_pixels: (bp * grid_h * grid_w) as u64,
        dynamic_scaling_count: 0,
        grid: grid_out,
        score: contour_score.total,
        longest_line_bytes,
        initial_time,
        profile: RenderProfile {
            glyph_match: glyph_match_time,
            smoothing_prepare: smoothing_prepare_time,
            smoothing_search: smoothing_search_time,
            shape_refine: shape_refine_time,
            final_score: disc_score_time,
            encode: encode_time,
            contour_cache_hits: contour_cache_end.0.saturating_sub(contour_cache_start.0),
            contour_cache_misses: contour_cache_end.1.saturating_sub(contour_cache_start.1),
            ..RenderProfile::default()
        },
    }
}
pub fn render_braille(
    image_luma: &AnsiImage,
    image_chroma: &AnsiImage,
    args: &RenderArgs,
    render: Render,
) -> RenderResult {
    let h = image_luma.bitmap.len();
    let w = image_luma.bitmap[0].len();
    let mut err = vec![vec![0.0; w]; h];
    let (mut min_l, mut max_l) = (255u32, 0u32);
    for row in &image_luma.bitmap {
        for &px in row {
            let l = luma(&unpack_rgb(px)) as u32;
            min_l = min_l.min(l);
            max_l = max_l.max(l);
        }
    }
    let thr = min_l + ((max_l - min_l).max(1) / 2);
    let mut out = String::new();
    let mut alt_out = String::new();
    let mut grid_out = Vec::with_capacity(h.div_ceil(4));
    let prepared_overlays: Vec<_> = args.overlays.iter().map(PreparedTextOverlay::new).collect();
    for y in (0..h).step_by(4) {
        let mut row_vec = Vec::with_capacity(w.div_ceil(2));
        let mut first = true;
        let mut alt_first = true;
        let mut last_fg = None;
        let mut last_bg = None;
        let mut last_bold = false;
        let mut last_italic = false;
        let mut last_underline = false;
        let mut alt_last_fg = None;
        let mut alt_last_bg = None;
        let mut alt_last_bold = false;
        let mut alt_last_italic = false;
        let mut alt_last_underline = false;

        if (render != Render::Irc || args.as_preview) && y == 0 {
            out.push_str("\x1b[0m");
        }

        for x in (0..w).step_by(2) {
            let mut braille = 0x2800;
            let mut counts: HashMap<Colour, usize> = HashMap::new();
            let mut alt_counts: HashMap<Colour, usize> = HashMap::new();
            for &(dx, dy, bit) in &POSITIONS {
                let yy = y + dy;
                let xx = x + dx;
                if yy < h && xx < w {
                    let rgb_l = unpack_rgb(image_luma.bitmap[yy][xx]);
                    let mut lum = luma(&rgb_l) as f64 + err[yy][xx];
                    lum = lum.clamp(0.0, 255.0);
                    let dot = lum > thr as f64;
                    let e = lum - if dot { 255.0 } else { 0.0 };
                    if xx + 1 < w {
                        err[yy][xx + 1] += e * 7.0 / 16.0
                    }
                    if yy + 1 < h {
                        if xx > 0 {
                            err[yy + 1][xx - 1] += e * 3.0 / 16.0
                        }
                        err[yy + 1][xx] += e * 5.0 / 16.0;
                        if xx + 1 < w {
                            err[yy + 1][xx + 1] += e * 1.0 / 16.0
                        }
                    }
                    if dot {
                        braille |= bit;
                        let col = pick_colour(
                            &AnsiPixel::new(&image_chroma.bitmap[yy][xx], args.grayscale_tolerance),
                            render,
                            args,
                        );
                        *counts.entry(col).or_insert(0) += 1;

                        if args.as_preview {
                            let alt_col = pick_colour(
                                &AnsiPixel::new(
                                    &image_chroma.bitmap[yy][xx],
                                    args.grayscale_tolerance,
                                ),
                                Render::Irc,
                                args,
                            );
                            *alt_counts.entry(alt_col).or_insert(0) += 1;
                        }
                    }
                }
            }
            let mut final_fg = counts
                .into_iter()
                .max_by_key(|&(_, count)| count)
                .map(|(c, _)| c)
                .unwrap_or(match render {
                    Render::Ansi | Render::Irc => Colour::Index(0),
                    Render::Ansi24 => Colour::RGB([0, 0, 0]),
                });

            let mut final_bg = None;
            let mut final_ch = std::char::from_u32(braille).unwrap_or(' ');
            let mut final_bold = false;
            let mut final_italic = false;
            let mut final_underline = false;

            for prepared in &prepared_overlays {
                let ov = prepared.overlay;
                if let Some(opt_c) = prepared.at((x / 2) as i32, (y / 4) as i32) {
                    if opt_c.is_none() && ov.bg.is_none() {
                        continue;
                    }
                    if let Some(fg_param) = &ov.fg {
                        final_fg = match fg_param {
                            crate::args::ColorSpec::Index(i) => Colour::Index(*i),
                            crate::args::ColorSpec::Rgb(rgb) => Colour::RGB(*rgb),
                        };
                    }
                    if let Some(bg_param) = &ov.bg {
                        final_bg = Some(match bg_param {
                            crate::args::ColorSpec::Index(i) => Colour::Index(*i),
                            crate::args::ColorSpec::Rgb(rgb) => Colour::RGB(*rgb),
                        });
                    }
                    if let Some(c) = opt_c {
                        final_ch = c;
                    } else {
                        final_ch = ' ';
                    }
                    final_bold = ov.bold;
                    final_italic = ov.italic;
                    final_underline = ov.underline;
                    break;
                }
            }

            row_vec.push(RenderedCell {
                char: final_ch,
                fg: colour_to_rgb(&final_fg, render),
                bg: final_bg
                    .as_ref()
                    .map(|colour| colour_to_rgb(colour, render)),
                inverted: false,
                bold: final_bold,
                italic: final_italic,
                underline: final_underline,
            });

            emit_colourized(
                &mut out,
                render,
                args.as_preview,
                final_fg,
                final_bg.clone(),
                final_ch,
                &mut first,
                &mut last_fg,
                &mut last_bg,
                final_bold,
                final_italic,
                final_underline,
                &mut last_bold,
                &mut last_italic,
                &mut last_underline,
                None,
            );

            if args.as_preview {
                let mut alt_fg = alt_counts
                    .into_iter()
                    .max_by_key(|&(_, count)| count)
                    .map(|(c, _)| c)
                    .unwrap_or(Colour::Index(0));

                let mut alt_bg = None;
                let mut alt_ch = std::char::from_u32(braille).unwrap_or(' ');
                let mut alt_bold = false;
                let mut alt_italic = false;
                let mut alt_underline = false;

                for prepared in &prepared_overlays {
                    let ov = prepared.overlay;
                    if let Some(opt_c) = prepared.at((x / 2) as i32, (y / 4) as i32) {
                        if opt_c.is_none() && ov.bg.is_none() {
                            continue;
                        }
                        if let Some(fg_param) = &ov.fg {
                            alt_fg = match fg_param {
                                crate::args::ColorSpec::Index(i) => Colour::Index(*i),
                                crate::args::ColorSpec::Rgb(rgb) => Colour::RGB(*rgb),
                            };
                        }
                        if let Some(bg_param) = &ov.bg {
                            alt_bg = Some(match bg_param {
                                crate::args::ColorSpec::Index(i) => Colour::Index(*i),
                                crate::args::ColorSpec::Rgb(rgb) => Colour::RGB(*rgb),
                            });
                        }
                        if let Some(c) = opt_c {
                            alt_ch = c;
                        } else {
                            alt_ch = ' ';
                        }
                        alt_bold = ov.bold;
                        alt_italic = ov.italic;
                        alt_underline = ov.underline;
                        break;
                    }
                }

                emit_colourized(
                    &mut alt_out,
                    Render::Irc,
                    false,
                    alt_fg,
                    alt_bg,
                    alt_ch,
                    &mut alt_first,
                    &mut alt_last_fg,
                    &mut alt_last_bg,
                    alt_bold,
                    alt_italic,
                    alt_underline,
                    &mut alt_last_bold,
                    &mut alt_last_italic,
                    &mut alt_last_underline,
                    None,
                );
            }
        }
        let end_seq = match render {
            Render::Ansi | Render::Ansi24 => "\x1b[0m\n",
            Render::Irc => "\x0f\n",
        };
        out.push_str(end_seq);
        if args.as_preview {
            alt_out.push_str("\x0f\n");
        }
        grid_out.push(row_vec);
    }

    let longest_line_bytes = if args.as_preview && render == Render::Irc {
        alt_out.lines().map(|l| l.len()).max().unwrap_or(0)
    } else {
        out.lines().map(|l| l.len()).max().unwrap_or(0)
    };

    RenderResult {
        content: out.trim_end_matches('\n').to_string(),
        save_content: if args.as_preview && render == Render::Irc {
            Some(alt_out.trim_end_matches('\n').into())
        } else {
            None
        },
        error_count: 0,
        total_pixels: (h * w) as u64,
        dynamic_scaling_count: 0,
        grid: grid_out,
        score: 0.0,
        longest_line_bytes,
        initial_time: std::time::Duration::ZERO,
        profile: RenderProfile::default(),
    }
}

fn colour_to_rgb(colour: &Colour, render: Render) -> [u8; 3] {
    match colour {
        Colour::RGB(rgb) => *rgb,
        Colour::Index(index) => {
            let palette = if render == Render::Irc {
                &IRC99[..]
            } else {
                &ANSI256[..]
            };
            unpack_rgb(palette[(*index as usize).min(palette.len() - 1)])
        }
    }
}

fn pick_colour(pixel: &AnsiPixel, r: Render, args: &RenderArgs) -> Colour {
    match r {
        Render::Ansi => Colour::Index(if args.nograyscale && pixel.ansi_distinct {
            pixel.ansi_ng
        } else {
            pixel.ansi_std
        }),
        Render::Irc => Colour::Index(if args.nograyscale && pixel.irc_distinct {
            pixel.irc_ng
        } else {
            pixel.irc_std
        }),
        Render::Ansi24 => Colour::RGB(unpack_rgb(pixel.orig)),
    }
}

pub fn save_as_png(
    result: &RenderResult,
    glyphs: &GlyphStore,
    path: &str,
    metadata_json: &str,
) -> std::io::Result<()> {
    let Some(image) = render_result_image(result, glyphs) else {
        return Ok(());
    };
    let rgb = image.to_rgb8();
    let (width, height) = rgb.dimensions();

    // Write PNG with metadata using the png crate
    let file = std::fs::File::create(path)?;
    let w = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(w, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);

    // Add settings metadata as a tEXt chunk
    if !metadata_json.is_empty() {
        encoder
            .add_text_chunk("img2irc_settings".to_string(), metadata_json.to_string())
            .map_err(std::io::Error::other)?;
    }

    let mut writer = encoder.write_header().map_err(std::io::Error::other)?;
    writer
        .write_image_data(rgb.as_raw())
        .map_err(std::io::Error::other)?;
    Ok(())
}

pub struct GlyphBitmapIndex {
    indices: HashMap<char, usize>,
    fallback: Option<usize>,
}

pub fn prepare_glyph_bitmap_index(glyphs: &GlyphStore) -> GlyphBitmapIndex {
    GlyphBitmapIndex {
        indices: glyphs
            .glyphs
            .iter()
            .enumerate()
            .map(|(index, (ch, _, _))| (*ch, index))
            .collect(),
        fallback: glyphs.glyphs.iter().position(|(ch, _, _)| *ch == '?'),
    }
}

fn procedural_braille_bitmap(ch: char, w: usize, h: usize) -> Option<Vec<Vec<u8>>> {
    let code = ch as u32;
    if !(0x2800..=0x28FF).contains(&code) {
        return None;
    }
    let mask = code - 0x2800;
    let mut cell = vec![vec![0u8; w]; h];
    let dots = [
        (0, 0, 0x01),
        (0, 1, 0x02),
        (0, 2, 0x04),
        (1, 0, 0x08),
        (1, 1, 0x10),
        (1, 2, 0x20),
        (0, 3, 0x40),
        (1, 3, 0x80),
    ];
    for &(col, row, bit) in &dots {
        if mask & bit != 0 {
            let y_start = (row * h) / 4;
            let y_end = ((row + 1) * h) / 4;
            let x_start = (col * w) / 2;
            let x_end = ((col + 1) * w) / 2;
            for y in y_start..y_end.min(h) {
                for x in x_start..x_end.min(w) {
                    cell[y][x] = 1;
                }
            }
        }
    }
    Some(cell)
}

fn render_result_pixels_indexed<const CHANNELS: usize>(
    result: &RenderResult,
    glyphs: &GlyphStore,
    bitmap_index: &GlyphBitmapIndex,
) -> Option<(u32, u32, Vec<u8>)> {
    debug_assert!(CHANNELS >= 3);
    let first_row = result.grid.first()?;
    if first_row.is_empty() {
        return None;
    }

    let (gw, gh) = glyphs.metrics;
    if gw == 0 || gh == 0 {
        return None;
    }

    let rows = result.grid.len();
    let cols = first_row.len();
    let width = cols.checked_mul(gw)?;
    let height = rows.checked_mul(gh)?;
    // Initialize every pixel as opaque. RGB callers overwrite the first three
    // channels, while RGBA callers retain 255 in the alpha channel.
    let mut pixels = vec![255; width.checked_mul(height)?.checked_mul(CHANNELS)?];
    for (row_idx, row) in result.grid.iter().enumerate() {
        for (col_idx, cell) in row.iter().take(cols).enumerate() {
            let procedural_storage;
            let bitmap: Option<&[Vec<u8>]> = if cell.char == ' ' || cell.char == '\0' {
                None
            } else if let Some(index) = bitmap_index.indices.get(&cell.char) {
                glyphs.glyphs.get(*index).map(|(_, bitmap, _)| bitmap.as_slice())
            } else if let Some(pb) = crate::font::procedural_block_bitmap(cell.char, gw, gh) {
                procedural_storage = pb;
                Some(procedural_storage.as_slice())
            } else if let Some(pb) = procedural_braille_bitmap(cell.char, gw, gh) {
                procedural_storage = pb;
                Some(procedural_storage.as_slice())
            } else {
                bitmap_index
                    .fallback
                    .and_then(|index| glyphs.glyphs.get(index))
                    .map(|(_, bitmap, _)| bitmap.as_slice())
            };
            let background = cell.bg.unwrap_or([0, 0, 0]);
            for y in 0..gh {
                let output_start =
                    ((row_idx * gh + y) * width + col_idx * gw) * CHANNELS;
                let output_end = output_start + gw * CHANNELS;
                let output_row = &mut pixels[output_start..output_end];
                if let Some(bitmap_row) = bitmap.and_then(|bitmap| bitmap.get(y)) {
                    for (x, pixel) in output_row.chunks_exact_mut(CHANNELS).enumerate() {
                        let color = if bitmap_row.get(x).is_some_and(|bit| *bit != 0) {
                            cell.fg
                        } else {
                            background
                        };
                        pixel[..3].copy_from_slice(&color);
                    }
                } else {
                    for pixel in output_row.chunks_exact_mut(CHANNELS) {
                        pixel[..3].copy_from_slice(&background);
                    }
                }
            }
        }
    }

    Some((width as u32, height as u32, pixels))
}

fn render_result_pixels<const CHANNELS: usize>(
    result: &RenderResult,
    glyphs: &GlyphStore,
) -> Option<(u32, u32, Vec<u8>)> {
    let bitmap_index = prepare_glyph_bitmap_index(glyphs);
    render_result_pixels_indexed::<CHANNELS>(result, glyphs, &bitmap_index)
}

/// Rasterize a rendered character grid to tightly packed, opaque RGBA pixels.
/// Browser callers can pass this directly to `ImageData`, avoiding an
/// intermediate PNG encode/decode cycle.
pub fn render_result_rgba(
    result: &RenderResult,
    glyphs: &GlyphStore,
) -> Option<(u32, u32, Vec<u8>)> {
    render_result_pixels::<4>(result, glyphs)
}

pub fn render_result_rgba_indexed(
    result: &RenderResult,
    glyphs: &GlyphStore,
    bitmap_index: &GlyphBitmapIndex,
) -> Option<(u32, u32, Vec<u8>)> {
    render_result_pixels_indexed::<4>(result, glyphs, bitmap_index)
}

/// Rasterize a rendered character grid using the exact glyph bitmaps selected
/// by the renderer. Keeping this in memory lets both PNG export and terminal
/// graphics previews share one source of truth without an encode/decode round
/// trip through a temporary PNG.
pub fn render_result_image(result: &RenderResult, glyphs: &GlyphStore) -> Option<DynamicImage> {
    let (width, height, pixels) = render_result_pixels::<3>(result, glyphs)?;
    RgbImage::from_raw(width, height, pixels).map(DynamicImage::ImageRgb8)
}

fn get_transformed_block_pixels_into(
    orig: &[PrePixel],
    gw: usize,
    gh: usize,
    sx: f32,
    sy: f32,
    out: &mut Vec<PrePixel>,
) {
    out.clear();
    let cx = gw as f32 / 2.0;
    let cy = gh as f32 / 2.0;
    for y in 0..gh {
        for x in 0..gw {
            let u = ((x as f32 - cx) / sx + cx).clamp(0.0, (gw - 1) as f32);
            let v = ((y as f32 - cy) / sy + cy).clamp(0.0, (gh - 1) as f32);
            let x0 = u.floor() as usize;
            let x1 = (x0 + 1).min(gw - 1);
            let y0 = v.floor() as usize;
            let y1 = (y0 + 1).min(gh - 1);
            let wx = u - x0 as f32;
            let wy = v - y0 as f32;
            let p00 = &orig[y0 * gw + x0];
            let p10 = &orig[y0 * gw + x1];
            let p01 = &orig[y1 * gw + x0];
            let p11 = &orig[y1 * gw + x1];
            let lerp = |a: u64, b: u64, t: f32| a as f32 * (1.0 - t) + b as f32 * t;
            let r = lerp(
                lerp(p00.r, p10.r, wx) as u64,
                lerp(p01.r, p11.r, wx) as u64,
                wy,
            ) as u64;
            let g = lerp(
                lerp(p00.g, p10.g, wx) as u64,
                lerp(p01.g, p11.g, wx) as u64,
                wy,
            ) as u64;
            let b = lerp(
                lerp(p00.b, p10.b, wx) as u64,
                lerp(p01.b, p11.b, wx) as u64,
                wy,
            ) as u64;
            out.push(PrePixel {
                r,
                g,
                b,
                sq: r * r + g * g + b * b,
                ansi_part: p00.ansi_part,
                irc_part: p00.irc_part,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn laid_out_strings(text: &str, width: Option<usize>) -> Vec<String> {
        layout_overlay_text(text, width)
            .into_iter()
            .map(|line| line.into_iter().collect())
            .collect()
    }

    #[test]
    fn overlay_wrap_prefers_unicode_break_opportunities() {
        assert_eq!(
            laid_out_strings("hello world again", Some(11)),
            ["hello world", "again"]
        );
        assert_eq!(
            laid_out_strings("alpha-beta gamma", Some(10)),
            ["alpha-beta", "gamma"]
        );
    }

    #[test]
    fn overlay_wrap_hard_breaks_only_overlong_words() {
        assert_eq!(
            laid_out_strings("supercalifragilistic", Some(6)),
            ["superc", "alifra", "gilist", "ic"]
        );
    }

    #[test]
    fn overlay_wrap_preserves_explicit_and_empty_lines() {
        assert_eq!(
            laid_out_strings("first line\n\nsecond", Some(20)),
            ["first line", "", "second"]
        );
        assert_eq!(laid_out_strings("first\n", Some(20)), ["first", ""]);
    }

    #[test]
    fn overlay_wrap_uses_cjk_break_boundaries() {
        assert_eq!(
            laid_out_strings("日本語テキスト", Some(3)),
            ["日本語", "テキス", "ト"]
        );
    }

    #[test]
    fn applying_text_overlays_updates_the_export_grid() {
        let base_cell = RenderedCell {
            char: '.',
            fg: [10, 20, 30],
            bg: Some([1, 2, 3]),
            inverted: false,
            bold: false,
            italic: false,
            underline: false,
        };
        let base = RenderResult {
            content: ".".to_string(),
            save_content: None,
            error_count: 0,
            total_pixels: 1,
            dynamic_scaling_count: 0,
            grid: vec![vec![base_cell]],
            score: 0.0,
            longest_line_bytes: 1,
            initial_time: std::time::Duration::ZERO,
            profile: RenderProfile::default(),
        };
        let overlay = crate::args::TextOverlay {
            text: "X".to_string(),
            source_text: None,
            figlet_font: None,
            x: 0,
            y: 0,
            w: 1,
            h: 1,
            fg: Some(crate::args::ColorSpec::Rgb([240, 230, 220])),
            bg: None,
            wrap: true,
            auto_grow: true,
            transparent_spaces: false,
            bold: true,
            italic: false,
            underline: true,
        };

        let overlaid = apply_overlays_to_result(&base, &[overlay], Render::Ansi24, true, 5);

        assert_eq!(overlaid.grid[0][0].char, 'X');
        assert_eq!(overlaid.grid[0][0].fg, [240, 230, 220]);
        assert_eq!(overlaid.grid[0][0].bg, Some([1, 2, 3]));
        assert!(overlaid.grid[0][0].bold);
        assert!(overlaid.grid[0][0].underline);
    }

    #[test]
    fn transparent_overlay_spaces_preserve_the_complete_base_cell() {
        let cells = ['a', 'b', 'c']
            .into_iter()
            .enumerate()
            .map(|(index, character)| RenderedCell {
                char: character,
                fg: [10 + index as u8, 20, 30],
                bg: Some([1, 2 + index as u8, 3]),
                inverted: index == 1,
                bold: index == 1,
                italic: index == 1,
                underline: index == 1,
            })
            .collect::<Vec<_>>();
        let base = RenderResult {
            content: "abc".to_string(),
            save_content: None,
            error_count: 0,
            total_pixels: 3,
            dynamic_scaling_count: 0,
            grid: vec![cells],
            score: 0.0,
            longest_line_bytes: 3,
            initial_time: std::time::Duration::ZERO,
            profile: RenderProfile::default(),
        };
        let overlay = crate::args::TextOverlay {
            text: "X X".to_string(),
            source_text: Some("A".to_string()),
            figlet_font: Some("test".to_string()),
            x: 0,
            y: 0,
            w: 3,
            h: 1,
            fg: Some(crate::args::ColorSpec::Rgb([240, 230, 220])),
            bg: Some(crate::args::ColorSpec::Rgb([8, 9, 10])),
            wrap: false,
            auto_grow: false,
            transparent_spaces: true,
            bold: false,
            italic: false,
            underline: false,
        };

        let overlaid = apply_overlays_to_result(&base, &[overlay], Render::Ansi24, true, 5);

        assert_eq!(overlaid.grid[0][0].char, 'X');
        assert_eq!(overlaid.grid[0][2].char, 'X');
        let preserved = &overlaid.grid[0][1];
        assert_eq!(preserved.char, 'b');
        assert_eq!(preserved.fg, [11, 20, 30]);
        assert_eq!(preserved.bg, Some([1, 3, 3]));
        assert!(preserved.inverted);
        assert!(preserved.bold);
        assert!(preserved.italic);
        assert!(preserved.underline);
    }

    #[test]
    fn rendered_result_image_uses_cached_font_bitmaps_and_cell_colours() {
        let glyphs = GlyphStore {
            glyphs: vec![(
                'X',
                vec![vec![1, 0], vec![0, 1]],
                "Test Font".to_string(),
            )],
            groups: HashMap::new(),
            selected: std::collections::HashSet::from(['X']),
            metrics: (2, 2),
            float_metrics: (2.0, 2.0),
        };
        let result = RenderResult {
            content: "X".to_string(),
            save_content: None,
            error_count: 0,
            total_pixels: 4,
            dynamic_scaling_count: 0,
            grid: vec![vec![RenderedCell {
                char: 'X',
                fg: [240, 10, 20],
                bg: Some([1, 2, 3]),
                inverted: false,
                bold: false,
                italic: false,
                underline: false,
            }]],
            score: 0.0,
            longest_line_bytes: 1,
            initial_time: std::time::Duration::ZERO,
            profile: RenderProfile::default(),
        };

        let image = render_result_image(&result, &glyphs).unwrap().to_rgb8();
        assert_eq!(image.dimensions(), (2, 2));
        assert_eq!(image.get_pixel(0, 0).0, [240, 10, 20]);
        assert_eq!(image.get_pixel(1, 0).0, [1, 2, 3]);
        assert_eq!(image.get_pixel(0, 1).0, [1, 2, 3]);
        assert_eq!(image.get_pixel(1, 1).0, [240, 10, 20]);

        let (width, height, rgba) = render_result_rgba(&result, &glyphs).unwrap();
        assert_eq!((width, height), (2, 2));
        assert_eq!(
            rgba,
            vec![240, 10, 20, 255, 1, 2, 3, 255, 1, 2, 3, 255, 240, 10, 20, 255,]
        );
    }

    #[test]
    fn rendered_result_image_rejects_empty_output() {
        let glyphs = GlyphStore {
            glyphs: Vec::new(),
            groups: HashMap::new(),
            selected: std::collections::HashSet::new(),
            metrics: (1, 1),
            float_metrics: (1.0, 1.0),
        };
        assert!(render_result_image(&RenderResult::from(""), &glyphs).is_none());
    }

    #[test]
    fn braille_render_populates_the_structured_grid() {
        let glyphs = GlyphStore {
            glyphs: Vec::new(),
            groups: HashMap::new(),
            selected: std::collections::HashSet::new(),
            metrics: (2, 4),
            float_metrics: (2.0, 4.0),
        };
        let image = PhotonImage::new(vec![255; 2 * 4 * 4], 2, 4);
        let canvas = AnsiImage::new(image, glyphs, 32, false);
        let cli = crate::args::Args::parse_from(["img2irc", "--braille", "--width", "1"]);
        let args = crate::args_to_render_args(&cli);

        let result = render_braille(&canvas, &canvas, &args, Render::Ansi);

        assert_eq!(result.grid.len(), 1);
        assert_eq!(result.grid[0].len(), 1);
        assert!((0x2800..=0x28ff).contains(&(result.grid[0][0].char as u32)));
    }

    fn make_test_glyph(ch: char, bitmap: &[u8], gw: usize, gh: usize) -> FastGlyph {
        let top = bitmap[0..gw].to_vec();
        let bottom = bitmap[(gh - 1) * gw..gh * gw].to_vec();
        let mut left = Vec::with_capacity(gh);
        let mut right = Vec::with_capacity(gh);
        for y in 0..gh {
            left.push(bitmap[y * gw]);
            right.push(bitmap[y * gw + (gw - 1)]);
        }

        let mut ones = Vec::new();
        let mut zeros = Vec::new();
        for (idx, &bit) in bitmap.iter().enumerate() {
            if bit == 1 {
                ones.push(idx);
            } else {
                zeros.push(idx);
            }
        }

        let mut vertical_cuts = Vec::new();
        if gw > 1 {
            for y in 0..gh {
                for x in 0..(gw - 1) {
                    vertical_cuts.push((bitmap[y * gw + x] != bitmap[y * gw + x + 1]) as u8);
                }
            }
        }

        let mut horizontal_cuts = Vec::new();
        if gh > 1 {
            for y in 0..(gh - 1) {
                for x in 0..gw {
                    horizontal_cuts.push((bitmap[y * gw + x] != bitmap[(y + 1) * gw + x]) as u8);
                }
            }
        }

        FastGlyph {
            ch,
            bitmap: bitmap.to_vec(),
            ones,
            zeros,
            vertical_cuts,
            horizontal_cuts,
            top,
            bottom,
            left,
            right,
            coverage: bitmap.iter().filter(|&&bit| bit == 1).count() as f32 / bitmap.len() as f32,
            exit_top: -1.0,
            exit_bottom: -1.0,
            exit_left: -1.0,
            exit_right: -1.0,
            exit_top_inv: -1.0,
            exit_bottom_inv: -1.0,
            exit_left_inv: -1.0,
            exit_right_inv: -1.0,
            mask: 0,
        }
    }

    fn raster_from_bitmap(
        bitmap: &[u8],
        gw: usize,
        gh: usize,
        fg: [u8; 3],
        bg: [u8; 3],
    ) -> Vec<[u8; 3]> {
        let mut raster = Vec::with_capacity(bitmap.len());
        for y in 0..gh {
            for x in 0..gw {
                raster.push(if bitmap[y * gw + x] == 1 { fg } else { bg });
            }
        }
        raster
    }

    #[test]
    fn spectral_score_penalizes_checkerboard_more_than_single_edge() {
        let width = 8;
        let height = 8;
        let mut smooth = vec![0u8; width * height];
        let mut checker = vec![0u8; width * height];

        for y in 0..height {
            for x in 0..width {
                smooth[y * width + x] = (x < width / 2) as u8;
                checker[y * width + x] = ((x + y) % 2 == 0) as u8;
            }
        }

        let fg = [255, 255, 255];
        let bg = [0, 0, 0];
        let smooth_raster = raster_from_bitmap(&smooth, width, height, fg, bg);
        let checker_raster = raster_from_bitmap(&checker, width, height, fg, bg);

        let smooth_score = local_patch_frequency_score(&smooth_raster, width, height);
        let checker_score = local_patch_frequency_score(&checker_raster, width, height);

        assert!(
            checker_score > smooth_score,
            "checker_score={} smooth_score={}",
            checker_score,
            smooth_score
        );
    }

    #[test]
    fn extract_local_patch_is_fixed_size_and_centered_at_edges() {
        let gw = 2;
        let gh = 3;
        let grid_w = 4;
        let grid_h = 4;
        let raster_w = grid_w * gw;
        let raster_h = grid_h * gh;
        let mut raster = vec![[0u8; 3]; raster_w * raster_h];

        for cell_y in 0..grid_h {
            for cell_x in 0..grid_w {
                let color = [cell_x as u8 * 40, cell_y as u8 * 50, 0];
                for py in 0..gh {
                    for px in 0..gw {
                        let x = cell_x * gw + px;
                        let y = cell_y * gh + py;
                        raster[y * raster_w + x] = color;
                    }
                }
            }
        }

        let patch = extract_local_patch(&raster, 0, 0, grid_w, grid_h, gw, gh);
        assert_eq!(patch.width, gw * 3);
        assert_eq!(patch.height, gh * 3);
        assert_eq!(patch.cell_offset_x, gw);
        assert_eq!(patch.cell_offset_y, gh);

        for py in 0..gh {
            for px in 0..gw {
                let left = patch.pixels[py * patch.width + px];
                let center = patch.pixels[py * patch.width + gw + px];
                assert_eq!(left, center);
            }
        }
    }

    #[test]
    fn extract_center_cell_pixels_returns_single_cell_only() {
        let gw = 2;
        let gh = 2;
        let grid_w = 2;
        let grid_h = 2;
        let raster = vec![
            [1, 0, 0],
            [1, 0, 0],
            [2, 0, 0],
            [2, 0, 0],
            [1, 0, 0],
            [1, 0, 0],
            [2, 0, 0],
            [2, 0, 0],
            [3, 0, 0],
            [3, 0, 0],
            [4, 0, 0],
            [4, 0, 0],
            [3, 0, 0],
            [3, 0, 0],
            [4, 0, 0],
            [4, 0, 0],
        ];

        let cell = extract_center_cell_pixels(&raster, 1, 1, grid_w, gw, gh);
        assert_eq!(cell.len(), gw * gh);
        assert!(cell.iter().all(|px| *px == [4, 0, 0]));
        let _ = grid_h;
    }

    #[test]
    fn discontinuity_score_penalizes_broken_seam_more_than_smooth_join() {
        let gw = 4;
        let gh = 4;
        let left_bitmap = [
            0, 0, 1, 1, //
            0, 0, 1, 1, //
            0, 0, 1, 1, //
            0, 0, 1, 1,
        ];
        let smooth_right_bitmap = [
            1, 1, 0, 0, //
            1, 1, 0, 0, //
            1, 1, 0, 0, //
            1, 1, 0, 0,
        ];
        let broken_right_bitmap = [
            1, 1, 1, 1, //
            1, 1, 1, 1, //
            0, 0, 0, 0, //
            0, 0, 0, 0,
        ];

        let glyphs = vec![
            make_test_glyph('l', &left_bitmap, gw, gh),
            make_test_glyph('s', &smooth_right_bitmap, gw, gh),
            make_test_glyph('b', &broken_right_bitmap, gw, gh),
        ];
        let grid_data = vec![
            BlockData {
                fg: [255, 255, 255],
                bg: [0, 0, 0],
                fg_col: None,
                bg_col: None,
                alt_fg_col: None,
                alt_bg_col: None,
            },
            BlockData {
                fg: [255, 255, 255],
                bg: [0, 0, 0],
                fg_col: None,
                bg_col: None,
                alt_fg_col: None,
                alt_bg_col: None,
            },
        ];

        let smooth_state = vec![(0, false), (1, false)];
        let broken_state = vec![(0, false), (2, false)];

        let smooth_raster = build_rendered_raster(&grid_data, &smooth_state, &glyphs, 2, 1, gw, gh);
        let broken_raster = build_rendered_raster(&grid_data, &broken_state, &glyphs, 2, 1, gw, gh);

        let smooth_score = cell_discontinuity_score(0, &smooth_raster, 2, 1, gw, gh);
        let broken_score = cell_discontinuity_score(0, &broken_raster, 2, 1, gw, gh);

        assert!(
            broken_score > smooth_score,
            "broken_score={} smooth_score={}",
            broken_score,
            smooth_score
        );
    }

    #[test]
    fn discontinuity_score_does_not_flag_straight_colour_seam() {
        let gw = 4;
        let gh = 4;
        let grid_w = 2;
        let grid_h = 1;
        let raster = vec![
            [255, 0, 0],
            [255, 0, 0],
            [255, 0, 0],
            [255, 0, 0],
            [0, 255, 0],
            [0, 255, 0],
            [0, 255, 0],
            [0, 255, 0],
            [255, 0, 0],
            [255, 0, 0],
            [255, 0, 0],
            [255, 0, 0],
            [0, 255, 0],
            [0, 255, 0],
            [0, 255, 0],
            [0, 255, 0],
            [255, 0, 0],
            [255, 0, 0],
            [255, 0, 0],
            [255, 0, 0],
            [0, 255, 0],
            [0, 255, 0],
            [0, 255, 0],
            [0, 255, 0],
            [255, 0, 0],
            [255, 0, 0],
            [255, 0, 0],
            [255, 0, 0],
            [0, 255, 0],
            [0, 255, 0],
            [0, 255, 0],
            [0, 255, 0],
        ];

        let score = cell_discontinuity_score(0, &raster, grid_w, grid_h, gw, gh);
        assert!(score <= 1.0, "score={}", score);
    }

    #[test]
    fn discontinuity_score_flags_large_stair_step() {
        let gw = 4;
        let gh = 4;
        let grid_w = 2;
        let grid_h = 1;
        let raster = vec![
            [0, 0, 0],
            [0, 0, 0],
            [255, 255, 255],
            [255, 255, 255],
            [255, 255, 255],
            [255, 255, 255],
            [0, 0, 0],
            [0, 0, 0],
            [0, 0, 0],
            [0, 0, 0],
            [255, 255, 255],
            [255, 255, 255],
            [255, 255, 255],
            [255, 255, 255],
            [0, 0, 0],
            [0, 0, 0],
            [0, 0, 0],
            [0, 0, 0],
            [255, 255, 255],
            [255, 255, 255],
            [0, 0, 0],
            [0, 0, 0],
            [255, 255, 255],
            [255, 255, 255],
            [0, 0, 0],
            [0, 0, 0],
            [255, 255, 255],
            [255, 255, 255],
            [0, 0, 0],
            [0, 0, 0],
            [255, 255, 255],
            [255, 255, 255],
        ];

        let score = cell_discontinuity_score(0, &raster, grid_w, grid_h, gw, gh);
        assert!(score > 10.0, "score={}", score);
    }

    #[test]
    fn irc99_nearest_match_skips_user_definable_base_colours() {
        assert_eq!(nearest_hex_colour_fast(0x000000, &IRC99), 88);
        assert_eq!(nearest_hex_colour_fast(0xFFFFFF, &IRC99), 98);
        assert_eq!(
            nearest_distinctly_chromatic_hex_colour(0xFF0000, &IRC99, 0),
            Some(52)
        );
    }

    #[test]
    fn shape_cache_identity_tracks_glyph_set_content() {
        let a = make_test_glyph('a', &[0, 1, 0, 1], 2, 2);
        let b = make_test_glyph('b', &[1, 0, 1, 0], 2, 2);
        let changed = make_test_glyph('a', &[0, 1, 1, 0], 2, 2);
        let original = glyph_set_cache_hash(&[a.clone(), b.clone()], 2, 2);
        assert_ne!(
            original,
            glyph_set_cache_hash(&[b.clone(), a.clone()], 2, 2)
        );
        assert_ne!(original, glyph_set_cache_hash(&[changed, b.clone()], 2, 2));
        assert_ne!(original, glyph_set_cache_hash(&[a, b], 1, 4));
    }

    #[test]
    fn score_fix_band_candidate_delta_matches_repainted_preview() {
        let grid_w = 3;
        let grid_h = 3;
        let gw = 12;
        let gh = 25;
        let preview_width = grid_w * crate::contour_score::PREVIEW_CELL_WIDTH;
        let preview_height = grid_h * crate::contour_score::PREVIEW_CELL_HEIGHT;
        let preview = vec![[8, 16, 24]; preview_width * preview_height];
        let bitmap = (0..gw * gh)
            .map(|idx| {
                let x = idx % gw;
                let y = idx / gw;
                ((x * 3 + y * 5) % 11 < 5) as u8
            })
            .collect::<Vec<_>>();
        let glyph = make_test_glyph('x', &bitmap, gw, gh);
        let glyph_preview = score_fix_preview_glyph(&glyph, gw, gh);
        let empty_glyph = make_test_glyph(' ', &vec![0; gw * gh], gw, gh);
        let glyph_previews = vec![
            score_fix_preview_glyph(&empty_glyph, gw, gh),
            glyph_preview,
        ];
        let data = BlockData {
            fg: [240, 96, 12],
            bg: [8, 16, 24],
            fg_col: None,
            bg_col: None,
            alt_fg_col: None,
            alt_bg_col: None,
        };
        let grid_data = vec![data.clone(); grid_w * grid_h];
        let grid_state = vec![(0usize, false); grid_w * grid_h];
        let changed_idx = 4;
        let initial_internal = score_fix_preview_internal_lengths(&preview, grid_w, grid_h);
        let mut painted = preview.clone();
        paint_global_preview_cell(
            &mut painted,
            preview_width,
            changed_idx,
            grid_w,
            &data,
            &glyph,
            true,
            gw,
            gh,
        );
        let painted_internal =
            score_fix_preview_internal_lengths(&painted, grid_w, grid_h);
        assert_eq!(
            glyph_preview.internal_boundary_length,
            painted_internal[changed_idx]
        );
        let initial_global =
            score_fix_grid_boundary_length(&preview, &initial_internal, grid_w, grid_h);
        let painted_global =
            score_fix_grid_boundary_length(&painted, &painted_internal, grid_w, grid_h);
        let current_contribution = score_fix_center_boundary_contribution(
            &grid_state,
            &grid_data,
            &glyph_previews,
            changed_idx,
            grid_w,
            grid_h,
            None,
            false,
        );
        let candidate_contribution = score_fix_center_boundary_contribution(
            &grid_state,
            &grid_data,
            &glyph_previews,
            changed_idx,
            grid_w,
            grid_h,
            Some((1, true)),
            false,
        );
        assert_eq!(
            initial_global - current_contribution + candidate_contribution,
            painted_global
        );
        let band_only = crate::contour_score::ContourScoreWeights {
            bending: 0.0,
            endpoints: 0.0,
            junctions: 0.0,
            fragments: 0.0,
            fidelity: 0.0,
            boundary_balance: 0.1,
            peak_sensitivity: 0.0,
        };

        for center_idx in 0..grid_w * grid_h {
            let virtual_length = score_fix_patch_boundary_length_with_candidate(
                &preview,
                &initial_internal,
                center_idx,
                changed_idx,
                &data,
                glyph_preview,
                true,
                glyph_preview.internal_boundary_length,
                grid_w,
                grid_h,
            );
            let painted_length = score_fix_patch_boundary_length(
                &painted,
                &painted_internal,
                center_idx,
                grid_w,
                grid_h,
            );
            assert_eq!(virtual_length, painted_length, "center={center_idx}");
            let bit_parallel_length = score_fix_patch_boundary_length_from_cells(
                &grid_state,
                &grid_data,
                &glyph_previews,
                center_idx,
                Some((changed_idx, (1, true))),
                grid_w,
                grid_h,
            );
            assert_eq!(
                bit_parallel_length, painted_length,
                "bit-parallel center={center_idx}"
            );
            let patch = extract_local_patch(
                &painted,
                center_idx % grid_w,
                center_idx / grid_w,
                grid_w,
                grid_h,
                crate::contour_score::PREVIEW_CELL_WIDTH,
                crate::contour_score::PREVIEW_CELL_HEIGHT,
            );
            let exact_length = crate::contour_score::score_rgb_with_weights(
                &patch.pixels,
                patch.width,
                patch.height,
                crate::contour_score::PREVIEW_CELL_WIDTH,
                &band_only,
            )
            .boundary_length;
            assert_eq!(painted_length, exact_length, "center={center_idx}");
        }
    }

    #[test]
    fn score_fix_spiral_ranks_visit_every_cell_once() {
        for (grid_w, grid_h) in [(1, 1), (1, 5), (5, 1), (2, 2), (4, 3), (5, 4)] {
            let ranks = score_fix_spiral_ranks(grid_w, grid_h);
            let mut sorted = ranks.clone();
            sorted.sort_unstable();
            assert_eq!(sorted, (0..grid_w * grid_h).collect::<Vec<_>>());
        }
        assert_eq!(
            score_fix_spiral_ranks(3, 3),
            vec![0, 1, 2, 7, 8, 3, 6, 5, 4]
        );
    }

    #[test]
    fn geometry_state_pool_removes_uniform_and_render_duplicate_states() {
        let diagonal = make_test_glyph('a', &[1, 0, 0, 1], 2, 2);
        let complement = make_test_glyph('b', &[0, 1, 1, 0], 2, 2);
        let uniform = make_test_glyph('c', &[1, 1, 1, 1], 2, 2);
        let states = distinct_nonuniform_glyph_states(&[diagonal, complement, uniform]);
        assert_eq!(states, vec![(0, false), (0, true)]);
    }

    #[test]
    fn geometry_top_n_counts_unique_nonuniform_alternatives() {
        let glyphs = vec![
            make_test_glyph('a', &[1, 0, 0, 1], 2, 2),
            make_test_glyph('b', &[0, 1, 1, 0], 2, 2),
            make_test_glyph('c', &[1, 1, 1, 1], 2, 2),
            make_test_glyph('d', &[1, 0, 1, 0], 2, 2),
        ];
        let catalog = geometry_rendered_state_catalog(&glyphs);
        let costs = vec![0, 5, 1, 2, 0, 0, 3, 4];
        let choices = top_unique_geometry_choices(&costs, (0, false), 2, &catalog);
        assert_eq!(choices, vec![(1, false), (3, false), (0, false)]);
    }

    #[test]
    fn curvature_flow_is_stable_for_a_line_and_penalizes_a_spur() {
        let width = 11;
        let height = 11;
        let dark = [0, 0, 0];
        let light = [255, 255, 255];
        let mut straight = vec![light; width * height];
        for y in 0..height {
            for x in 0..5 {
                straight[y * width + x] = dark;
            }
        }
        let mut spurred = straight.clone();
        spurred[5 * width + 5] = dark;

        let straight_score = curvature_flow_regularity(&straight, width, height);
        let spur_score = curvature_flow_regularity(&spurred, width, height);
        assert!(straight_score <= 1e-9, "straight_score={straight_score}");
        assert!(
            spur_score > straight_score,
            "spur_score={spur_score} straight_score={straight_score}"
        );
    }
}
