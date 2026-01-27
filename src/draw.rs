use crate::args::{Args, Render};
use crate::font::GlyphStore;
use crate::palette::{IRC99, ANSI256};
use photon_rs::PhotonImage;
use image::{ImageBuffer, Rgb};

use std::collections::HashMap;
use std::iter::repeat;
use std::sync::Mutex;
use once_cell::sync::Lazy;

struct FastGlyph {
    ch: char,
    bitmap: Vec<u8>,
    edge_bitmap: Vec<u8>,
    // Boundary signatures for continuity (smooth mode)
    top: Vec<u8>,
    bottom: Vec<u8>,
    left: Vec<u8>,
    right: Vec<u8>,
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
fn nearest_hex_colour_fast(col: u32, palette: &[u32]) -> u8 {
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
#[inline] fn make_rgb_u32(px: &[u8]) -> u32 {
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
enum Colour { Index(u8), RGB([u8;3]) }

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

fn pick_colour(pixel: &AnsiPixel, r: Render, args: &Args) -> Colour {
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

fn emit_colourized(
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

/// Render an image as block‐glyph ANSI/IRC art, using only the glyph groups in `args.blocks`.
pub fn render_blocks(
    image: &AnsiImage,
    args: &Args,
    render: Render,
) -> RenderResult {
    if args.smooth.is_some() {
        return render_blocks_smooth(image, args, render);
    }

    if image.glyphs.glyphs.is_empty() {
        return "Error: no glyphs available".into();
    }

    let bg_opt_rgb = args.bg_opt.as_ref().map(|s| unpack_rgb(parse_hex_color(s)));

    let (gh, gw) = image.glyphs.metrics;
    let bp = gh * gw;

    let glyphs: Vec<FastGlyph> = image
        .glyphs
        .glyphs
        .iter()
        .filter(|(ch, _)| image.glyphs.selected.contains(ch))
        .filter(|(ch, _)| {
            if *ch == ' ' && !args.blocks.is_empty() {
                return args.blocks.iter().any(|block| {
                    image
                        .glyphs
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
        .collect();

    if glyphs.is_empty() {
        return "Error: no glyphs selected".into();
    }
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
            
            // We need to keep track of the BEST match across all scales
            // and the pixel data associated with it (for error calc, if we want strictness, 
            // but error calc usually runs on UN-SCALED pixels to be fair? 
            // Actually, if we distort the image to match, the error should probably be against the original?
            // But dynamic scaling implies "maybe the image really meant this".
            // Let's calculate error against the ORIGINAL pixels for consistency, 
            // OR the transformed ones.
            // PROMPT: "try to rescale local blocks to find better matching glyphs"
            // Usually this means we want to find the glyph that LOOKS best.
            // I will use the *transformed* pixels for matching, but for the final "Bitmap Error" metric
            // used by the global optimizer, we should probably compare against the ORIGINAL pixels
            // to avoid "cheating" by shrinking the block to a solid color.
            // So: Match using Transformed. Report Error using Original.
            
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
                let working_pixels = if *sx == 1.0 && *sy == 1.0 {
                    original_pixels_rgb.clone()
                } else {
                    get_transformed_block_pixels(&original_pixels_rgb, gw, gh, *sx, *sy)
                };

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
                        // Recalculate edges for transformed pixels
                        let threshold_sq = 60 * 60 * 3; 
                        for y in 0..gh {
                            for x in 0..gw {
                                let i = y * gw + x;
                                let p = &working_pixels[i];
                                let mut is_edge = false;
                                let check = |idx2: usize| -> bool {
                                    let p2 = &working_pixels[idx2];
                                    let dist = (p.r as i64 - p2.r as i64).pow(2) + 
                                               (p.g as i64 - p2.g as i64).pow(2) + 
                                               (p.b as i64 - p2.b as i64).pow(2);
                                    dist as u64 > threshold_sq
                                };
                                if x > 0 && check(i - 1) { is_edge = true; }
                                else if x + 1 < gw && check(i + 1) { is_edge = true; }
                                else if y > 0 && check(i - gw) { is_edge = true; }
                                else if y + 1 < gh && check(i + gw) { is_edge = true; }
                                current_block_edges[i] = if is_edge { 1 } else { 0 };
                            }
                        }
                    }

                    let mut best_cost = u64::MAX;
                    let mut best_glyph_idx = 0;
                    let mut best_chosen_fg_color = [0u8; 3];
                    let mut best_chosen_bg_color = [0u8; 3];
                    let mut best_inv = false;

                    for (gi, fglyph) in glyphs.iter().enumerate() {
                        let mut sum_ones = PrePixel { r:0, g:0, b:0, sq:0 };
                        let mut count_ones = 0u64;
                        let mut sum_zeros = PrePixel { r:0, g:0, b:0, sq:0 };
                        let mut count_zeros = 0u64;

                        for i in 0..bp {
                            let px = &working_pixels[i];
                            if fglyph.bitmap[i] == 1 {
                                sum_ones.r += px.r; sum_ones.g += px.g; sum_ones.b += px.b; sum_ones.sq += px.sq;
                                count_ones += 1;
                            } else {
                                sum_zeros.r += px.r; sum_zeros.g += px.g; sum_zeros.b += px.b; sum_zeros.sq += px.sq;
                                count_zeros += 1;
                            }
                        }

                        let avg_ones = if count_ones > 0 {
                            [(sum_ones.r / count_ones) as u8, (sum_ones.g / count_ones) as u8, (sum_ones.b / count_ones) as u8]
                        } else { [0,0,0] };
                        let avg_zeros = if count_zeros > 0 {
                            [(sum_zeros.r / count_zeros) as u8, (sum_zeros.g / count_zeros) as u8, (sum_zeros.b / count_zeros) as u8]
                        } else { [0,0,0] };

                        let bg_ref = if let Some(bg_c) = bg_opt_rgb { Some(bg_c) } else { None };
                        let use_coverage_heuristic = args.fg_only && bg_ref.is_none();
                        let penalty_per_miss = 1000u64;
                        
                        let smooth_weight = args.smooth.unwrap_or(0.0).clamp(0.0, 1.0);
                        let color_weight = 1.0 - smooth_weight;
                        let continuity_scalar = 195_075.0;

                        // Cost Calculation (Normal)
                        let cost_no_invert = if use_coverage_heuristic {
                             let term1 = sum_ones.sq;
                             let term2 = 2 * (avg_ones[0] as u64 * sum_ones.r + avg_ones[1] as u64 * sum_ones.g + avg_ones[2] as u64 * sum_ones.b);
                             let term3 = count_ones * (avg_ones[0] as u64 * avg_ones[0] as u64 + avg_ones[1] as u64 * avg_ones[1] as u64 + avg_ones[2] as u64 * avg_ones[2] as u64);
                             (term1 + term3).saturating_sub(term2) + (count_zeros * penalty_per_miss)
                        } else {
                            let target_bg = bg_ref.unwrap_or(avg_zeros);
                            let t1_1 = sum_ones.sq;
                            let t2_1 = 2 * (avg_ones[0] as u64 * sum_ones.r + avg_ones[1] as u64 * sum_ones.g + avg_ones[2] as u64 * sum_ones.b);
                            let t3_1 = count_ones * (avg_ones[0] as u64 * avg_ones[0] as u64 + avg_ones[1] as u64 * avg_ones[1] as u64 + avg_ones[2] as u64 * avg_ones[2] as u64);
                            let c1 = (t1_1 + t3_1).saturating_sub(t2_1);
                            let t1_0 = sum_zeros.sq;
                            let t2_0 = 2 * (target_bg[0] as u64 * sum_zeros.r + target_bg[1] as u64 * sum_zeros.g + target_bg[2] as u64 * sum_zeros.b);
                            let t3_0 = count_zeros * (target_bg[0] as u64 * target_bg[0] as u64 + target_bg[1] as u64 * target_bg[1] as u64 + target_bg[2] as u64 * target_bg[2] as u64);
                            let c0 = (t1_0 + t3_0).saturating_sub(t2_0);
                            c1 + c0
                        };

                        let total_cost_no_invert = if smooth_weight > 0.0 {
                            let mut hamming = 0u64;
                            if let Some((ln_glyph, ln_inv)) = left_neighbor_data {
                                for y in 0..gh {
                                    let ln_bit = if ln_inv { 1 - ln_glyph.right[y] } else { ln_glyph.right[y] };
                                    if ln_bit != fglyph.left[y] { hamming += 1; }
                                }
                            }
                            if let Some((tn_glyph, tn_inv)) = top_neighbor_data {
                                for x in 0..gw {
                                    let tn_bit = if tn_inv { 1 - tn_glyph.bottom[x] } else { tn_glyph.bottom[x] };
                                    if tn_bit != fglyph.top[x] { hamming += 1; }
                                }
                            }
                            let continuity_cost = (hamming as f64 * continuity_scalar) as u64;
                            (cost_no_invert as f64 * color_weight as f64 + continuity_cost as f64 * smooth_weight as f64) as u64
                        } else {
                            cost_no_invert
                        };

                        if total_cost_no_invert < best_cost {
                            best_cost = total_cost_no_invert;
                            best_glyph_idx = gi;
                            best_chosen_fg_color = avg_ones;
                            best_chosen_bg_color = if use_coverage_heuristic { [0,0,0] } else { bg_ref.unwrap_or(avg_zeros) };
                            best_inv = false;
                        }

                        // Cost Calculation (Inverted)
                        let cost_invert = if use_coverage_heuristic {
                             let t1_0 = sum_zeros.sq;
                             let t2_0 = 2 * (avg_zeros[0] as u64 * sum_zeros.r + avg_zeros[1] as u64 * sum_zeros.g + avg_zeros[2] as u64 * sum_zeros.b);
                             let t3_0 = count_zeros * (avg_zeros[0] as u64 * avg_zeros[0] as u64 + avg_zeros[1] as u64 * avg_zeros[1] as u64 + avg_zeros[2] as u64 * avg_zeros[2] as u64);
                             (t1_0 + t3_0).saturating_sub(t2_0) + (count_ones * penalty_per_miss)
                        } else {
                             let target_bg = bg_ref.unwrap_or(avg_ones);
                             let t1_0 = sum_zeros.sq;
                             let t2_0 = 2 * (avg_zeros[0] as u64 * sum_zeros.r + avg_zeros[1] as u64 * sum_zeros.g + avg_zeros[2] as u64 * sum_zeros.b);
                             let t3_0 = count_zeros * (avg_zeros[0] as u64 * avg_zeros[0] as u64 + avg_zeros[1] as u64 * avg_zeros[1] as u64 + avg_zeros[2] as u64 * avg_zeros[2] as u64);
                             let c_fg = (t1_0 + t3_0).saturating_sub(t2_0);
                             let t1_1 = sum_ones.sq;
                             let t2_1 = 2 * (target_bg[0] as u64 * sum_ones.r + target_bg[1] as u64 * sum_ones.g + target_bg[2] as u64 * sum_ones.b);
                             let t3_1 = count_ones * (target_bg[0] as u64 * target_bg[0] as u64 + target_bg[1] as u64 * target_bg[1] as u64 + target_bg[2] as u64 * target_bg[2] as u64);
                             let c_bg = (t1_1 + t3_1).saturating_sub(t2_1);
                             c_fg + c_bg
                        };

                        let total_cost_invert = if smooth_weight > 0.0 {
                            let mut hamming = 0u64;
                            if let Some((ln_glyph, ln_inv)) = left_neighbor_data {
                                for y in 0..gh {
                                    let ln_bit = if ln_inv { 1 - ln_glyph.right[y] } else { ln_glyph.right[y] };
                                    if ln_bit != (1 - fglyph.left[y]) { hamming += 1; }
                                }
                            }
                            if let Some((tn_glyph, tn_inv)) = top_neighbor_data {
                                for x in 0..gw {
                                    let tn_bit = if tn_inv { 1 - tn_glyph.bottom[x] } else { tn_glyph.bottom[x] };
                                    if tn_bit != (1 - fglyph.top[x]) { hamming += 1; }
                                }
                            }
                            let continuity_cost = (hamming as f64 * continuity_scalar) as u64;
                            (cost_invert as f64 * color_weight as f64 + continuity_cost as f64 * smooth_weight as f64) as u64
                        } else {
                            cost_invert
                        };

                        if total_cost_invert < best_cost {
                            best_cost = total_cost_invert;
                            best_glyph_idx = gi;
                            best_chosen_fg_color = avg_zeros;
                            best_chosen_bg_color = if use_coverage_heuristic { [0,0,0] } else { bg_ref.unwrap_or(avg_ones) };
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
                    // Convert working pixels to indices
                    current_block_indices.clear();
                    for i in 0..bp {
                        let px_rgb = &working_pixels[i];
                        // Convert to u32 for cache lookup
                        let rgb_u32 = ((px_rgb.r as u32) << 16) | ((px_rgb.g as u32) << 8) | (px_rgb.b as u32);
                        // We use a simplified mapping here, using cache if possible but we have RGB.
                        // We need to map RGB -> Index.
                        // We can use nearest_hex_colour_fast.
                        // Note: args.grayscale_tolerance is checked against the pixel.
                        
                        // Check if pixel is distinct from gray
                        let min_val = px_rgb.r.min(px_rgb.g.min(px_rgb.b));
                        let max_val = px_rgb.r.max(px_rgb.g.max(px_rgb.b));
                        let is_gray = max_val.saturating_sub(min_val) <= args.grayscale_tolerance as u64;
                        
                        let idx = match render {
                            Render::Ansi => {
                                let std = nearest_hex_colour_fast(rgb_u32, &ANSI256);
                                if args.nograyscale && !is_gray {
                                    nearest_distinctly_chromatic_hex_colour(rgb_u32, &ANSI256, args.grayscale_tolerance).unwrap_or(std)
                                } else { std }
                            },
                            Render::Irc => {
                                let std = nearest_hex_colour_fast(rgb_u32, &IRC99);
                                if args.nograyscale && !is_gray {
                                    nearest_distinctly_chromatic_hex_colour(rgb_u32, &IRC99, args.grayscale_tolerance).unwrap_or(std)
                                } else { std }
                            },
                            _ => 0,
                        };
                        current_block_indices.push(idx);
                    }

                    let bg_opt_idx = if let Some(bg_rgb) = bg_opt_rgb {
                        let bg_u32 = make_rgb_u32(&bg_rgb);
                        match render {
                            Render::Ansi => Some(nearest_hex_colour_fast(bg_u32, &ANSI256) as usize),
                            Render::Irc => Some(nearest_hex_colour_fast(bg_u32, &IRC99) as usize),
                            _ => None,
                        }
                    } else { None };

                    let mut best = (usize::MAX, 0usize, 0u8, 0u8, false);
                    let smooth_weight = args.smooth.unwrap_or(0.0).clamp(0.0, 1.0);
                    let color_weight = 1.0 - smooth_weight;
                    let edge_penalty_idx = 1000usize;

                    for (gi, fglyph) in glyphs.iter().enumerate() {
                        for &inv in &[false, true] {
                            let mut fg_tot: usize = 0;
                            let mut bg_tot: usize = 0;
                            let mut fg_cnt = [0usize; 256];
                            let mut bg_cnt = [0usize; 256];

                            for i in 0..bp {
                                let col_idx = current_block_indices[i] as usize;
                                if fglyph.bitmap[i] ^ (inv as u8) == 1 {
                                    fg_tot += 1;
                                    fg_cnt[col_idx] += 1;
                                } else {
                                    bg_tot += 1;
                                    bg_cnt[col_idx] += 1;
                                }
                            }

                            let (fgi, fgm_val) = fg_cnt.iter().enumerate().max_by_key(|&(_, c)| c).map(|(i, c)| (i, *c)).unwrap_or((0,0));
                            let (bgi_std, bgm_effective) = if let Some(idx) = bg_opt_idx {
                                (idx, bg_cnt[idx])
                            } else {
                                bg_cnt.iter().enumerate().max_by_key(|&(_, c)| c).map(|(i, c)| (i, *c)).unwrap_or((0,0))
                            };

                            let cost_base = if args.fg_only && bg_opt_idx.is_none() {
                                let total = bp;
                                let score = (2 * fgm_val as isize) - (fg_tot as isize);
                                ((total as isize) - score) as usize
                            } else {
                                (fg_tot.saturating_sub(fgm_val)) + (bg_tot.saturating_sub(bgm_effective))
                            };

                            let mut extra_cost = 0usize;
                            if smooth_weight > 0.0 {
                                if args.smooth.is_some() {
                                    for i in 0..bp {
                                        // current_block_edges calculated on original pixels? Or scaled?
                                        // For indexed we don't have easy edge detection on indices.
                                        // Skipping edge check for indexed dynamic scaling for now or use glyph edge?
                                        // Using fglyph.edge_bitmap vs nothing.
                                    }
                                }
                                let mut hamming = 0usize;
                                if let Some((ln_glyph, ln_inv)) = left_neighbor_data {
                                    for y in 0..gh {
                                        let ln_bit = if ln_inv { 1 - ln_glyph.right[y] } else { ln_glyph.right[y] };
                                        let curr_bit = if inv { 1 - fglyph.left[y] } else { fglyph.left[y] };
                                        if ln_bit != curr_bit { hamming += 1; }
                                    }
                                }
                                if let Some((tn_glyph, tn_inv)) = top_neighbor_data {
                                    for x in 0..gw {
                                        let tn_bit = if tn_inv { 1 - tn_glyph.bottom[x] } else { tn_glyph.bottom[x] };
                                        let curr_bit = if inv { 1 - fglyph.top[x] } else { fglyph.top[x] };
                                        if tn_bit != curr_bit { hamming += 1; }
                                    }
                                }
                                extra_cost += hamming;
                            }

                            let cost = (cost_base as f32 * color_weight + extra_cost as f32 * smooth_weight) as usize;
                            if cost < best.0 {
                                best = (cost, gi, fgi as u8, bgi_std as u8, inv);
                                if cost == 0 { break; }
                            }
                        }
                        if best.0 == 0 { break; }
                    }

                    if (best.0 as u64) < best_overall_cost {
                        best_overall_cost = best.0 as u64;
                        chosen_glyph_index = best.1;
                        let (fg_idx, bg_idx) = if best.4 { (best.3, best.2) } else { (best.2, best.3) };
                        chosen_glyph_inverted = best.4;
                        final_glyph = glyphs[best.1].ch;
                        final_fg = Colour::Index(fg_idx);
                        final_bg = if args.fg_only { None } else { Some(Colour::Index(bg_idx)) };
                        best_match_data = (best.1, best.4, [0,0,0], [0,0,0], fg_idx, bg_idx);
                        best_scale_is_identity = *sx == 1.0 && *sy == 1.0;
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
                    
                    if bit == 1 { if dist_bg < dist_fg { block_error += 1; } }
                    else { if !args.fg_only { if dist_fg < dist_bg { block_error += 1; } } }
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
                    if bit == 1 { if col_idx != fg_idx { block_error += 1; } }
                    else { if !args.fg_only { if col_idx != bg_idx { block_error += 1; } } }
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
    args: &Args,
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

pub fn save_as_png(result: &RenderResult, glyphs: &GlyphStore, path: &str) -> Result<(), image::ImageError> {
    if result.grid.is_empty() { return Ok(()); }
    let rows = result.grid.len();
    let cols = result.grid[0].len();
    let (gh, gw) = glyphs.metrics; 
    
    let img_width = (cols * gw) as u32;
    let img_height = (rows * gh) as u32;
    
    let mut img = ImageBuffer::new(img_width, img_height);
    
    // Build quick lookup map
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
                            if let Some(bg) = cell.bg { Rgb(bg) } else { Rgb([0, 0, 0]) }
                        };
                        
                        let target_x = start_x + x as u32;
                        let target_y = start_y + y as u32;
                        if target_x < img_width && target_y < img_height {
                            img.put_pixel(target_x, target_y, px);
                        }
                    }
                }
            } else {
                // If glyph not found (e.g. braille or space if missing), fill with BG
                // Space is usually in store. Braille chars are not.
                // For Braille, we might want to handle it or skip.
                // Assuming block mode mainly.
                if let Some(bg) = cell.bg {
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
    }
    
    img.save(path)
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

fn render_blocks_smooth(
    image: &AnsiImage,
    args: &Args,
    render: Render,
) -> RenderResult {
    if image.glyphs.glyphs.is_empty() {
        return "Error: no glyphs available".into();
    }

    let bg_opt_rgb = args.bg_opt.as_ref().map(|s| unpack_rgb(parse_hex_color(s)));
    let (gh, gw) = image.glyphs.metrics;
    let bp = gh * gw;

    // Pre-calculate blocks width/height
    let blocks_width = if !image.block.is_empty() { image.block[0].len() } else { 0 };

    let glyphs: Vec<FastGlyph> = image
        .glyphs
        .glyphs
        .iter()
        .filter(|(ch, _)| image.glyphs.selected.contains(ch))
        .filter(|(ch, _)| {
            if *ch == ' ' && !args.blocks.is_empty() {
                return args.blocks.iter().any(|block| {
                    image
                        .glyphs
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
            
            let mut top = Vec::with_capacity(gw);
            let mut bottom = Vec::with_capacity(gw);
            let mut left = Vec::with_capacity(gh);
            let mut right = Vec::with_capacity(gh);

            for x in 0..gw {
                top.push(bitmap[x]);
                bottom.push(bitmap[(gh - 1) * gw + x]);
            }
            for y in 0..gh {
                left.push(bitmap[y * gw]);
                right.push(bitmap[y * gw + (gw - 1)]);
            }

            FastGlyph { 
                ch: *ch, 
                bitmap, 
                edge_bitmap: vec![],
                top, bottom, left, right 
            }
        })
        .collect();

    if glyphs.is_empty() {
        return "Error: no glyphs selected".into();
    }

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
                let working_pixels = if *sx == 1.0 && *sy == 1.0 {
                    original_pixels_rgb.clone()
                } else {
                    get_transformed_block_pixels(&original_pixels_rgb, gw, gh, *sx, *sy)
                };
                
                // Note: We update 'current_block_pixels_rgb' because the loop body logic might use it?
                // The loop body below uses 'current_block_pixels_rgb'. 
                // Wait, in my previous replacement for render_blocks I used 'working_pixels'.
                // Here I should update 'current_block_pixels_rgb' to be 'working_pixels' so I don't have to rewrite the whole inner body references.
                current_block_pixels_rgb = working_pixels;

                // --- 1. Find Optimal Colors (Standard "Non-Smooth" Pass) ---
                let mut std_best_cost = u64::MAX;
                let mut locked_fg = [0u8; 3];
                let mut locked_bg = [0u8; 3];
                
                let mut sum_ones;
                let mut sum_zeros;

                for fglyph in glyphs.iter() {
                    sum_ones = PrePixel { r:0, g:0, b:0, sq:0 };
                    sum_zeros = PrePixel { r:0, g:0, b:0, sq:0 };
                    let mut count_ones = 0u64;
                    let mut count_zeros = 0u64;

                    for i in 0..bp {
                        let px = &current_block_pixels_rgb[i];
                        if fglyph.bitmap[i] == 1 {
                            sum_ones.r += px.r; sum_ones.g += px.g; sum_ones.b += px.b; sum_ones.sq += px.sq;
                            count_ones += 1;
                        } else {
                            sum_zeros.r += px.r; sum_zeros.g += px.g; sum_zeros.b += px.b; sum_zeros.sq += px.sq;
                            count_zeros += 1;
                        }
                    }

                    let avg_ones = if count_ones > 0 {
                        [(sum_ones.r / count_ones) as u8, (sum_ones.g / count_ones) as u8, (sum_ones.b / count_ones) as u8]
                    } else { [0,0,0] };
                    let avg_zeros = if count_zeros > 0 {
                        [(sum_zeros.r / count_zeros) as u8, (sum_zeros.g / count_zeros) as u8, (sum_zeros.b / count_zeros) as u8]
                    } else { [0,0,0] };

                    let bg_ref = if let Some(bg_c) = bg_opt_rgb { Some(bg_c) } else { None };
                    let use_coverage_heuristic = args.fg_only && bg_ref.is_none();
                    let penalty_per_miss = 1000u64;

                    let cost_no_inv = if use_coverage_heuristic {
                         let term1 = sum_ones.sq;
                         let term2 = 2 * (avg_ones[0] as u64 * sum_ones.r + avg_ones[1] as u64 * sum_ones.g + avg_ones[2] as u64 * sum_ones.b);
                         let term3 = count_ones * (avg_ones[0] as u64 * avg_ones[0] as u64 + avg_ones[1] as u64 * avg_ones[1] as u64 + avg_ones[2] as u64 * avg_ones[2] as u64);
                         (term1 + term3).saturating_sub(term2) + (count_zeros * penalty_per_miss)
                    } else {
                        let target_bg = bg_ref.unwrap_or(avg_zeros);
                        let t1_1 = sum_ones.sq;
                        let t2_1 = 2 * (avg_ones[0] as u64 * sum_ones.r + avg_ones[1] as u64 * sum_ones.g + avg_ones[2] as u64 * sum_ones.b);
                        let t3_1 = count_ones * (avg_ones[0] as u64 * avg_ones[0] as u64 + avg_ones[1] as u64 * avg_ones[1] as u64 + avg_ones[2] as u64 * avg_ones[2] as u64);
                        let cost_ones = (t1_1 + t3_1).saturating_sub(t2_1);
                        let t1_0 = sum_zeros.sq;
                        let t2_0 = 2 * (target_bg[0] as u64 * sum_zeros.r + target_bg[1] as u64 * sum_zeros.g + target_bg[2] as u64 * sum_zeros.b);
                        let t3_0 = count_zeros * (target_bg[0] as u64 * target_bg[0] as u64 + target_bg[1] as u64 * target_bg[1] as u64 + target_bg[2] as u64 * target_bg[2] as u64);
                        let cost_zeros = (t1_0 + t3_0).saturating_sub(t2_0);
                        cost_ones + cost_zeros
                    };

                    if cost_no_inv < std_best_cost {
                        std_best_cost = cost_no_inv;
                        locked_fg = avg_ones;
                        locked_bg = if use_coverage_heuristic { [0,0,0] } else { bg_ref.unwrap_or(avg_zeros) };
                    }

                    let cost_inv = if use_coverage_heuristic {
                         let t1_0 = sum_zeros.sq;
                         let t2_0 = 2 * (avg_zeros[0] as u64 * sum_zeros.r + avg_zeros[1] as u64 * sum_zeros.g + avg_zeros[2] as u64 * sum_zeros.b);
                         let t3_0 = count_zeros * (avg_zeros[0] as u64 * avg_zeros[0] as u64 + avg_zeros[1] as u64 * avg_zeros[1] as u64 + avg_zeros[2] as u64 * avg_zeros[2] as u64);
                         (t1_0 + t3_0).saturating_sub(t2_0) + (count_ones * penalty_per_miss)
                    } else {
                        let target_bg = bg_ref.unwrap_or(avg_ones);
                        let t1_0 = sum_zeros.sq;
                        let t2_0 = 2 * (avg_zeros[0] as u64 * sum_zeros.r + avg_zeros[1] as u64 * sum_zeros.g + avg_zeros[2] as u64 * sum_zeros.b);
                        let t3_0 = count_zeros * (avg_zeros[0] as u64 * avg_zeros[0] as u64 + avg_zeros[1] as u64 * avg_zeros[1] as u64 + avg_zeros[2] as u64 * avg_zeros[2] as u64);
                        let cost_fg = (t1_0 + t3_0).saturating_sub(t2_0);
                        let t1_1 = sum_ones.sq;
                        let t2_1 = 2 * (target_bg[0] as u64 * sum_ones.r + target_bg[1] as u64 * sum_ones.g + target_bg[2] as u64 * sum_ones.b);
                        let t3_1 = count_ones * (target_bg[0] as u64 * target_bg[0] as u64 + target_bg[1] as u64 * target_bg[1] as u64 + target_bg[2] as u64 * target_bg[2] as u64);
                        let cost_bg = (t1_1 + t3_1).saturating_sub(t2_1);
                        cost_fg + cost_bg
                    };

                    if cost_inv < std_best_cost {
                        std_best_cost = cost_inv;
                        locked_fg = avg_zeros;
                        locked_bg = if use_coverage_heuristic { [0,0,0] } else { bg_ref.unwrap_or(avg_ones) };
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
                let bg_ref = if let Some(bg_c) = bg_opt_rgb { Some(bg_c) } else { None };
                let use_coverage_heuristic = args.fg_only && bg_ref.is_none();
                let penalty_per_miss = 1000u64;

                for (gi, fglyph) in glyphs.iter().enumerate() {
                    let mut cost_internal_no_inv = 0u64;
                    let mut cost_internal_inv = 0u64;
                    
                    for i in 0..bp {
                        let px = &current_block_pixels_rgb[i];
                        let dist_fg = (px.sq + fg_p.sq).saturating_sub(2 * (px.r * fg_p.r + px.g * fg_p.g + px.b * fg_p.b));
                        let dist_bg = (px.sq + bg_p.sq).saturating_sub(2 * (px.r * bg_p.r + px.g * bg_p.g + px.b * bg_p.b));
                        
                        if fglyph.bitmap[i] == 1 {
                            cost_internal_no_inv += dist_fg;
                            if use_coverage_heuristic { cost_internal_inv += penalty_per_miss; } else { cost_internal_inv += dist_bg; }
                        } else {
                            if use_coverage_heuristic { cost_internal_no_inv += penalty_per_miss; } else { cost_internal_no_inv += dist_bg; }
                            cost_internal_inv += dist_fg;
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
                    if !args.fg_only {
                        if dist_fg < dist_bg { block_error += 1; }
                    }
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


fn get_transformed_block_pixels(
    orig_pixels: &[PrePixel],
    gw: usize,
    gh: usize,
    scale_x: f32,
    scale_y: f32
) -> Vec<PrePixel> {
    let mut out = Vec::with_capacity(gw * gh);
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
    out
}
