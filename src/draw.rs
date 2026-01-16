use crate::args::{Args, Render};
use crate::font::GlyphStore;
use crate::palette::{IRC99, ANSI256};
use photon_rs::PhotonImage;

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

/// Render an image as block‐glyph ANSI/IRC art, using only the glyph groups in `args.blocks`.
pub fn render_blocks(
    image: &AnsiImage,
    args: &Args,
    render: Render,
) -> String {
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

    // Pre-allocate buffer for pixel data to avoid repeated allocations
    let mut current_block_pixels_rgb: Vec<PrePixel> = Vec::with_capacity(bp);
    // For indexed modes
    let mut current_block_indices: Vec<u8> = Vec::with_capacity(bp);
    // For smooth mode
    let mut current_block_edges: Vec<u8> = vec![0u8; bp];

    // Track selected glyphs for continuity (smooth mode)
    let blocks_width = if !image.block.is_empty() { image.block[0].len() } else { 0 };
    let mut previous_row_glyph_indices: Vec<(usize, bool)> = vec![(usize::MAX, false); blocks_width];

    for (row_idx, block_row) in image.block.iter().enumerate() {
        let mut first = true;
        let mut last_fg: Option<Colour> = None; // Assuming Colour is in parent or use crate::image::Colour
        let mut last_bg: Option<Colour> = None;
        let mut left_neighbor_glyph_idx: (usize, bool) = (usize::MAX, false);

        for (col_idx, blk) in block_row.iter().enumerate() {
            let final_glyph: char;
            let final_fg: Colour;
            let final_bg: Option<Colour>;
            let mut chosen_glyph_index: usize = 0;
            let mut chosen_glyph_inverted: bool = false;

            // Get neighbors for continuity
            let left_neighbor_data = if left_neighbor_glyph_idx.0 != usize::MAX {
                Some((&glyphs[left_neighbor_glyph_idx.0], left_neighbor_glyph_idx.1))
            } else { None };
            
            let top_neighbor_data = if previous_row_glyph_indices[col_idx].0 != usize::MAX {
                Some((&glyphs[previous_row_glyph_indices[col_idx].0], previous_row_glyph_indices[col_idx].1))
            } else { None };

            if render == Render::Ansi24 {
                // Ansi24 specific glyph matching using direct RGB values
                current_block_pixels_rgb.clear();
                for y_idx in 0..gh {
                    for x_idx in 0..gw {
                        let px = blk.pixels[y_idx][x_idx].orig;
                        let [r, g, b] = unpack_rgb(px);
                        let r = r as u64;
                        let g = g as u64;
                        let b = b as u64;
                        current_block_pixels_rgb.push(PrePixel {
                            r, g, b,
                            sq: r*r + g*g + b*b
                        });
                    }
                }

                if args.smooth.is_some() {
                    // Calculate edges for the current block
                    // Higher threshold to avoid detecting noise as edges (which would match dither patterns)
                    let threshold_sq = 60 * 60 * 3; 
                    for y in 0..gh {
                        for x in 0..gw {
                            let i = y * gw + x;
                            let p = &current_block_pixels_rgb[i];
                            let mut is_edge = false;
                            
                            // Check neighbors
                            let check = |idx2: usize| -> bool {
                                let p2 = &current_block_pixels_rgb[idx2];
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

                for (gi, fglyph) in glyphs.iter().enumerate() {
                    let mut sum_ones = PrePixel { r:0, g:0, b:0, sq:0 };
                    let mut count_ones = 0u64;
                    let mut sum_zeros = PrePixel { r:0, g:0, b:0, sq:0 };
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
                    
                    // Smoothness weighting
                    let smooth_weight = args.smooth.unwrap_or(0.0).clamp(0.0, 1.0);
                    let color_weight = 1.0 - smooth_weight;
                    let continuity_scalar = 195_075.0; // Max per-pixel squared error in RGB

                    // --- Cost Calculation (Normal: 1=FG, 0=BG) ---
                    let cost_no_invert = if use_coverage_heuristic {
                        // Heuristic: FG(1) matches avg_ones. BG(0) is ignored but penalized.
                         let term1 = sum_ones.sq;
                         let term2 = 2 * (avg_ones[0] as u64 * sum_ones.r + avg_ones[1] as u64 * sum_ones.g + avg_ones[2] as u64 * sum_ones.b);
                         let term3 = count_ones * (avg_ones[0] as u64 * avg_ones[0] as u64 + avg_ones[1] as u64 * avg_ones[1] as u64 + avg_ones[2] as u64 * avg_ones[2] as u64);
                         let cost_fg = (term1 + term3).saturating_sub(term2);
                         cost_fg + (count_zeros * penalty_per_miss)
                    } else {
                        // Standard: FG(1)=avg_ones, BG(0)=avg_zeros (or bg_ref)
                        let target_bg = bg_ref.unwrap_or(avg_zeros);
                        
                        // Cost ones
                        let t1_1 = sum_ones.sq;
                        let t2_1 = 2 * (avg_ones[0] as u64 * sum_ones.r + avg_ones[1] as u64 * sum_ones.g + avg_ones[2] as u64 * sum_ones.b);
                        let t3_1 = count_ones * (avg_ones[0] as u64 * avg_ones[0] as u64 + avg_ones[1] as u64 * avg_ones[1] as u64 + avg_ones[2] as u64 * avg_ones[2] as u64);
                        let cost_ones = (t1_1 + t3_1).saturating_sub(t2_1);
                        
                        // Cost zeros
                        let t1_0 = sum_zeros.sq;
                        let t2_0 = 2 * (target_bg[0] as u64 * sum_zeros.r + target_bg[1] as u64 * sum_zeros.g + target_bg[2] as u64 * sum_zeros.b);
                        let t3_0 = count_zeros * (target_bg[0] as u64 * target_bg[0] as u64 + target_bg[1] as u64 * target_bg[1] as u64 + target_bg[2] as u64 * target_bg[2] as u64);
                        let cost_zeros = (t1_0 + t3_0).saturating_sub(t2_0);
                        
                        cost_ones + cost_zeros
                    };

                    let total_cost_no_invert = if smooth_weight > 0.0 {
                        // Calculate Hamming distance for continuity
                        let mut hamming = 0u64;
                        if let Some((ln_glyph, ln_inv)) = left_neighbor_data {
                            for y in 0..gh {
                                let ln_bit = if ln_inv { 1 - ln_glyph.right[y] } else { ln_glyph.right[y] };
                                let curr_bit = fglyph.left[y]; // Not inverted
                                if ln_bit != curr_bit { hamming += 1; }
                            }
                        }
                        if let Some((tn_glyph, tn_inv)) = top_neighbor_data {
                            for x in 0..gw {
                                let tn_bit = if tn_inv { 1 - tn_glyph.bottom[x] } else { tn_glyph.bottom[x] };
                                let curr_bit = fglyph.top[x]; // Not inverted
                                if tn_bit != curr_bit { hamming += 1; }
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
                    }
                    
                    if best_cost == 0 {
                         // Perfect match
                    }

                    // --- Cost Calculation (Inverted: 0=FG, 1=BG) ---
                    let cost_invert = if use_coverage_heuristic {
                         // Heuristic: FG(0) matches avg_zeros. BG(1) is ignored but penalized.
                         let t1_0 = sum_zeros.sq;
                         let t2_0 = 2 * (avg_zeros[0] as u64 * sum_zeros.r + avg_zeros[1] as u64 * sum_zeros.g + avg_zeros[2] as u64 * sum_zeros.b);
                         let t3_0 = count_zeros * (avg_zeros[0] as u64 * avg_zeros[0] as u64 + avg_zeros[1] as u64 * avg_zeros[1] as u64 + avg_zeros[2] as u64 * avg_zeros[2] as u64);
                         let cost_fg = (t1_0 + t3_0).saturating_sub(t2_0);
                         
                         cost_fg + (count_ones * penalty_per_miss)
                    } else {
                         // Standard: FG(0)=avg_zeros, BG(1)=avg_ones (or bg_ref)
                         let target_bg = bg_ref.unwrap_or(avg_ones);
                         
                         // Cost zeros (FG)
                         let t1_0 = sum_zeros.sq;
                         let t2_0 = 2 * (avg_zeros[0] as u64 * sum_zeros.r + avg_zeros[1] as u64 * sum_zeros.g + avg_zeros[2] as u64 * sum_zeros.b);
                         let t3_0 = count_zeros * (avg_zeros[0] as u64 * avg_zeros[0] as u64 + avg_zeros[1] as u64 * avg_zeros[1] as u64 + avg_zeros[2] as u64 * avg_zeros[2] as u64);
                         let cost_fg = (t1_0 + t3_0).saturating_sub(t2_0);
                         
                         // Cost ones (BG)
                         let t1_1 = sum_ones.sq;
                         let t2_1 = 2 * (target_bg[0] as u64 * sum_ones.r + target_bg[1] as u64 * sum_ones.g + target_bg[2] as u64 * sum_ones.b);
                         let t3_1 = count_ones * (target_bg[0] as u64 * target_bg[0] as u64 + target_bg[1] as u64 * target_bg[1] as u64 + target_bg[2] as u64 * target_bg[2] as u64);
                         let cost_bg = (t1_1 + t3_1).saturating_sub(t2_1);
                         
                         cost_fg + cost_bg
                    };

                    let total_cost_invert = if smooth_weight > 0.0 {
                        // Calculate Hamming distance for continuity
                        let mut hamming = 0u64;
                        if let Some((ln_glyph, ln_inv)) = left_neighbor_data {
                            for y in 0..gh {
                                let ln_bit = if ln_inv { 1 - ln_glyph.right[y] } else { ln_glyph.right[y] };
                                let curr_bit = 1 - fglyph.left[y]; // Inverted
                                if ln_bit != curr_bit { hamming += 1; }
                            }
                        }
                        if let Some((tn_glyph, tn_inv)) = top_neighbor_data {
                            for x in 0..gw {
                                let tn_bit = if tn_inv { 1 - tn_glyph.bottom[x] } else { tn_glyph.bottom[x] };
                                let curr_bit = 1 - fglyph.top[x]; // Inverted
                                if tn_bit != curr_bit { hamming += 1; }
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
                        chosen_glyph_inverted = true;
                    }

                    if best_cost == 0 {
                        break;
                    }
                }

                chosen_glyph_index = best_glyph_idx;
                let glyph_char = glyphs[best_glyph_idx].ch;
                final_glyph = glyph_char;
                final_fg = Colour::RGB(best_chosen_fg_color);
                final_bg = if args.fg_only { None } else { Some(Colour::RGB(best_chosen_bg_color)) };

            } else {
                // Palette-based glyph matching logic for Render::Ansi and Render::Irc
                current_block_indices.clear();
                for y_idx in 0..gh {
                    for x_idx in 0..gw {
                        let p = &blk.pixels[y_idx][x_idx];
                        let distinct_orig = !is_near_grayscale(p.orig, args.grayscale_tolerance);
                        let idx = match render {
                            Render::Ansi if args.nograyscale && distinct_orig => p.ansi_ng,
                            Render::Ansi => p.ansi_std,
                            Render::Irc if args.nograyscale && distinct_orig => p.irc_ng,
                            Render::Irc => p.irc_std,
                            _ => p.ansi_std, // Fallback, Ansi24 is handled above
                        };
                        current_block_indices.push(idx);
                    }
                }

                let bg_opt_idx = if let Some(bg_rgb) = bg_opt_rgb {
                    let bg_u32 = make_rgb_u32(&bg_rgb);
                    match render {
                        Render::Ansi => Some(nearest_hex_colour_fast(bg_u32, &ANSI256) as usize),
                        Render::Irc => Some(nearest_hex_colour_fast(bg_u32, &IRC99) as usize),
                        _ => None,
                    }
                } else {
                    None
                };

                let mut best = (usize::MAX, 0usize, 0u8, 0u8, false);
                
                // Smoothness weighting
                let smooth_weight = args.smooth.unwrap_or(0.0).clamp(0.0, 1.0);
                let color_weight = 1.0 - smooth_weight;
                
                // Edge penalty for indexed mode
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

                        let (fgi, fgm_val) = fg_cnt
                            .iter()
                            .enumerate()
                            .max_by_key(|&(_, c)| c)
                            .map(|(i, c_ref)| (i, *c_ref))
                            .unwrap_or((0usize, 0usize));
                        
                        // Calculate standard BG stats (max or forced)
                        let (bgi_std, bgm_effective) = if let Some(idx) = bg_opt_idx {
                            (idx, bg_cnt[idx])
                        } else {
                            bg_cnt.iter().enumerate().max_by_key(|&(_, c)| c).map(|(i, c)| (i, *c)).unwrap_or((0,0))
                        };

                        let cost_base = if args.fg_only && bg_opt_idx.is_none() {
                            // Heuristic: Score = 2 * Matches - Total_FG_Pixels
                            // Cost = Total_Pixels - Score
                            let total_pixels = bp;
                            let score = (2 * fgm_val as isize) - (fg_tot as isize);
                            ((total_pixels as isize) - score) as usize
                        } else {
                            (fg_tot.saturating_sub(fgm_val)) + (bg_tot.saturating_sub(bgm_effective))
                        };

                        let mut extra_cost = 0usize;
                        if smooth_weight > 0.0 {
                            // Edge structural cost
                            if args.smooth.is_some() {
                                for i in 0..bp {
                                    if current_block_edges[i] != fglyph.edge_bitmap[i] {
                                        extra_cost += edge_penalty_idx;
                                    }
                                }
                            }

                            // Continuity cost (Hamming distance)
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
                            if cost == 0 {
                                break;
                            }
                        }
                    }
                    if best.0 == 0 {
                        break;
                    }
                }

                chosen_glyph_index = best.1;
                let glyph_entry = glyphs[best.1].ch;
                let (fg_idx, bg_idx) = if best.4 { (best.3, best.2) } else { (best.2, best.3) };

                final_glyph = glyph_entry;
                final_fg = Colour::Index(fg_idx);
                final_bg = if args.fg_only { None } else { Some(Colour::Index(bg_idx)) };
            }

            // Update neighbor tracking
            left_neighbor_glyph_idx = (chosen_glyph_index, chosen_glyph_inverted);
            previous_row_glyph_indices[col_idx] = (chosen_glyph_index, chosen_glyph_inverted);

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
            Render::Irc => "\x0f\n", // IRC color reset
        });
    }
    out.trim_end_matches('\n').into()
}

pub fn render_braille(
    image_luma: &AnsiImage,
    image_chroma: &AnsiImage,
    args: &Args,
    render: Render,
) -> String {
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
    out.trim_end_matches('\n').to_string()
}

pub fn luma(rgb: &[u8;3]) -> u8 {
    let r = rgb[0] as f32; let g = rgb[1] as f32; let b = rgb[2] as f32;
    (0.299*r + 0.587*g + 0.114*b).round() as u8
}

fn dist_sq(p1: &[u8; 3], p2: &[u8; 3]) -> u64 {
    let dr = p1[0] as i64 - p2[0] as i64;
    let dg = p1[1] as i64 - p2[1] as i64;
    let db = p1[2] as i64 - p2[2] as i64;
    (dr * dr + dg * dg + db * db) as u64
}

fn render_blocks_smooth(
    image: &AnsiImage,
    args: &Args,
    render: Render,
) -> String {
    if image.glyphs.glyphs.is_empty() {
        return "Error: no glyphs available".into();
    }

    let bg_opt_rgb = args.bg_opt.as_ref().map(|s| unpack_rgb(parse_hex_color(s)));
    let (gh, gw) = image.glyphs.metrics;
    let bp = gh * gw;
    let img_h = image.bitmap.len();
    let img_w = if img_h > 0 { image.bitmap[0].len() } else { 0 };

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
    let mut current_block_pixels_rgb: Vec<PrePixel> = Vec::with_capacity(bp);
    
    // Weights
    let smooth_weight = args.smooth.unwrap_or(1.0).clamp(0.0, 1.0); 

    // Track the bottom edge colors of the previous row (for Top continuity)
    // Initialized with "transparent" or image pixels? 
    // Ideally image pixels if we want to anchor to the top edge of the image.
    // Dimensions: blocks_width * [Colour; gw]
    let mut prev_row_bottom_edges: Vec<Vec<[u8; 3]>> = Vec::with_capacity(blocks_width);
    
    // Initialize with top image pixels if available
    for col_idx in 0..blocks_width {
        let global_x = col_idx * gw;
        let mut edge = Vec::with_capacity(gw);
        for x in 0..gw {
            if global_x + x < img_w && 0 < img_h { 
                // Use row 0? Or row -1 (virtual)? Use row 0 for now.
                // Actually, continuity checks "My Top vs Neighbor Bottom".
                // If Row 0, "Neighbor Bottom" is the image edge?
                // Let's use image row 0 pixels. But wait, row 0 top matches image row -1 (non existent).
                // Let's just use image row 0 pixels as the "target" for the first row's top edge?
                // No, standard loop compares [Top] vs [Neighbor Bottom].
                // If we initialize `prev_row_bottom_edges` with Image Row -1 (impossible), or just empty/black.
                // Better: Check if row_idx == 0 inside loop.
                edge.push([0,0,0]); 
            } else {
                edge.push([0,0,0]);
            }
        }
        prev_row_bottom_edges.push(edge);
    }
    
    // Pre-fill prev_row_bottom_edges with actual image data if row_idx=0 logic is complex?
    // Let's stick to: "If row > 0, compare against prev_row. Else compare against image top edge?"
    // The loop handles this.

    for (row_idx, block_row) in image.block.iter().enumerate() {
        let mut first = true;
        let mut last_fg: Option<Colour> = None;
        let mut last_bg: Option<Colour> = None;
        let global_y = row_idx * gh;

        // Track the right edge of the left neighbor (for Left continuity)
        let mut left_neighbor_right_edge: Vec<[u8; 3]> = Vec::with_capacity(gh);
        // Initialize with image pixels to the left?
        if global_y < img_h {
            for _ in 0..gh {
                // If global_x=0, left is -1.
                // We can use image[y][0] as a reference or just black.
                 left_neighbor_right_edge.push([0,0,0]);
            }
        } else {
             left_neighbor_right_edge.resize(gh, [0,0,0]);
        }

        for (col_idx, blk) in block_row.iter().enumerate() {
            let global_x = col_idx * gw;
            let final_glyph: char;
            let final_fg: Colour;
            let final_bg: Option<Colour>;

            // --- 1. Calculate Internal Colors & Cost ---
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

            let mut best_cost = u64::MAX;
            let mut best_glyph_idx = 0;
            let mut best_chosen_fg_color = [0u8; 3];
            let mut best_chosen_bg_color = [0u8; 3];
            let mut best_chosen_inv = false;

            // Resolve references to neighbors
            let top_neighbor_edge = &prev_row_bottom_edges[col_idx];
            let left_neighbor_edge = &left_neighbor_right_edge;

            // Should we compare against Image Boundary for the outer edges?
            // If row_idx == 0, Top Neighbor is Image Top.
            // If col_idx == 0, Left Neighbor is Image Left.
            // Let's use image pixels for strict boundary matching if possible.
            // This anchors the smooth window.
            
            // Optimization: If weight > 0, pre-fetch image boundary pixels
            
            if smooth_weight > 0.0 {
                if row_idx == 0 && global_y == 0 { // Top of image
                     // No image pixels above 0. Use 0? Or just skip?
                     // Usually continuity is "Don't mismatch".
                }
                if col_idx == 0 && global_x == 0 { // Left of image
                }
            }

            for (gi, fglyph) in glyphs.iter().enumerate() {
                // ... (Internal cost stats calculation same as before) ...
                let mut sum_ones = PrePixel { r:0, g:0, b:0, sq:0 };
                let mut count_ones = 0u64;
                let mut sum_zeros = PrePixel { r:0, g:0, b:0, sq:0 };
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

                // Calc Internal Cost (No Inv)
                let cost_internal_no_inv = if use_coverage_heuristic {
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

                // Calc Boundary Cost (No Inv)
                let mut boundary_cost_no_inv = 0u64;
                if smooth_weight > 0.0 {
                    let target_bg = bg_ref.unwrap_or(avg_zeros);
                    
                    // Top Continuity
                    if row_idx > 0 {
                        for x in 0..gw {
                            let my_px = if fglyph.top[x] == 1 { avg_ones } else { target_bg };
                            boundary_cost_no_inv += dist_sq(&my_px, &top_neighbor_edge[x]);
                        }
                    } else if global_y > 0 {
                        // Compare against image top edge?
                        // Optional: keeps top clean
                        for x in 0..gw {
                            if global_x + x < img_w {
                                let img_px = unpack_rgb(image.bitmap[global_y - 1][global_x + x]);
                                let my_px = if fglyph.top[x] == 1 { avg_ones } else { target_bg };
                                boundary_cost_no_inv += dist_sq(&my_px, &img_px);
                            }
                        }
                    }

                    // Left Continuity
                    if col_idx > 0 {
                         for y in 0..gh {
                            let my_px = if fglyph.left[y] == 1 { avg_ones } else { target_bg };
                            boundary_cost_no_inv += dist_sq(&my_px, &left_neighbor_edge[y]);
                         }
                    } else if global_x > 0 {
                        // Compare against image left edge
                         for y in 0..gh {
                            if global_y + y < img_h {
                                let img_px = unpack_rgb(image.bitmap[global_y + y][global_x - 1]);
                                let my_px = if fglyph.left[y] == 1 { avg_ones } else { target_bg };
                                boundary_cost_no_inv += dist_sq(&my_px, &img_px);
                            }
                         }
                    }
                }

                let total_no_inv = (cost_internal_no_inv as f64 + boundary_cost_no_inv as f64 * smooth_weight as f64) as u64;
                if total_no_inv < best_cost {
                    best_cost = total_no_inv;
                    best_glyph_idx = gi;
                    best_chosen_fg_color = avg_ones;
                    best_chosen_bg_color = if use_coverage_heuristic { [0,0,0] } else { bg_ref.unwrap_or(avg_zeros) };
                    best_chosen_inv = false;
                }

                // Calc Internal Cost (Inv)
                let cost_internal_inv = if use_coverage_heuristic {
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

                // Calc Boundary Cost (Inv)
                let mut boundary_cost_inv = 0u64;
                if smooth_weight > 0.0 {
                    let target_bg = bg_ref.unwrap_or(avg_ones);
                    
                    // Top Continuity
                    if row_idx > 0 {
                        for x in 0..gw {
                            // Inverted: 0->FG(avg_zeros), 1->BG(target_bg)
                            let my_px = if fglyph.top[x] == 0 { avg_zeros } else { target_bg };
                            boundary_cost_inv += dist_sq(&my_px, &top_neighbor_edge[x]);
                        }
                    } else if global_y > 0 {
                        for x in 0..gw {
                            if global_x + x < img_w {
                                let img_px = unpack_rgb(image.bitmap[global_y - 1][global_x + x]);
                                let my_px = if fglyph.top[x] == 0 { avg_zeros } else { target_bg };
                                boundary_cost_inv += dist_sq(&my_px, &img_px);
                            }
                        }
                    }

                    // Left Continuity
                    if col_idx > 0 {
                         for y in 0..gh {
                            let my_px = if fglyph.left[y] == 0 { avg_zeros } else { target_bg };
                            boundary_cost_inv += dist_sq(&my_px, &left_neighbor_edge[y]);
                         }
                    } else if global_x > 0 {
                         for y in 0..gh {
                            if global_y + y < img_h {
                                let img_px = unpack_rgb(image.bitmap[global_y + y][global_x - 1]);
                                let my_px = if fglyph.left[y] == 0 { avg_zeros } else { target_bg };
                                boundary_cost_inv += dist_sq(&my_px, &img_px);
                            }
                         }
                    }
                }

                let total_inv = (cost_internal_inv as f64 + boundary_cost_inv as f64 * smooth_weight as f64) as u64;
                if total_inv < best_cost {
                    best_cost = total_inv;
                    best_glyph_idx = gi;
                    best_chosen_fg_color = avg_zeros;
                    best_chosen_bg_color = if use_coverage_heuristic { [0,0,0] } else { bg_ref.unwrap_or(avg_ones) };
                    best_chosen_inv = true;
                }

                if best_cost == 0 { break; }
            }

            final_glyph = glyphs[best_glyph_idx].ch;
            
            // --- Update Neighbors for next iteration ---
            let fglyph = &glyphs[best_glyph_idx];
            
            // Note: best_chosen_bg_color is already correct BG (avg_zeros for NoInv, avg_ones for Inv).
            // But if bg_ref was used, it might differ.
            // Simplified: Use the resolved colors we just calculated cost with.
            // For Inverted: FG=avg_zeros, BG=target_bg (which is bg_ref OR avg_ones).
            let (res_fg, res_bg) = if best_chosen_inv {
                (best_chosen_fg_color, best_chosen_bg_color) 
            } else {
                (best_chosen_fg_color, best_chosen_bg_color)
            };
            
            // Prepare edges for tracking
            // Right Edge -> becomes next Left Neighbor
            left_neighbor_right_edge.clear();
            for y in 0..gh {
                let bit = if best_chosen_inv { 1 - fglyph.right[y] } else { fglyph.right[y] };
                left_neighbor_right_edge.push(if bit == 1 { res_fg } else { res_bg });
            }

            // Bottom Edge -> becomes next Top Neighbor (in prev_row_bottom_edges)
            let mut bottom_edge = Vec::with_capacity(gw);
            for x in 0..gw {
                let bit = if best_chosen_inv { 1 - fglyph.bottom[x] } else { fglyph.bottom[x] };
                bottom_edge.push(if bit == 1 { res_fg } else { res_bg });
            }
            prev_row_bottom_edges[col_idx] = bottom_edge;


            // Map the chosen RGB colors to the target render mode
            match render {
                Render::Ansi24 => {
                    final_fg = Colour::RGB(best_chosen_fg_color);
                    final_bg = if args.fg_only { None } else { Some(Colour::RGB(best_chosen_bg_color)) };
                },
                Render::Ansi => {
                    let fg_u32 = make_rgb_u32(&best_chosen_fg_color);
                    let fg_idx = nearest_hex_colour_fast(fg_u32, &ANSI256);
                    final_fg = Colour::Index(fg_idx);
                    
                    final_bg = if args.fg_only { 
                        None 
                    } else {
                        let bg_u32 = make_rgb_u32(&best_chosen_bg_color);
                        let bg_idx = nearest_hex_colour_fast(bg_u32, &ANSI256);
                        Some(Colour::Index(bg_idx))
                    };
                },
                Render::Irc => {
                    let fg_u32 = make_rgb_u32(&best_chosen_fg_color);
                    let fg_idx = nearest_hex_colour_fast(fg_u32, &IRC99);
                    final_fg = Colour::Index(fg_idx);
                    
                    final_bg = if args.fg_only { 
                        None 
                    } else {
                        let bg_u32 = make_rgb_u32(&best_chosen_bg_color);
                        let bg_idx = nearest_hex_colour_fast(bg_u32, &IRC99);
                        Some(Colour::Index(bg_idx))
                    };
                }
            }

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
    }
    out.trim_end_matches('\n').into()
}

