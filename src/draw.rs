use std::collections::HashMap;
use std::fmt::Write as _;
use std::iter::repeat;
use std::sync::Mutex;
use once_cell::sync::Lazy;
use crate::args::{RenderArgs, Render};
use crate::font::GlyphStore;
use crate::palette::{IRC99, ANSI256};
use photon_rs::PhotonImage;
use image::Rgb;
use rayon::prelude::*;

#[derive(Clone)]
pub struct FastGlyph {
    pub ch: char,
    pub bitmap: Vec<u8>,
    pub right: Vec<u8>,
    pub coverage: f32,
    pub sc: ShapeContext,
    pub profile: BoundaryProfile,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BoundaryProfile {
    pub top: u32,
    pub bottom: u32,
    pub left: u32,
    pub right: u32,
}

use std::f32::consts::PI;

#[derive(Debug, Clone)]
pub struct ShapeContext {
    pub histogram: [f32; 60], // Increased bins for better resolution (e.g., 5 radial * 12 angular)
}

fn compute_shape_context(bitmap: &[u8], width: usize, height: usize) -> ShapeContext {
    // 1. configuration: Log-Polar setup
    // Standard SC often uses 5 radial bins and 12 angular bins = 60 dimensions
    const R_BINS: usize = 5;
    const A_BINS: usize = 12;
    let mut sc = [0.0; R_BINS * A_BINS];

    let cx = width as f32 / 2.0;
    let cy = height as f32 / 2.0;
    
    // Normalizing radius: We want the furthest point to define the scale boundary
    // to make the descriptor scale-invariant.
    let max_dist = (width as f32 / 2.0).hypot(height as f32 / 2.0);
    
    // Pre-compute log base for radial binning
    // We map log(r) from log(r_min) to log(r_max)
    let r_min = 1.0f32; // Avoid log(0)
    let log_r_max = max_dist.ln();
    let log_r_min = r_min.ln();
    let r_scale = R_BINS as f32 / (log_r_max - log_r_min);

    let mut total_weight = 0.0;

    for y in 0..height {
        for x in 0..width {
            // 2. Edge Detection
            // Only process if this pixel is ON and implies a boundary
            // (i.e., it has at least one OFF neighbor or is on the image frame)
            if bitmap[y * width + x] == 1 && is_edge(bitmap, x, y, width, height) {
                
                let dx = x as f32 - cx;
                let dy = y as f32 - cy;
                let r = dx.hypot(dy);

                // Skip center point to avoid singularity
                if r < r_min { continue; }

                // --- 3. Soft Binning Logic ---

                // Calculate Angular Position (continuous 0..A_BINS)
                let theta = dy.atan2(dx); 
                let theta_norm = if theta < 0.0 { theta + 2.0 * PI } else { theta };
                let a_pos = theta_norm / (2.0 * PI / A_BINS as f32);
                
                // Calculate Radial Position (continuous 0..R_BINS) based on Log(r)
                let r_pos = (r.ln() - log_r_min) * r_scale;

                // We distribute the vote (1.0) to the 4 nearest bins (2 angular * 2 radial)
                // This is "Bilinear Interpolation" in the histogram space.
                distribute_vote(&mut sc, a_pos, r_pos, A_BINS, R_BINS);
                
                total_weight += 1.0;
            }
        }
    }

    // Normalize (L1 norm ensures scale invariance)
    if total_weight > 0.0 {
        for val in sc.iter_mut() {
            *val /= total_weight;
        }
    }

    ShapeContext { histogram: sc }
}

// Helper: Simple 4-connectivity edge check
fn is_edge(bitmap: &[u8], x: usize, y: usize, w: usize, h: usize) -> bool {
    if x == 0 || y == 0 || x == w - 1 || y == h - 1 { return true; }
    
    // Check 4 neighbors. If any are 0, this is an edge pixel.
    let idx = y * w + x;
    if bitmap[idx - 1] == 0 { return true; }     // Left
    if bitmap[idx + 1] == 0 { return true; }     // Right
    if bitmap[idx - w] == 0 { return true; }     // Top
    if bitmap[idx + w] == 0 { return true; }     // Bottom
    
    false
}

// Helper: Distribute weight to neighbors
fn distribute_vote(
    sc: &mut [f32], 
    a_pos: f32, 
    r_pos: f32, 
    a_bins: usize, 
    r_bins: usize
) {
    // Angular handling (Cyclic: index 11 wraps to 0)
    let a_idx0 = a_pos.floor() as usize % a_bins;
    let a_idx1 = (a_idx0 + 1) % a_bins;
    let a_w1 = a_pos - a_pos.floor(); // Weight for the next bin
    let a_w0 = 1.0 - a_w1;            // Weight for current bin

    // Radial handling (Clamped: indices don't wrap)
    // We clamp r_pos to be within valid range logic
    let r_clamped = r_pos.max(0.0).min((r_bins - 1) as f32 - 0.0001);
    let r_idx0 = r_clamped.floor() as usize;
    let r_idx1 = r_idx0 + 1;
    let r_w1 = r_clamped - r_clamped.floor();
    let r_w0 = 1.0 - r_w1;

    // Add weighted contributions to the 4 corners
    // If r is at the very max, r_idx1 might be out of bounds, so we check.
    
    // Bin (r0, a0)
    sc[r_idx0 * a_bins + a_idx0] += r_w0 * a_w0;
    
    // Bin (r0, a1)
    sc[r_idx0 * a_bins + a_idx1] += r_w0 * a_w1;

    if r_idx1 < r_bins {
        // Bin (r1, a0)
        sc[r_idx1 * a_bins + a_idx0] += r_w1 * a_w0;
        
        // Bin (r1, a1)
        sc[r_idx1 * a_bins + a_idx1] += r_w1 * a_w1;
    }
}

fn compute_boundary_profile(bitmap: &[u8], width: usize, height: usize) -> BoundaryProfile {
    let mut top = 0u32;
    let mut bottom = 0u32;
    let mut left = 0u32;
    let mut right = 0u32;

    // Top and Bottom rows
    for x in 0..width.min(32) {
        if bitmap[x] == 1 { top |= 1 << x; }
        if bitmap[(height - 1) * width + x] == 1 { bottom |= 1 << x; }
    }

    // Left and Right columns
    for y in 0..height.min(32) {
        if bitmap[y * width] == 1 { left |= 1 << y; }
        if bitmap[y * width + (width - 1)] == 1 { right |= 1 << y; }
    }

    BoundaryProfile { top, bottom, left, right }
}

#[derive(Clone, Copy, Debug)]
pub struct PrePixel {
    pub r: u64, pub g: u64, b: u64,
    pub sq: u64,
}

/// Braille dot positions (for render_braille)
const POSITIONS: [(usize, usize, u32); 8] = [
    (0, 0, 0x01), (0, 1, 0x02), (0, 2, 0x04), (1, 0, 0x08),
    (1, 1, 0x10), (1, 2, 0x20), (0, 3, 0x40), (1, 3, 0x80),
];

/// Cache for colour conversions: orig → (ansi_std, irc_std, ansi_ng, irc_ng)
static COLOR_CACHE: Lazy<Mutex<HashMap<u32, (u8, u8, u8, u8)>>> = 
    Lazy::new(|| Mutex::new(HashMap::with_capacity(8_192)));

#[inline(always)]
pub fn nearest_hex_colour_fast(col: u32, palette: &[u32]) -> u8 {
    let pr = ((col >> 16) & 0xFF) as i32;
    let pg = ((col >>  8) & 0xFF) as i32;
    let pb = ((col >>  0) & 0xFF) as i32;

    let mut best_i = 0u8;
    let mut best_d = u32::MAX;
    if palette.is_empty() { return 0; }

    // Better: Just iterate all and skip inside loop for simplicity and performance (branch prediction handles this well)
    for (i, &p) in palette.iter().enumerate() {
        if palette.len() == 99 && i >= 2 && i < 16 { continue; }

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

#[inline] 
pub fn make_rgb_u32(px: &[u8]) -> u32 {
    if px.len() < 3 { return 0; }
    ((px[0] as u32) << 16) | ((px[1] as u32) << 8) | (px[2] as u32)
}
#[inline] pub fn unpack_rgb(rgb: u32) -> [u8;3] {
    [(rgb >> 16) as u8, (rgb >> 8) as u8, (rgb >> 0) as u8]
}

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
    let mut bg_changed = *first || last_bg.as_ref() != bg.as_ref();

    // Optimization: If char is space, we don't need to update FG (in ANSI)
    // because space makes the FG color invisible.
    // Also applies to Braille Blank.
    // For IRC, we can only do this if BG is not being set (fg-only),
    // otherwise we risk losing the BG color setting in the sequence.
    let is_space = glyph == ' ' || glyph == '\u{2800}';
    let can_skip_fg = match render {
        Render::Ansi | Render::Ansi24 => true,
        Render::Irc => bg.is_none(),
    };

    if is_space && can_skip_fg {
        fg_changed = false;
    }

    if fg_changed { *last_fg = Some(fg.clone()); }
    if bg_changed { *last_bg = bg.clone(); }

    if fg_changed || bg_changed {
        match render {
            Render::Ansi | Render::Ansi24 => {
                if let Some(bg_c) = &bg {
                    // Optimized ANSI emission: Don't reset (\x1b[0), just overwrite.
                    out.push_str("\x1b[");
                    let mut semi = false;
                    
                    if fg_changed {
                        match fg {
                            Colour::Index(i) => write!(out, "38;5;{}", i).unwrap(),
                            Colour::RGB([r, g, b]) => write!(out, "38;2;{};{};{}", r, g, b).unwrap(),
                        }
                        semi = true;
                    }
                    
                    if bg_changed {
                        if semi { out.push(';'); }
                        match bg_c {
                            Colour::Index(i) => write!(out, "48;5;{}", i).unwrap(),
                            Colour::RGB([r, g, b]) => write!(out, "48;2;{};{};{}", r, g, b).unwrap(),
                        }
                    }
                    out.push('m');
                } else {
                    if fg_changed {
                        match fg {
                            Colour::Index(i) => write!(out, "\x1b[38;5;{}m", i).unwrap(),
                            Colour::RGB([r, g, b]) => write!(out, "\x1b[38;2;{};{};{}m", r, g, b).unwrap(),
                        }
                    }
                }
            },
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
            let mut right = Vec::with_capacity(gh);
            for y in 0..gh {
                right.push(bitmap[y * gw + (gw - 1)]);
            }


            let ones = bitmap.iter().filter(|&&b| b == 1).count();
            let coverage = ones as f32 / bp as f32;
            let sc = compute_shape_context(&bitmap, gw, gh);
            let profile = compute_boundary_profile(&bitmap, gw, gh);

            FastGlyph {
                ch: *ch,
                bitmap,
                right,
                coverage,
                sc,
                profile,
            }
        })
        .collect()
}



fn get_transformed_block_pixels_into(orig: &[PrePixel], gw: usize, gh: usize, sx: f32, sy: f32, out: &mut Vec<PrePixel>) {
    out.clear();
    let cx = gw as f32 / 2.0; let cy = gh as f32 / 2.0;
    for y in 0..gh {
        for x in 0..gw {
            let u = ((x as f32 - cx) / sx + cx).clamp(0.0, (gw - 1) as f32);
            let v = ((y as f32 - cy) / sy + cy).clamp(0.0, (gh - 1) as f32);
            let x0 = u.floor() as usize; let x1 = (x0 + 1).min(gw - 1);
            let y0 = v.floor() as usize; let y1 = (y0 + 1).min(gh - 1);
            let wx = u - x0 as f32; let wy = v - y0 as f32;
            let p00 = &orig[y0 * gw + x0]; let p10 = &orig[y0 * gw + x1];
            let p01 = &orig[y1 * gw + x0]; let p11 = &orig[y1 * gw + x1];
            let lerp = |a: u64, b: u64, t: f32| a as f32 * (1.0 - t) + b as f32 * t ;
            let r = lerp(lerp(p00.r, p10.r, wx) as u64, lerp(p01.r, p11.r, wx) as u64, wy) as u64;
            let g = lerp(lerp(p00.g, p10.g, wx) as u64, lerp(p01.g, p11.g, wx) as u64, wy) as u64;
            let b = lerp(lerp(p00.b, p10.b, wx) as u64, lerp(p01.b, p11.b, wx) as u64, wy) as u64;
            out.push(PrePixel { r, g, b, sq: r*r + g*g + b*b });
        }
    }
}

pub fn render_blocks(
    image: &AnsiImage,
    args: &RenderArgs,
    render: Render,
    glyphs: &[FastGlyph],
) -> RenderResult {


    if glyphs.is_empty() {
        return "Error: no glyphs available".into();
    }

    let (gh, gw) = image.glyphs.metrics;
    let bp = gh * gw;

    let mut out = String::new();
    let mut total_error = 0u64;
    let mut total_pixels = 0u64;

    // Pre-allocate buffer for pixel data to avoid repeated allocations
    let mut current_block_pixels_rgb: Vec<PrePixel> = Vec::with_capacity(bp);
    // For indexed modes
    let mut current_block_indices: Vec<u8> = Vec::with_capacity(bp);

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
        let mut last_fg: Option<Colour> = None; 
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
                }
                let working_pixels = &working_pixels_buffer;

                if args.shape {
                    // --- Shape Context Matching ---
                    let mut block_bitmap = vec![0u8; bp];
                    for i in 0..bp {
                        let px = &working_pixels[i];
                        let luma = (0.299 * px.r as f32 + 0.587 * px.g as f32 + 0.114 * px.b as f32) as u8;
                        if luma > 128 { block_bitmap[i] = 1; }
                    }

                    // Compute SC
                    let block_sc = compute_shape_context(&block_bitmap, gw, gh);

                    // Find Best Match (Chi-Squared Distance)
                    let mut best_dist = f32::MAX;
                    let mut best_glyph_idx = 0;

                    for (gi, fglyph) in glyphs.iter().enumerate() {
                        let mut dist = 0.0;
                        for k in 0..60 {
                            let a = block_sc.histogram[k];
                            let b = fglyph.sc.histogram[k];
                            if a + b > 1e-6 {
                                dist += (a - b).powi(2) / (a + b);
                            }
                        }
                        dist *= 0.5;

                        if dist < best_dist {
                            best_dist = dist;
                            best_glyph_idx = gi;
                        }
                    }

                    // Set Result
                    if (best_dist as u64) < best_overall_cost {
                        best_overall_cost = best_dist as u64; // Hack: storing float cost in u64 for compatibility
                        chosen_glyph_index = best_glyph_idx;
                        chosen_glyph_inverted = false; 
                        final_glyph = glyphs[best_glyph_idx].ch;
                        // Use average color of the block for FG, Black for BG (or transparent)
                        // Calculate proper colors for the selected glyph
                        let best_glyph = &glyphs[best_glyph_idx];
                        // Calculate dominant colors (Mode) for the selected glyph
                        let best_glyph = &glyphs[best_glyph_idx];
                        let mut counts_on: HashMap<[u8; 3], usize> = HashMap::new();
                        let mut counts_off: HashMap<[u8; 3], usize> = HashMap::new();

                        for i in 0..bp {
                            let px = &working_pixels[i];
                            let color = [px.r as u8, px.g as u8, px.b as u8];
                            if best_glyph.bitmap[i] == 1 {
                                *counts_on.entry(color).or_insert(0) += 1;
                            } else {
                                *counts_off.entry(color).or_insert(0) += 1;
                            }
                        }

                        let avg_fg = counts_on.into_iter().max_by_key(|&(_, count)| count).map(|(c, _)| c).unwrap_or([255, 255, 255]);
                        let avg_bg = counts_off.into_iter().max_by_key(|&(_, count)| count).map(|(c, _)| c).unwrap_or([0, 0, 0]);

                        final_fg = Colour::RGB(avg_fg);
                        final_bg = Some(Colour::RGB(avg_bg));
                        best_match_data = (best_glyph_idx, false, avg_fg, avg_bg, 0, 0);
                        best_scale_is_identity = *sx == 1.0 && *sy == 1.0;
                    }

                } else if render == Render::Ansi24 {
                    // --- Ansi24 Matching ---
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
                        
                        // Cost Calculation (Normal)
                        // Assume fg_only is false
                        let target_bg = avg_zeros;
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
                            best_chosen_bg_color = avg_zeros;
                            best_inv = false;
                        }

                        // Cost Calculation (Inverted)
                        // Assume fg_only is false
                        let target_bg = avg_ones;
                        let t1_0 = sum_zeros_sq;
                        let t2_0 = 2 * (avg_zeros[0] as u64 * sum_zeros_r + avg_zeros[1] as u64 * sum_zeros_g + avg_zeros[2] as u64 * sum_zeros_b);
                        let t3_0 = count_zeros * (avg_zeros[0] as u64 * avg_zeros[0] as u64 + avg_zeros[1] as u64 * avg_zeros[1] as u64 + avg_zeros[2] as u64 * avg_zeros[2] as u64);
                        let c_fg = (t1_0 + t3_0).saturating_sub(t2_0);
                        let t1_1 = sum_ones_sq;
                        let t2_1 = 2 * (target_bg[0] as u64 * sum_ones_r + target_bg[1] as u64 * sum_ones_g + target_bg[2] as u64 * sum_ones_b);
                        let t3_1 = count_ones * (target_bg[0] as u64 * target_bg[0] as u64 + target_bg[1] as u64 * target_bg[1] as u64 + target_bg[2] as u64 * target_bg[2] as u64);
                        let c_bg = (t1_1 + t3_1).saturating_sub(t2_1);
                        let cost_invert = c_fg + c_bg;

                        if cost_invert < best_cost {
                            best_cost = cost_invert;
                            best_glyph_idx = gi;
                            best_chosen_fg_color = avg_ones;
                            best_chosen_bg_color = avg_zeros;
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
                        final_bg = Some(Colour::RGB(best_chosen_bg_color));
                        best_match_data = (best_glyph_idx, best_inv, best_chosen_fg_color, best_chosen_bg_color, 0, 0);
                        best_scale_is_identity = *sx == 1.0 && *sy == 1.0;
                    }

                } else {
                    // --- Indexed Matching ---
                    current_block_indices.clear();
                    for i in 0..bp {
                        let px_rgb = &working_pixels[i];
                        let rgb_u32 = ((px_rgb.r as u32) << 16) | ((px_rgb.g as u32) << 8) | (px_rgb.b as u32);
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

                    let mut best = (usize::MAX, 0usize, 0u8, 0u8, false);
                    let color_weight = 1.0; 

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
                            let (bgi_std, bgm_effective) = bg_cnt.iter().enumerate().max_by_key(|&(_, c)| c).map(|(i, c)| (i, *c)).unwrap_or((0,0));

                            let cost_base = (fg_tot.saturating_sub(fgm_val)) + (bg_tot.saturating_sub(bgm_effective));

                            let cost = (cost_base as f32 * color_weight) as usize;
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
                        final_bg = Some(Colour::Index(bg_idx));
                        best_match_data = (best.1, best.4, [0,0,0], [0,0,0], fg_idx, bg_idx);
                        best_scale_is_identity = *sx == 1.0 && *sy == 1.0;
                    }
                }
            } // End of scales loop

            if !best_scale_is_identity {
                dynamic_scaling_count += 1;
            }

            // Update neighbor tracking
            left_neighbor_glyph_idx = (chosen_glyph_index, chosen_glyph_inverted);
            previous_row_glyph_indices[col_idx] = (chosen_glyph_index, chosen_glyph_inverted);
            
            // Calculate final error against ORIGINAL pixels
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
                    else { if dist_fg < dist_bg { block_error += 1; } } 
                }
            } else {
                let fg_idx = best_match_data.4 as usize;
                let bg_idx = best_match_data.5 as usize;
                
                for i in 0..bp {
                    let px_rgb = &original_pixels_rgb[i];
                    let rgb_u32 = ((px_rgb.r as u32) << 16) | ((px_rgb.g as u32) << 8) | (px_rgb.b as u32);
                    let col_idx = nearest_hex_colour_fast(rgb_u32, if render == Render::Ansi { &ANSI256 } else { &IRC99 }) as usize;
                    
                    let bit = fglyph.bitmap[i] ^ (inv as u8);
                    if bit == 1 { if col_idx != fg_idx { block_error += 1; } } 
                    else { if col_idx != bg_idx { block_error += 1; } } 
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

            if let Some(bg_c) = &final_bg {
                if &final_fg == bg_c {
                    final_glyph = ' ';
                }
            }

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
        out.push_str(match render { Render::Ansi | Render::Ansi24 => "\x1b[0m\n", Render::Irc => "\x0f\n" });
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

pub fn render_blocks_smooth(
    image: &AnsiImage,
    args: &RenderArgs,
    render: Render,
    glyphs: &[FastGlyph],
) -> RenderResult {
    let (gh, gw) = image.glyphs.metrics;
    let h = image.bitmap.len();
    let w = image.bitmap[0].len();
    let rows = h / gh;
    let cols = w / gw;
    let bp = gh * gw;

    // 1. Pre-calculate candidates (Top K)
    // 1. Pre-calculate candidates (Top K)
    let k_candidates = args.smooth_candidates;
    if k_candidates == 0 { return render_blocks(image, args, render, glyphs); } // Fallback if 0
    // Use serial iterator to ensure perfect determinism, or carefully check parallel logic.
    // Switching to serial for stability debugging.
    let candidates_grid: Vec<Vec<Vec<(usize, bool, f32)>>> = (0..rows).map(|r| {
        (0..cols).map(|c| {
            // Extract block
            // Extract block and perform K-Means binarization
            let mut block_bitmap = vec![0u8; bp];
            let y_base = r * gh;
            let x_base = c * gw;
            let mut pixels = Vec::with_capacity(bp);

            for y in 0..gh {
                for x in 0..gw {
                    if y_base + y < h && x_base + x < w {
                         let px = &image.bitmap[y_base + y][x_base + x];
                         pixels.push(unpack_rgb(*px));
                    } else {
                        pixels.push([0, 0, 0]); // Padding
                    }
                }
            }

            // Init K-Means (Min/Max Luma seed)
            let mut c1 = [0u8; 3];
            let mut c2 = [255u8; 3];
            let mut min_l = 255;
            let mut max_l = 0;
            
            for &p in &pixels {
                let l = luma(&p);
                if l < min_l { min_l = l; c1 = p; }
                if l > max_l { max_l = l; c2 = p; }
            }
            if min_l == max_l { c2 = [255 - c1[0], 255 - c1[1], 255 - c1[2]]; } // Degenerate case

            // 2 Iterations
            for _ in 0..2 {
                let mut sum1 = [0u32; 3]; let mut cnt1 = 0;
                let mut sum2 = [0u32; 3]; let mut cnt2 = 0;
                
                for &p in &pixels {
                    let d1 = (p[0] as i32 - c1[0] as i32).pow(2) + (p[1] as i32 - c1[1] as i32).pow(2) + (p[2] as i32 - c1[2] as i32).pow(2);
                    let d2 = (p[0] as i32 - c2[0] as i32).pow(2) + (p[1] as i32 - c2[1] as i32).pow(2) + (p[2] as i32 - c2[2] as i32).pow(2);
                    if d1 < d2 {
                        sum1[0] += p[0] as u32; sum1[1] += p[1] as u32; sum1[2] += p[2] as u32; cnt1 += 1;
                    } else {
                        sum2[0] += p[0] as u32; sum2[1] += p[1] as u32; sum2[2] += p[2] as u32; cnt2 += 1;
                    }
                }
                if cnt1 > 0 { c1 = [(sum1[0]/cnt1) as u8, (sum1[1]/cnt1) as u8, (sum1[2]/cnt1) as u8]; }
                if cnt2 > 0 { c2 = [(sum2[0]/cnt2) as u8, (sum2[1]/cnt2) as u8, (sum2[2]/cnt2) as u8]; }
            }

            // Assign bits (Use C2 as "1" - convention usually brighter/fg)
            // But we can check which is dominant or brighter. Let's assume lighter is FG (1).
            let use_c2_as_1 = luma(&c2) > luma(&c1);

            for (i, &p) in pixels.iter().enumerate() {
                let d1 = (p[0] as i32 - c1[0] as i32).pow(2) + (p[1] as i32 - c1[1] as i32).pow(2) + (p[2] as i32 - c1[2] as i32).pow(2);
                let d2 = (p[0] as i32 - c2[0] as i32).pow(2) + (p[1] as i32 - c2[1] as i32).pow(2) + (p[2] as i32 - c2[2] as i32).pow(2);
                if (d2 < d1) == use_c2_as_1 {
                    block_bitmap[i] = 1;
                }
            }
            
            // Compute SC if needed
            let sc_opt = if args.shape {
                 Some(compute_shape_context(&block_bitmap, gw, gh))
            } else { None };

            // Score all glyphs
            let mut scores: Vec<(usize, bool, f32)> = Vec::with_capacity(glyphs.len() * 2);
            
            for (gi, glyph) in glyphs.iter().enumerate() {
                for &inv in &[false, true] {
                    let cost = if args.shape {
                         // Chi-squared distance
                         let mut d = 0.0;
                         let target_sc = sc_opt.as_ref().unwrap();
                         // Invert logic for SC is tricky if precomputed.
                         // For now, assume SC is rotation/scale invariant enough or ignore invert for SC match
                         // OR: Compute SC for inverted block?
                         // "Inverted" SC is just SC of inverted bitmap.
                         // If we want to support invert with SC, we need inverted matching.
                         // For simplicity, let's treat SC as "shape" and ignore color inversion for shape logic
                         // but BoundaryProfile needs invert.
                         // Let's stick to non-inverted for SC shape matching primarily, or handle invert.
                         // Just use non-inverted SC for now as "shape" usually implies positive shape.
                         if inv { 1e9 } else {
                             for k in 0..60 {
                                 let a = target_sc.histogram[k];
                                 let b = glyph.sc.histogram[k];
                                 if a + b > 1e-4 {
                                     d += (a - b).powi(2) / (a + b);
                                 }
                             }
                             d * 0.5
                         }
                    } else {
                        // Pixel error
                        let mut err = 0.0;
                        for i in 0..bp {
                            let bit = glyph.bitmap[i] ^ (inv as u8);
                             if bit != block_bitmap[i] { err += 1.0; }
                        }
                        err
                    };
                    scores.push((gi, inv, cost));
                }
            }
            // Sort and keep top K
            scores.sort_by(|a, b| {
                a.2.partial_cmp(&b.2)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.0.cmp(&b.0)) // Tie-breaker: Glyph Index
            });
            scores.truncate(k_candidates);
            scores
        }).collect()
    }).collect();

    // 2. Initialize grid
    let mut grid_indices: Vec<Vec<(usize, bool)>> = candidates_grid.iter().map(|row| {
        row.iter().map(|cands| (cands[0].0, cands[0].1)).collect()
    }).collect();

    // 3. Iterative Optimization (ICM)
    let lambda = 5.0; // Smoothness weight (Increased)
    let iterations = 6; // Even number for balance

    for iter in 0..iterations {
        let forward = iter % 2 == 0;
        let row_iter: Vec<usize> = if forward { (0..rows).collect() } else { (0..rows).rev().collect() };
        
        for &r in &row_iter {
            let col_iter: Vec<usize> = if forward { (0..cols).collect() } else { (0..cols).rev().collect() };
            for &c in &col_iter {
                let candidates = &candidates_grid[r][c];
                
                // Get neighbors' profiles
                let top_p = if r > 0 { Some(glyphs[grid_indices[r-1][c].0].profile) } else { None };
                let left_p = if c > 0 { Some(glyphs[grid_indices[r][c-1].0].profile) } else { None };
                let bottom_p = if r < rows - 1 { Some(glyphs[grid_indices[r+1][c].0].profile) } else { None };
                let right_p = if c < cols - 1 { Some(glyphs[grid_indices[r][c+1].0].profile) } else { None };
                
                let top_inv = if r > 0 { grid_indices[r-1][c].1 } else { false };
                let left_inv = if c > 0 { grid_indices[r][c-1].1 } else { false };
                let bottom_inv = if r < rows - 1 { grid_indices[r+1][c].1 } else { false };
                let right_inv = if c < cols - 1 { grid_indices[r][c+1].1 } else { false };

                // Adaptive Threshold Check
                if args.smooth_threshold > 0.0 {
                    let (cur_gi, cur_inv) = grid_indices[r][c];
                    let gp = glyphs[cur_gi].profile;
                    let mut cur_boundary_cost = 0.0;
                    
                    // Top
                    if let Some(tp) = top_p {
                        let my_top = if cur_inv { !gp.top } else { gp.top };
                        let mask = (1 << gw) - 1;
                        let my_top_masked = my_top & mask;
                        let neighbor_bottom = if top_inv { !tp.bottom } else { tp.bottom };
                        let diff = my_top_masked ^ (neighbor_bottom & mask);
                        cur_boundary_cost += diff.count_ones() as f32;
                    }
                    // Left
                    if let Some(lp) = left_p {
                        let my_left = if cur_inv { !gp.left } else { gp.left };
                        let mask = (1 << gh) - 1;
                        let diff = (my_left & mask) ^ ((if left_inv { !lp.right } else { lp.right }) & mask);
                        cur_boundary_cost += diff.count_ones() as f32;
                    }
                    // Bottom
                    if r < rows - 1 {
                        let (b_gi, b_inv) = grid_indices[r+1][c];
                        let bp = glyphs[b_gi].profile;
                        let my_bottom = if cur_inv { !gp.bottom } else { gp.bottom };
                        let mask = (1 << gw) - 1;
                        let diff = (my_bottom & mask) ^ ((if b_inv { !bp.top } else { bp.top }) & mask);
                        cur_boundary_cost += diff.count_ones() as f32;
                    }
                    // Right
                     if c < cols - 1 {
                        let (r_gi, r_inv) = grid_indices[r][c+1];
                        let rp_ = glyphs[r_gi].profile;
                        let my_right = if cur_inv { !gp.right } else { gp.right };
                        let mask = (1 << gh) - 1;
                        let diff = (my_right & mask) ^ ((if r_inv { !rp_.left } else { rp_.left }) & mask);
                        cur_boundary_cost += diff.count_ones() as f32;
                    }

                    if cur_boundary_cost < args.smooth_threshold as f32 {
                        continue;
                    }
                }

                let mut best_local_cost = f32::MAX;
                let mut best_idx = (0, false);

                for &(gi, inv, base_cost) in candidates {
                    let gp = glyphs[gi].profile;
                    let mut boundary_cost = 0.0;
                    
                    // Top Boundary
                    if let Some(tp) = top_p {
                        let my_top = if inv { !gp.top } else { gp.top }; // bitwise NOT for invert? 
                        // Wait, !u32 flips all bits. We only care about width bits.
                        // Mask: (1 << gw) - 1
                        let mask = (1 << gw) - 1;
                        let my_top_masked = my_top & mask;
                        let neighbor_bottom = if top_inv { !tp.bottom } else { tp.bottom };
                        let neighbor_bottom_masked = neighbor_bottom & mask;
                        
                        // Hamming distance
                        let diff = my_top_masked ^ neighbor_bottom_masked;
                        boundary_cost += diff.count_ones() as f32;
                    }
                    
                    // Left Boundary
                    if let Some(lp) = left_p {
                        let my_left = if inv { !gp.left } else { gp.left };
                        let mask = (1 << gh) - 1;
                        let my_left_masked = my_left & mask;
                        let neighbor_right = if left_inv { !lp.right } else { lp.right };
                        let neighbor_right_masked = neighbor_right & mask;
                        
                        boundary_cost += (my_left_masked ^ neighbor_right_masked).count_ones() as f32;
                    }

                    // Bottom and Right (lookahead for symmetry or just lag? in ICM we use current state of neighbors)
                    if let Some(bp) = bottom_p {
                         let my_bot = if inv { !gp.bottom } else { gp.bottom };
                         let mask = (1 << gw) - 1;
                         let n_top = if bottom_inv { !bp.top } else { bp.top };
                         boundary_cost += ((my_bot & mask) ^ (n_top & mask)).count_ones() as f32;
                    }
                    
                    if let Some(rp) = right_p {
                        let my_right = if inv { !gp.right } else { gp.right };
                        let mask = (1 << gh) - 1;
                        let n_left = if right_inv { !rp.left } else { rp.left };
                         boundary_cost += ((my_right & mask) ^ (n_left & mask)).count_ones() as f32;
                    }

                    let total_cost = base_cost + lambda * boundary_cost;
                    if total_cost < best_local_cost {
                        best_local_cost = total_cost;
                        best_idx = (gi, inv);
                    }
                }
                grid_indices[r][c] = best_idx;
            }
        }
    }

    // 4. Render Final
    let mut out = String::new();
    let mut grid = Vec::new();

    for r in 0..rows {
        let mut first = true;
        let mut last_fg = None;
        let mut last_bg = None;
        let mut row_vec = Vec::new();
        
        for c in 0..cols {
            let (gi, inv) = grid_indices[r][c];
            let raw_glyph = &glyphs[gi];
            let ch = raw_glyph.ch;
            
            // Color Logic (Dominant)
            // Need pixel loop again? Or just use average/mode logic.
            // For speed, let's reuse mode logic here.
            
            let y_base = r * gh;
            let x_base = c * gw;
            let mut counts_on: HashMap<[u8; 3], usize> = HashMap::new();
            let mut counts_off: HashMap<[u8; 3], usize> = HashMap::new();

            // Extract pixels and count
            for y in 0..gh {
                for x in 0..gw {
                    if y_base + y < h && x_base + x < w {
                         let px = &image.bitmap[y_base + y][x_base + x];
                         let color = unpack_rgb(*px);
                         let bit = raw_glyph.bitmap[y * gw + x] ^ (inv as u8);
                         if bit == 1 {
                             *counts_on.entry(color).or_insert(0) += 1;
                         } else {
                             *counts_off.entry(color).or_insert(0) += 1;
                         }
                    }
                }
            }
            
            let fg_rgb = counts_on.into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)))
                .map(|(c, _)| c)
                .unwrap_or([255, 255, 255]);
                
            let bg_rgb = counts_off.into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)))
                .map(|(c, _)| c)
                .unwrap_or([0, 0, 0]);
            
            let (mut final_fg, mut final_bg) = if inv {
                (Colour::RGB(bg_rgb), Some(Colour::RGB(fg_rgb)))
            } else {
                (Colour::RGB(fg_rgb), Some(Colour::RGB(bg_rgb)))
            };

            if render == Render::Irc {
                if let Colour::RGB(c) = final_fg {
                    let idx = nearest_hex_colour_fast(make_rgb_u32(&c), &IRC99);
                    final_fg = Colour::Index(idx);
                }
                if let Some(Colour::RGB(c)) = final_bg {
                    let idx = nearest_hex_colour_fast(make_rgb_u32(&c), &IRC99);
                    final_bg = Some(Colour::Index(idx));
                }
            } else if render == Render::Ansi {
                if let Colour::RGB(c) = final_fg {
                    let idx = nearest_hex_colour_fast(make_rgb_u32(&c), &ANSI256);
                    final_fg = Colour::Index(idx);
                }
                if let Some(Colour::RGB(c)) = final_bg {
                    let idx = nearest_hex_colour_fast(make_rgb_u32(&c), &ANSI256);
                    final_bg = Some(Colour::Index(idx));
                }
            }
            

            
            // Sync grid colors with quantized colors
            let grid_fg = match final_fg.clone() {
                Colour::Index(i) => {
                    let p: &[u32] = if render == Render::Irc { &IRC99 } else { &ANSI256 };
                    if (i as usize) < p.len() { unpack_rgb(p[i as usize]) } else { fg_rgb }
                },
                Colour::RGB(c) => c,
            };
            
            let grid_bg = final_bg.clone().map(|c| match c {
                 Colour::Index(i) => {
                    let p: &[u32] = if render == Render::Irc { &IRC99 } else { &ANSI256 };
                    if (i as usize) < p.len() { unpack_rgb(p[i as usize]) } else { [0,0,0] }
                },
                Colour::RGB(c) => c,
            }).unwrap_or(bg_rgb);

            row_vec.push(RenderedCell { char: ch, fg: grid_fg, bg: Some(grid_bg) });
            
            emit_colourized(&mut out, render, final_fg, final_bg, ch, &mut first, &mut last_fg, &mut last_bg);
        }
        out.push_str(match render { Render::Ansi | Render::Ansi24 => "\x1b[0m\n", Render::Irc => "\x0f\n" });
        grid.push(row_vec);
    }

    RenderResult {
        content: out.trim_end_matches('\n').into(),
        error_count: 0,
        total_pixels: (h * w) as u64,
        dynamic_scaling_count: 0,
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

    for y in (0..h).step_by(4) {
        let mut first = true;
        let mut last_fg: Option<Colour> = None;
        let mut last_bg: Option<Colour> = None;
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
                        braille |= bit;
                        let px_chroma_orig = image_chroma.bitmap[yy][xx];
                        let ansi_pixel_chroma = AnsiPixel::new(&px_chroma_orig, args.grayscale_tolerance);
                        let col = pick_colour(&ansi_pixel_chroma, render, args);
                        *counts.entry(col).or_insert(0) += 1;
                    }
                }
            }

            let fg = counts.into_iter().max_by_key(|&(_, count)| count).map(|(c, _)| c).unwrap_or(match render {
                Render::Ansi | Render::Irc => Colour::Index(0),
                Render::Ansi24 => Colour::RGB([0,0,0]),
            });

            let glyph = std::char::from_u32(braille).unwrap_or(' ');
            emit_colourized(&mut out, render, fg, None, glyph, &mut first, &mut last_fg, &mut last_bg);
        }
        out.push_str(match render { Render::Ansi | Render::Ansi24 => "\x1b[0m\n", Render::Irc => "\x0f\n" });
    }

    RenderResult {
        content: out.trim_end_matches('\n').to_string(),
        error_count: 0,
        total_pixels: (h*w) as u64,
        dynamic_scaling_count: 0,
        grid: Vec::new(),
    }
}

fn luma(rgb: &[u8;3]) -> u8 {
    (0.299*rgb[0] as f32 + 0.587*rgb[1] as f32 + 0.114*rgb[2] as f32).round() as u8
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

pub fn calculate_smoothness(result: &RenderResult, _glyphs: &GlyphStore) -> f64 {
    if result.grid.is_empty() { return 0.0; }
    let rows = result.grid.len();
    let cols = result.grid[0].len();
    if rows == 0 || cols == 0 { return 0.0; }

    let mut total_diff = 0.0;
    let mut count = 0;

    for r in 0..rows {
        for c in 0..cols {
            let cell = &result.grid[r][c];
            // Reconstruct the average "color" of the cell.
            // Simplified: treat FG as the dominant color.
            // Better: We don't have per-pixel info here easily without re-rendering the glyph.
            // Let's rely on the FG/BG continuity. 
            // Smoothness = (Difference with Right neighbor) + (Difference with Bottom neighbor)
            
            let my_color = cell.fg; // Treating FG as the representative color for now
            let [r1, g1, b1] = [my_color[0] as f64, my_color[1] as f64, my_color[2] as f64];

            // Right neighbor
            if c + 1 < cols {
                let right_cell = &result.grid[r][c+1];
                let right_color = right_cell.fg;
                let [r2, g2, b2] = [right_color[0] as f64, right_color[1] as f64, right_color[2] as f64];
                total_diff += ((r1-r2).powi(2) + (g1-g2).powi(2) + (b1-b2).powi(2)).sqrt();
                count += 1;
            }

            // Bottom neighbor
            if r + 1 < rows {
                let bottom_cell = &result.grid[r+1][c];
                let bottom_color = bottom_cell.fg;
                let [r2, g2, b2] = [bottom_color[0] as f64, bottom_color[1] as f64, bottom_color[2] as f64];
                total_diff += ((r1-r2).powi(2) + (g1-g2).powi(2) + (b1-b2).powi(2)).sqrt();
                count += 1;
            }
        }
    }

    if count > 0 {
        total_diff / count as f64 // Average difference. Lower is smoother.
    } else {
        0.0
    }
}

pub fn calculate_smoothness_relative(_result: &RenderResult, _glyphs: &GlyphStore) -> f64 {
    0.0
}

pub fn calculate_frequency_metric(_result: &RenderResult, _image: &AnsiImage, _glyphs: &GlyphStore) -> f64 {
    0.0
}

pub fn save_as_png(
    result: &RenderResult,
    glyphs: &GlyphStore,
    path: &str,
    _metadata: &str
) -> std::io::Result<()> {
    if result.grid.is_empty() { return Ok(()); }
    
    // Calculate dimensions
    let (gh, gw) = glyphs.metrics;
    let rows = result.grid.len();
    let cols = result.grid[0].len();
    let width = cols * gw;
    let height = rows * gh;

    let mut img = image::ImageBuffer::new(width as u32, height as u32);

    for (r, row) in result.grid.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            let x_base = c * gw;
            let y_base = r * gh;
            
            // Get glyph bitmap
            // Linear search for glyph in store (slow but fine for debug/candidates)
            // Ideally FastGlyph would link back but we only have char.
            // We use `glyphs.glyphs` (Vec<(char, Vec<Vec<u8>>)>)
            let bmp_opt = glyphs.glyphs.iter().find(|(ch, _)| *ch == cell.char).map(|(_, b)| b);
            
            if let Some(bmp) = bmp_opt {
                for y in 0..gh {
                    for x in 0..gw {
                        // Check bounds in bmp
                        let on = if y < bmp.len() && x < bmp[0].len() { bmp[y][x] == 1 } else { false };
                        let color = if on { cell.fg } else { if let Some(bg) = cell.bg { bg } else { [0,0,0] } };
                        img.put_pixel((x_base + x) as u32, (y_base + y) as u32, image::Rgb(color));
                    }
                }
            } else {
                // If glyph not found, fill with bg
                for y in 0..gh {
                    for x in 0..gw {
                        let color = if let Some(bg) = cell.bg { bg } else { [0,0,0] };
                         img.put_pixel((x_base + x) as u32, (y_base + y) as u32, image::Rgb(color));
                    }
                }
            }
        }
    }
    
    img.save(path).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
}
