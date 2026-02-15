use crate::args::{Render, RenderArgs};
use crate::font::GlyphStore;
use crate::palette::{ANSI256, IRC99};
use image::Rgb;
use once_cell::sync::Lazy;
use photon_rs::PhotonImage;
use rayon::prelude::*;
use std::collections::HashMap;
use std::f32::consts::PI as F32_PI;
use std::f64::consts::PI as F64_PI;
use std::fmt::Write as _;
use std::iter::repeat;
use std::sync::Mutex;

#[derive(Clone)]
pub struct FastGlyph {
    pub ch: char,
    pub bitmap: Vec<u8>,
    pub top: Vec<u8>,
    pub bottom: Vec<u8>,
    pub left: Vec<u8>,
    pub right: Vec<u8>,
    pub coverage: f32,
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

    let has_selection = !glyphs.selected.is_empty();

    glyphs
        .glyphs
        .iter()
        .filter(|(ch, _)| !has_selection || glyphs.selected.contains(ch))
        .filter(|(ch, _)| {
            if *ch == ' ' && !args.blocks.is_empty() {
                return args.blocks.iter().any(|block| {
                    glyphs
                        .groups
                        .get(&block.to_lowercase())
                        .map_or(false, |chars| chars.contains(&' '))
                });
            }
            true
        })
        .map(|(ch, bmp)| {
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

            let ones = bitmap.iter().filter(|&&b| b == 1).count();
            let coverage = ones as f32 / bp as f32;

            FastGlyph {
                ch: *ch,
                bitmap,
                top,
                bottom,
                left,
                right,
                coverage,
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
    let pb = ((col >> 0) & 0xFF) as i32;

    let mut best_i = 0u8;
    let mut best_d = u32::MAX;
    if palette.is_empty() {
        return 0;
    }

    for (i, &p) in palette.iter().enumerate() {
        if palette.len() == 99 && i >= 2 && i < 16 {
            continue;
        }

        let dr = pr - (((p >> 16) & 0xFF) as i32);
        let dg = pg - (((p >> 8) & 0xFF) as i32);
        let db = pb - (((p >> 0) & 0xFF) as i32);
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
    let pb = ((col >> 0) & 0xFF) as i32;

    let mut best_i: Option<u8> = None;
    let mut best_d = u32::MAX;

    for (i, &p) in palette.iter().enumerate() {
        if is_near_grayscale(p, tolerance) {
            continue;
        }
        let dr = pr - (((p >> 16) & 0xFF) as i32);
        let dg = pg - (((p >> 8) & 0xFF) as i32);
        let db = pb - (((p >> 0) & 0xFF) as i32);
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
    let pb = ((target >> 0) & 0xFF) as i32;

    for (i, &p) in palette.iter().enumerate() {
        if palette.len() == 99 && i >= 2 && i < 16 {
            continue;
        }

        let p_is_chromatic = !is_near_grayscale(p, 0); // Strict check for palette

        if target_is_chromatic == p_is_chromatic {
            let r = ((p >> 16) & 0xFF) as i32;
            let g = ((p >> 8) & 0xFF) as i32;
            let b = ((p >> 0) & 0xFF) as i32;
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
    [(rgb >> 16) as u8, (rgb >> 8) as u8, (rgb >> 0) as u8]
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

        if bitmap.len() % 2 != 0 {
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

    let ph = ((px_rows.len() + gh - 1) / gh) * gh;
    let pw = if px_rows.is_empty() {
        0
    } else {
        ((px_rows[0].len() + gw - 1) / gw) * gw
    };

    for r in &mut px_rows {
        if r.len() < pw {
            r.extend(repeat(AnsiPixel::new(&0, tolerance)).take(pw - r.len()));
        }
    }
    if px_rows.len() < ph {
        let blank = vec![AnsiPixel::new(&0, tolerance); pw];
        px_rows.extend(repeat(blank).take(ph - px_rows.len()));
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
    fg: Colour,
    bg: Option<Colour>,
    glyph: char,
    first: &mut bool,
    last_fg: &mut Option<Colour>,
    last_bg: &mut Option<Colour>,
) {
    let mut fg_changed = *first || last_fg.as_ref() != Some(&fg);
    let bg_changed = *first || last_bg.as_ref() != bg.as_ref();

    let is_space = glyph == ' ' || glyph == '\u{2800}';
    let can_skip_fg = match render {
        Render::Ansi | Render::Ansi24 | Render::Preview => true,
        Render::Irc => bg.is_none(),
    };

    if is_space && can_skip_fg {
        fg_changed = false;
    }

    if fg_changed {
        *last_fg = Some(fg.clone());
    }
    if bg_changed {
        *last_bg = bg.clone();
    }

    if fg_changed || bg_changed {
        match render {
            Render::Ansi | Render::Ansi24 | Render::Preview => {
                if let Some(bg_c) = &bg {
                    out.push_str("\x1b[");
                    let mut semi = false;

                    if fg_changed {
                        match fg {
                            Colour::Index(i) => write!(out, "38;5;{}", i).unwrap(),
                            Colour::RGB([r, g, b]) => {
                                write!(out, "38;2;{};{};{}", r, g, b).unwrap()
                            }
                        }
                        semi = true;
                    }

                    if bg_changed {
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
                } else {
                    if fg_changed {
                        match fg {
                            Colour::Index(i) => write!(out, "\x1b[38;5;{}m", i).unwrap(),
                            Colour::RGB([r, g, b]) => {
                                write!(out, "\x1b[38;2;{};{};{}m", r, g, b).unwrap()
                            }
                        }
                    }
                }
            }
            Render::Irc => {
                out.push('\x03');
                match fg {
                    Colour::Index(i) => write!(out, "{:02}", i).unwrap(),
                    _ => out.push_str("01"),
                }
                if let Some(bg_c) = &bg {
                    out.push(',');
                    match bg_c {
                        Colour::Index(i) => write!(out, "{:02}", i).unwrap(),
                        _ => out.push_str("00"),
                    }
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
}

pub struct RenderResult {
    pub content: String,
    pub save_content: Option<String>,
    pub error_count: u64,
    pub total_pixels: u64,
    pub dynamic_scaling_count: u64,
    pub grid: Vec<Vec<RenderedCell>>,
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
        }
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

    // --- Phase 1: Pre-calculate Block Data (Colors & Pixels) ---
    struct BlockData {
        pixels: Vec<PrePixel>,
        fg: [u8; 3],            // High Color
        bg: [u8; 3],            // Low Color
        fg_col: Option<Colour>, // Cached Colour enum
        bg_col: Option<Colour>,
        alt_fg_col: Option<Colour>, // For Preview mode: IRC version
        alt_bg_col: Option<Colour>,
        is_flat: bool,
    }

    let scales = vec![(1.0, 1.0)];

    // Parallel Processing
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

            for blk in block_row {
                // 1. Capture Base Pixels
                let mut original_pixels = Vec::with_capacity(bp);
                for y in 0..gh {
                    for x in 0..gw {
                        let px_orig = blk.pixels[y][x].orig;
                        let rgb = unpack_rgb(px_orig);

                        // DEBUG
                        use std::sync::atomic::{AtomicBool, Ordering};
                        static PRINTED_PX: AtomicBool = AtomicBool::new(false);
                        if !PRINTED_PX.swap(true, Ordering::SeqCst) {
                            log::info!(
                                "DEBUG: Original Pixel at ({},{}) = {:06x} RGB({},{},{})",
                                x,
                                y,
                                px_orig,
                                rgb[0],
                                rgb[1],
                                rgb[2]
                            );
                        }

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

                let mut best_overall_cost = u64::MAX;
                let mut best_overall_glyph = 0;
                let mut best_overall_inv = false;
                let mut best_overall_fg = [0u8; 3];
                let mut best_overall_bg = [0u8; 3];
                let mut best_block_pixels = original_pixels.clone();

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

                    // Pre-calc Sums for this scale
                    let mut block_sum_r = 0u64;
                    let mut block_sum_g = 0u64;
                    let mut block_sum_b = 0u64;
                    let mut block_sum_sq = 0u64;
                    for p in pixels {
                        block_sum_r += p.r;
                        block_sum_g += p.g;
                        block_sum_b += p.b;
                        block_sum_sq += p.sq;
                    }

                    // 2. Find Best Fit for Scale
                    let mut scale_best_cost = u64::MAX;
                    let mut scale_best_glyph = 0;
                    let mut scale_best_inv = false;
                    let mut scale_best_fg = [0u8; 3];
                    let mut scale_best_bg = [0u8; 3];

                    if render != Render::Ansi24 {
                        // --- Indexed Mode (Frequency/Mode Logic) ---
                        let palette: &[u32] = if render == Render::Ansi {
                            &ANSI256
                        } else {
                            &IRC99
                        };
                        let mut indices = Vec::with_capacity(bp);
                        for p in pixels {
                            let rgb_u32 = ((p.r as u32) << 16) | ((p.g as u32) << 8) | p.b as u32;
                            let idx = match render {
                                Render::Ansi => nearest_strict_partitioned_hex_colour(
                                    rgb_u32,
                                    &ANSI256,
                                    args.grayscale_tolerance,
                                ),
                                Render::Irc | Render::Preview => {
                                    nearest_strict_partitioned_hex_colour(
                                        rgb_u32,
                                        &IRC99,
                                        args.grayscale_tolerance,
                                    )
                                }
                                _ => 0,
                            };
                            indices.push(idx);
                        }

                        for (gi, fglyph) in glyphs.iter().enumerate() {
                            for inv in [false, true] {
                                let mut fg_cnt = [0u16; 256];
                                let mut bg_cnt = [0u16; 256];
                                let mut fg_len = 0;
                                let mut bg_len = 0;
                                for i in 0..bp {
                                    let val = indices[i] as usize;
                                    if fglyph.bitmap[i] ^ (inv as u8) == 1 {
                                        fg_cnt[val] += 1;
                                        fg_len += 1;
                                    } else {
                                        bg_cnt[val] += 1;
                                        bg_len += 1;
                                    }
                                }
                                let (fgi, fx) = fg_cnt
                                    .iter()
                                    .enumerate()
                                    .max_by_key(|&(_, c)| c)
                                    .unwrap_or((0, &0));
                                let (bgi, bx) = bg_cnt
                                    .iter()
                                    .enumerate()
                                    .max_by_key(|&(_, c)| c)
                                    .unwrap_or((0, &0));

                                // Note: We removed fg_only support, so logic is simplified to standard
                                let cost = ((fg_len as u64) - (*fx as u64))
                                    + ((bg_len as u64) - (*bx as u64));

                                if cost < scale_best_cost {
                                    scale_best_cost = cost;
                                    scale_best_glyph = gi;
                                    scale_best_inv = inv;
                                    scale_best_fg = unpack_rgb(palette[fgi.min(palette.len() - 1)]);
                                    scale_best_bg = unpack_rgb(palette[bgi.min(palette.len() - 1)]);
                                }
                            }
                        }
                    } else {
                        // --- TrueColor Mode (RGB Average + Noise Threshold) ---
                        let avg_r = if bp > 0 { block_sum_r / bp as u64 } else { 0 };
                        let avg_g = if bp > 0 { block_sum_g / bp as u64 } else { 0 };
                        let avg_b = if bp > 0 { block_sum_b / bp as u64 } else { 0 };
                        let term_sq = avg_r * avg_r + avg_g * avg_g + avg_b * avg_b;
                        let term_linear =
                            avg_r * block_sum_r + avg_g * block_sum_g + avg_b * block_sum_b;
                        let uniform_cost =
                            (block_sum_sq + (bp as u64 * term_sq)).saturating_sub(2 * term_linear);
                        let noise_gain_threshold = 800u64;

                        for (gi, fglyph) in glyphs.iter().enumerate() {
                            let mut sum_ones_r = 0u64;
                            let mut sum_ones_g = 0u64;
                            let mut sum_ones_b = 0u64;
                            let mut sum_ones_sq = 0u64;
                            let mut count_ones = 0u64;
                            for i in 0..bp {
                                if fglyph.bitmap[i] == 1 {
                                    let px = &pixels[i];
                                    sum_ones_r += px.r;
                                    sum_ones_g += px.g;
                                    sum_ones_b += px.b;
                                    sum_ones_sq += px.sq;
                                    count_ones += 1;
                                }
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

                            let (target_bg_norm, penalty_norm) = (avg_zeros, 0);
                            let (target_bg_inv, penalty_inv) = (avg_ones, 0);

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
                                * (target_bg_norm[0] as u64 * sum_zeros_r
                                    + target_bg_norm[1] as u64 * sum_zeros_g
                                    + target_bg_norm[2] as u64 * sum_zeros_b);
                            let t3_0 = count_zeros
                                * (target_bg_norm[0] as u64 * target_bg_norm[0] as u64
                                    + target_bg_norm[1] as u64 * target_bg_norm[1] as u64
                                    + target_bg_norm[2] as u64 * target_bg_norm[2] as u64);
                            let c0 = (t1_0 + t3_0).saturating_sub(t2_0);

                            let cost_normal = c1 + c0 + penalty_norm;
                            if cost_normal < scale_best_cost {
                                scale_best_cost = cost_normal;
                                scale_best_glyph = gi;
                                scale_best_inv = false;
                                scale_best_fg = avg_ones;
                                scale_best_bg = target_bg_norm;
                            }

                            let t1_0 = sum_zeros_sq;
                            let t2_0 = 2
                                * (avg_zeros[0] as u64 * sum_zeros_r
                                    + avg_zeros[1] as u64 * sum_zeros_g
                                    + avg_zeros[2] as u64 * sum_zeros_b);
                            let t3_0 = count_zeros
                                * (avg_zeros[0] as u64 * avg_zeros[0] as u64
                                    + avg_zeros[1] as u64 * avg_zeros[1] as u64
                                    + avg_zeros[2] as u64 * avg_zeros[2] as u64);
                            let c_fg = (t1_0 + t3_0).saturating_sub(t2_0);

                            let t1_1 = sum_ones_sq;
                            let t2_1 = 2
                                * (target_bg_inv[0] as u64 * sum_ones_r
                                    + target_bg_inv[1] as u64 * sum_ones_g
                                    + target_bg_inv[2] as u64 * sum_ones_b);
                            let t3_1 = count_ones
                                * (target_bg_inv[0] as u64 * target_bg_inv[0] as u64
                                    + target_bg_inv[1] as u64 * target_bg_inv[1] as u64
                                    + target_bg_inv[2] as u64 * target_bg_inv[2] as u64);
                            let c_bg = (t1_1 + t3_1).saturating_sub(t2_1);

                            let cost_inverted = c_fg + c_bg + penalty_inv;
                            if cost_inverted < scale_best_cost {
                                scale_best_cost = cost_inverted;
                                scale_best_glyph = gi;
                                scale_best_inv = true;
                                scale_best_fg = avg_zeros;
                                scale_best_bg = target_bg_inv;
                            }
                        }

                        let gain = uniform_cost.saturating_sub(scale_best_cost);
                        if args.smooth && gain < noise_gain_threshold {
                            let flat_col = [avg_r as u8, avg_g as u8, avg_b as u8];
                            scale_best_fg = flat_col;
                            scale_best_bg = flat_col;
                        }
                    }

                    if scale_best_cost < best_overall_cost {
                        best_overall_cost = scale_best_cost;
                        best_overall_glyph = scale_best_glyph;
                        best_overall_inv = scale_best_inv;
                        best_overall_fg = scale_best_fg;
                        best_overall_bg = scale_best_bg;
                        best_block_pixels = pixels.to_vec();
                    }
                }

                let best_fg_rgb = best_overall_fg;
                let best_bg_rgb = best_overall_bg;
                let best_glyph_idx = best_overall_glyph;
                let best_inv = best_overall_inv;
                let pixels = best_block_pixels;

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
                    } else if render == Render::Preview {
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
                            Some(Colour::RGB(hi_rgb)),
                            Some(Colour::RGB(lo_rgb)),
                            Some(Colour::Index(hi_idx)),
                            Some(Colour::Index(lo_idx)),
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

                let contrast = (final_fg[0] as i32 - final_bg[0] as i32).pow(2)
                    + (final_fg[1] as i32 - final_bg[1] as i32).pow(2)
                    + (final_fg[2] as i32 - final_bg[2] as i32).pow(2);
                let is_flat = contrast < 400;

                row_data.push(BlockData {
                    pixels,
                    fg: final_fg,
                    bg: final_bg,
                    fg_col: fg_obj,
                    bg_col: bg_obj,
                    alt_fg_col: alt_fg_obj,
                    alt_bg_col: alt_bg_obj,
                    is_flat,
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

    // --- Phase 1.5: Color Smoothing (Fix Aliasing Jitter) ---
    if args.smooth {
        if let Some(flag) = &abort_flag {
            if flag.load(std::sync::atomic::Ordering::Relaxed) {
                return "Cancelled".into();
            }
        }
        let smoothing_threshold_sq = 40u64 * 40u64;
        let dist_sq_local = |a: [u8; 3], b: [u8; 3]| -> u64 {
            let dr = a[0] as i64 - b[0] as i64;
            let dg = a[1] as i64 - b[1] as i64;
            let db = a[2] as i64 - b[2] as i64;
            (dr * dr + dg * dg + db * db) as u64
        };

        for y in 0..grid_h {
            for x in 0..grid_w {
                let idx = y * grid_w + x;
                let mut new_fg = grid_data[idx].fg;
                let mut new_bg = grid_data[idx].bg;
                let mut changed = false;

                if x > 0 {
                    let left_idx = idx - 1;
                    if dist_sq_local(new_fg, grid_data[left_idx].fg) < smoothing_threshold_sq {
                        new_fg = [
                            ((new_fg[0] as u16 + grid_data[left_idx].fg[0] as u16) / 2) as u8,
                            ((new_fg[1] as u16 + grid_data[left_idx].fg[1] as u16) / 2) as u8,
                            ((new_fg[2] as u16 + grid_data[left_idx].fg[2] as u16) / 2) as u8,
                        ];
                        changed = true;
                    }
                    if dist_sq_local(new_bg, grid_data[left_idx].bg) < smoothing_threshold_sq {
                        new_bg = [
                            ((new_bg[0] as u16 + grid_data[left_idx].bg[0] as u16) / 2) as u8,
                            ((new_bg[1] as u16 + grid_data[left_idx].bg[1] as u16) / 2) as u8,
                            ((new_bg[2] as u16 + grid_data[left_idx].bg[2] as u16) / 2) as u8,
                        ];
                        changed = true;
                    }
                }
                if y > 0 {
                    let top_idx = idx - grid_w;
                    if dist_sq_local(new_fg, grid_data[top_idx].fg) < smoothing_threshold_sq {
                        new_fg = [
                            ((new_fg[0] as u16 + grid_data[top_idx].fg[0] as u16) / 2) as u8,
                            ((new_fg[1] as u16 + grid_data[top_idx].fg[1] as u16) / 2) as u8,
                            ((new_fg[2] as u16 + grid_data[top_idx].fg[2] as u16) / 2) as u8,
                        ];
                        changed = true;
                    }
                    if dist_sq_local(new_bg, grid_data[top_idx].bg) < smoothing_threshold_sq {
                        new_bg = [
                            ((new_bg[0] as u16 + grid_data[top_idx].bg[0] as u16) / 2) as u8,
                            ((new_bg[1] as u16 + grid_data[top_idx].bg[1] as u16) / 2) as u8,
                            ((new_bg[2] as u16 + grid_data[top_idx].bg[2] as u16) / 2) as u8,
                        ];
                        changed = true;
                    }
                }

                if changed {
                    grid_data[idx].fg = new_fg;
                    grid_data[idx].bg = new_bg;
                    if render == Render::Ansi24 || render == Render::Preview {
                        grid_data[idx].fg_col = Some(Colour::RGB(new_fg));
                        grid_data[idx].bg_col = Some(Colour::RGB(new_bg));
                    }
                }
            }
        }
    }

    // --- Phase 3: Iterative Smoothing (Gauss-Seidel) ---
    if args.smooth {
        let iterations = 10;

        for pass in 0..iterations {
            if let Some(flag) = &abort_flag {
                if flag.load(std::sync::atomic::Ordering::Relaxed) {
                    return "Cancelled".into();
                }
            }
            let forward = pass % 2 == 0;
            let indices: Vec<usize> = if forward {
                (0..grid_h * grid_w).collect()
            } else {
                (0..grid_h * grid_w).rev().collect()
            };

            for &curr_idx in &indices {
                let cx = curr_idx % grid_w;
                let cy = curr_idx / grid_w;
                let data = &grid_data[curr_idx];

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
                let mut d_block_sum_r = 0u64;
                let mut d_block_sum_g = 0u64;
                let mut d_block_sum_b = 0u64;
                let mut d_block_sum_sq = 0u64;
                for p in &data.pixels {
                    d_block_sum_r += p.r;
                    d_block_sum_g += p.g;
                    d_block_sum_b += p.b;
                    d_block_sum_sq += p.sq;
                }

                let edge_importance_weight = 4;
                let dist_sq = |a: [u8; 3], b: [u8; 3]| -> u64 {
                    let dr = a[0] as i64 - b[0] as i64;
                    let dg = a[1] as i64 - b[1] as i64;
                    let db = a[2] as i64 - b[2] as i64;
                    (dr * dr + dg * dg + db * db) as u64
                };

                let mut best_cost = u64::MAX;
                let mut best_choice = grid_state[curr_idx];

                for (gi, fglyph) in glyphs.iter().enumerate() {
                    for inv in [false, true] {
                        let (my_fg, my_bg) = if inv {
                            (data.bg, data.fg)
                        } else {
                            (data.fg, data.bg)
                        };
                        let mut current_edge_error = 0u64;

                        if cy > 0 {
                            let n_idx = curr_idx - grid_w;
                            let (ni, niv) = grid_state[n_idx];
                            for i in 0..gw {
                                let n_on = (glyphs[ni].bottom[i] ^ (niv as u8)) == 1;
                                let n_col = if n_on == niv {
                                    grid_data[n_idx].bg
                                } else {
                                    grid_data[n_idx].fg
                                };
                                let my_on = (fglyph.top[i] ^ (inv as u8)) == 1;
                                let my_col = if my_on { my_fg } else { my_bg };
                                current_edge_error += dist_sq(n_col, my_col);
                            }
                        }
                        if cy < grid_h - 1 {
                            let n_idx = curr_idx + grid_w;
                            let (ni, niv) = grid_state[n_idx];
                            for i in 0..gw {
                                let n_on = (glyphs[ni].top[i] ^ (niv as u8)) == 1;
                                let n_col = if n_on == niv {
                                    grid_data[n_idx].bg
                                } else {
                                    grid_data[n_idx].fg
                                };
                                let my_on = (fglyph.bottom[i] ^ (inv as u8)) == 1;
                                let my_col = if my_on { my_fg } else { my_bg };
                                current_edge_error += dist_sq(n_col, my_col);
                            }
                        }
                        if cx > 0 {
                            let n_idx = curr_idx - 1;
                            let (ni, niv) = grid_state[n_idx];
                            for i in 0..gh {
                                let n_on = (glyphs[ni].right[i] ^ (niv as u8)) == 1;
                                let n_col = if n_on == niv {
                                    grid_data[n_idx].bg
                                } else {
                                    grid_data[n_idx].fg
                                };
                                let my_on = (fglyph.left[i] ^ (inv as u8)) == 1;
                                let my_col = if my_on { my_fg } else { my_bg };
                                current_edge_error += dist_sq(n_col, my_col);
                            }
                        }
                        if cx < grid_w - 1 {
                            let n_idx = curr_idx + 1;
                            let (ni, niv) = grid_state[n_idx];
                            for i in 0..gh {
                                let n_on = (glyphs[ni].left[i] ^ (niv as u8)) == 1;
                                let n_col = if n_on == niv {
                                    grid_data[n_idx].bg
                                } else {
                                    grid_data[n_idx].fg
                                };
                                let my_on = (fglyph.right[i] ^ (inv as u8)) == 1;
                                let my_col = if my_on { my_fg } else { my_bg };
                                current_edge_error += dist_sq(n_col, my_col);
                            }
                        }

                        if cy > 0 && cx > 0 {
                            let n_idx = (curr_idx - grid_w) - 1;
                            let (ni, niv) = grid_state[n_idx];
                            let n_on = (glyphs[ni].bottom[gw - 1] ^ (niv as u8)) == 1;
                            let n_col = if n_on == niv {
                                grid_data[n_idx].bg
                            } else {
                                grid_data[n_idx].fg
                            };
                            let my_on = (fglyph.top[0] ^ (inv as u8)) == 1;
                            let my_col = if my_on { my_fg } else { my_bg };
                            current_edge_error += dist_sq(n_col, my_col);
                        }
                        if cy > 0 && cx < grid_w - 1 {
                            let n_idx = (curr_idx - grid_w) + 1;
                            let (ni, niv) = grid_state[n_idx];
                            let n_on = (glyphs[ni].bottom[0] ^ (niv as u8)) == 1;
                            let n_col = if n_on == niv {
                                grid_data[n_idx].bg
                            } else {
                                grid_data[n_idx].fg
                            };
                            let my_on = (fglyph.top[gw - 1] ^ (inv as u8)) == 1;
                            let my_col = if my_on { my_fg } else { my_bg };
                            current_edge_error += dist_sq(n_col, my_col);
                        }
                        if cy < grid_h - 1 && cx > 0 {
                            let n_idx = (curr_idx + grid_w) - 1;
                            let (ni, niv) = grid_state[n_idx];
                            let n_on = (glyphs[ni].top[gw - 1] ^ (niv as u8)) == 1;
                            let n_col = if n_on == niv {
                                grid_data[n_idx].bg
                            } else {
                                grid_data[n_idx].fg
                            };
                            let my_on = (fglyph.bottom[0] ^ (inv as u8)) == 1;
                            let my_col = if my_on { my_fg } else { my_bg };
                            current_edge_error += dist_sq(n_col, my_col);
                        }
                        if cy < grid_h - 1 && cx < grid_w - 1 {
                            let n_idx = (curr_idx + grid_w) + 1;
                            let (ni, niv) = grid_state[n_idx];
                            let n_on = (glyphs[ni].top[0] ^ (niv as u8)) == 1;
                            let n_col = if n_on == niv {
                                grid_data[n_idx].bg
                            } else {
                                grid_data[n_idx].fg
                            };
                            let my_on = (fglyph.bottom[gw - 1] ^ (inv as u8)) == 1;
                            let my_col = if my_on { my_fg } else { my_bg };
                            current_edge_error += dist_sq(n_col, my_col);
                        }

                        let edge_cost = current_edge_error * edge_importance_weight;
                        if edge_cost >= best_cost {
                            continue;
                        }

                        let mut sum_ones_r = 0u64;
                        let mut sum_ones_g = 0u64;
                        let mut sum_ones_b = 0u64;
                        let mut sum_ones_sq = 0u64;
                        let mut count_ones = 0u64;
                        for i in 0..bp {
                            if fglyph.bitmap[i] ^ (inv as u8) == 1 {
                                let px = &data.pixels[i];
                                sum_ones_r += px.r;
                                sum_ones_g += px.g;
                                sum_ones_b += px.b;
                                sum_ones_sq += px.sq;
                                count_ones += 1;
                            }
                        }
                        let count_zeros = bp as u64 - count_ones;
                        let c1 = (sum_ones_sq + count_ones * fg_p.sq).saturating_sub(
                            2 * (fg_p.r * sum_ones_r + fg_p.g * sum_ones_g + fg_p.b * sum_ones_b),
                        );
                        let c0 = (d_block_sum_sq - sum_ones_sq + count_zeros * bg_p.sq)
                            .saturating_sub(
                                2 * (bg_p.r * (d_block_sum_r - sum_ones_r)
                                    + bg_p.g * (d_block_sum_g - sum_ones_g)
                                    + bg_p.b * (d_block_sum_b - sum_ones_b)),
                            );

                        let total = edge_cost + c1 + c0;

                        if total < best_cost {
                            best_cost = total;
                            best_choice = (gi, inv);
                        }
                    }
                }

                grid_state[curr_idx] = best_choice;
            }
        }
    }

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
        let mut alt_last_fg: Option<Colour> = None;
        let mut alt_last_bg: Option<Colour> = None;

        for x in 0..grid_w {
            let idx = y * grid_w + x;
            let data = &grid_data[idx];
            let (gi, inv) = grid_state[idx];
            let glyph = &glyphs[gi];
            let ch = glyph.ch;

            let (draw_fg, draw_bg, draw_fg_raw, draw_bg_raw) = if inv {
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
                if render != Render::Ansi24 {
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
                if render != Render::Ansi24 {
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
            if let Some(bg_c) = &draw_bg {
                if &draw_fg == bg_c {
                    final_char = ' ';
                }
            }

            row_vec.push(RenderedCell {
                char: final_char,
                fg: draw_fg_raw,
                bg: draw_bg_raw,
                inverted: inv,
            });
            emit_colourized(
                &mut out,
                render,
                draw_fg.clone(),
                draw_bg.clone(),
                final_char,
                &mut first,
                &mut last_fg,
                &mut last_bg,
            );

            if render == Render::Preview {
                let (alt_fg, alt_bg) = if inv {
                    (
                        data.alt_bg_col.clone().unwrap_or(Colour::Index(0)),
                        data.alt_fg_col.clone(),
                    )
                } else {
                    (
                        data.alt_fg_col.clone().unwrap_or(Colour::Index(0)),
                        data.alt_bg_col.clone(),
                    )
                };
                emit_colourized(
                    &mut alt_out,
                    Render::Irc,
                    alt_fg,
                    alt_bg,
                    final_char,
                    &mut alt_first,
                    &mut alt_last_fg,
                    &mut alt_last_bg,
                );
            }
        }
        let end_seq = match render {
            Render::Ansi | Render::Ansi24 | Render::Preview => "\x1b[0m\n",
            Render::Irc => "\x0f\n",
        };
        out.push_str(end_seq);
        if render == Render::Preview {
            alt_out.push_str("\x0f\n");
        }
        grid_out.push(row_vec);
    }
    RenderResult {
        content: out.trim_end_matches('\n').into(),
        save_content: if render == Render::Preview {
            Some(alt_out.trim_end_matches('\n').into())
        } else {
            None
        },
        error_count: total_error,
        total_pixels: (bp * grid_h * grid_w) as u64,
        dynamic_scaling_count: 0,
        grid: grid_out,
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
        let mut alt_last_fg = None;
        let mut alt_last_bg = None;
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

                        if render == Render::Preview {
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
            let fg = counts
                .into_iter()
                .max_by_key(|&(_, count)| count)
                .map(|(c, _)| c)
                .unwrap_or(match render {
                    Render::Ansi | Render::Irc => Colour::Index(0),
                    Render::Ansi24 | Render::Preview => Colour::RGB([0, 0, 0]),
                });
            emit_colourized(
                &mut out,
                render,
                fg,
                None,
                std::char::from_u32(braille).unwrap_or(' '),
                &mut first,
                &mut last_fg,
                &mut last_bg,
            );

            if render == Render::Preview {
                let alt_fg = alt_counts
                    .into_iter()
                    .max_by_key(|&(_, count)| count)
                    .map(|(c, _)| c)
                    .unwrap_or(Colour::Index(0));
                emit_colourized(
                    &mut alt_out,
                    Render::Irc,
                    alt_fg,
                    None,
                    std::char::from_u32(braille).unwrap_or(' '),
                    &mut alt_first,
                    &mut alt_last_fg,
                    &mut alt_last_bg,
                );
            }
        }
        let end_seq = match render {
            Render::Ansi | Render::Ansi24 | Render::Preview => "\x1b[0m\n",
            Render::Irc => "\x0f\n",
        };
        out.push_str(end_seq);
        if render == Render::Preview {
            alt_out.push_str("\x0f\n");
        }
    }
    RenderResult {
        content: out.trim_end_matches('\n').to_string(),
        save_content: if render == Render::Preview {
            Some(alt_out.trim_end_matches('\n').into())
        } else {
            None
        },
        error_count: 0,
        total_pixels: (h * w) as u64,
        dynamic_scaling_count: 0,
        grid: Vec::new(),
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
        Render::Preview => {
            let idx = if args.nograyscale && distinct {
                pixel.irc_ng
            } else {
                pixel.irc_std
            };
            Colour::RGB(unpack_rgb(IRC99[idx as usize]))
        }
    }
}

pub fn calculate_smoothness(
    result: &RenderResult,
    glyphs: &[FastGlyph],
    _args: &RenderArgs,
) -> f64 {
    if result.grid.is_empty() {
        return 0.0;
    }
    let (rows, cols) = (result.grid.len(), result.grid[0].len());
    let gh = glyphs[0].left.len();
    let gw = glyphs[0].top.len();
    let (width, height) = (cols * gw, rows * gh);
    let mut gray = vec![0.0f64; width * height];
    let char_map: HashMap<char, &FastGlyph> = glyphs.iter().map(|g| (g.ch, g)).collect();
    let mut conn_mismatch = 0.0;
    for r in 0..rows {
        for c in 0..cols {
            let cell = &result.grid[r][c];
            if let Some(g) = char_map.get(&cell.char) {
                for gy in 0..gh {
                    for gx in 0..gw {
                        if (g.bitmap[gy * gw + gx] ^ (cell.inverted as u8)) == 1 {
                            gray[(r * gh + gy) * width + (c * gw + gx)] = 255.0;
                        }
                    }
                }
                if r > 0 {
                    if let Some(tg) = char_map.get(&result.grid[r - 1][c].char) {
                        let mut diff = 0;
                        for i in 0..gw {
                            let my_v = g.top[i] ^ (cell.inverted as u8);
                            let n_v = tg.bottom[i] ^ (result.grid[r - 1][c].inverted as u8);
                            if my_v != n_v {
                                diff += 1;
                            }
                        }
                        conn_mismatch += diff as f64;
                    }
                }
                if c > 0 {
                    if let Some(lg) = char_map.get(&result.grid[r][c - 1].char) {
                        let mut diff = 0;
                        for i in 0..gh {
                            let my_v = g.left[i] ^ (cell.inverted as u8);
                            let n_v = lg.right[i] ^ (result.grid[r][c - 1].inverted as u8);
                            if my_v != n_v {
                                diff += 1;
                            }
                        }
                        conn_mismatch += diff as f64;
                    }
                }
            }
        }
    }
    let mut smooth_gray = gray.clone();
    let mut temp_gray = smooth_gray.clone();
    for _ in 0..8 {
        for y in 1..height - 1 {
            for x in 1..width - 1 {
                let i = y * width + x;
                temp_gray[i] = (smooth_gray[i - width - 1]
                    + 2.0 * smooth_gray[i - width]
                    + smooth_gray[i - width + 1]
                    + 2.0 * smooth_gray[i - 1]
                    + 4.0 * smooth_gray[i]
                    + 2.0 * smooth_gray[i + 1]
                    + smooth_gray[i + width - 1]
                    + 2.0 * smooth_gray[i + width]
                    + smooth_gray[i + width + 1])
                    / 16.0;
            }
        }
        smooth_gray.copy_from_slice(&temp_gray);
    }
    let mut gradx = vec![0.0; width * height];
    let mut grady = vec![0.0; width * height];
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let (w0, w1, w2) = ((y - 1) * width, y * width, (y + 1) * width);
            gradx[w1 + x] = -1.0 * smooth_gray[w0 + x - 1] + 1.0 * smooth_gray[w0 + x + 1]
                - 2.0 * smooth_gray[w1 + x - 1]
                + 2.0 * smooth_gray[w1 + x + 1]
                - 1.0 * smooth_gray[w2 + x - 1]
                + 1.0 * smooth_gray[w2 + x + 1];
            grady[w1 + x] = -1.0 * smooth_gray[w0 + x - 1]
                - 2.0 * smooth_gray[w0 + x]
                - 1.0 * smooth_gray[w0 + x + 1]
                + 1.0 * smooth_gray[w2 + x - 1]
                + 2.0 * smooth_gray[w2 + x]
                + 1.0 * smooth_gray[w2 + x + 1];
        }
    }
    let mut dir = vec![0.0; width * height];
    let mut mag = vec![0.0; width * height];
    let mut total_mag = 0.0;
    for i in 0..width * height {
        let m = (gradx[i] * gradx[i] + grady[i] * grady[i]).sqrt();
        if m > 0.001 {
            dir[i] = grady[i].atan2(gradx[i]);
            mag[i] = m;
            total_mag += m;
        }
    }
    let mut wiggle = 0.0;
    if total_mag > 0.0 {
        let diff = |a: f64, b: f64| {
            let mut d = a - b;
            while d > F64_PI {
                d -= 2.0 * F64_PI;
            }
            while d < -F64_PI {
                d += 2.0 * F64_PI;
            }
            d
        };
        for y in 1..height - 1 {
            for x in 1..width - 1 {
                let i = y * width + x;
                if mag[i] > 0.0 {
                    let dx = diff(dir[i + 1], dir[i]);
                    let dy = diff(dir[i + width], dir[i]);
                    wiggle += mag[i] * (dx * dx + dy * dy);
                }
            }
        }
    }

    // Normalize connectivity score by total boundary length, not area.
    // Boundary length scales linearly with resolution (L), Mismatch count scales linearly (L).
    // Ratio is L/L = Dimensionless.
    let total_boundary_pixels =
        (rows.saturating_sub(1) * cols * gw) + (rows * cols.saturating_sub(1) * gh);
    let conn_score = if total_boundary_pixels > 0 {
        conn_mismatch * 100.0 / total_boundary_pixels as f64
    } else {
        0.0
    };

    // Normalize wiggle score.
    // Wiggle sum ~ Constant (corners). Total Mag ~ Linear (L).
    // Wiggle/Mag ~ 1/L.
    // To make it invariant, multiply by characteristic length (L).
    let scale_factor = (gw + gh) as f64 / 2.0;
    let wiggle_score = if total_mag > 0.0 {
        (wiggle * 100.0 / total_mag) * scale_factor
    } else {
        0.0
    };

    conn_score + wiggle_score
}

pub fn save_as_png(
    result: &RenderResult,
    glyphs: &GlyphStore,
    path: &str,
    _metadata: &str,
) -> std::io::Result<()> {
    if result.grid.is_empty() {
        return Ok(());
    }
    let (gw, gh) = glyphs.metrics;
    let (rows, cols) = (result.grid.len(), result.grid[0].len());
    let mut img = image::ImageBuffer::new((cols * gw) as u32, (rows * gh) as u32);
    for (r, row) in result.grid.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            let (xb, yb) = (c * gw, r * gh);
            if let Some(bmp) = glyphs
                .glyphs
                .iter()
                .find(|(ch, _)| *ch == cell.char)
                .map(|(_, b)| b)
            {
                for y in 0..gh {
                    for x in 0..gw {
                        let on = if y < bmp.len() && x < bmp[0].len() {
                            bmp[y][x] == 1
                        } else {
                            false
                        };
                        img.put_pixel(
                            (xb + x) as u32,
                            (yb + y) as u32,
                            Rgb(if on {
                                cell.fg
                            } else {
                                cell.bg.unwrap_or([0, 0, 0])
                            }),
                        );
                    }
                }
            }
        }
    }
    img.save(path)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
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
