use crate::args::{Render, RenderArgs};
use crate::font::GlyphStore;
use crate::palette::{ANSI256, IRC99};
use once_cell::sync::Lazy;
use photon_rs::PhotonImage;
use rayon::prelude::*;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Mutex;

#[derive(Clone)]
pub struct FastGlyph {
    pub ch: char,
    pub bitmap: Vec<u8>,
    pub ones: Vec<usize>,
    pub zeros: Vec<usize>,
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
            .filter(|(ch, _)| glyphs.selected.contains(ch))
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
    fg: Colour,
    bg: Option<Colour>,
    glyph: char,
    first: &mut bool,
    last_fg: &mut Option<Colour>,
    last_bg: &mut Option<Colour>,
    lookahead_fg: Option<&Colour>,
) {
    let is_space = glyph == ' ' || glyph == '\u{2800}';

    // Determine what actually changed
    let fg_changed = *first || last_fg.as_ref() != Some(&fg);
    let bg_changed = *first || last_bg.as_ref() != bg.as_ref();

    // For spaces, FG is invisible (no glyph to color), so we never need to emit FG for them.
    // We also don't update last_fg for spaces, so when a real glyph appears next,
    // we compare against the most recent *visible* FG.
    let need_fg = fg_changed && !is_space;

    // BG always matters (it fills the cell), so we need it when changed
    let need_bg = bg_changed;

    if need_fg {
        *last_fg = Some(fg.clone());
    }
    if need_bg {
        *last_bg = bg.clone();
    }

    if need_fg || need_bg {
        match render {
            Render::Ansi | Render::Ansi24 | Render::Preview => {
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
                    _ => out.push_str("1"),
                }

                if let Some(bg_c) = &bg {
                    out.push(',');
                    match bg_c {
                        Colour::Index(i) => write!(out, "{}", i).unwrap(),
                        _ => out.push_str("0"),
                    }
                }

                // Track exactly what we emitted to the terminal so next cell diffs against it
                // (e.g. if we used lookahead, last_fg is now whatever the lookahead was)
                *last_fg = Some(effective_fg.clone());
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
        fg: [u8; 3],            // High Color
        bg: [u8; 3],            // Low Color
        fg_col: Option<Colour>, // Cached Colour enum
        bg_col: Option<Colour>,
        alt_fg_col: Option<Colour>, // For Preview mode: IRC version
        alt_bg_col: Option<Colour>,
    }

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

                    let mut best_overall_cost = u64::MAX;
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

                        let mut p_r = [0u64; 256];
                        let mut p_g = [0u64; 256];
                        let mut p_b = [0u64; 256];
                        let mut p_sq = [0u64; 256];

                        for (i, p) in pixels.iter().enumerate().take(256) {
                            block_sum_r += p.r;
                            block_sum_g += p.g;
                            block_sum_b += p.b;
                            block_sum_sq += p.sq;
                            p_r[i] = p.r;
                            p_g[i] = p.g;
                            p_b[i] = p.b;
                            p_sq[i] = p.sq;
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
                                let rgb_u32 =
                                    ((p.r as u32) << 16) | ((p.g as u32) << 8) | p.b as u32;
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
                                let invert_opts: &[bool] = if args.no_glyph_invert {
                                    &[false]
                                } else {
                                    &[false, true]
                                };
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
                                    let (fgi_raw, fx) = fg_cnt
                                        .iter()
                                        .enumerate()
                                        .max_by_key(|&(_, c)| c)
                                        .unwrap_or((0, &0));
                                    let (bgi_raw, bx) = bg_cnt
                                        .iter()
                                        .enumerate()
                                        .max_by_key(|&(_, c)| c)
                                        .unwrap_or((0, &0));

                                    // Fix: when a partition is empty, its argmax is
                                    // meaningless. Inherit the other partition's index
                                    // so that uniform cells get fg==bg correctly.
                                    let fgi = if fg_len == 0 { bgi_raw } else { fgi_raw };
                                    let bgi = if bg_len == 0 { fgi_raw } else { bgi_raw };

                                    let cost = ((fg_len as u64) - (*fx as u64))
                                        + ((bg_len as u64) - (*bx as u64));

                                    if cost < scale_best_cost {
                                        scale_best_cost = cost;
                                        scale_best_glyph = gi;
                                        scale_best_inv = inv;
                                        scale_best_fg =
                                            unpack_rgb(palette[fgi.min(palette.len() - 1)]);
                                        scale_best_bg =
                                            unpack_rgb(palette[bgi.min(palette.len() - 1)]);
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
                            let uniform_cost = (block_sum_sq + (bp as u64 * term_sq))
                                .saturating_sub(2 * term_linear);
                            let noise_gain_threshold = 800u64;

                            for (gi, fglyph) in glyphs.iter().enumerate() {
                                let mut sum_ones_r = 0u64;
                                let mut sum_ones_g = 0u64;
                                let mut sum_ones_b = 0u64;
                                let mut sum_ones_sq = 0u64;
                                let count_ones = fglyph.ones.len() as u64;

                                for &i in &fglyph.ones {
                                    // SAFETY: fglyph.ones contains valid indices < bp
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
                                if cost_normal < scale_best_cost {
                                    scale_best_cost = cost_normal;
                                    scale_best_glyph = gi;
                                    scale_best_inv = false;
                                    scale_best_fg = avg_ones;
                                    scale_best_bg = avg_zeros;
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
                        }
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

    // // --- Phase 1.5: Color Smoothing (Fix Aliasing Jitter) ---
    if args.smooth {
        if let Some(flag) = &abort_flag {
            if flag.load(std::sync::atomic::Ordering::Relaxed) {
                return "Cancelled".into();
            }
        }
        let smoothing_threshold_sq = args.misc1 as u64;
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

    // --- Phase 2: Precompute internal block-glyph costs ---
    let block_glyph_costs: Vec<Vec<u64>> = if args.smooth || args.fix_disc {
        grid_data
            .par_iter()
            .enumerate()
            .map(|(curr_idx, data)| {
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
                // We could use static sizing since bp is small, but Vec is fine too.
                let mut diff = vec![0i64; bp];

                for i in 0..bp {
                    let lx = i % gw;
                    let ly = i / gw;
                    let rgb = unpack_rgb(blk.pixels[ly][lx].orig);
                    let p_r = rgb[0] as u64;
                    let p_g = rgb[1] as u64;
                    let p_b = rgb[2] as u64;
                    let p_sq = p_r * p_r + p_g * p_g + p_b * p_b;

                    // Binary classification: is this pixel closer to FG or BG?
                    let c_fg = p_sq + fg_p.sq - 2 * (p_r * fg_p.r + p_g * fg_p.g + p_b * fg_p.b);
                    let c_bg = p_sq + bg_p.sq - 2 * (p_r * bg_p.r + p_g * bg_p.g + p_b * bg_p.b);
                    // 0 if pixel is closer to BG (correct assignment), 1 if closer to FG (mismatch)
                    let bg_wins = if c_bg <= c_fg { 0i64 } else { 1i64 };
                    base_cost += bg_wins as u64;
                    // diff[i] = cost_if_fg - cost_if_bg: negative means FG is better
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
                costs
            })
            .collect()
    } else {
        Vec::new()
    };

    if args.smooth || args.fix_disc {
        return "Cancelled".into();
    }

    // --- Phase 3: Iterative Smoothing (Gauss-Seidel) ---
    if args.smooth {
        let iterations = args.misc3 as usize;
        log::debug!("Starting Phase 3 with {} iterations", iterations);

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
                if curr_idx % 128 == 0 {
                    if let Some(flag) = &abort_flag {
                        if flag.load(std::sync::atomic::Ordering::Relaxed) {
                            return "Cancelled".into();
                        }
                    }
                }

                let cx = curr_idx % grid_w;
                let cy = curr_idx / grid_w;
                let data = &grid_data[curr_idx];

                let edge_importance_weight = args.misc2 as u64;
                // Binary match/mismatch: cost is 0 (colors match) or 1 (colors differ)
                let dist_bin = |a: [u8; 3], b: [u8; 3]| -> u64 {
                    if a == b {
                        0
                    } else {
                        1
                    }
                };

                let mut top_diff = [0i64; 256];
                let mut bottom_diff = [0i64; 256];
                let mut left_diff = [0i64; 256];
                let mut right_diff = [0i64; 256];
                let mut corner_diff = [0i64; 4];
                let mut edge_base_cost = 0u64;

                if cy > 0 {
                    let n_idx = curr_idx - grid_w;
                    let (ni, niv) = grid_state[n_idx];
                    for i in 0..gw {
                        let n_on = (glyphs[ni].bottom[i] ^ (niv as u8)) == 1;
                        let n_col = if n_on {
                            grid_data[n_idx].fg
                        } else {
                            grid_data[n_idx].bg
                        };
                        let c_fg = dist_bin(n_col, data.fg);
                        let c_bg = dist_bin(n_col, data.bg);
                        edge_base_cost += c_bg;
                        top_diff[i] = c_fg as i64 - c_bg as i64;
                    }
                }
                if cy < grid_h - 1 {
                    let n_idx = curr_idx + grid_w;
                    let (ni, niv) = grid_state[n_idx];
                    for i in 0..gw {
                        let n_on = (glyphs[ni].top[i] ^ (niv as u8)) == 1;
                        let n_col = if n_on {
                            grid_data[n_idx].fg
                        } else {
                            grid_data[n_idx].bg
                        };
                        let c_fg = dist_bin(n_col, data.fg);
                        let c_bg = dist_bin(n_col, data.bg);
                        edge_base_cost += c_bg;
                        bottom_diff[i] = c_fg as i64 - c_bg as i64;
                    }
                }
                if cx > 0 {
                    let n_idx = curr_idx - 1;
                    let (ni, niv) = grid_state[n_idx];
                    for i in 0..gh {
                        let n_on = (glyphs[ni].right[i] ^ (niv as u8)) == 1;
                        let n_col = if n_on {
                            grid_data[n_idx].fg
                        } else {
                            grid_data[n_idx].bg
                        };
                        let c_fg = dist_bin(n_col, data.fg);
                        let c_bg = dist_bin(n_col, data.bg);
                        edge_base_cost += c_bg;
                        left_diff[i] = c_fg as i64 - c_bg as i64;
                    }
                }
                if cx < grid_w - 1 {
                    let n_idx = curr_idx + 1;
                    let (ni, niv) = grid_state[n_idx];
                    for i in 0..gh {
                        let n_on = (glyphs[ni].left[i] ^ (niv as u8)) == 1;
                        let n_col = if n_on {
                            grid_data[n_idx].fg
                        } else {
                            grid_data[n_idx].bg
                        };
                        let c_fg = dist_bin(n_col, data.fg);
                        let c_bg = dist_bin(n_col, data.bg);
                        edge_base_cost += c_bg;
                        right_diff[i] = c_fg as i64 - c_bg as i64;
                    }
                }

                if cy > 0 && cx > 0 {
                    let n_idx = (curr_idx - grid_w) - 1;
                    let (ni, niv) = grid_state[n_idx];
                    let n_on = (glyphs[ni].bottom[gw - 1] ^ (niv as u8)) == 1;
                    let n_col = if n_on {
                        grid_data[n_idx].fg
                    } else {
                        grid_data[n_idx].bg
                    };
                    let c_fg = dist_bin(n_col, data.fg);
                    let c_bg = dist_bin(n_col, data.bg);
                    edge_base_cost += c_bg;
                    corner_diff[0] = c_fg as i64 - c_bg as i64;
                }
                if cy > 0 && cx < grid_w - 1 {
                    let n_idx = (curr_idx - grid_w) + 1;
                    let (ni, niv) = grid_state[n_idx];
                    let n_on = (glyphs[ni].bottom[0] ^ (niv as u8)) == 1;
                    let n_col = if n_on {
                        grid_data[n_idx].fg
                    } else {
                        grid_data[n_idx].bg
                    };
                    let c_fg = dist_bin(n_col, data.fg);
                    let c_bg = dist_bin(n_col, data.bg);
                    edge_base_cost += c_bg;
                    corner_diff[1] = c_fg as i64 - c_bg as i64;
                }
                if cy < grid_h - 1 && cx > 0 {
                    let n_idx = (curr_idx + grid_w) - 1;
                    let (ni, niv) = grid_state[n_idx];
                    let n_on = (glyphs[ni].top[gw - 1] ^ (niv as u8)) == 1;
                    let n_col = if n_on {
                        grid_data[n_idx].fg
                    } else {
                        grid_data[n_idx].bg
                    };
                    let c_fg = dist_bin(n_col, data.fg);
                    let c_bg = dist_bin(n_col, data.bg);
                    edge_base_cost += c_bg;
                    corner_diff[2] = c_fg as i64 - c_bg as i64;
                }
                if cy < grid_h - 1 && cx < grid_w - 1 {
                    let n_idx = (curr_idx + grid_w) + 1;
                    let (ni, niv) = grid_state[n_idx];
                    let n_on = (glyphs[ni].top[0] ^ (niv as u8)) == 1;
                    let n_col = if n_on {
                        grid_data[n_idx].fg
                    } else {
                        grid_data[n_idx].bg
                    };
                    let c_fg = dist_bin(n_col, data.fg);
                    let c_bg = dist_bin(n_col, data.bg);
                    edge_base_cost += c_bg;
                    corner_diff[3] = c_fg as i64 - c_bg as i64;
                }

                let total_edge_diff: i64 = top_diff.iter().sum::<i64>()
                    + bottom_diff.iter().sum::<i64>()
                    + left_diff.iter().sum::<i64>()
                    + right_diff.iter().sum::<i64>()
                    + corner_diff.iter().sum::<i64>();

                let mut best_cost = u64::MAX;
                let mut best_choice = grid_state[curr_idx];

                for (gi, fglyph) in glyphs.iter().enumerate() {
                    let mut dot = 0i64;
                    for i in 0..gw {
                        dot += top_diff[i] * (fglyph.top[i] as i64);
                        dot += bottom_diff[i] * (fglyph.bottom[i] as i64);
                    }
                    for i in 0..gh {
                        dot += left_diff[i] * (fglyph.left[i] as i64);
                        dot += right_diff[i] * (fglyph.right[i] as i64);
                    }
                    dot += corner_diff[0] * (fglyph.top[0] as i64);
                    dot += corner_diff[1] * (fglyph.top[gw - 1] as i64);
                    dot += corner_diff[2] * (fglyph.bottom[0] as i64);
                    dot += corner_diff[3] * (fglyph.bottom[gw - 1] as i64);

                    let edge_cost_normal =
                        ((edge_base_cost as i64 + dot) as u64) * edge_importance_weight;
                    let total_normal = edge_cost_normal + block_glyph_costs[curr_idx][gi * 2];
                    if total_normal < best_cost {
                        best_cost = total_normal;
                        best_choice = (gi, false);
                    }

                    if !args.no_glyph_invert {
                        // Inverted glyph flips all edge bits: bitmap[i] → (1 - bitmap[i])
                        // So dot_inv = Σ diff[i] * (1 - bitmap[i]) = total_edge_diff - dot
                        let dot_inv = total_edge_diff - dot;
                        let edge_cost_inverted =
                            ((edge_base_cost as i64 + dot_inv) as u64) * edge_importance_weight;
                        let total_inverted =
                            edge_cost_inverted + block_glyph_costs[curr_idx][gi * 2 + 1];
                        if total_inverted < best_cost {
                            best_cost = total_inverted;
                            best_choice = (gi, true);
                        }
                    }
                }

                grid_state[curr_idx] = best_choice;
            }
        }
    }

    // --- Phase 5: Boundary Continuity Resolution ---
    //
    // Constraint-satisfaction approach: no weights, no knobs.
    //
    // For each cell with fixable discontinuities (same detection as show_disc):
    //   1. Build edge constraints from neighbors. For each boundary pixel,
    //      the neighbor renders a definite color. If that color is in our
    //      palette (FG or BG), we know the exact effective bitmap value
    //      (0=BG, 1=FG) needed to match. That's a hard constraint.
    //   2. Score each candidate glyph by how many constraints it satisfies
    //      ("continuity score"). No weights — just count matches.
    //   3. Pick the glyph with the highest continuity score. Break ties
    //      by internal fidelity (fewer misclassified source pixels).
    //   4. Iterate until convergence (max 10 passes).
    //
    // Only parameter: fix_strength (same threshold show_disc uses).
    if args.fix_disc {
        let disc_threshold = args.fix_strength as usize;

        for pass in 0..10 {
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
                let my_fg = grid_data[curr_idx].fg;
                let my_bg = grid_data[curr_idx].bg;

                // Skip uniform cells (FG == BG: glyph is invisible)
                if my_fg == my_bg {
                    continue;
                }

                // Build edge constraints and count fixable mismatches.
                //
                // For each boundary pixel, if the neighbor renders a color
                // that's in our palette, we set a constraint:
                //   want=1 means "effective FG here" (bitmap ^ inv == 1)
                //   want=0 means "effective BG here" (bitmap ^ inv == 0)
                // 255 = unconstrained (neighbor's color not in our palette)
                const UNCONSTRAINED: u8 = 255;
                let mut top_want = [UNCONSTRAINED; 256];
                let mut bottom_want = [UNCONSTRAINED; 256];
                let mut left_want = [UNCONSTRAINED; 256];
                let mut right_want = [UNCONSTRAINED; 256];
                let mut fixable_count = 0usize;

                macro_rules! build_edge {
                    ($cond:expr, $n_idx:expr, $len:expr, $wants:expr,
                     $my_edge_fn:expr, $n_edge_fn:expr) => {
                        if $cond {
                            let n_idx = $n_idx;
                            let (ni, niv) = grid_state[n_idx];
                            let n_fg = grid_data[n_idx].fg;
                            let n_bg = grid_data[n_idx].bg;
                            for i in 0..$len {
                                // What the neighbor renders at this boundary pixel
                                let n_col = if (($n_edge_fn)(ni, i) ^ (niv as u8)) == 1 {
                                    n_fg
                                } else {
                                    n_bg
                                };
                                // What we currently render
                                let my_bmp = ($my_edge_fn)(cur_gi, i);
                                let my_col = if (my_bmp ^ (cur_inv as u8)) == 1 {
                                    my_fg
                                } else {
                                    my_bg
                                };
                                // Count fixable mismatch (same logic as show_disc)
                                if my_col != n_col {
                                    let my_in_n = my_col == n_fg || my_col == n_bg;
                                    let n_in_me = n_col == my_fg || n_col == my_bg;
                                    if my_in_n || n_in_me {
                                        fixable_count += 1;
                                    }
                                }
                                // Set constraint: what effective value do we need
                                // to render the neighbor's color?
                                if n_col == my_fg {
                                    $wants[i] = 1; // need FG
                                } else if n_col == my_bg {
                                    $wants[i] = 0; // need BG
                                }
                                // else: neighbor's color isn't in our palette → unconstrained
                            }
                        }
                    };
                }

                build_edge!(
                    cy > 0,
                    curr_idx - grid_w,
                    gw,
                    top_want,
                    |gi: usize, i: usize| glyphs[gi].top[i],
                    |ni: usize, i: usize| glyphs[ni].bottom[i]
                );
                build_edge!(
                    cy < grid_h - 1,
                    curr_idx + grid_w,
                    gw,
                    bottom_want,
                    |gi: usize, i: usize| glyphs[gi].bottom[i],
                    |ni: usize, i: usize| glyphs[ni].top[i]
                );
                build_edge!(
                    cx > 0,
                    curr_idx - 1,
                    gh,
                    left_want,
                    |gi: usize, i: usize| glyphs[gi].left[i],
                    |ni: usize, i: usize| glyphs[ni].right[i]
                );
                build_edge!(
                    cx < grid_w - 1,
                    curr_idx + 1,
                    gh,
                    right_want,
                    |gi: usize, i: usize| glyphs[gi].right[i],
                    |ni: usize, i: usize| glyphs[ni].left[i]
                );

                // Only repair cells that show_disc would highlight
                if fixable_count <= disc_threshold {
                    continue;
                }

                // Score each candidate glyph by constraint satisfaction.
                // No weights, no cost functions — just count matching edge pixels.
                let mut best_continuity = 0u32;
                let mut best_internal = u64::MAX;
                let mut best_choice = (cur_gi, cur_inv);

                for (gi, glyph) in glyphs.iter().enumerate() {
                    let max_inv = if args.no_glyph_invert { 1 } else { 2 };
                    for inv_idx in 0..max_inv {
                        let inv_u8 = inv_idx as u8;

                        // Count how many constraints this glyph satisfies
                        let mut continuity = 0u32;

                        for i in 0..gw {
                            if top_want[i] != UNCONSTRAINED
                                && (glyph.top[i] ^ inv_u8) == top_want[i]
                            {
                                continuity += 1;
                            }
                            if bottom_want[i] != UNCONSTRAINED
                                && (glyph.bottom[i] ^ inv_u8) == bottom_want[i]
                            {
                                continuity += 1;
                            }
                        }
                        for i in 0..gh {
                            if left_want[i] != UNCONSTRAINED
                                && (glyph.left[i] ^ inv_u8) == left_want[i]
                            {
                                continuity += 1;
                            }
                            if right_want[i] != UNCONSTRAINED
                                && (glyph.right[i] ^ inv_u8) == right_want[i]
                            {
                                continuity += 1;
                            }
                        }

                        // Tiebreaker: internal fidelity (fewer misclassified pixels)
                        let internal = block_glyph_costs
                            .get(curr_idx)
                            .and_then(|v| v.get(gi * 2 + inv_idx).copied())
                            .unwrap_or(u64::MAX);

                        // Primary: max continuity. Secondary: min internal cost.
                        if continuity > best_continuity
                            || (continuity == best_continuity && internal < best_internal)
                        {
                            best_continuity = continuity;
                            best_internal = internal;
                            best_choice = (gi, inv_idx == 1);
                        }
                    }
                }

                if best_choice != (cur_gi, cur_inv) {
                    grid_state[curr_idx] = best_choice;
                    changes += 1;
                }
            }

            log::info!("fix_disc pass {}: {} cells changed", pass, changes);
            if changes == 0 {
                break;
            }
        }
    }

    // --- Discontinuity Highlighting (non-destructive overlay) ---
    // Compares the ACTUAL RENDERED COLOR at each edge pixel pair across cell
    // boundaries. Only counts mismatches where the colors COULD have matched
    // (i.e. the mismatched color exists in the neighbor's FG/BG palette),
    // which excludes natural color transitions and only flags structural breaks.
    let disc_overlay: Vec<Option<([u8; 3], [u8; 3])>> = if args.show_discontinuities {
        let disc_threshold = args.fix_strength as usize;

        // Precompute rendered FG/BG colors for each cell
        let cell_colors: Vec<([u8; 3], [u8; 3])> = grid_data.iter().map(|d| (d.fg, d.bg)).collect();

        (0..grid_h * grid_w)
            .map(|curr_idx| {
                let cx = curr_idx % grid_w;
                let cy = curr_idx / grid_w;
                let (gi, inv) = grid_state[curr_idx];
                let glyph = &glyphs[gi];
                let (my_fg, my_bg) = cell_colors[curr_idx];

                let mut fixable_mismatches = 0usize;

                // Rendered color of a pixel given its bitmap value and inversion
                // (bitmap_val ^ inv) == 1 → FG, else → BG
                let my_pixel_color = |bmp_val: u8| -> [u8; 3] {
                    if (bmp_val ^ (inv as u8)) == 1 {
                        my_fg
                    } else {
                        my_bg
                    }
                };

                macro_rules! check_edge {
                    ($cond:expr, $n_idx:expr, $len:expr, $my_edge:expr, $n_edge_fn:expr) => {
                        if $cond {
                            let n_idx = $n_idx;
                            let (ni, niv) = grid_state[n_idx];
                            let (n_fg, n_bg) = cell_colors[n_idx];
                            for i in 0..$len {
                                let my_col = my_pixel_color($my_edge[i]);
                                let n_bmp = $n_edge_fn(ni, i);
                                let n_col = if (n_bmp ^ (niv as u8)) == 1 {
                                    n_fg
                                } else {
                                    n_bg
                                };

                                if my_col != n_col {
                                    // Only count if this is "fixable": the mismatched color
                                    // exists in the other cell's palette. Natural transitions
                                    // (e.g., white meeting black across a boundary) won't be
                                    // fixable because white isn't in the black cell's palette.
                                    let my_in_n = my_col == n_fg || my_col == n_bg;
                                    let n_in_me = n_col == my_fg || n_col == my_bg;
                                    if my_in_n || n_in_me {
                                        fixable_mismatches += 1;
                                    }
                                }
                            }
                        }
                    };
                }

                check_edge!(
                    cy > 0,
                    curr_idx - grid_w,
                    gw,
                    glyph.top,
                    |ni: usize, i: usize| glyphs[ni].bottom[i]
                );
                check_edge!(
                    cy < grid_h - 1,
                    curr_idx + grid_w,
                    gw,
                    glyph.bottom,
                    |ni: usize, i: usize| glyphs[ni].top[i]
                );
                check_edge!(
                    cx > 0,
                    curr_idx - 1,
                    gh,
                    glyph.left,
                    |ni: usize, i: usize| glyphs[ni].right[i]
                );
                check_edge!(
                    cx < grid_w - 1,
                    curr_idx + 1,
                    gh,
                    glyph.right,
                    |ni: usize, i: usize| glyphs[ni].left[i]
                );

                if fixable_mismatches > disc_threshold {
                    let max_edge = (2 * gw + 2 * gh) as f32;
                    let severity = (fixable_mismatches as f32 / max_edge).min(1.0);
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

    // Precompute cheapest glyph for uniform cells (FG==BG, glyph invisible).
    // Space/braille blank allow skipping the FG color code (~3 byte saving),
    // so they get a bonus. Among remaining glyphs, prefer smallest UTF-8 encoding.
    let cheapest_uniform_glyph: Option<char> = if glyphs.is_empty() {
        None
    } else {
        let mut best_ch = glyphs[0].ch;
        let mut best_cost = usize::MAX;
        for g in glyphs.iter() {
            let is_blank = g.ch == ' ' || g.ch == '\u{2800}';
            // Effective cost: glyph bytes + estimated FG overhead (0 for blanks, ~3 for others)
            let cost = g.ch.len_utf8() + if is_blank { 0 } else { 3 };
            if cost < best_cost {
                best_cost = cost;
                best_ch = g.ch;
            }
        }
        Some(best_ch)
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
        let mut alt_last_fg: Option<Colour> = None;
        let mut alt_last_bg: Option<Colour> = None;

        // Pass 1: Collect processed cell data for the row
        let mut row_cells: Vec<(Colour, Option<Colour>, char)> = Vec::with_capacity(grid_w);
        let mut alt_row_cells: Vec<(Colour, Option<Colour>, char)> = Vec::with_capacity(grid_w);

        for x in 0..grid_w {
            let idx = y * grid_w + x;
            let data = &grid_data[idx];
            let (gi, inv) = grid_state[idx];
            let glyph = &glyphs[gi];
            let ch = glyph.ch;

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
            if overlay.is_none() {
                if let Some(uniform_ch) = cheapest_uniform_glyph {
                    if let Some(bg_c) = &draw_bg {
                        if &draw_fg == bg_c {
                            final_char = uniform_ch;
                        }
                    }
                }
            }

            row_vec.push(RenderedCell {
                char: final_char,
                fg: draw_fg_raw,
                bg: draw_bg_raw,
                inverted: inv,
            });

            row_cells.push((draw_fg.clone(), draw_bg.clone(), final_char));

            if render == Render::Preview {
                let (alt_fg, alt_bg) = if overlay.is_some() {
                    (draw_fg.clone(), draw_bg.clone())
                } else if inv {
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
                // Compute uniform-glyph replacement for the IRC alt output
                // using the IRC99 colors, not the ANSI256-based final_char.
                let mut alt_final_char = ch;
                if overlay.is_none() {
                    if let Some(uniform_ch) = cheapest_uniform_glyph {
                        if let Some(bg_c) = &alt_bg {
                            if &alt_fg == bg_c {
                                alt_final_char = uniform_ch;
                            }
                        }
                    }
                }
                alt_row_cells.push((alt_fg, alt_bg, alt_final_char));
            }
        }

        // Pass 2: Emit with lookahead for FG optimization
        for (i, (fg, bg, ch)) in row_cells.iter().enumerate() {
            // Find next non-space cell's FG for lookahead
            let lookahead = if render == Render::Irc || (render == Render::Preview) {
                row_cells[i + 1..].iter().find_map(|(f, _, c)| {
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
                fg.clone(),
                bg.clone(),
                *ch,
                &mut first,
                &mut last_fg,
                &mut last_bg,
                lookahead,
            );
        }

        if render == Render::Preview {
            for (i, (fg, bg, ch)) in alt_row_cells.iter().enumerate() {
                let lookahead = alt_row_cells[i + 1..].iter().find_map(|(f, _, c)| {
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
                    fg.clone(),
                    bg.clone(),
                    *ch,
                    &mut alt_first,
                    &mut alt_last_fg,
                    &mut alt_last_bg,
                    lookahead,
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
                None,
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
                    None,
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
