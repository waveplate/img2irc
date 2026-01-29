use crate::args::{RenderArgs, Render};
use crate::font::GlyphStore;
use crate::palette::{IRC99, ANSI256};
use photon_rs::PhotonImage;
use image::{ImageBuffer, Rgb};

use std::collections::HashMap;
use std::iter::repeat;
use std::sync::Mutex;
use once_cell::sync::Lazy;

#[derive(Clone)]
pub struct FastGlyph {
    pub ch: char,
    pub bitmap: Vec<u8>,
    pub edge_bitmap: Vec<u8>,
    // Boundary signatures for continuity (smooth mode)
    pub top: Vec<u8>,
    pub bottom: Vec<u8>,
    pub left: Vec<u8>,
    pub right: Vec<u8>,
}

#[derive(Clone, Copy)]
struct PrePixel {
    r: u64, g: u64, b: u64,
    sq: u64,
}

/// Braille dot positions (for render_braille)
const POSITIONS: [(usize, usize, u32); 8] = [
    (0, 0, 0x01), (0, 1, 0x02), (0, 2, 0x04), (1, 0, 0x08),
    (1, 1, 0x10), (1, 2, 0x20), (0, 3, 0x40), (1, 3, 0x80),
];

/// Cache for colour conversions: orig → (ansi_std, irc_std, ansi_ng, irc_ng)
static COLOR_CACHE: Lazy<Mutex<HashMap<u32, (u8, u8, u8, u8)>>> = 
    Lazy::new(|| Mutex::new(HashMap::with_capacity(8_192)));

/// Fast squared-distance search
#[inline(always)]
pub fn nearest_hex_colour_fast(col: u32, palette: &[u32]) -> u8 {
    let pr = ((col >> 16) & 0xFF) as i32;
    let pg = ((col >>  8) & 0xFF) as i32;
    let pb = ((col >>  0) & 0xFF) as i32;

    let mut best_i = 0u8;
    let mut best_d = u32::MAX;
    if palette.is_empty() { return 0; }

    for (i, &p) in palette.iter().enumerate() {
        let dr = pr - (((p >> 16) & 0xFF) as i32);
        let dg = pg - (((p >>  8) & 0xFF) as i32);
        let db = pb - (((p >>  0) & 0xFF) as i32);
        let d  = (dr*dr + dg*dg + db*db) as u32;
        if d < best_d {
            best_d = d;
            best_i = i as u8;
            if d == 0 { break; }
        }
    }
    best_i
}

/// Fast squared-distance search for a distinctly chromatic (not near-grayscale) colour in the palette.
/// Returns None if no such colour is found.
#[inline(always)]
fn nearest_distinctly_chromatic_hex_colour(col: u32, palette: &[u32], tolerance: u8) -> Option<u8> {
    let pr = ((col >> 16) & 0xFF) as i32;
    let pg = ((col >>  8) & 0xFF) as i32;
    let pb = ((col >>  0) & 0xFF) as i32;

    let mut best_i: Option<u8> = None;
    let mut best_d = u32::MAX;

    for (i, &p) in palette.iter().enumerate() {
        if is_near_grayscale(p, tolerance) { continue; }
        let dr = pr - (((p >> 16) & 0xFF) as i32);
        let dg = pg - (((p >>  8) & 0xFF) as i32);
        let db = pb - (((p >>  0) & 0xFF) as i32);
        let d  = (dr*dr + dg*dg + db*db) as u32;
        if d < best_d {
            best_d = d;
            best_i = Some(i as u8);
            if d == 0 { break; }
        }
    }
    best_i
}

/// Pack [R,G,B] → u32
#[inline] 
pub fn make_rgb_u32(px: &[u8]) -> u32 {
    if px.len() < 3 { return 0; }
    ((px[0] as u32) << 16) | ((px[1] as u32) << 8) | (px[2] as u32)
}
/// Unpack u32 → [R,G,B]
#[inline] fn unpack_rgb(rgb: u32) -> [u8;3] {
    [(rgb >> 16) as u8, (rgb >> 8) as u8, (rgb >> 0) as u8]
}

/// True if the colour is "near grayscale" within a given tolerance.
#[inline] fn is_near_grayscale(col: u32, tolerance: u8) -> bool {
    let [r,g,b] = unpack_rgb(col);
    let min_val = r.min(g.min(b));
    let max_val = r.max(g.max(b));
    max_val.saturating_sub(min_val) <= tolerance
}

#[derive(Debug, Clone)]
pub struct AnsiImage {
    pub bitmap: Vec<Vec<u32>>,
    pub block:  Vec<Vec<AnsiPixelBlock>>,
    pub glyphs: GlyphStore,
}

#[derive(Debug, Clone, Copy)]
pub struct AnsiPixel {
    pub orig:     u32,
    pub ansi_std: u8,
    pub irc_std:  u8,
    pub ansi_ng:  u8,
    pub irc_ng:   u8,
}

#[derive(Debug, Clone)]
pub struct AnsiPixelBlock {
    pub pixels: Vec<Vec<AnsiPixel>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Colour { Index(u8), RGB([u8;3]) }

impl AnsiPixel {
    #[inline]
    pub fn new(px: &u32, tolerance: u8) -> Self {
        if let Some(&(a, i, an, in_)) = COLOR_CACHE.lock().unwrap().get(px) {
            return AnsiPixel { orig: *px, ansi_std: a, irc_std: i, ansi_ng: an, irc_ng: in_ };
        }

        let ansi_std_val = nearest_hex_colour_fast(*px, &ANSI256);
        let irc_std_val  = nearest_hex_colour_fast(*px, &IRC99);

        let mut ansi_ng_val = ansi_std_val;
        let mut irc_ng_val  = irc_std_val;

        if !is_near_grayscale(*px, tolerance) {
            if let Some(idx) = nearest_distinctly_chromatic_hex_colour(*px, &ANSI256, tolerance) {
                ansi_ng_val = idx;
            }
            if let Some(idx) = nearest_distinctly_chromatic_hex_colour(*px, &IRC99, tolerance) {
                irc_ng_val = idx;
            }
        }

        COLOR_CACHE.lock().unwrap().insert(*px, (ansi_std_val, irc_std_val, ansi_ng_val, irc_ng_val));
        AnsiPixel { orig: *px, ansi_std: ansi_std_val, irc_std: irc_std_val, ansi_ng: ansi_ng_val, irc_ng: irc_ng_val }
    }
}

impl AnsiImage {
    pub fn new(img: PhotonImage, glyphs: GlyphStore, tolerance: u8) -> Self {
        let w = img.get_width() as usize;
        let raw = img.get_raw_pixels();
        let flat: Vec<u32> = raw.chunks(4).map(make_rgb_u32).collect();
        let mut bitmap: Vec<Vec<u32>> = flat.chunks(w).map(|r| r.to_vec()).collect();

        if bitmap.len() % 2 != 0 { bitmap.push(vec![0; w]); }
        for row in &mut bitmap {
            if row.len() % 2 != 0 { row.push(0); }
        }

        let block = block_bitmap(&bitmap, &glyphs, tolerance);
        AnsiImage { bitmap, block, glyphs }
    }
}

fn block_bitmap(src: &Vec<Vec<u32>>, glyphs: &GlyphStore, tolerance: u8) -> Vec<Vec<AnsiPixelBlock>> {
    if glyphs.glyphs.is_empty() { return Vec::new(); }
    let (gh, gw) = glyphs.metrics;
    if gh == 0 || gw == 0 { return Vec::new(); }

    let mut px_rows: Vec<Vec<AnsiPixel>> = src.iter()
        .map(|row| row.iter().map(|px| AnsiPixel::new(px, tolerance)).collect())
        .collect();

    let ph = ((px_rows.len() + gh - 1) / gh) * gh;
    let pw = if px_rows.is_empty() { 0 } else { ((px_rows[0].len() + gw - 1) / gw) * gw };

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
            row.push(AnsiPixelBlock { pixels: block_pixels });
        }
        out.push(row);
    }
    out
}

fn pick_colour(pixel: &AnsiPixel, r: Render, args: &RenderArgs) -> Colour {
    let distinct = !is_near_grayscale(pixel.orig, args.grayscale_tolerance);
    match r {
        Render::Ansi => {
            let idx = if args.nograyscale && distinct { pixel.ansi_ng } else { pixel.ansi_std };
            Colour::Index(idx)
        }
        Render::Irc => {
            let idx = if args.nograyscale && distinct { pixel.irc_ng } else { pixel.irc_std };
            Colour::Index(idx)
        }
        Render::Ansi24 => Colour::RGB(unpack_rgb(pixel.orig)),
    }
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
    match render {
        Render::Ansi => {
            let fg_idx = if let Colour::Index(i) = fg { i } else { 0 };
            let bg_idx = bg.clone().and_then(|b| if let Colour::Index(i) = b { Some(i) } else { None });
            if *first || Some(fg.clone()) != *last_fg || bg != *last_bg {
                if let Some(b) = bg_idx {
                    out.push_str(&format!("\x1b[38;5;{}m\x1b[48;5;{}m", fg_idx, b));
                } else {
                    out.push_str(&format!("\x1b[38;5;{}m", fg_idx));
                }
                *last_fg = Some(fg.clone());
                *last_bg = bg.clone();
            }
            out.push(glyph);
        }
        Render::Ansi24 => {
            let fg_rgb = match fg {
                Colour::RGB(c) => c,
                Colour::Index(_) => panic!("Ansi24 should not use indexed colours"),
            };
            let bg_rgb = bg.map(|b| match b {
                Colour::RGB(c) => c,
                Colour::Index(_) => panic!("Ansi24 should not use indexed colours"),
            });
            let fg_col = Colour::RGB(fg_rgb);
            let bg_col = bg_rgb.map(Colour::RGB);

            if *first || Some(fg_col.clone()) != *last_fg || bg_col != *last_bg {
                if let Some(bc) = bg_rgb {
                    out.push_str(&format!(
                        "\x1b[38;2;{};{};{}m\x1b[48;2;{};{};{}m",
                        fg_rgb[0], fg_rgb[1], fg_rgb[2],
                        bc[0], bc[1], bc[2]
                    ));
                } else {
                    out.push_str(&format!(
                        "\x1b[38;2;{};{};{}m",
                        fg_rgb[0], fg_rgb[1], fg_rgb[2]
                    ));
                }
                *last_fg = Some(fg_col);
                *last_bg = bg_col;
            }
            out.push(glyph);
        }
        Render::Irc => {
            let fg_idx = if let Colour::Index(i) = fg { i.min(98) } else { 0 }; // IRC99 indices are 0-98
            let bg_idx = bg.clone().and_then(|b| if let Colour::Index(i) = b { Some(i.min(98)) } else { None });
            if *first || Some(fg.clone()) != *last_fg || bg != *last_bg {
                if let Some(b) = bg_idx {
                    out.push_str(&format!("\x03{},{}", fg_idx, b));
                } else {
                    out.push_str(&format!("\x03{}", fg_idx));
                }
                *last_fg = Some(fg.clone());
                *last_bg = bg.clone();
            }
            out.push(glyph);
        }
    }
    *first = false;
}

fn parse_hex_color(s: &str) -> u32 {
    u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or(0)
}

#[derive(Clone, Debug)]
pub struct RenderedCell {
    pub char: char,
    pub fg: [u8; 3],
    pub bg: Option<[u8; 3]>,
}

pub struct RenderResult {
    pub content: String,
    pub error_count: u64,
    pub total_pixels: u64,
    pub dynamic_scaling_count: u64,
    pub grid: Vec<Vec<RenderedCell>>,
}

impl From<&str> for RenderResult {
    fn from(s: &str) -> Self {
        RenderResult { content: s.to_string(), error_count: 0, total_pixels: 0, dynamic_scaling_count: 0, grid: Vec::new() }
    }
}

pub fn prepare_glyphs(glyphs: &GlyphStore, args: &RenderArgs) -> Vec<FastGlyph> {
    if glyphs.glyphs.is_empty() { return Vec::new(); }
    let (gh, gw) = glyphs.metrics;
    let bp = gh * gw;
    
    glyphs.glyphs.iter()
        .filter(|(ch, _)| glyphs.selected.contains(ch))
        .filter(|(ch, _)| {
            if *ch == ' ' && !args.blocks.is_empty() {
                return args.blocks.iter().any(|block| {
                    glyphs.groups.get(&block.to_lowercase())
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
            
            let mut edge_bitmap = vec![0u8; bp];
            if args.smooth.is_some() {
                for y in 0..gh {
                    for x in 0..gw {
                        let i = y * gw + x;
                        let v = bitmap[i];
                        let mut is_edge = false;
                        if x > 0 && bitmap[i - 1] != v { is_edge = true; }
                        else if x + 1 < gw && bitmap[i + 1] != v { is_edge = true; }
                        else if y > 0 && bitmap[i - gw] != v { is_edge = true; }
                        else if y + 1 < gh && bitmap[i + gw] != v { is_edge = true; }
                        
                        if is_edge { edge_bitmap[i] = 1; }
                    }
                }
            }
            
            // Extract boundaries for continuity check
            let mut top = Vec::with_capacity(gw);
            let mut bottom = Vec::with_capacity(gw);
            let mut left = Vec::with_capacity(gh);
            let mut right = Vec::with_capacity(gh);

            // Top and Bottom rows
            for x in 0..gw {
                top.push(bitmap[x]);
                bottom.push(bitmap[(gh - 1) * gw + x]);
            }
            // Left and Right columns
            for y in 0..gh {
                left.push(bitmap[y * gw]);
                right.push(bitmap[y * gw + (gw - 1)]);
            }

            FastGlyph { 
                ch: *ch, 
                bitmap, 
                edge_bitmap,
                top, bottom, left, right 
            }
        })
        .collect()
}

/// Render an image as block‐glyph ANSI/IRC art, using only the glyph groups in `args.blocks`.
pub fn render_blocks(
    image: &AnsiImage,
    args: &RenderArgs,
    render: Render,
    glyphs: &[FastGlyph],
) -> RenderResult {
    if args.smooth.is_some() {
        return render_blocks_smooth(image, args, render, glyphs);
    }

    if glyphs.is_empty() {
        return "Error: no glyphs available".into();
    }

    let bg_opt_rgb = args.bg_opt.as_ref().map(|s| unpack_rgb(parse_hex_color(s)));

    let (gh, gw) = image.glyphs.metrics;
    let bp = gh * gw;

    let mut out = String::new();
    let mut total_error = 0u64;
    let mut total_pixels = 0u64;

    // Pre-allocate buffer for pixel data to avoid repeated allocations
    let mut current_block_pixels_rgb: Vec<PrePixel> = Vec::with_capacity(bp);
    // For indexed modes
    let mut current_block_indices: Vec<u8> = Vec::with_capacity(bp);
    // For smooth mode
    let mut current_block_edges: Vec<u8> = vec![0u8; bp];

    // Track selected glyphs for continuity (smooth mode)
    let blocks_width = if !image.block.is_empty() { image.block[0].len() } else { 0 };
    let mut previous_row_glyph_indices: Vec<(usize, bool)> = vec![(usize::MAX, false); blocks_width];
    let mut dynamic_scaling_count = 0u64;

    // Dynamic Scaling Factors
    let fuzz = args.fuzz.max(0.0);
    let scales = if fuzz > 0.0001 {
        let f = fuzz.max(0.001);
        vec![
            (1.0, 1.0),
            (1.0 - f, 1.0),
            (1.0 + f, 1.0),
            (1.0, 1.0 - f),
            (1.0, 1.0 + f),
        ]
    } else {
        vec![(1.0, 1.0)]
    };

    let mut grid: Vec<Vec<RenderedCell>> = Vec::new();
    let mut working_pixels_buffer = Vec::with_capacity(bp); // Reuse buffer

    for (_row_idx, block_row) in image.block.iter().enumerate() {
        let mut grid_row = Vec::new();
        let mut first = true;
        let mut last_fg: Option<Colour> = None; // Assuming Colour is in parent or use crate::image::Colour
        let mut last_bg: Option<Colour> = None;
        let mut left_neighbor_glyph_idx: (usize, bool) = (usize::MAX, false);

        for (col_idx, blk) in block_row.iter().enumerate() {
            total_pixels += bp as u64;
            let mut best_overall_cost = u64::MAX;
            let mut final_glyph = ' ';
            let mut final_fg = Colour::Index(0);
            let mut final_bg: Option<Colour> = None;
            let mut chosen_glyph_index = 0;
            let mut chosen_glyph_inverted = false;
            let mut best_scale_is_identity = true;
            
            let mut best_match_data = (0usize, false, [0u8;3], [0u8;3], 0u8, 0u8); // (idx, inv, fg_rgb, bg_rgb, fg_idx, bg_idx)

            // Populate base pixels once
            current_block_pixels_rgb.clear();
            for y_idx in 0..gh {
                for x_idx in 0..gw {
                    let px = blk.pixels[y_idx][x_idx].orig;
                    let [r, g, b] = unpack_rgb(px);
                    current_block_pixels_rgb.push(PrePixel {
                        r: r as u64, g: g as u64, b: b as u64,
                        sq: (r as u64).pow(2) + (g as u64).pow(2) + (b as u64).pow(2)
                    });
                }
            }
            let original_pixels_rgb = current_block_pixels_rgb.clone();

            for (sx, sy) in &scales {
                // Generate pixels for this scale
                if *sx == 1.0 && *sy == 1.0 {
                    // Just reference original
                    working_pixels_buffer.clear();
                    working_pixels_buffer.extend_from_slice(&original_pixels_rgb);
                } else {
                    get_transformed_block_pixels_into(&original_pixels_rgb, gw, gh, *sx, *sy, &mut working_pixels_buffer);
                };
                let working_pixels = &working_pixels_buffer;

                // Get neighbors for continuity
                let left_neighbor_data = if left_neighbor_glyph_idx.0 != usize::MAX {
                    Some((&glyphs[left_neighbor_glyph_idx.0], left_neighbor_glyph_idx.1))
                } else { None };
                
                let top_neighbor_data = if previous_row_glyph_indices[col_idx].0 != usize::MAX {
                    Some((&glyphs[previous_row_glyph_indices[col_idx].0], previous_row_glyph_indices[col_idx].1))
                } else { None };

                if render == Render::Ansi24 {
                    // --- Ansi24 Matching ---
                    if args.smooth.is_some() {
                         // (Same as before, reusing logic if needed, but smooth uses dedicated function now)
                         // But if user passes --smooth in args, we redirect at start.
                         // So this branch is for NON-smooth mode only (unless logic changes).
                         // The top of function handles smooth redirect.
                    }

                    let mut best_cost = u64::MAX;
                    let mut best_glyph_idx = 0;
                    let mut best_chosen_fg_color = [0u8; 3];
                    let mut best_chosen_bg_color = [0u8; 3];
                    let mut best_inv = false;
                    
                    // Optimization: Pre-calculate block sum
                    let mut block_sum_r = 0u64;
                    let mut block_sum_g = 0u64;
                    let mut block_sum_b = 0u64;
                    let mut block_sum_sq = 0u64;
                    for p in working_pixels {
                        block_sum_r += p.r; block_sum_g += p.g; block_sum_b += p.b; block_sum_sq += p.sq;
                    }

                    for (gi, fglyph) in glyphs.iter().enumerate() {
                        let mut sum_ones_r = 0u64;
                        let mut sum_ones_g = 0u64;
                        let mut sum_ones_b = 0u64;
                        let mut sum_ones_sq = 0u64;
                        let mut count_ones = 0u64;

                        for i in 0..bp {
                            if fglyph.bitmap[i] == 1 {
                                let px = &working_pixels[i];
                                sum_ones_r += px.r; sum_ones_g += px.g; sum_ones_b += px.b; sum_ones_sq += px.sq;
                                count_ones += 1;
                            }
                        }
                        
                        let count_zeros = bp as u64 - count_ones;
                        
                        // Derive Zeros
                        let sum_zeros_r = block_sum_r - sum_ones_r;
                        let sum_zeros_g = block_sum_g - sum_ones_g;
                        let sum_zeros_b = block_sum_b - sum_ones_b;
                        let sum_zeros_sq = block_sum_sq - sum_ones_sq;

                                                let avg_ones = if count_ones > 0 {
                                                    [((sum_ones_r / count_ones) as u8), ((sum_ones_g / count_ones) as u8), ((sum_ones_b / count_ones) as u8)]
                                                } else { [0,0,0] };
                                                let avg_zeros = if count_zeros > 0 {
                                                    [((sum_zeros_r / count_zeros) as u8), ((sum_zeros_g / count_zeros) as u8), ((sum_zeros_b / count_zeros) as u8)]
                                                } else { [0,0,0] };
                        let target_bg = if args.fg_only {
                            bg_opt_rgb.unwrap_or([0, 0, 0])
                        } else {
                            bg_opt_rgb.unwrap_or(avg_zeros)
                        };
                        
                        // Cost Calculation (Normal)
                        let t1_1 = sum_ones_sq;
                        let t2_1 = 2 * (avg_ones[0] as u64 * sum_ones_r + avg_ones[1] as u64 * sum_ones_g + avg_ones[2] as u64 * sum_ones_b);
                        let t3_1 = count_ones * (avg_ones[0] as u64 * avg_ones[0] as u64 + avg_ones[1] as u64 * avg_ones[1] as u64 + avg_ones[2] as u64 * avg_ones[2] as u64);
                        let c1 = (t1_1 + t3_1).saturating_sub(t2_1);
                        let t1_0 = sum_zeros_sq;
                        let t2_0 = 2 * (target_bg[0] as u64 * sum_zeros_r + target_bg[1] as u64 * sum_zeros_g + target_bg[2] as u64 * sum_zeros_b);
                        let t3_0 = count_zeros * (target_bg[0] as u64 * target_bg[0] as u64 + target_bg[1] as u64 * target_bg[1] as u64 + target_bg[2] as u64 * target_bg[2] as u64);
                        let c0 = (t1_0 + t3_0).saturating_sub(t2_0);
                        let cost_no_invert = c1 + c0;

                        if cost_no_invert < best_cost {
                            best_cost = cost_no_invert;
                            best_glyph_idx = gi;
                            best_chosen_fg_color = avg_ones;
                            best_chosen_bg_color = target_bg;
                            best_inv = false;
                        }

                        // Cost Calculation (Inverted)
                        let target_bg_inv = if args.fg_only {
                             bg_opt_rgb.unwrap_or([0, 0, 0])
                        } else {
                             bg_opt_rgb.unwrap_or(avg_ones)
                        };

                        let t1_0 = sum_zeros_sq;
                        let t2_0 = 2 * (avg_zeros[0] as u64 * sum_zeros_r + avg_zeros[1] as u64 * sum_zeros_g + avg_zeros[2] as u64 * sum_zeros_b);
                        let t3_0 = count_zeros * (avg_zeros[0] as u64 * avg_zeros[0] as u64 + avg_zeros[1] as u64 * avg_zeros[1] as u64 + avg_zeros[2] as u64 * avg_zeros[2] as u64);
                        let c_fg = (t1_0 + t3_0).saturating_sub(t2_0);

                        let t1_1 = sum_ones_sq;
                        let t2_1 = 2 * (target_bg_inv[0] as u64 * sum_ones_r + target_bg_inv[1] as u64 * sum_ones_g + target_bg_inv[2] as u64 * sum_ones_b);
                        let t3_1 = count_ones * (target_bg_inv[0] as u64 * target_bg_inv[0] as u64 + target_bg_inv[1] as u64 * target_bg_inv[1] as u64 + target_bg_inv[2] as u64 * target_bg_inv[2] as u64);
                        let c_bg = (t1_1 + t3_1).saturating_sub(t2_1);
                        let cost_invert = c_fg + c_bg;

                        if cost_invert < best_cost {
                            best_cost = cost_invert;
                            best_glyph_idx = gi;
                            best_chosen_fg_color = avg_zeros;
                            best_chosen_bg_color = target_bg_inv;
                            best_inv = true;
                        }

                        if best_cost == 0 { break; }
                    }

                    if best_cost < best_overall_cost {
                        best_overall_cost = best_cost;
                        chosen_glyph_index = best_glyph_idx;
                        chosen_glyph_inverted = best_inv;
                        final_glyph = glyphs[best_glyph_idx].ch;
                        final_fg = Colour::RGB(best_chosen_fg_color);
                        final_bg = if args.fg_only { None } else { Some(Colour::RGB(best_chosen_bg_color)) };
                        best_match_data = (best_glyph_idx, best_inv, best_chosen_fg_color, best_chosen_bg_color, 0, 0);
                        best_scale_is_identity = *sx == 1.0 && *sy == 1.0;
                    }

                } else {
                    // --- Indexed Matching ---
                    let bg_opt_idx = args.bg_opt.as_ref().map(|s| nearest_hex_colour_fast(parse_hex_color(s), if render == Render::Ansi { &ANSI256 } else { &IRC99 }));
                    
                    let mut block_sum_r = 0u64;
                    let mut block_sum_g = 0u64;
                    let mut block_sum_b = 0u64;
                    let mut block_sum_sq = 0u64;
                    for p in working_pixels {
                        block_sum_r += p.r; block_sum_g += p.g; block_sum_b += p.b; block_sum_sq += p.sq;
                    }

                    for (gi, fglyph) in glyphs.iter().enumerate() {
                        for inv in [false, true] {
                            // 1. Determine optimal FG color using average RGB of pixels under 'ones'
                            let mut sum_ones_r = 0u64;
                            let mut sum_ones_g = 0u64;
                            let mut sum_ones_b = 0u64;
                            let mut sum_ones_sq = 0u64;
                            let mut count_ones = 0u64;

                            for i in 0..bp {
                                if fglyph.bitmap[i] ^ (inv as u8) == 1 {
                                    let px = &working_pixels[i];
                                    sum_ones_r += px.r; sum_ones_g += px.g; sum_ones_b += px.b; sum_ones_sq += px.sq;
                                    count_ones += 1;
                                }
                            }
                            let count_zeros = bp as u64 - count_ones;
                            let sum_zeros_r = block_sum_r - sum_ones_r;
                            let sum_zeros_g = block_sum_g - sum_ones_g;
                            let sum_zeros_b = block_sum_b - sum_ones_b;
                            let sum_zeros_sq = block_sum_sq - sum_ones_sq;

                            let avg_ones = if count_ones > 0 {
                                [(sum_ones_r / count_ones) as u8, (sum_ones_g / count_ones) as u8, (sum_ones_b / count_ones) as u8]
                            } else { [0,0,0] };
                            let avg_zeros = if count_zeros > 0 {
                                [(sum_zeros_r / count_zeros) as u8, (sum_zeros_g / count_zeros) as u8, (sum_zeros_b / count_zeros) as u8]
                            } else { [0,0,0] };

                            let palette: &[u32] = if render == Render::Ansi { &ANSI256 } else { &IRC99 };
                            
                            let fg_idx = nearest_hex_colour_fast(make_rgb_u32(&avg_ones), palette);
                            let bg_idx = if args.fg_only {
                                bg_opt_idx.unwrap_or(0) as u8
                            } else {
                                nearest_hex_colour_fast(make_rgb_u32(&avg_zeros), palette)
                            };

                            let fg_rgb = unpack_rgb(palette[fg_idx as usize]);
                            let bg_rgb = unpack_rgb(palette[bg_idx as usize]);

                            let fg_p = PrePixel { 
                                r: fg_rgb[0] as u64, g: fg_rgb[1] as u64, b: fg_rgb[2] as u64, 
                                sq: (fg_rgb[0] as u64).pow(2) + (fg_rgb[1] as u64).pow(2) + (fg_rgb[2] as u64).pow(2) 
                            };
                            let bg_p = PrePixel { 
                                r: bg_rgb[0] as u64, g: bg_rgb[1] as u64, b: bg_rgb[2] as u64, 
                                sq: (bg_rgb[0] as u64).pow(2) + (bg_rgb[1] as u64).pow(2) + (bg_rgb[2] as u64).pow(2) 
                            };

                            // Euclidean Error Calculation
                            let t1_1 = sum_ones_sq;
                            let t2_1 = 2 * (fg_p.r * sum_ones_r + fg_p.g * sum_ones_g + fg_p.b * sum_ones_b);
                            let t3_1 = count_ones * fg_p.sq;
                            let c1 = (t1_1 + t3_1).saturating_sub(t2_1);

                            let t1_0 = sum_zeros_sq;
                            let t2_0 = 2 * (bg_p.r * sum_zeros_r + bg_p.g * sum_zeros_g + bg_p.b * sum_zeros_b);
                            let t3_0 = count_zeros * bg_p.sq;
                            let c0 = (t1_0 + t3_0).saturating_sub(t2_0);

                            let cost = c1 + c0;

                            if cost < best_overall_cost {
                                best_overall_cost = cost;
                                chosen_glyph_index = gi;
                                chosen_glyph_inverted = inv;
                                final_glyph = glyphs[gi].ch;
                                final_fg = Colour::Index(fg_idx);
                                final_bg = if args.fg_only { None } else { Some(Colour::Index(bg_idx)) };
                                best_match_data = (gi, inv, [0,0,0], [0,0,0], fg_idx, bg_idx);
                                best_scale_is_identity = *sx == 1.0 && *sy == 1.0;
                            }
                        }
                    }
                }
            }

            if !best_scale_is_identity {
                dynamic_scaling_count += 1;
            }

            // Update neighbor tracking
            left_neighbor_glyph_idx = (chosen_glyph_index, chosen_glyph_inverted);
            previous_row_glyph_indices[col_idx] = (chosen_glyph_index, chosen_glyph_inverted);
            
            // Calculate final error against ORIGINAL pixels
            // This ensures fairness and that we don't just pick a scale that makes the block empty.
            let fglyph = &glyphs[chosen_glyph_index];
            let inv = chosen_glyph_inverted;
            let mut block_error = 0u64;
            
            if render == Render::Ansi24 {
                let fg_rgb = best_match_data.2;
                let bg_rgb = best_match_data.3;
                let fg_p = PrePixel { 
                    r: fg_rgb[0] as u64, g: fg_rgb[1] as u64, b: fg_rgb[2] as u64, 
                    sq: (fg_rgb[0] as u64).pow(2) + (fg_rgb[1] as u64).pow(2) + (fg_rgb[2] as u64).pow(2) 
                };
                let bg_p = PrePixel { 
                    r: bg_rgb[0] as u64, g: bg_rgb[1] as u64, b: bg_rgb[2] as u64, 
                    sq: (bg_rgb[0] as u64).pow(2) + (bg_rgb[1] as u64).pow(2) + (bg_rgb[2] as u64).pow(2) 
                };
                
                for i in 0..bp {
                    let px = &original_pixels_rgb[i];
                    let dist_fg = (px.sq + fg_p.sq).saturating_sub(2 * (px.r * fg_p.r + px.g * fg_p.g + px.b * fg_p.b));
                    let dist_bg = (px.sq + bg_p.sq).saturating_sub(2 * (px.r * bg_p.r + px.g * bg_p.g + px.b * bg_p.b));
                    
                    let bit = fglyph.bitmap[i] ^ (inv as u8);
                    
                    if bit == 1 { if dist_bg < dist_fg { block_error += 1; } } // If pixel should be FG, but BG is closer
                    else { if dist_fg < dist_bg { block_error += 1; } } // If pixel should be BG, but FG is closer
                }
            } else {
                let fg_idx = best_match_data.4 as usize;
                let bg_idx = best_match_data.5 as usize;
                
                // We need indices for ORIGINAL pixels to calculate error fairly?
                // Or we can just re-quantize original pixels.
                for i in 0..bp {
                    let px_rgb = &original_pixels_rgb[i];
                    let rgb_u32 = ((px_rgb.r as u32) << 16) | ((px_rgb.g as u32) << 8) | (px_rgb.b as u32);
                    // Fast re-quantize (skipping some flags for speed, or match loop logic)
                    let col_idx = nearest_hex_colour_fast(rgb_u32, if render == Render::Ansi { &ANSI256 } else { &IRC99 }) as usize;
                    
                    let bit = fglyph.bitmap[i] ^ (inv as u8);
                    if bit == 1 { if col_idx != fg_idx { block_error += 1; } } // If pixel should be FG, but original is not that color
                    else { if col_idx != bg_idx { block_error += 1; } } // If pixel should be BG, but original is not that color
                }
            }
            total_error += block_error;

            let fg_rgb = match final_fg {
                Colour::RGB(c) => c,
                Colour::Index(i) => {
                    let hex = match render {
                        Render::Irc => IRC99[i as usize % 99],
                        _ => ANSI256[i as usize],
                    };
                    unpack_rgb(hex)
                }
            };
            let bg_rgb = final_bg.clone().map(|c| match c {
                Colour::RGB(c) => c,
                Colour::Index(i) => {
                    let hex = match render {
                        Render::Irc => IRC99[i as usize % 99],
                        _ => ANSI256[i as usize],
                    };
                    unpack_rgb(hex)
                }
            });
            grid_row.push(RenderedCell { char: final_glyph, fg: fg_rgb, bg: bg_rgb });

            emit_colourized(
                &mut out,
                render,
                final_fg,
                final_bg,
                final_glyph,
                &mut first,
                &mut last_fg,
                &mut last_bg,
            );
        }
        out.push_str(match render {
            Render::Ansi | Render::Ansi24 => "\x1b[0m\n",
            Render::Irc => "\x0f\n",
        });
        grid.push(grid_row);
    }
    RenderResult {
        content: out.trim_end_matches('\n').into(),
        error_count: total_error,
        total_pixels: total_pixels,
        dynamic_scaling_count,
        grid,
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

    let (mut min_l, mut max_l) = (255u32,0u32);
    for row in &image_luma.bitmap { for &px in row { let l = luma(&unpack_rgb(px)) as u32; min_l = min_l.min(l); max_l = max_l.max(l); } }
    let thr = min_l + ((max_l - min_l).max(1)/2);

    let mut out = String::new();
    let mut first = true;
    let mut last_fg: Option<Colour> = None;
    let mut last_bg: Option<Colour> = None;

    for y in (0..h).step_by(4) {
        for x in (0..w).step_by(2) {
            let mut braille = 0x2800;
            let mut counts: HashMap<Colour,usize> = HashMap::new();

            for &(dx,dy,bit) in &POSITIONS {
                let yy=y+dy; let xx=x+dx;
                if yy<h && xx<w {
                    let pxl_luma_orig = image_luma.bitmap[yy][xx];
                    let rgb_l = unpack_rgb(pxl_luma_orig);
                    let mut lum = luma(&rgb_l) as f64 + err[yy][xx];
                    lum = lum.clamp(0.0,255.0);
                    let newp_is_dot = lum > thr as f64;
                    let e = lum - if newp_is_dot { 255.0 } else { 0.0 };
                    if xx+1<w { err[yy][xx+1]+=e*7.0/16.0 }
                    if yy+1<h {
                        if xx>0 { err[yy+1][xx-1]+=e*3.0/16.0 }
                        err[yy+1][xx]+=e*5.0/16.0;
                        if xx+1<w { err[yy+1][xx+1]+=e*1.0/16.0 }
                    }
                    if newp_is_dot {
                        braille|=bit;
                        let px_chroma_orig = image_chroma.bitmap[yy][xx];
                        let ansi_pixel_chroma = AnsiPixel::new(&px_chroma_orig, args.grayscale_tolerance);
                        let col = pick_colour(&ansi_pixel_chroma, render, args);
                        *counts.entry(col).or_insert(0)+=1;
                    }
                }
            }

            let fg = counts.into_iter().max_by_key(|&(_,c)|c).map(|(c,_)|c)
                .unwrap_or_else(|| {
                    let default_px_val = image_chroma.bitmap.get(y).and_then(|r| r.get(x)).copied().unwrap_or(0);
                    let default_ansi_pixel = AnsiPixel::new(&default_px_val, args.grayscale_tolerance);
                    pick_colour(&default_ansi_pixel, render, args)
                });

            let glyph = char::from_u32(braille).unwrap_or(' ');
            emit_colourized(&mut out, render, fg, None, glyph, &mut first, &mut last_fg, &mut last_bg);
        }
        out.push_str(match render {
            Render::Ansi | Render::Ansi24 => "\x1b[0m\n",
            Render::Irc    => "\x0f\n",
        });
    }
    RenderResult {
        content: out.trim_end_matches('\n').to_string(),
        error_count: 0,
        total_pixels: (w * h) as u64,
        dynamic_scaling_count: 0,
        grid: Vec::new(),
    }
}

pub fn luma(rgb: &[u8;3]) -> u8 {
    let r = rgb[0] as f32; let g = rgb[1] as f32; let b = rgb[2] as f32;
    (0.299*r + 0.587*g + 0.114*b).round() as u8
}

pub fn save_as_png(result: &RenderResult, glyphs: &GlyphStore, path_str: &str, metadata: &str) -> Result<(), Box<dyn std::error::Error>> {
    if result.grid.is_empty() { return Ok(()); }
    let path = std::path::Path::new(path_str);
    let final_path = if path.is_dir() {
        let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        path.join(format!("img2irc_{}.png", timestamp))
    } else {
        path.to_path_buf()
    };

    let rows = result.grid.len();
    let cols = result.grid[0].len();
    let (gh, gw) = glyphs.metrics; 
    
    let img_width = (cols * gw) as u32;
    let img_height = (rows * gh) as u32;
    
    let mut img = ImageBuffer::new(img_width, img_height);
    
    let glyph_map: HashMap<char, &Vec<Vec<u8>>> = glyphs.glyphs.iter().map(|(c, b)| (*c, b)).collect();
    
    for (r, row_cells) in result.grid.iter().enumerate() {
        for (c, cell) in row_cells.iter().enumerate() {
            let start_x = (c * gw) as u32;
            let start_y = (r * gh) as u32;
            
            if let Some(bitmap) = glyph_map.get(&cell.char) {
                for (y, row_data) in bitmap.iter().enumerate() {
                    for (x, &pixel) in row_data.iter().enumerate() {
                        let px = if pixel == 1 {
                            Rgb(cell.fg)
                        } else {
                            cell.bg.map(Rgb).unwrap_or(Rgb([0, 0, 0]))
                        };
                        let target_x = start_x + x as u32;
                        let target_y = start_y + y as u32;
                        if target_x < img_width && target_y < img_height {
                            img.put_pixel(target_x, target_y, px);
                        }
                    }
                }
            } else if let Some(bg) = cell.bg {
                for y in 0..gh {
                    for x in 0..gw {
                         let target_x = start_x + x as u32;
                         let target_y = start_y + y as u32;
                         if target_x < img_width && target_y < img_height {
                             img.put_pixel(target_x, target_y, Rgb(bg));
                         }
                    }
                }
            }
        }
    }
    
    let file = std::fs::File::create(&final_path)?;
    let mut writer = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(writer, img_width, img_height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    
    if !metadata.is_empty() {
        // Encode metadata as base64 to avoid ISO 8859-1 issues with unicode characters
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let encoded_metadata = STANDARD.encode(metadata);
        if let Err(e) = encoder.add_text_chunk("Comment".to_string(), encoded_metadata) {
            eprintln!("Warning: Failed to add metadata chunk: {}", e);
        }
    }

    let mut writer = encoder.write_header()?;
    writer.write_image_data(img.as_raw())?;
    drop(writer);

    eprintln!("Saved PNG to {}", final_path.display());
    Ok(())
}

pub fn calculate_smoothness(result: &RenderResult, glyphs: &GlyphStore) -> f64 {
    if result.grid.is_empty() { return 0.0; }
    let rows = result.grid.len();
    let cols = result.grid[0].len();
    let (gh, gw) = glyphs.metrics;
    
    let glyph_map: HashMap<char, &Vec<Vec<u8>>> = glyphs.glyphs.iter().map(|(c, b)| (*c, b)).collect();
    
    let mut total_variation = 0.0;
    let mut total_pixels = 0;

    let get_pixel = |r: usize, c: usize, y: usize, x: usize| -> Option<[u8; 3]> {
        if r >= rows || c >= cols { return None; }
        let cell = &result.grid[r][c];
        if let Some(bitmap) = glyph_map.get(&cell.char) {
            if y < bitmap.len() && x < bitmap[y].len() {
                if bitmap[y][x] == 1 {
                    Some(cell.fg)
                } else {
                    Some(cell.bg.unwrap_or([0, 0, 0]))
                }
            } else {
                Some(cell.bg.unwrap_or([0, 0, 0]))
            }
        } else {
            Some(cell.bg.unwrap_or([0, 0, 0]))
        }
    };

    for r in 0..rows {
        for c in 0..cols {
            for y in 0..gh {
                for x in 0..gw {
                    let p1 = match get_pixel(r, c, y, x) {
                        Some(p) => p,
                        None => continue,
                    };
                    
                    // Right
                    let (nr, nc, ny, nx) = if x + 1 < gw { (r, c, y, x + 1) } else { (r, c + 1, y, 0) };
                    if let Some(p2) = get_pixel(nr, nc, ny, nx) {
                        let d = (p1[0] as f64 - p2[0] as f64).powi(2) + (p1[1] as f64 - p2[1] as f64).powi(2) + (p1[2] as f64 - p2[2] as f64).powi(2);
                        total_variation += d;
                    }

                    // Bottom
                    let (nr, nc, ny, nx) = if y + 1 < gh { (r, c, y + 1, x) } else { (r + 1, c, 0, x) };
                    if let Some(p2) = get_pixel(nr, nc, ny, nx) {
                        let d = (p1[0] as f64 - p2[0] as f64).powi(2) + (p1[1] as f64 - p2[1] as f64).powi(2) + (p1[2] as f64 - p2[2] as f64).powi(2);
                        total_variation += d;
                    }
                    
                    total_pixels += 1;
                }
            }
        }
    }

    if total_pixels == 0 { 0.0 } else { total_variation / (total_pixels as f64) }
}

pub fn calculate_smoothness_relative(result: &RenderResult, glyphs: &GlyphStore) -> f64 {
    let base_smoothness = calculate_smoothness(result, glyphs);
    let total_glyphs = (result.grid.len() * result.grid.get(0).map(|r| r.len()).unwrap_or(0)) as f64;
    // Penalize larger renders by multiplying smoothness by sqrt of glyph count?
    // User asked for "relative to total number of glyphs". 
    // If smoothness decreases as N increases (1/N behaviour), multiplying by N makes it constant-ish.
    // Let's multiply by total_glyphs.
    base_smoothness * total_glyphs
}

pub fn calculate_frequency_metric(result: &RenderResult, original: &AnsiImage, glyphs: &GlyphStore) -> f64 {
    // "Prefers smooth error over sharp error"
    // 1. Calculate Error Image (Pixel Diff)
    // 2. Calculate Roughness (Smoothness metric) of Error Image
    
    if result.grid.is_empty() { return 0.0; }
    let rows = result.grid.len();
    let cols = result.grid[0].len();
    let (gh, gw) = glyphs.metrics;
    
    let glyph_map: HashMap<char, &Vec<Vec<u8>>> = glyphs.glyphs.iter().map(|(c, b)| (*c, b)).collect();
    let mut total_variation = 0.0;
    let mut count = 0;

    let get_pixel_diff = |r: usize, c: usize, y: usize, x: usize| -> Option<[i32; 3]> {
        if r >= rows || c >= cols { return None; }
        
        // Rendered Pixel
        let cell = &result.grid[r][c];
        let rendered_rgb = if let Some(bitmap) = glyph_map.get(&cell.char) {
            if y < bitmap.len() && x < bitmap[y].len() {
                if bitmap[y][x] == 1 { cell.fg } else { cell.bg.unwrap_or([0,0,0]) }
            } else { cell.bg.unwrap_or([0,0,0]) }
        } else { cell.bg.unwrap_or([0,0,0]) };

        // Original Pixel
        let orig_y = r * gh + y;
        let orig_x = c * gw + x;
        if orig_y >= original.bitmap.len() || orig_x >= original.bitmap[0].len() { return None; }
        
        let orig_u32 = original.bitmap[orig_y][orig_x];
        let orig_rgb = unpack_rgb(orig_u32);

        Some([
            rendered_rgb[0] as i32 - orig_rgb[0] as i32,
            rendered_rgb[1] as i32 - orig_rgb[1] as i32,
            rendered_rgb[2] as i32 - orig_rgb[2] as i32,
        ])
    };

    for r in 0..rows {
        for c in 0..cols {
            for y in 0..gh {
                for x in 0..gw {
                    let d1 = match get_pixel_diff(r, c, y, x) { Some(d) => d, None => continue };
                    
                    // Compare Error pixel with right neighbor Error pixel
                    // Right
                    let (nr, nc, ny, nx) = if x + 1 < gw { (r, c, y, x + 1) } else { (r, c + 1, y, 0) };
                    if let Some(d2) = get_pixel_diff(nr, nc, ny, nx) {
                        // Squared difference of ERRORS
                        // (E1 - E2)^2
                        let dist = (d1[0] - d2[0]).pow(2) + (d1[1] - d2[1]).pow(2) + (d1[2] - d2[2]).pow(2);
                        total_variation += dist as f64;
                    }
                    
                    // Bottom
                    let (nr, nc, ny, nx) = if y + 1 < gh { (r, c, y + 1, x) } else { (r + 1, c, 0, x) };
                    if let Some(d2) = get_pixel_diff(nr, nc, ny, nx) {
                        let dist = (d1[0] - d2[0]).pow(2) + (d1[1] - d2[1]).pow(2) + (d1[2] - d2[2]).pow(2);
                        total_variation += dist as f64;
                    }
                    count += 1;
                }
            }
        }
    }
    
    if count == 0 { 0.0 } else { total_variation / count as f64 }
}

fn render_blocks_smooth(
    image: &AnsiImage,
    args: &RenderArgs,
    render: Render,
    glyphs: &[FastGlyph],
) -> RenderResult {
    if glyphs.is_empty() {
        return "Error: no glyphs available".into();
    }

    let bg_opt_rgb = args.bg_opt.as_ref().map(|s| unpack_rgb(parse_hex_color(s)));
    let (gh, gw) = image.glyphs.metrics;
    let bp = gh * gw;

    // Pre-calculate blocks width/height
    let blocks_width = if !image.block.is_empty() { image.block[0].len() } else { 0 };

    let mut out = String::new();
    let mut total_error = 0u64;
    let mut total_pixels = 0u64;

    let mut current_block_pixels_rgb: Vec<PrePixel> = Vec::with_capacity(bp);
    
    // Weights
    let smooth_weight = args.smooth.unwrap_or(1.0).clamp(0.0, 1.0); 
    let continuity_scalar = 10_000.0; 

    // Track the bottom edge bits of the previous row (for Top continuity)
    // 0 = BG, 1 = FG. Initialized to 0 (BG).
    let mut prev_row_bottom_bits: Vec<Vec<u8>> = vec![vec![0u8; gw]; blocks_width];
    let mut dynamic_scaling_count = 0u64;
    
    // Dynamic Scaling Factors
    let fuzz = args.fuzz.max(0.0);
    let scales = if fuzz > 0.0001 {
        let f = fuzz.max(0.001);
        vec![
            (1.0, 1.0),
            (1.0 - f, 1.0),
            (1.0 + f, 1.0),
            (1.0, 1.0 - f),
            (1.0, 1.0 + f),
        ]
    } else {
        vec![(1.0, 1.0)]
    };

    let mut grid: Vec<Vec<RenderedCell>> = Vec::new();
    let mut working_pixels_buffer = Vec::with_capacity(bp);
    
    // Optimization: Reuse distance cache vectors
    let mut cost_cache_fg_no_inv = vec![0u64; bp];
    let mut cost_cache_bg_no_inv = vec![0u64; bp];
    let mut cost_cache_fg_inv = vec![0u64; bp];
    let mut cost_cache_bg_inv = vec![0u64; bp];

    for (row_idx, block_row) in image.block.iter().enumerate() {
        let mut grid_row = Vec::new();
        let mut first = true;
        let mut last_fg: Option<Colour> = None;
        let mut last_bg: Option<Colour> = None;

        // Track the right edge bits of the left neighbor (for Left continuity)
        let mut left_neighbor_right_bits: Vec<u8> = vec![0u8; gh];

        for (col_idx, blk) in block_row.iter().enumerate() {
            total_pixels += bp as u64;
            
            let mut best_overall_cost = u64::MAX;
            let mut final_glyph = ' ';
            let mut final_fg = Colour::Index(0);
            let mut final_bg: Option<Colour> = None;
            let mut best_match_data = (0usize, false, [0u8;3], [0u8;3]); // (glyph_idx, inv, fg, bg)
            let mut best_scale_is_identity = true;

            // Read original block pixels
            current_block_pixels_rgb.clear();
            for y_idx in 0..gh {
                for x_idx in 0..gw {
                    let px = blk.pixels[y_idx][x_idx].orig;
                    let [r, g, b] = unpack_rgb(px);
                    current_block_pixels_rgb.push(PrePixel {
                        r: r as u64, g: g as u64, b: b as u64,
                        sq: (r as u64).pow(2) + (g as u64).pow(2) + (b as u64).pow(2)
                    });
                }
            }
            let original_pixels_rgb = current_block_pixels_rgb.clone();
            
            let top_neighbor_bits = &prev_row_bottom_bits[col_idx];
            let left_neighbor_bits = &left_neighbor_right_bits;

            for (sx, sy) in &scales {
                if *sx == 1.0 && *sy == 1.0 {
                    working_pixels_buffer.clear();
                    working_pixels_buffer.extend_from_slice(&original_pixels_rgb);
                } else {
                    get_transformed_block_pixels_into(&original_pixels_rgb, gw, gh, *sx, *sy, &mut working_pixels_buffer);
                };
                let current_block_pixels_rgb = &working_pixels_buffer;

                // --- 1. Find Optimal Colors (Standard "Non-Smooth" Pass) ---
                let mut std_best_cost = u64::MAX;
                let mut locked_fg = [0u8; 3];
                let mut locked_bg = [0u8; 3];
                
                // Optimization: Pre-calculate block sum
                let mut block_sum_r = 0u64;
                let mut block_sum_g = 0u64;
                let mut block_sum_b = 0u64;
                let mut block_sum_sq = 0u64;
                for p in current_block_pixels_rgb {
                    block_sum_r += p.r; block_sum_g += p.g; block_sum_b += p.b; block_sum_sq += p.sq;
                }

                for fglyph in glyphs.iter() {
                    let mut sum_ones_r = 0u64;
                    let mut sum_ones_g = 0u64;
                    let mut sum_ones_b = 0u64;
                    let mut sum_ones_sq = 0u64;
                    let mut count_ones = 0u64;

                    for i in 0..bp {
                        if fglyph.bitmap[i] == 1 {
                            let px = &current_block_pixels_rgb[i];
                            sum_ones_r += px.r; sum_ones_g += px.g; sum_ones_b += px.b; sum_ones_sq += px.sq;
                            count_ones += 1;
                        }
                    }
                    let count_zeros = bp as u64 - count_ones;

                    // Derive Zeros
                    let sum_zeros_r = block_sum_r - sum_ones_r;
                    let sum_zeros_g = block_sum_g - sum_ones_g;
                    let sum_zeros_b = block_sum_b - sum_ones_b;
                    let sum_zeros_sq = block_sum_sq - sum_ones_sq;

                    let avg_ones = if count_ones > 0 {
                        [(sum_ones_r / count_ones) as u8, (sum_ones_g / count_ones) as u8, (sum_ones_b / count_ones) as u8]
                    } else { [0,0,0] };
                    let avg_zeros = if count_zeros > 0 {
                        [(sum_zeros_r / count_zeros) as u8, (sum_zeros_g / count_zeros) as u8, (sum_zeros_b / count_zeros) as u8]
                    } else { [0,0,0] };

                    let target_bg = if args.fg_only {
                        bg_opt_rgb.unwrap_or([0, 0, 0])
                    } else {
                        bg_opt_rgb.unwrap_or(avg_zeros)
                    };

                    // Cost Calculation (Normal)
                    let t1_1 = sum_ones_sq;
                    let t2_1 = 2 * (avg_ones[0] as u64 * sum_ones_r + avg_ones[1] as u64 * sum_ones_g + avg_ones[2] as u64 * sum_ones_b);
                    let t3_1 = count_ones * (avg_ones[0] as u64 * avg_ones[0] as u64 + avg_ones[1] as u64 * avg_ones[1] as u64 + avg_ones[2] as u64 * avg_ones[2] as u64);
                    let cost_ones = (t1_1 + t3_1).saturating_sub(t2_1);
                    let t1_0 = sum_zeros_sq;
                    let t2_0 = 2 * (target_bg[0] as u64 * sum_zeros_r + target_bg[1] as u64 * sum_zeros_g + target_bg[2] as u64 * sum_zeros_b);
                    let t3_0 = count_zeros * (target_bg[0] as u64 * target_bg[0] as u64 + target_bg[1] as u64 * target_bg[1] as u64 + target_bg[2] as u64 * target_bg[2] as u64);
                    let cost_zeros = (t1_0 + t3_0).saturating_sub(t2_0);
                    let cost_no_inv = cost_ones + cost_zeros;

                    if cost_no_inv < std_best_cost {
                        std_best_cost = cost_no_inv;
                        locked_fg = avg_ones;
                        locked_bg = target_bg;
                    }

                    // Cost Calculation (Inverted)
                    let target_bg_inv = if args.fg_only {
                         bg_opt_rgb.unwrap_or([0, 0, 0])
                    } else {
                         bg_opt_rgb.unwrap_or(avg_ones)
                    };

                    let t1_0 = sum_zeros_sq;
                    let t2_0 = 2 * (avg_zeros[0] as u64 * sum_zeros_r + avg_zeros[1] as u64 * sum_zeros_g + avg_zeros[2] as u64 * sum_zeros_b);
                    let t3_0 = count_zeros * (avg_zeros[0] as u64 * avg_zeros[0] as u64 + avg_zeros[1] as u64 * avg_zeros[1] as u64 + avg_zeros[2] as u64 * avg_zeros[2] as u64);
                    let cost_fg = (t1_0 + t3_0).saturating_sub(t2_0);

                    let t1_1 = sum_ones_sq;
                    let t2_1 = 2 * (target_bg_inv[0] as u64 * sum_ones_r + target_bg_inv[1] as u64 * sum_ones_g + target_bg_inv[2] as u64 * sum_ones_b);
                    let t3_1 = count_ones * (target_bg_inv[0] as u64 * target_bg_inv[0] as u64 + target_bg_inv[1] as u64 * target_bg_inv[1] as u64 + target_bg_inv[2] as u64 * target_bg_inv[2] as u64);
                    let cost_bg = (t1_1 + t3_1).saturating_sub(t2_1);
                    let cost_inv = cost_fg + cost_bg;

                    if cost_inv < std_best_cost {
                        std_best_cost = cost_inv;
                        locked_fg = avg_zeros;
                        locked_bg = target_bg_inv;
                    }
                    
                    if std_best_cost == 0 { break; }
                }

                // --- 2. Smooth Pass (Using Locked Colors) ---
                let mut best_cost = u64::MAX;
                let mut best_glyph_idx = 0;
                let mut best_chosen_inv = false;
                
                let fg_p = PrePixel { 
                    r: locked_fg[0] as u64, g: locked_fg[1] as u64, b: locked_fg[2] as u64, 
                    sq: (locked_fg[0] as u64).pow(2) + (locked_fg[1] as u64).pow(2) + (locked_fg[2] as u64).pow(2) 
                };
                let bg_p = PrePixel { 
                    r: locked_bg[0] as u64, g: locked_bg[1] as u64, b: locked_bg[2] as u64, 
                    sq: (locked_bg[0] as u64).pow(2) + (locked_bg[1] as u64).pow(2) + (locked_bg[2] as u64).pow(2) 
                };

                // Pre-calculate per-pixel costs for FG and BG
                // cost_cache_fg[i] = error if pixel i is FG
                // cost_cache_bg[i] = error if pixel i is BG
                for i in 0..bp {
                    let px = &current_block_pixels_rgb[i];
                    let dist_fg = (px.sq + fg_p.sq).saturating_sub(2 * (px.r * fg_p.r + px.g * fg_p.g + px.b * fg_p.b));
                    let dist_bg = (px.sq + bg_p.sq).saturating_sub(2 * (px.r * bg_p.r + px.g * bg_p.g + px.b * bg_p.b));
                    
                    cost_cache_fg_no_inv[i] = dist_fg;
                    cost_cache_bg_no_inv[i] = dist_bg;
                    
                    // For inverted case:
                    // If glyph is inverted, then bitmap 0 -> FG, bitmap 1 -> BG.
                    // But we iterate glyph bitmap. 
                    // If inverted: 
                    //   raw 1 -> visual 0 (BG) -> cost dist_bg
                    //   raw 0 -> visual 1 (FG) -> cost dist_fg
                    
                    cost_cache_fg_inv[i] = dist_bg;
                    cost_cache_bg_inv[i] = dist_fg;
                }

                for (gi, fglyph) in glyphs.iter().enumerate() {
                    let mut cost_internal_no_inv = 0u64;
                    let mut cost_internal_inv = 0u64;
                    
                    // Optimized inner loop: No math, just lookups
                    for i in 0..bp {
                        if fglyph.bitmap[i] == 1 {
                            cost_internal_no_inv += cost_cache_fg_no_inv[i];
                            cost_internal_inv += cost_cache_fg_inv[i];
                        } else {
                            cost_internal_no_inv += cost_cache_bg_no_inv[i];
                            cost_internal_inv += cost_cache_bg_inv[i];
                        }
                    }
                    
                    // Continuity
                    let mut boundary_cost_no_inv = 0u64;
                    if smooth_weight > 0.0 {
                        if row_idx > 0 {
                            for x in 0..gw { if fglyph.top[x] != top_neighbor_bits[x] { boundary_cost_no_inv += 1; } }
                        }
                        if col_idx > 0 {
                             for y in 0..gh { if fglyph.left[y] != left_neighbor_bits[y] { boundary_cost_no_inv += 1; } }
                        }
                    }
                    let boundary_val_no_inv = boundary_cost_no_inv as f64 * continuity_scalar;
                    let total_no_inv = cost_internal_no_inv as f64 + (boundary_val_no_inv * smooth_weight as f64);
                    
                    if total_no_inv < (best_cost as f64) {
                        best_cost = total_no_inv as u64;
                        best_glyph_idx = gi;
                        best_chosen_inv = false;
                    }
                    
                    let mut boundary_cost_inv = 0u64;
                    if smooth_weight > 0.0 {
                        if row_idx > 0 {
                            for x in 0..gw { if (1 - fglyph.top[x]) != top_neighbor_bits[x] { boundary_cost_inv += 1; } }
                        }
                        if col_idx > 0 {
                             for y in 0..gh { if (1 - fglyph.left[y]) != left_neighbor_bits[y] { boundary_cost_inv += 1; } }
                        }
                    }
                    let boundary_val_inv = boundary_cost_inv as f64 * continuity_scalar;
                    let total_inv = cost_internal_inv as f64 + (boundary_val_inv * smooth_weight as f64);
                    
                    if total_inv < (best_cost as f64) {
                        best_cost = total_inv as u64;
                        best_glyph_idx = gi;
                        best_chosen_inv = true;
                    }
                    
                    if best_cost == 0 { break; }
                }

                if best_cost < best_overall_cost {
                    best_overall_cost = best_cost;
                    let glyph_char = glyphs[best_glyph_idx].ch;
                    final_glyph = glyph_char;
                    best_match_data = (best_glyph_idx, best_chosen_inv, locked_fg, locked_bg);
                    best_scale_is_identity = *sx == 1.0 && *sy == 1.0;
                }
            }

            if !best_scale_is_identity {
                dynamic_scaling_count += 1;
            }

            // --- Apply Best Result ---
            let (best_glyph_idx, best_chosen_inv, best_chosen_fg_color, best_chosen_bg_color) = best_match_data;
            let fglyph = &glyphs[best_glyph_idx];
            
            // Update Neighbors
            left_neighbor_right_bits.clear();
            for y in 0..gh {
                let bit = if best_chosen_inv { 1 - fglyph.right[y] } else { fglyph.right[y] };
                left_neighbor_right_bits.push(bit);
            }

            let mut bottom_edge = Vec::with_capacity(gw);
            for x in 0..gw {
                let bit = if best_chosen_inv { 1 - fglyph.bottom[x] } else { fglyph.bottom[x] };
                bottom_edge.push(bit);
            }
            prev_row_bottom_bits[col_idx] = bottom_edge;

            // Map Colors
            match render {
                Render::Ansi24 => {
                    final_fg = Colour::RGB(best_chosen_fg_color);
                    final_bg = if args.fg_only { None } else { Some(Colour::RGB(best_chosen_bg_color)) };
                },
                Render::Ansi => {
                    let fg_u32 = make_rgb_u32(&best_chosen_fg_color);
                    let fg_idx = nearest_hex_colour_fast(fg_u32, &ANSI256);
                    final_fg = Colour::Index(fg_idx);
                    final_bg = if args.fg_only { None } else {
                        let bg_u32 = make_rgb_u32(&best_chosen_bg_color);
                        let bg_idx = nearest_hex_colour_fast(bg_u32, &ANSI256);
                        Some(Colour::Index(bg_idx))
                    };
                },
                Render::Irc => {
                    let fg_u32 = make_rgb_u32(&best_chosen_fg_color);
                    let fg_idx = nearest_hex_colour_fast(fg_u32, &IRC99);
                    final_fg = Colour::Index(fg_idx);
                    final_bg = if args.fg_only { None } else {
                        let bg_u32 = make_rgb_u32(&best_chosen_bg_color);
                        let bg_idx = nearest_hex_colour_fast(bg_u32, &IRC99);
                        Some(Colour::Index(bg_idx))
                    };
                }
            }

            // --- Error Calculation ---
            // Using ORIGINAL pixels (original_pixels_rgb)
            let mut block_error = 0u64;
            let inv = best_chosen_inv;
            let fg_p = PrePixel { 
                r: best_chosen_fg_color[0] as u64, g: best_chosen_fg_color[1] as u64, b: best_chosen_fg_color[2] as u64, 
                sq: (best_chosen_fg_color[0] as u64).pow(2) + (best_chosen_fg_color[1] as u64).pow(2) + (best_chosen_fg_color[2] as u64).pow(2) 
            };
            let bg_p = PrePixel { 
                r: best_chosen_bg_color[0] as u64, g: best_chosen_bg_color[1] as u64, b: best_chosen_bg_color[2] as u64, 
                sq: (best_chosen_bg_color[0] as u64).pow(2) + (best_chosen_bg_color[1] as u64).pow(2) + (best_chosen_bg_color[2] as u64).pow(2) 
            };
            
            for i in 0..bp {
                let px = &original_pixels_rgb[i];
                let dist_fg = (px.sq + fg_p.sq).saturating_sub(2 * (px.r * fg_p.r + px.g * fg_p.g + px.b * fg_p.b));
                let dist_bg = (px.sq + bg_p.sq).saturating_sub(2 * (px.r * bg_p.r + px.g * bg_p.g + px.b * bg_p.b));
                
                let bit = fglyph.bitmap[i] ^ (inv as u8);
                if bit == 1 {
                    if dist_bg < dist_fg { block_error += 1; }
                } else {
                    if dist_fg < dist_bg { block_error += 1; }
                }
            }
            total_error += block_error;

            let fg_rgb = match final_fg {
                Colour::RGB(c) => c,
                Colour::Index(i) => {
                    let hex = match render {
                        Render::Irc => IRC99[i as usize % 99],
                        _ => ANSI256[i as usize],
                    };
                    unpack_rgb(hex)
                }
            };
            let bg_rgb = final_bg.clone().map(|c| match c {
                Colour::RGB(c) => c,
                Colour::Index(i) => {
                    let hex = match render {
                        Render::Irc => IRC99[i as usize % 99],
                        _ => ANSI256[i as usize],
                    };
                    unpack_rgb(hex)
                }
            });
            grid_row.push(RenderedCell { char: final_glyph, fg: fg_rgb, bg: bg_rgb });

            emit_colourized(
                &mut out,
                render,
                final_fg,
                final_bg,
                final_glyph,
                &mut first,
                &mut last_fg,
                &mut last_bg,
            );
        }
        out.push_str(match render {
            Render::Ansi | Render::Ansi24 => "\x1b[0m\n",
            Render::Irc => "\x0f\n",
        });
        grid.push(grid_row);
    }
    RenderResult {
        content: out.trim_end_matches('\n').into(),
        error_count: total_error,
        total_pixels: total_pixels,
        dynamic_scaling_count,
        grid,
    }
}


fn get_transformed_block_pixels_into(
    orig_pixels: &[PrePixel],
    gw: usize,
    gh: usize,
    scale_x: f32,
    scale_y: f32,
    out: &mut Vec<PrePixel>
) {
    out.clear();
    let cx = gw as f32 / 2.0;
    let cy = gh as f32 / 2.0;

    for y in 0..gh {
        for x in 0..gw {
            // Map target (x,y) to source (u,v)
            let u = (x as f32 - cx) / scale_x + cx;
            let v = (y as f32 - cy) / scale_y + cy;

            // Clamping to edges (0.0 to limit)
            let u_clamped = u.clamp(0.0, (gw - 1) as f32);
            let v_clamped = v.clamp(0.0, (gh - 1) as f32);
            
            let x0 = u_clamped.floor() as usize;
            let x1 = (x0 + 1).min(gw - 1);
            let y0 = v_clamped.floor() as usize;
            let y1 = (y0 + 1).min(gh - 1);
            
            let wx = u_clamped - x0 as f32;
            let wy = v_clamped - y0 as f32;
            
            let idx00 = y0 * gw + x0;
            let idx10 = y0 * gw + x1;
            let idx01 = y1 * gw + x0;
            let idx11 = y1 * gw + x1;
            
            let p00 = &orig_pixels[idx00];
            let p10 = &orig_pixels[idx10];
            let p01 = &orig_pixels[idx01];
            let p11 = &orig_pixels[idx11];

            let lerp = |a: u64, b: u64, t: f32| (a as f32 * (1.0 - t) + b as f32 * t);
            
            let r_top = lerp(p00.r, p10.r, wx);
            let r_bot = lerp(p01.r, p11.r, wx);
            let r = lerp(r_top as u64, r_bot as u64, wy) as u64;

            let g_top = lerp(p00.g, p10.g, wx);
            let g_bot = lerp(p01.g, p11.g, wx);
            let g = lerp(g_top as u64, g_bot as u64, wy) as u64;

            let b_top = lerp(p00.b, p10.b, wx);
            let b_bot = lerp(p01.b, p11.b, wx);
            let b = lerp(b_top as u64, b_bot as u64, wy) as u64;
            
            out.push(PrePixel {
                r, g, b,
                sq: r*r + g*g + b*b
            });
        }
    }
}
