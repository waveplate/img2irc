use crate::args::{Render, RenderArgs};
use crate::font::GlyphStore;
use crate::palette::{ANSI256, IRC99};
use image::{Rgb, RgbImage};
use once_cell::sync::Lazy;
use photon_rs::PhotonImage;
use rayon::prelude::*;
use std::collections::HashMap;
use std::f32::consts::PI;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

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
}

#[derive(Clone, Copy, Debug)]
pub struct PrePixel {
    pub r: u64,
    pub g: u64,
    b: u64,
    pub sq: u64,
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
            .filter(|(ch, _, _)| glyphs.selected.contains(ch))
            .collect()
    } else {
        // If not filtering, include everything
        glyphs.glyphs.iter().collect()
    };

    // Fallback: if filtering resulted in no glyphs, use all available glyphs
    if filtered_glyphs.is_empty() {
        filtered_glyphs = glyphs.glyphs.iter().collect();
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

static COLOR_CACHE: Lazy<Mutex<HashMap<u32, (u8, u8, u8, u8)>>> =
    Lazy::new(|| Mutex::new(HashMap::with_capacity(8_192)));

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

    for (i, &p) in palette.iter().enumerate() {
        if palette.len() == 99 && (2..16).contains(&i) {
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

    for (i, &p) in palette.iter().enumerate() {
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

    for (i, &p) in palette.iter().enumerate() {
        if palette.len() == 99 && (2..16).contains(&i) {
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

    if best_d == u32::MAX {
        nearest_hex_colour_fast(target, palette)
    } else {
        best_i
    }
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
        if let Some(&(a, i, an, in_)) = COLOR_CACHE.lock().unwrap().get(px) {
            return AnsiPixel {
                orig: *px,
                ansi_std: a,
                irc_std: i,
                ansi_ng: an,
                irc_ng: in_,
            };
        }

        let ansi_std_val = nearest_hex_colour_fast(*px, &ANSI256);
        let irc_std_val = nearest_hex_colour_fast(*px, &IRC99);

        let mut ansi_ng_val = ansi_std_val;
        let mut irc_ng_val = irc_std_val;

        if !is_near_grayscale(*px, tolerance) {
            if let Some(idx) = nearest_distinctly_chromatic_hex_colour(*px, &ANSI256, tolerance) {
                ansi_ng_val = idx;
            }
            if let Some(idx) = nearest_distinctly_chromatic_hex_colour(*px, &IRC99, tolerance) {
                irc_ng_val = idx;
            }
        }

        COLOR_CACHE
            .lock()
            .unwrap()
            .insert(*px, (ansi_std_val, irc_std_val, ansi_ng_val, irc_ng_val));
        AnsiPixel {
            orig: *px,
            ansi_std: ansi_std_val,
            irc_std: irc_std_val,
            ansi_ng: ansi_ng_val,
            irc_ng: irc_ng_val,
        }
    }
}

impl AnsiImage {
    pub fn new(img: PhotonImage, glyphs: GlyphStore, tolerance: u8) -> Self {
        let w = img.get_width() as usize;
        let raw = img.get_raw_pixels();
        let flat: Vec<u32> = raw.chunks(4).map(make_rgb_u32).collect();
        let mut bitmap: Vec<Vec<u32>> = flat.chunks(w).map(|r| r.to_vec()).collect();

        if !bitmap.len().is_multiple_of(2) {
            bitmap.push(vec![0; w]);
        }
        for row in &mut bitmap {
            if row.len() % 2 != 0 {
                row.push(0);
            }
        }

        let block = block_bitmap(&bitmap, &glyphs, tolerance);
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
) -> Vec<Vec<AnsiPixelBlock>> {
    if glyphs.glyphs.is_empty() {
        return Vec::new();
    }
    let (gw, gh) = glyphs.metrics;
    if gh == 0 || gw == 0 {
        return Vec::new();
    }

    let mut px_rows: Vec<Vec<AnsiPixel>> = src
        .iter()
        .map(|row| row.iter().map(|px| AnsiPixel::new(px, tolerance)).collect())
        .collect();

    let ph = px_rows.len().div_ceil(gh) * gh;
    let pw = if px_rows.is_empty() {
        0
    } else {
        px_rows[0].len().div_ceil(gw) * gw
    };

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
    for y in (0..ph).step_by(gh) {
        let mut row = Vec::with_capacity(pw / gw);
        for x in (0..pw).step_by(gw) {
            let mut block_pixels = vec![vec![AnsiPixel::new(&0, tolerance); gw]; gh];
            for j in 0..gh {
                for i in 0..gw {
                    block_pixels[j][i] = px_rows[y + j][x + i];
                }
            }
            row.push(AnsiPixelBlock {
                pixels: block_pixels,
            });
        }
        out.push(row);
    }
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

    if need_style {
        if render == Render::Irc {
            if bold_changed { out.push('\x02'); }
            if italic_changed { out.push('\x1D'); }
            if underline_changed { out.push('\x1F'); }
        } else if !as_preview {
            if bold_changed { write!(out, "\x1b[{}m", if bold { "1" } else { "22" }).unwrap(); }
            if italic_changed { write!(out, "\x1b[{}m", if italic { "3" } else { "23" }).unwrap(); }
            if underline_changed { write!(out, "\x1b[{}m", if underline { "4" } else { "24" }).unwrap(); }
        }
        *last_bold = bold;
        *last_italic = italic;
        *last_underline = underline;
    }

    if need_fg {
        *last_fg = Some(fg.clone());
    }
    if need_bg {
        *last_bg = bg.clone();
    }

    if need_fg || need_bg || need_style {
        if as_preview {
            if need_style {
                if bold_changed { write!(out, "\x1b[{}m", if bold { "1" } else { "22" }).unwrap(); }
                if italic_changed { write!(out, "\x1b[{}m", if italic { "3" } else { "23" }).unwrap(); }
                if underline_changed { write!(out, "\x1b[{}m", if underline { "4" } else { "24" }).unwrap(); }
            }
            if let Some(bg_c) = &bg {
                out.push_str("\x1b[");
                let mut semi = false;

                if need_fg {
                    match fg {
                        Colour::Index(i) => {
                            let [r, g, b] = if render == Render::Irc {
                                crate::draw::unpack_rgb(crate::palette::IRC99[i as usize])
                            } else {
                                crate::draw::unpack_rgb(crate::palette::ANSI256[i as usize])
                            };
                            write!(out, "38;2;{};{};{}", r, g, b).unwrap()
                        },
                        Colour::RGB([r, g, b]) => {
                            write!(out, "38;2;{};{};{}", r, g, b).unwrap()
                        }
                    }
                    semi = true;
                }

                if need_bg {
                    if semi {
                        out.push(';');
                    }
                    match bg_c {
                        Colour::Index(i) => {
                            let [r, g, b] = if render == Render::Irc {
                                crate::draw::unpack_rgb(crate::palette::IRC99[*i as usize])
                            } else {
                                crate::draw::unpack_rgb(crate::palette::ANSI256[*i as usize])
                            };
                            write!(out, "48;2;{};{};{}", r, g, b).unwrap()
                        },
                        Colour::RGB([r, g, b]) => {
                            write!(out, "48;2;{};{};{}", r, g, b).unwrap()
                        }
                    }
                }
                out.push('m');
            } else if need_fg {
                match fg {
                    Colour::Index(i) => {
                        let [r, g, b] = if render == Render::Irc {
                            crate::draw::unpack_rgb(crate::palette::IRC99[i as usize])
                        } else {
                            crate::draw::unpack_rgb(crate::palette::ANSI256[i as usize])
                        };
                        write!(out, "\x1b[38;2;{};{};{}m", r, g, b).unwrap()
                    },
                    Colour::RGB([r, g, b]) => {
                        write!(out, "\x1b[38;2;{};{};{}m", r, g, b).unwrap()
                    }
                }
            } else if bg_changed && bg.is_none() {
                out.push_str("\x1b[49m");
            } else if need_style {
                // If only style changed but not colors, we already handled it above, 
                // but we need to make sure we don't fall through to nothing.
            }
        } else {
            match render {
                Render::Ansi | Render::Ansi24 => {
                    if need_style {
                        if bold_changed { write!(out, "\x1b[{}m", if bold { "1" } else { "22" }).unwrap(); }
                        if italic_changed { write!(out, "\x1b[{}m", if italic { "3" } else { "23" }).unwrap(); }
                        if underline_changed { write!(out, "\x1b[{}m", if underline { "4" } else { "24" }).unwrap(); }
                    }
                    if let Some(bg_c) = &bg {
                        out.push_str("\x1b[");
                        let mut semi = false;

                        if need_fg {
                            match fg {
                                Colour::Index(i) => write!(out, "38;5;{}", i).unwrap(),
                                Colour::RGB([r, g, b]) => {
                                    write!(out, "38;2;{};{};{}", r, g, b).unwrap()
                                }
                            }
                            semi = true;
                        }

                        if need_bg {
                            if semi {
                                out.push(';');
                            }
                            match bg_c {
                                Colour::Index(i) => write!(out, "48;5;{}", i).unwrap(),
                                Colour::RGB([r, g, b]) => {
                                    write!(out, "48;2;{};{};{}", r, g, b).unwrap()
                                }
                            }
                        }
                        out.push('m');
                    } else if need_fg {
                        match fg {
                            Colour::Index(i) => write!(out, "\x1b[38;5;{}m", i).unwrap(),
                            Colour::RGB([r, g, b]) => {
                                write!(out, "\x1b[38;2;{};{};{}m", r, g, b).unwrap()
                            }
                        }
                    } else if bg_changed && bg.is_none() {
                        out.push_str("\x1b[49m");
                    }
                }
                Render::Irc => {
                    // IRC clients often reset the background color if \x03FG is used without a BG.
                    // We MUST always output \x03FG,BG when any color state changes.
                    let effective_fg = if is_space {
                        lookahead_fg.or(last_fg.as_ref()).unwrap_or(&fg)
                    } else {
                        &fg
                    };

                    out.push('\x03');
                    match effective_fg {
                        Colour::Index(i) => write!(out, "{}", i).unwrap(),
                        Colour::RGB([r, g, b]) => {
                            let col = ((*r as u32) << 16) | ((*g as u32) << 8) | (*b as u32);
                            let idx = nearest_hex_colour_fast(col, &crate::palette::IRC99[..]);
                            write!(out, "{}", idx).unwrap()
                        }
                    }

                    if let Some(bg_c) = &bg {
                        out.push(',');
                        match bg_c {
                            Colour::Index(i) => write!(out, "{}", i).unwrap(),
                            Colour::RGB([r, g, b]) => {
                                let col = ((*r as u32) << 16) | ((*g as u32) << 8) | (*b as u32);
                                let idx = nearest_hex_colour_fast(col, &crate::palette::IRC99[..]);
                                write!(out, "{}", idx).unwrap()
                            }
                        }
                    }
                    *last_fg = Some(effective_fg.clone());
                }
            }
        }
        *first = false;
    }
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
    pub fix_disc_time: Option<std::time::Duration>,
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
            fix_disc_time: None,
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

            for ov in overlays {
                if let Some(opt_c) = get_overlay_at(ov, cx, cy) {
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
                            let hex = if (*i as usize) < palette.len() { palette[*i as usize] } else { 0xFFFFFF };
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
                            let hex = if (*i as usize) < palette.len() { palette[*i as usize] } else { 0x000000 };
                            [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8]
                        }
                    });
                    break;
                }
            }

            let (draw_fg_raw, draw_bg_raw, draw_char, draw_bold, draw_italic, draw_underline) = if let Some(ch) = ov_char {
                let fg_raw = ov_fg.unwrap_or(cell.fg);
                let bg_raw = ov_bg.or(Some(cell.bg.unwrap_or([0, 0, 0])));
                (fg_raw, bg_raw, ch, ov_bold, ov_italic, ov_underline)
            } else {
                (cell.fg, cell.bg, cell.char, cell.bold, cell.italic, cell.underline)
            };

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
        grid: base.grid.clone(),
        error_count: base.error_count,
        total_pixels: base.total_pixels,
        dynamic_scaling_count: base.dynamic_scaling_count,
        score: base.score,
        longest_line_bytes,
        initial_time: base.initial_time,
        fix_disc_time: base.fix_disc_time,
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

#[derive(Clone)]
struct QuantizedCellReference {
    mask: Vec<u8>,
}

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

fn local_patch_frequency_score(patch_pixels: &[[u8; 3]], width: usize, height: usize) -> f32 {
    spectral_high_frequency_score(patch_pixels, width, height)
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

fn dump_fft_fix_change(
    root: &Path,
    pass: usize,
    cell_x: usize,
    cell_y: usize,
    before_patch: &LocalPatch,
    after_patch: &LocalPatch,
    gw: usize,
    gh: usize,
) -> std::io::Result<(String, String, String, String)> {
    let pass_dir = root.join("changes").join(format!("pass_{:02}", pass));
    let raw_dir = pass_dir.join("raw");
    let marked_dir = pass_dir.join("marked");
    fs::create_dir_all(&raw_dir)?;
    fs::create_dir_all(&marked_dir)?;

    let before_name = format!("y{:04}_x{:04}_before.png", cell_y, cell_x);
    let after_name = format!("y{:04}_x{:04}_after.png", cell_y, cell_x);

    save_rgb_pixels_as_png(
        &raw_dir.join(&before_name),
        &before_patch.pixels,
        before_patch.width,
        before_patch.height,
    )?;
    save_rgb_pixels_as_png(
        &raw_dir.join(&after_name),
        &after_patch.pixels,
        after_patch.width,
        after_patch.height,
    )?;

    let before_marked = with_center_outline(before_patch, gw, gh);
    let after_marked = with_center_outline(after_patch, gw, gh);
    save_rgb_pixels_as_png(
        &marked_dir.join(&before_name),
        &before_marked,
        before_patch.width,
        before_patch.height,
    )?;
    save_rgb_pixels_as_png(
        &marked_dir.join(&after_name),
        &after_marked,
        after_patch.width,
        after_patch.height,
    )?;

    Ok((
        format!("changes/pass_{:02}/raw/{}", pass, before_name),
        format!("changes/pass_{:02}/raw/{}", pass, after_name),
        format!("changes/pass_{:02}/marked/{}", pass, before_name),
        format!("changes/pass_{:02}/marked/{}", pass, after_name),
    ))
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

fn best_candidate_pool_pre(
    costs: &[u64],
    pool_size: usize,
) -> Vec<(usize, bool, u64)> {
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

fn current_cell_frequency_score(
    raster: &[[u8; 3]],
    cell_x: usize,
    cell_y: usize,
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
) -> f32 {
    let patch = extract_local_patch(raster, cell_x, cell_y, grid_w, grid_h, gw, gh);
    local_patch_frequency_score(&patch.pixels, patch.width, patch.height)
}

fn candidate_cell_frequency_score(
    raster: &[[u8; 3]],
    cell_x: usize,
    cell_y: usize,
    data: &BlockData,
    glyph: &FastGlyph,
    inv: bool,
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
) -> f32 {
    let mut patch = extract_local_patch(raster, cell_x, cell_y, grid_w, grid_h, gw, gh);
    paint_cell_pixels(
        &mut patch.pixels,
        patch.width,
        patch.cell_offset_x,
        patch.cell_offset_y,
        data,
        glyph,
        inv,
        gw,
        gh,
    );
    local_patch_frequency_score(&patch.pixels, patch.width, patch.height)
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

fn build_quantized_cell_references(
    image: &AnsiImage,
    grid_data: &[BlockData],
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
) -> Vec<QuantizedCellReference> {
    (0..grid_w * grid_h)
        .into_par_iter()
        .map(|idx| {
            let cell_x = idx % grid_w;
            let cell_y = idx / grid_w;
            let blk = &image.block[cell_y][cell_x];
            let data = &grid_data[idx];

            if data.fg == data.bg {
                return QuantizedCellReference {
                    mask: vec![0u8; gw * gh],
                };
            }

            let fg_r = data.fg[0] as i64;
            let fg_g = data.fg[1] as i64;
            let fg_b = data.fg[2] as i64;
            let bg_r = data.bg[0] as i64;
            let bg_g = data.bg[1] as i64;
            let bg_b = data.bg[2] as i64;

            let mut mask = Vec::with_capacity(gw * gh);
            for y in 0..gh {
                for x in 0..gw {
                    let rgb = unpack_rgb(blk.pixels[y][x].orig);
                    let r = rgb[0] as i64;
                    let g = rgb[1] as i64;
                    let b = rgb[2] as i64;
                    let fg_cost = (r - fg_r).pow(2) as u64
                        + (g - fg_g).pow(2) as u64
                        + (b - fg_b).pow(2) as u64;
                    let bg_cost = (r - bg_r).pow(2) as u64
                        + (g - bg_g).pow(2) as u64
                        + (b - bg_b).pow(2) as u64;
                    mask.push((fg_cost <= bg_cost) as u8);
                }
            }

            QuantizedCellReference { mask }
        })
        .collect()
}

#[inline(always)]
fn shifted_mask_sample(
    mask: &[u8],
    gw: usize,
    gh: usize,
    x: usize,
    y: usize,
    dx: isize,
    dy: isize,
) -> u8 {
    let src_x = (x as isize - dx).clamp(0, gw.saturating_sub(1) as isize) as usize;
    let src_y = (y as isize - dy).clamp(0, gh.saturating_sub(1) as isize) as usize;
    mask[src_y * gw + src_x]
}

fn shifted_reference_match_cost(
    mask: &[u8],
    glyph: &FastGlyph,
    inv: bool,
    gw: usize,
    gh: usize,
    dx: isize,
    dy: isize,
) -> u32 {
    let inv_u8 = inv as u8;
    let mut cost = 0u32;
    for y in 0..gh {
        for x in 0..gw {
            let reference = shifted_mask_sample(mask, gw, gh, x, y, dx, dy);
            let candidate = glyph.bitmap[y * gw + x] ^ inv_u8;
            cost += (candidate != reference) as u32;
        }
    }
    cost
}

fn best_shifted_reference_match_with_radius(
    mask: &[u8],
    glyph: &FastGlyph,
    inv: bool,
    gw: usize,
    gh: usize,
    shift_radius: isize,
) -> (u32, i8, i8) {
    let mut best_cost = u32::MAX;
    let mut best_dx = 0i8;
    let mut best_dy = 0i8;

    for dy in -shift_radius..=shift_radius {
        for dx in -shift_radius..=shift_radius {
            let cost = shifted_reference_match_cost(mask, glyph, inv, gw, gh, dx, dy);
            if cost < best_cost {
                best_cost = cost;
                best_dx = dx as i8;
                best_dy = dy as i8;
            }
        }
    }

    (best_cost, best_dx, best_dy)
}

#[inline(always)]
fn allowed_reference_cost(base_cost: u32, tolerance_ratio: f32, tolerance_pixels: u32) -> u32 {
    ((base_cost as f32 * (1.0 + tolerance_ratio)).ceil() as u32) + tolerance_pixels
}

fn cell_frequency_scores(
    raster: &[[u8; 3]],
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
) -> Vec<f32> {
    (0..grid_w * grid_h)
        .into_par_iter()
        .map(|idx| {
            let cell_x = idx % grid_w;
            let cell_y = idx / grid_w;
            current_cell_frequency_score(raster, cell_x, cell_y, grid_w, grid_h, gw, gh)
        })
        .collect()
}

#[inline(always)]
fn glyph_edge_color(bit: u8, inv: u8, data: &BlockData) -> [u8; 3] {
    if (bit ^ inv) == 1 {
        data.fg
    } else {
        data.bg
    }
}

fn continuity_hint_score(
    curr_idx: usize,
    candidate: (usize, bool),
    grid_data: &[BlockData],
    grid_state: &[(usize, bool)],
    glyphs: &[FastGlyph],
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
) -> u32 {
    let cx = curr_idx % grid_w;
    let cy = curr_idx / grid_w;
    let (gi, inv) = candidate;
    let glyph = &glyphs[gi];
    let data = &grid_data[curr_idx];
    let inv_u8 = inv as u8;
    let mut score = 0u32;

    if cx > 0 {
        let n_idx = curr_idx - 1;
        let (n_gi, n_inv) = grid_state[n_idx];
        let neighbor = &glyphs[n_gi];
        let neighbor_data = &grid_data[n_idx];
        let neighbor_inv = n_inv as u8;
        for i in 0..gh {
            let left = glyph_edge_color(neighbor.right[i], neighbor_inv, neighbor_data);
            let right = glyph_edge_color(glyph.left[i], inv_u8, data);
            score += (normalized_rgb_distance(left, right) < 0.01) as u32;
        }
    }

    if cx + 1 < grid_w {
        let n_idx = curr_idx + 1;
        let (n_gi, n_inv) = grid_state[n_idx];
        let neighbor = &glyphs[n_gi];
        let neighbor_data = &grid_data[n_idx];
        let neighbor_inv = n_inv as u8;
        for i in 0..gh {
            let left = glyph_edge_color(glyph.right[i], inv_u8, data);
            let right = glyph_edge_color(neighbor.left[i], neighbor_inv, neighbor_data);
            score += (normalized_rgb_distance(left, right) < 0.01) as u32;
        }
    }

    if cy > 0 {
        let n_idx = curr_idx - grid_w;
        let (n_gi, n_inv) = grid_state[n_idx];
        let neighbor = &glyphs[n_gi];
        let neighbor_data = &grid_data[n_idx];
        let neighbor_inv = n_inv as u8;
        for i in 0..gw {
            let top = glyph_edge_color(neighbor.bottom[i], neighbor_inv, neighbor_data);
            let bottom = glyph_edge_color(glyph.top[i], inv_u8, data);
            score += (normalized_rgb_distance(top, bottom) < 0.01) as u32;
        }
    }

    if cy + 1 < grid_h {
        let n_idx = curr_idx + grid_w;
        let (n_gi, n_inv) = grid_state[n_idx];
        let neighbor = &glyphs[n_gi];
        let neighbor_data = &grid_data[n_idx];
        let neighbor_inv = n_inv as u8;
        for i in 0..gw {
            let top = glyph_edge_color(glyph.bottom[i], inv_u8, data);
            let bottom = glyph_edge_color(neighbor.top[i], neighbor_inv, neighbor_data);
            score += (normalized_rgb_distance(top, bottom) < 0.01) as u32;
        }
    }

    score
}

fn max_continuity_samples(
    curr_idx: usize,
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
) -> u32 {
    let cx = curr_idx % grid_w;
    let cy = curr_idx / grid_w;
    let mut total = 0u32;
    if cx > 0 {
        total += gh as u32;
    }
    if cx + 1 < grid_w {
        total += gh as u32;
    }
    if cy > 0 {
        total += gw as u32;
    }
    if cy + 1 < grid_h {
        total += gw as u32;
    }
    total
}

fn cell_discontinuity_score(
    curr_idx: usize,
    grid_data: &[BlockData],
    grid_state: &[(usize, bool)],
    glyphs: &[FastGlyph],
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
) -> f32 {
    let data = &grid_data[curr_idx];
    if data.fg == data.bg {
        return 0.0;
    }

    let current_choice = grid_state[curr_idx];
    let continuity = continuity_hint_score(
        curr_idx,
        current_choice,
        grid_data,
        grid_state,
        glyphs,
        grid_w,
        grid_h,
        gw,
        gh,
    );
    let max_samples = max_continuity_samples(curr_idx, grid_w, grid_h, gw, gh);
    if max_samples == 0 {
        0.0
    } else {
        100.0 * (1.0 - continuity as f32 / max_samples as f32)
    }
}

fn all_cell_discontinuity_scores(
    grid_data: &[BlockData],
    grid_state: &[(usize, bool)],
    glyphs: &[FastGlyph],
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
) -> Vec<f32> {
    (0..grid_w * grid_h)
        .into_par_iter()
        .map(|idx| {
            cell_discontinuity_score(idx, grid_data, grid_state, glyphs, grid_w, grid_h, gw, gh)
        })
        .collect()
}

fn has_flat_uniform_neighbor_context(
    curr_idx: usize,
    grid_data: &[BlockData],
    grid_w: usize,
    grid_h: usize,
) -> bool {
    let cx = curr_idx % grid_w;
    let cy = curr_idx / grid_w;
    let mut uniform_color: Option<[u8; 3]> = None;
    let mut found_neighbor = false;

    let y0 = cy.saturating_sub(1);
    let y1 = (cy + 1).min(grid_h - 1);
    let x0 = cx.saturating_sub(1);
    let x1 = (cx + 1).min(grid_w - 1);

    for ny in y0..=y1 {
        for nx in x0..=x1 {
            if nx == cx && ny == cy {
                continue;
            }
            found_neighbor = true;
            let neighbor = &grid_data[ny * grid_w + nx];
            if neighbor.fg != neighbor.bg {
                return false;
            }
            if let Some(color) = uniform_color {
                if neighbor.fg != color {
                    return false;
                }
            } else {
                uniform_color = Some(neighbor.fg);
            }
        }
    }

    found_neighbor && uniform_color.is_some()
}

fn get_overlay_at(
    ov: &crate::args::TextOverlay,
    cx: i32,
    cy: i32,
) -> Option<Option<char>> {
    if cy < ov.y {
        return None;
    }
    let dy = (cy - ov.y) as usize;
    let lines: Vec<&str> = ov.text.split('\n').collect();

    if ov.w > 0 && ov.h > 0 {
        if cx < ov.x || cx >= ov.x + ov.w || cy >= ov.y + ov.h {
            return None; // Outside the box
        }
        
        let dx = cx - ov.x;
        
        if ov.wrap {
            // Reconstruct wrapped lines
            let mut wrapped_lines = Vec::new();
            for para in &lines {
                let mut current_line = Vec::new();
                for c in para.chars() {
                    if current_line.len() >= ov.w as usize {
                        wrapped_lines.push(current_line);
                        current_line = Vec::new();
                    }
                    current_line.push(c);
                }
                wrapped_lines.push(current_line);
            }
            if dy < wrapped_lines.len() {
                if dx < wrapped_lines[dy].len() as i32 {
                    return Some(Some(wrapped_lines[dy][dx as usize]));
                }
            }
        } else {
            if dy < lines.len() {
                let line_chars: Vec<char> = lines[dy].chars().collect();
                if dx < line_chars.len() as i32 {
                    return Some(Some(line_chars[dx as usize]));
                }
            }
        }
        return Some(None); // Inside box, but no char
    } else {
        if cx < ov.x {
            return None;
        }
        if dy < lines.len() {
            let dx = cx - ov.x;
            let line_chars: Vec<char> = lines[dy].chars().collect();
            if dx < line_chars.len() as i32 {
                return Some(Some(line_chars[dx as usize]));
            }
        }
        return None;
    }
}

pub fn render_blocks(
    image: &AnsiImage,
    args: &RenderArgs,
    render: Render,
    glyphs: &[FastGlyph],
) -> RenderResult {
    // Redirect to smooth renderer but with smoothing features effectively disabled by the 'args.smooth' flag check inside.
    // However, if args.smooth is false, we need to ensure render_blocks_smooth behaves correctly (it does, assuming I fixed the loop issue).
    // The user wants "quantized the same as is donen when --smooth is used".
    // This implies using the exact same pipeline.
    render_blocks_smooth(image, args, render, glyphs, None)
}

#[inline(always)]
fn luma(rgb: &[u8; 3]) -> u8 {
    (0.299 * rgb[0] as f32 + 0.587 * rgb[1] as f32 + 0.114 * rgb[2] as f32).round() as u8
}

pub fn render_blocks_smooth(
    image: &AnsiImage,
    args: &RenderArgs,
    render: Render,
    glyphs: &[FastGlyph],
    abort_flag: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> RenderResult {
    let process_start_time = std::time::Instant::now();

    if glyphs.is_empty() {
        return "Error: no glyphs available".into();
    }
    let (gw, gh) = image.glyphs.metrics;
    let bp = gh * gw;

    // Grid Dimensions
    let grid_h = image.block.len();
    let grid_w = if grid_h > 0 { image.block[0].len() } else { 0 };
    if grid_w == 0 {
        return "".into();
    }

    let needs_frequency_metric =
        args.show_discontinuities || args.fix_disc || args.score || args.fft_debug_dir.is_some();

    let scales = vec![(1.0, 1.0)];

    log::debug!("args.gpu = {}", args.gpu);
    // Parallel Processing
    let rows_results: Vec<(Vec<BlockData>, Vec<(usize, bool)>)> = if args.gpu {
        let gpu_res = crate::gpu::render_blocks_gpu(
            image,
            glyphs,
            render.clone(),
            args.grayscale_tolerance,
            bp as u32,
            grid_h * grid_w,
        );
        let mut results = Vec::with_capacity(grid_h);
        let mut r_idx = 0;
        for _y in 0..grid_h {
            let mut row_data = Vec::with_capacity(grid_w);
            let mut row_state = Vec::with_capacity(grid_w);
            for _x in 0..grid_w {
                let res = &gpu_res[r_idx];
                r_idx += 1;

                let (final_fg, final_bg, fg_obj, bg_obj, alt_fg_obj, alt_bg_obj) =
                    if render == Render::Ansi24 {
                        let final_fg = [res.fg_r as u8, res.fg_g as u8, res.fg_b as u8];
                        let final_bg = [res.bg_r as u8, res.bg_g as u8, res.bg_b as u8];
                        (
                            final_fg,
                            final_bg,
                            Some(Colour::RGB(final_fg)),
                            Some(Colour::RGB(final_bg)),
                            None,
                            None,
                        )
                    } else if render == Render::Irc {
                        let final_fg = [res.fg_r as u8, res.fg_g as u8, res.fg_b as u8];
                        let final_bg = [res.bg_r as u8, res.bg_g as u8, res.bg_b as u8];

                        let col_hi = make_rgb_u32(&final_fg);
                        let col_lo = make_rgb_u32(&final_bg);
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
                    } else if render == Render::Ansi {
                        let final_fg = [res.fg_r as u8, res.fg_g as u8, res.fg_b as u8];
                        let final_bg = [res.bg_r as u8, res.bg_g as u8, res.bg_b as u8];

                        let col_hi = make_rgb_u32(&final_fg);
                        let col_lo = make_rgb_u32(&final_bg);
                        let hi_idx = nearest_strict_partitioned_hex_colour(
                            col_hi,
                            &ANSI256,
                            args.grayscale_tolerance,
                        );
                        let lo_idx = nearest_strict_partitioned_hex_colour(
                            col_lo,
                            &ANSI256,
                            args.grayscale_tolerance,
                        );

                        let hi_rgb = unpack_rgb(ANSI256[hi_idx as usize]);
                        let lo_rgb = unpack_rgb(ANSI256[lo_idx as usize]);

                        (
                            hi_rgb,
                            lo_rgb,
                            Some(Colour::Index(hi_idx)),
                            Some(Colour::Index(lo_idx)),
                            None,
                            None,
                        )
                    } else {
                        let final_fg = [res.fg_r as u8, res.fg_g as u8, res.fg_b as u8];
                        let final_bg = [res.bg_r as u8, res.bg_g as u8, res.bg_b as u8];

                        let col_hi = make_rgb_u32(&final_fg);
                        let col_lo = make_rgb_u32(&final_bg);

                        let hi_ansi = nearest_strict_partitioned_hex_colour(
                            col_hi,
                            &ANSI256,
                            args.grayscale_tolerance,
                        );
                        let lo_ansi = nearest_strict_partitioned_hex_colour(
                            col_lo,
                            &ANSI256,
                            args.grayscale_tolerance,
                        );

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

                        let hi_rgb = unpack_rgb(ANSI256[hi_ansi as usize]);
                        let lo_rgb = unpack_rgb(ANSI256[lo_ansi as usize]);

                        (
                            hi_rgb,
                            lo_rgb,
                            Some(Colour::Index(hi_ansi)),
                            Some(Colour::Index(lo_ansi)),
                            Some(Colour::Index(hi_irc)),
                            Some(Colour::Index(lo_irc)),
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
                row_state.push((res.best_glyph as usize, res.best_inv != 0));
            }
            results.push((row_data, row_state));
        }
        results
    } else {
        let shape_cache = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::<u64, (usize, bool)>::new()));

        image
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

                for blk in block_row {
                    // 1. Capture Base Pixels
                    let mut original_pixels = Vec::with_capacity(bp);
                    for y in 0..gh {
                        for x in 0..gw {
                            let px_orig = blk.pixels[y][x].orig;
                            let rgb = unpack_rgb(px_orig);
                            let p = PrePixel {
                                r: rgb[0] as u64,
                                g: rgb[1] as u64,
                                b: rgb[2] as u64,
                                sq: (rgb[0] as u64).pow(2)
                                    + (rgb[1] as u64).pow(2)
                                    + (rgb[2] as u64).pow(2),
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

                        let mut block_sum_r = 0u64;
                        let mut block_sum_g = 0u64;
                        let mut block_sum_b = 0u64;
                        let mut block_sum_sq = 0u64;

                        let palette: &[u32] = if args.as_preview {
                            &IRC99
                        } else {
                            match render {
                                Render::Ansi | Render::Ansi24 => &ANSI256,
                                _ => &IRC99,
                            }
                        };
                        let mut indices = Vec::with_capacity(bp);

                        for p in pixels {
                            block_sum_r += p.r;
                            block_sum_g += p.g;
                            block_sum_b += p.b;
                            block_sum_sq += p.sq;
                            
                            let rgb_u32 = ((p.r as u32) << 16) | ((p.g as u32) << 8) | p.b as u32;
                            let idx = nearest_strict_partitioned_hex_colour(
                                rgb_u32,
                                palette,
                                args.grayscale_tolerance,
                            );
                            indices.push(idx);
                        }

                        let mut shape = Vec::with_capacity(bp);
                        let mut seen = Vec::new();
                        for &idx in &indices {
                            if let Some(pos) = seen.iter().position(|&c| c == idx) {
                                shape.push(pos as u8);
                            } else {
                                shape.push(seen.len() as u8);
                                seen.push(idx);
                            }
                        }

                        use std::hash::{Hash, Hasher};
                        let mut hasher = std::collections::hash_map::DefaultHasher::new();
                        shape.hash(&mut hasher);
                        let shape_hash = hasher.finish();

                        let cached = shape_cache.lock().unwrap().get(&shape_hash).copied();

                        let (scale_best_glyph, scale_best_inv) = if let Some(c) = cached {
                            c
                        } else {
                            let mut best_cost = u64::MAX;
                            let mut best_glyph = 0;
                            let mut best_inv = false;

                            if render != Render::Ansi24 {
                                for (gi, fglyph) in glyphs.iter().enumerate() {
                                    let invert_opts: &[bool] = &[false, true];
                                    for &inv in invert_opts {
                                        let mut fg_cnt = [0u16; 256];
                                        let mut bg_cnt = [0u16; 256];
                                        let mut fg_len = 0;
                                        let mut bg_len = 0;

                                        let (fg_iter, bg_iter) = if inv {
                                            (&fglyph.zeros, &fglyph.ones)
                                        } else {
                                            (&fglyph.ones, &fglyph.zeros)
                                        };

                                        for &i in fg_iter {
                                            fg_cnt[indices[i] as usize] += 1;
                                            fg_len += 1;
                                        }
                                        for &i in bg_iter {
                                            bg_cnt[indices[i] as usize] += 1;
                                            bg_len += 1;
                                        }
                                        let (_fgi_raw, fx) = fg_cnt
                                            .iter()
                                            .enumerate()
                                            .max_by_key(|&(_, c)| c)
                                            .unwrap_or((0, &0));
                                        let (_bgi_raw, bx) = bg_cnt
                                            .iter()
                                            .enumerate()
                                            .max_by_key(|&(_, c)| c)
                                            .unwrap_or((0, &0));

                                        let cost = ((fg_len as u64) - (*fx as u64))
                                            + ((bg_len as u64) - (*bx as u64));

                                        if cost < best_cost {
                                            best_cost = cost;
                                            best_glyph = gi;
                                            best_inv = inv;
                                        }
                                    }
                                }
                            } else {
                                for (gi, fglyph) in glyphs.iter().enumerate() {
                                    let mut sum_ones_r = 0u64;
                                    let mut sum_ones_g = 0u64;
                                    let mut sum_ones_b = 0u64;
                                    let mut sum_ones_sq = 0u64;
                                    let count_ones = fglyph.ones.len() as u64;

                                    for &i in &fglyph.ones {
                                        let px = unsafe { pixels.get_unchecked(i) };
                                        sum_ones_r += px.r;
                                        sum_ones_g += px.g;
                                        sum_ones_b += px.b;
                                        sum_ones_sq += px.sq;
                                    }
                                    let count_zeros = bp as u64 - count_ones;
                                    let sum_zeros_r = block_sum_r - sum_ones_r;
                                    let sum_zeros_g = block_sum_g - sum_ones_g;
                                    let sum_zeros_b = block_sum_b - sum_ones_b;
                                    let sum_zeros_sq = block_sum_sq - sum_ones_sq;

                                    let avg_ones = if count_ones > 0 {
                                        [
                                            (sum_ones_r / count_ones) as u8,
                                            (sum_ones_g / count_ones) as u8,
                                            (sum_ones_b / count_ones) as u8,
                                        ]
                                    } else {
                                        [0, 0, 0]
                                    };
                                    let avg_zeros = if count_zeros > 0 {
                                        [
                                            (sum_zeros_r / count_zeros) as u8,
                                            (sum_zeros_g / count_zeros) as u8,
                                            (sum_zeros_b / count_zeros) as u8,
                                        ]
                                    } else {
                                        [0, 0, 0]
                                    };

                                    let t1_1 = sum_ones_sq;
                                    let t2_1 = 2
                                        * (avg_ones[0] as u64 * sum_ones_r
                                            + avg_ones[1] as u64 * sum_ones_g
                                            + avg_ones[2] as u64 * sum_ones_b);
                                    let t3_1 = count_ones
                                        * (avg_ones[0] as u64 * avg_ones[0] as u64
                                            + avg_ones[1] as u64 * avg_ones[1] as u64
                                            + avg_ones[2] as u64 * avg_ones[2] as u64);
                                    let c1 = (t1_1 + t3_1).saturating_sub(t2_1);

                                    let t1_0 = sum_zeros_sq;
                                    let t2_0 = 2
                                        * (avg_zeros[0] as u64 * sum_zeros_r
                                            + avg_zeros[1] as u64 * sum_zeros_g
                                            + avg_zeros[2] as u64 * sum_zeros_b);
                                    let t3_0 = count_zeros
                                        * (avg_zeros[0] as u64 * avg_zeros[0] as u64
                                            + avg_zeros[1] as u64 * avg_zeros[1] as u64
                                            + avg_zeros[2] as u64 * avg_zeros[2] as u64);
                                    let c0 = (t1_0 + t3_0).saturating_sub(t2_0);

                                    let cost_normal = c1 + c0;
                                    if cost_normal < best_cost {
                                        best_cost = cost_normal;
                                        best_glyph = gi;
                                        best_inv = false;
                                    }
                                }
                            }

                            shape_cache.lock().unwrap().insert(shape_hash, (best_glyph, best_inv));
                            (best_glyph, best_inv)
                        };

                        let scale_best_fg;
                        let scale_best_bg;

                        if render != Render::Ansi24 {
                            let fglyph = &glyphs[scale_best_glyph];
                            let (fg_iter, bg_iter) = if scale_best_inv {
                                (&fglyph.zeros, &fglyph.ones)
                            } else {
                                (&fglyph.ones, &fglyph.zeros)
                            };

                            let mut fg_cnt = [0u16; 256];
                            let mut bg_cnt = [0u16; 256];
                            let mut fg_len = 0;
                            let mut bg_len = 0;

                            for &i in fg_iter {
                                fg_cnt[indices[i] as usize] += 1;
                                fg_len += 1;
                            }
                            for &i in bg_iter {
                                bg_cnt[indices[i] as usize] += 1;
                                bg_len += 1;
                            }
                            let (fgi_raw, _) = fg_cnt
                                .iter()
                                .enumerate()
                                .max_by_key(|&(_, c)| c)
                                .unwrap_or((0, &0));
                            let (bgi_raw, _) = bg_cnt
                                .iter()
                                .enumerate()
                                .max_by_key(|&(_, c)| c)
                                .unwrap_or((0, &0));

                            let fgi = if fg_len == 0 { bgi_raw } else { fgi_raw };
                            let bgi = if bg_len == 0 { fgi_raw } else { bgi_raw };

                            scale_best_fg = unpack_rgb(palette[fgi.min(palette.len() - 1)]);
                            scale_best_bg = unpack_rgb(palette[bgi.min(palette.len() - 1)]);
                        } else {
                            let fglyph = &glyphs[scale_best_glyph];
                            let count_ones = fglyph.ones.len() as u64;
                            let mut sum_ones_r = 0u64;
                            let mut sum_ones_g = 0u64;
                            let mut sum_ones_b = 0u64;

                            for &i in &fglyph.ones {
                                let px = unsafe { pixels.get_unchecked(i) };
                                sum_ones_r += px.r;
                                sum_ones_g += px.g;
                                sum_ones_b += px.b;
                            }
                            let count_zeros = bp as u64 - count_ones;
                            let sum_zeros_r = block_sum_r - sum_ones_r;
                            let sum_zeros_g = block_sum_g - sum_ones_g;
                            let sum_zeros_b = block_sum_b - sum_ones_b;

                            scale_best_fg = if count_ones > 0 {
                                [
                                    (sum_ones_r / count_ones) as u8,
                                    (sum_ones_g / count_ones) as u8,
                                    (sum_ones_b / count_ones) as u8,
                                ]
                            } else {
                                [0, 0, 0]
                            };
                            scale_best_bg = if count_zeros > 0 {
                                [
                                    (sum_zeros_r / count_zeros) as u8,
                                    (sum_zeros_g / count_zeros) as u8,
                                    (sum_zeros_b / count_zeros) as u8,
                                ]
                            } else {
                                [0, 0, 0]
                            };
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
            .collect()
    };

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

    if let (Some(root), Some(raster)) = (fft_debug_root.as_ref(), rendered_raster.as_ref()) {
        if let Err(err) = dump_fft_stage_inputs(root, "initial", raster, grid_w, grid_h, gw, gh) {
            log::warn!("failed to dump initial FFT inputs: {}", err);
        }
    }

    let quantized_references = if args.fix_disc {
        build_quantized_cell_references(image, &grid_data, grid_w, grid_h, gw, gh)
    } else {
        Vec::new()
    };

    let candidate_pool_size = args.fix_disc_pool_size.max(1) as usize;
    let shift_radius = args.fix_disc_shift_radius as isize;

    let (block_glyph_costs, block_glyph_pools): (Vec<Vec<u64>>, Vec<Vec<(usize, bool, u64, u32, i8, i8)>>) = if args.fix_disc {
        grid_data
            .par_iter()
            .enumerate()
            .map(|(curr_idx, data)| {
                let x = curr_idx % grid_w;
                let y = curr_idx / grid_w;
                let blk = &image.block[y][x];

                if let Some(flag) = &abort_flag {
                    if flag.load(std::sync::atomic::Ordering::Relaxed) {
                        return (Vec::new(), Vec::new());
                    }
                }
                let fg_p = PrePixel {
                    r: data.fg[0] as u64,
                    g: data.fg[1] as u64,
                    b: data.fg[2] as u64,
                    sq: (data.fg[0] as u64).pow(2)
                        + (data.fg[1] as u64).pow(2)
                        + (data.fg[2] as u64).pow(2),
                };
                let bg_p = PrePixel {
                    r: data.bg[0] as u64,
                    g: data.bg[1] as u64,
                    b: data.bg[2] as u64,
                    sq: (data.bg[0] as u64).pow(2)
                        + (data.bg[1] as u64).pow(2)
                        + (data.bg[2] as u64).pow(2),
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

                for fglyph in glyphs.iter() {
                    let mut dot = 0i64;
                    for &i in &fglyph.ones {
                        dot += unsafe { *diff.get_unchecked(i) };
                    }

                    let c_total_f = (base_cost as i64 + dot).max(0) as u64;
                    let c_total_t = (base_cost as i64 + total_diff - dot).max(0) as u64;

                    costs.push(c_total_f);
                    costs.push(c_total_t);
                }

                let pool = best_candidate_pool_pre(&costs, candidate_pool_size);
                
                let current_reference = &quantized_references[curr_idx];
                let pre_pool = pool.into_iter().map(|(gi, inv, internal)| {
                    let (ref_cost, sx, sy) = best_shifted_reference_match_with_radius(
                        &current_reference.mask,
                        &glyphs[gi],
                        inv,
                        gw,
                        gh,
                        shift_radius,
                    );
                    (gi, inv, internal, ref_cost, sx, sy)
                }).collect();

                (costs, pre_pool)
            })
            .unzip()
    } else {
        (Vec::new(), Vec::new())
    };

    let initial_time = process_start_time.elapsed();
    let mut fix_disc_time = None;

    if args.fix_disc {
        let raster = rendered_raster
            .as_mut()
            .expect("rendered raster should exist when fix_disc is enabled");
        let t_fix = std::time::Instant::now();
        let reference_tolerance_ratio = args.fix_disc_ref_tolerance_ratio.max(0.0);
        let reference_tolerance_pixels = args.fix_disc_ref_tolerance_pixels;
        let mut change_manifest = String::from(
            "pass,cell_x,cell_y,before_ref_cost,after_ref_cost,before_continuity,after_continuity,before_shift_x,before_shift_y,after_shift_x,after_shift_y,before_char,before_inv,after_char,after_inv,before_raw,after_raw,before_marked,after_marked\n",
        );

        for pass in 0..args.fix_disc_passes as usize {
            if let Some(flag) = &abort_flag {
                if flag.load(std::sync::atomic::Ordering::Relaxed) {
                    return "Cancelled".into();
                }
            }

            let mut changes = 0u32;
            let forward = pass % 2 == 0;
            let indices: Vec<usize> = if forward {
                (0..grid_h * grid_w).collect()
            } else {
                (0..grid_h * grid_w).rev().collect()
            };

            for &curr_idx in &indices {
                let cx = curr_idx % grid_w;
                let cy = curr_idx / grid_w;
                let (cur_gi, cur_inv) = grid_state[curr_idx];
                let data = &grid_data[curr_idx];

                if data.fg == data.bg {
                    continue;
                }
                if has_flat_uniform_neighbor_context(curr_idx, &grid_data, grid_w, grid_h) {
                    continue;
                }

                let current_choice = (cur_gi, cur_inv);
                let current_continuity = continuity_hint_score(
                    curr_idx,
                    current_choice,
                    &grid_data,
                    &grid_state,
                    glyphs,
                    grid_w,
                    grid_h,
                    gw,
                    gh,
                );
                let current_reference = quantized_references
                    .get(curr_idx)
                    .expect("quantized references should exist when fix_disc is enabled");
                let (current_ref_cost, current_dx, current_dy) =
                    best_shifted_reference_match_with_radius(
                        &current_reference.mask,
                        &glyphs[cur_gi],
                        cur_inv,
                        gw,
                        gh,
                        shift_radius,
                    );

                let mut best_ref_cost = current_ref_cost;
                let mut best_internal = block_glyph_costs
                    .get(curr_idx)
                    .and_then(|v| v.get(cur_gi * 2 + cur_inv as usize).copied())
                    .unwrap_or(u64::MAX);
                let mut best_continuity = current_continuity;
                let mut best_choice = current_choice;
                let mut best_shift = (current_dx, current_dy);

                let mut pool = block_glyph_pools.get(curr_idx).cloned().unwrap_or_default();
                if !pool.iter().any(|&(gi, inv, _, _, _, _)| (gi, inv) == current_choice) {
                    pool.push((cur_gi, cur_inv, best_internal, current_ref_cost as u32, current_dx as i8, current_dy as i8));
                }

                for (gi, candidate_inv, internal, ref_cost, shift_x, shift_y) in pool {
                    if (gi, candidate_inv) == current_choice {
                        continue;
                    }

                    let continuity = continuity_hint_score(
                        curr_idx,
                        (gi, candidate_inv),
                        &grid_data,
                        &grid_state,
                        glyphs,
                        grid_w,
                        grid_h,
                        gw,
                        gh,
                    );
                    let allowed_ref_cost = allowed_reference_cost(
                        current_ref_cost,
                        reference_tolerance_ratio,
                        reference_tolerance_pixels,
                    );

                    if (continuity > best_continuity && ref_cost <= allowed_ref_cost)
                        || (continuity == best_continuity && ref_cost < best_ref_cost)
                        || (continuity == best_continuity
                            && ref_cost == best_ref_cost
                            && internal < best_internal)
                    {
                        best_ref_cost = ref_cost;
                        best_internal = internal;
                        best_continuity = continuity;
                        best_choice = (gi, candidate_inv);
                        best_shift = (shift_x, shift_y);
                    }
                }

                if best_choice != current_choice
                    && ((best_continuity > current_continuity
                        && best_ref_cost
                            <= allowed_reference_cost(
                                current_ref_cost,
                                reference_tolerance_ratio,
                                reference_tolerance_pixels,
                            ))
                        || (best_continuity == current_continuity
                            && best_ref_cost < current_ref_cost))
                {
                    let before_patch = fft_debug_root.as_ref().map(|_| {
                        extract_local_patch(raster.as_slice(), cx, cy, grid_w, grid_h, gw, gh)
                    });
                    grid_state[curr_idx] = best_choice;
                    apply_choice_to_raster(
                        raster.as_mut_slice(),
                        &grid_data,
                        glyphs,
                        grid_w,
                        gw,
                        gh,
                        curr_idx,
                        best_choice,
                    );
                    if let (Some(root), Some(before_patch)) =
                        (fft_debug_root.as_ref(), before_patch.as_ref())
                    {
                        let after_patch =
                            extract_local_patch(raster.as_slice(), cx, cy, grid_w, grid_h, gw, gh);
                        match dump_fft_fix_change(
                            root,
                            pass,
                            cx,
                            cy,
                            before_patch,
                            &after_patch,
                            gw,
                            gh,
                        ) {
                            Ok((before_raw, after_raw, before_marked, after_marked)) => {
                                writeln!(
                                    change_manifest,
                                    "{},{},{},{},{},{},{},{},{},{},{},{:?},{},{:?},{},{},{},{},{}",
                                    pass,
                                    cx,
                                    cy,
                                    current_ref_cost,
                                    best_ref_cost,
                                    current_continuity,
                                    best_continuity,
                                    current_dx,
                                    current_dy,
                                    best_shift.0,
                                    best_shift.1,
                                    glyphs[cur_gi].ch,
                                    cur_inv,
                                    glyphs[best_choice.0].ch,
                                    best_choice.1,
                                    before_raw,
                                    after_raw,
                                    before_marked,
                                    after_marked
                                )
                                .ok();
                            }
                            Err(err) => {
                                log::warn!(
                                    "failed to dump FFT fix change for cell ({}, {}): {}",
                                    cx,
                                    cy,
                                    err
                                );
                            }
                        }
                    }
                    changes += 1;
                }
            }

            log::info!("fix_disc pass {}: {} cells changed", pass, changes);
            if changes == 0 {
                break;
            }
        }
        if let Some(root) = fft_debug_root.as_ref() {
            if let Err(err) = fs::write(root.join("changes.csv"), change_manifest) {
                log::warn!("failed to write FFT change manifest: {}", err);
            }
        }
        log::info!("fix_disc completed in {:?}", t_fix.elapsed());
        fix_disc_time = Some(t_fix.elapsed());
    }

    let cell_disc_scores =
        all_cell_discontinuity_scores(&grid_data, &grid_state, glyphs, grid_w, grid_h, gw, gh);
    let relative_disc_score = if cell_disc_scores.is_empty() {
        0.0
    } else {
        cell_disc_scores.iter().copied().sum::<f32>() as f64 / cell_disc_scores.len() as f64
    };

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
            .max(1.0);
        cell_disc_scores
            .into_iter()
            .map(|score| {
                if score > 0.5 {
                    let severity = (score / max_score).min(1.0);
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

    if args.show_discontinuities || args.score {
        log::info!("Image Discontinuity Score: {:.2}", relative_disc_score);
    }

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

    for y in 0..grid_h {
        let mut row_vec = Vec::new();
        let mut first = true;
        let mut alt_first = true;
        let mut last_fg: Option<Colour> = None;
        let mut last_bg: Option<Colour> = None;
        let mut last_bold = false;
        let mut last_italic = false;
        let mut last_underline = false;
        let mut alt_last_fg: Option<Colour> = None;
        let mut alt_last_bg: Option<Colour> = None;
        let mut alt_last_bold = false;
        let mut alt_last_italic = false;
        let mut alt_last_underline = false;

        // Pass 1: Collect processed cell data for the row
        let mut row_cells: Vec<(Colour, Option<Colour>, char, bool, bool, bool)> = Vec::with_capacity(grid_w);
        let mut alt_row_cells: Vec<(Colour, Option<Colour>, char, bool, bool, bool)> = Vec::with_capacity(grid_w);

        for x in 0..grid_w {
            let idx = y * grid_w + x;
            let data = &grid_data[idx];
            let (gi, inv) = grid_state[idx];
            let glyph = &glyphs[gi];
            let ch = glyph.ch;

            let mut text_overlay: Option<&crate::args::TextOverlay> = None;
            let mut overlay_char = None;
            for ov in &args.overlays {
                if let Some(opt_c) = get_overlay_at(ov, x as i32, y as i32) {
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
                        },
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
                        },
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
                inverted: inv,
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
                        nearest_distinctly_chromatic_hex_colour(col, &IRC99, args.grayscale_tolerance)
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
                        data.alt_bg_col.clone().unwrap_or_else(|| irc_colour_from_rgb(data.bg)),
                        Some(data.alt_fg_col.clone().unwrap_or_else(|| irc_colour_from_rgb(data.fg))),
                    )
                } else {
                    (
                        data.alt_fg_col.clone().unwrap_or_else(|| irc_colour_from_rgb(data.fg)),
                        Some(data.alt_bg_col.clone().unwrap_or_else(|| irc_colour_from_rgb(data.bg))),
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
                let lookahead = alt_row_cells[i + 1..].iter().find_map(|(f, _, c, _, _, _)| {
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
                    args.as_preview,
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
        score: relative_disc_score,
        longest_line_bytes,
        initial_time,
        fix_disc_time,
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
    for y in (0..h).step_by(4) {
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

            for ov in &args.overlays {
                if let Some(opt_c) = get_overlay_at(ov, (x / 2) as i32, (y / 4) as i32) {
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

                for ov in &args.overlays {
                    if let Some(opt_c) = get_overlay_at(ov, (x / 2) as i32, (y / 4) as i32) {
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
        grid: Vec::new(),
        score: 0.0,
        longest_line_bytes,
        initial_time: std::time::Duration::ZERO,
        fix_disc_time: None,
    }
}

fn pick_colour(pixel: &AnsiPixel, r: Render, args: &RenderArgs) -> Colour {
    let distinct = !is_near_grayscale(pixel.orig, args.grayscale_tolerance);
    match r {
        Render::Ansi => Colour::Index(if args.nograyscale && distinct {
            pixel.ansi_ng
        } else {
            pixel.ansi_std
        }),
        Render::Irc => Colour::Index(if args.nograyscale && distinct {
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
    if result.grid.is_empty() {
        return Ok(());
    }
    let (gw, gh) = glyphs.metrics;
    let (rows, cols) = (result.grid.len(), result.grid[0].len());
    let width = (cols * gw) as u32;
    let height = (rows * gh) as u32;

    // Build raw pixel buffer
    let mut pixels = vec![0u8; (width * height * 3) as usize];
    for (r, row) in result.grid.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            let (xb, yb) = (c * gw, r * gh);
            if let Some(bmp) = glyphs
                .glyphs
                .iter()
                .find(|(ch, _, _)| *ch == cell.char)
                .map(|(_, b, _)| b)
            {
                for y in 0..gh {
                    for x in 0..gw {
                        let on = if y < bmp.len() && x < bmp[0].len() {
                            bmp[y][x] == 1
                        } else {
                            false
                        };
                        let color = if on {
                            cell.fg
                        } else {
                            cell.bg.unwrap_or([0, 0, 0])
                        };
                        let px_idx = ((yb + y) as u32 * width + (xb + x) as u32) as usize * 3;
                        if px_idx + 2 < pixels.len() {
                            pixels[px_idx] = color[0];
                            pixels[px_idx + 1] = color[1];
                            pixels[px_idx + 2] = color[2];
                        }
                    }
                }
            }
        }
    }

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
        .write_image_data(&pixels)
        .map_err(std::io::Error::other)?;
    Ok(())
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
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn candidate_frequency_score_prefers_smoother_center_glyph() {
        let gw = 4;
        let gh = 4;
        let checker_bitmap = [1, 0, 1, 0, 0, 1, 0, 1, 1, 0, 1, 0, 0, 1, 0, 1];
        let smooth_bitmap = [1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0];

        let glyphs = vec![
            make_test_glyph('c', &checker_bitmap, gw, gh),
            make_test_glyph('s', &smooth_bitmap, gw, gh),
        ];

        let fg = [255, 255, 255];
        let bg = [0, 0, 0];
        let grid_data = vec![BlockData {
            fg,
            bg,
            fg_col: None,
            bg_col: None,
            alt_fg_col: None,
            alt_bg_col: None,
        }];
        let grid_state = vec![(0, false)];
        let raster = build_rendered_raster(&grid_data, &grid_state, &glyphs, 1, 1, gw, gh);

        let current_score = current_cell_frequency_score(&raster, 0, 0, 1, 1, gw, gh);
        let smoother_score = candidate_cell_frequency_score(
            &raster,
            0,
            0,
            &grid_data[0],
            &glyphs[1],
            false,
            1,
            1,
            gw,
            gh,
        );

        assert!(
            smoother_score < current_score,
            "smoother_score={} current_score={}",
            smoother_score,
            current_score
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

        let smooth_score =
            cell_discontinuity_score(0, &grid_data, &smooth_state, &glyphs, 2, 1, gw, gh);
        let broken_score =
            cell_discontinuity_score(0, &grid_data, &broken_state, &glyphs, 2, 1, gw, gh);

        assert!(
            broken_score > smooth_score,
            "broken_score={} smooth_score={}",
            broken_score,
            smooth_score
        );
    }
}
