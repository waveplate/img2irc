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
    let mut changed = *first;
    if last_fg.as_ref() != Some(&fg) { changed = true; *last_fg = Some(fg.clone()); }
    if last_bg.as_ref() != bg.as_ref() { changed = true; *last_bg = bg.clone(); }

    if changed {
        match render {
            Render::Ansi | Render::Ansi24 => {
                if let Some(bg_c) = &bg {
                    out.push_str("\x1b[0");
                    match fg {
                        Colour::Index(i) => write!(out, ";38;5;{}", i).unwrap(),
                        Colour::RGB([r, g, b]) => write!(out, ";38;2;{};{};{}", r, g, b).unwrap(),
                    }
                    match bg_c {
                        Colour::Index(i) => write!(out, ";48;5;{}", i).unwrap(),
                        Colour::RGB([r, g, b]) => write!(out, ";48;2;{};{};{}", r, g, b).unwrap(),
                    }
                    out.push('m');
                } else {
                    match fg {
                        Colour::Index(i) => write!(out, "\x1b[38;5;{}m", i).unwrap(),
                        Colour::RGB([r, g, b]) => write!(out, "\x1b[38;2;{};{};{}m", r, g, b).unwrap(),
                    }
                }
            },
            Render::Irc => {
                out.push('\x03');
                match fg {
                    Colour::Index(i) => write!(out, "{:02}", i % 99).unwrap(),
                    _ => out.push_str("01"),
                }
                if let Some(bg_c) = &bg {
                    out.push(',');
                    match bg_c {
                        Colour::Index(i) => write!(out, "{:02}", i % 99).unwrap(),
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

fn render_blocks_connected(
    image: &AnsiImage,
    args: &RenderArgs,
    render: Render,
    glyphs: &[FastGlyph],
) -> RenderResult {
    if glyphs.is_empty() { return "Error: no glyphs available".into(); }

    let (gh, gw) = image.glyphs.metrics;
    let bp = gh * gw;

    let mut out = String::new();
    let mut total_error = 0u64;
    let mut total_pixels = 0u64;

    let mut current_block_pixels_rgb: Vec<PrePixel> = Vec::with_capacity(bp);
    // Source Edge Signatures (0 = BG, 1 = FG)
    // We derive these from the source pixels using a luma threshold (127)
    let threshold = 127;
    let mut grid: Vec<Vec<RenderedCell>> = Vec::new();

    for (_row_idx, block_row) in image.block.iter().enumerate() {
        let mut grid_row = Vec::new();
        let mut first = true;
        let mut last_fg: Option<Colour> = None;
        let mut last_bg: Option<Colour> = None;

        for (_col_idx, blk) in block_row.iter().enumerate() {
            total_pixels += bp as u64;
            
            // 1. Phase 1: The "Contract" (Analyze Source Edges)
            let mut src_top = Vec::with_capacity(gw);
            let mut src_bottom = Vec::with_capacity(gw);
            let mut src_left = Vec::with_capacity(gh);
            let mut src_right = Vec::with_capacity(gh);

            current_block_pixels_rgb.clear();
            for y in 0..gh {
                for x in 0..gw {
                    let px_orig = blk.pixels[y][x].orig;
                    let rgb = unpack_rgb(px_orig);
                    let lum = luma(&rgb);
                    let bit = if lum > threshold { 1u8 } else { 0u8 };
                    
                    // Populate Edges
                    if y == 0 { src_top.push(bit); }
                    if y == gh - 1 { src_bottom.push(bit); }
                    if x == 0 { src_left.push(bit); }
                    if x == gw - 1 { src_right.push(bit); }

                    current_block_pixels_rgb.push(PrePixel {
                        r: rgb[0] as u64, g: rgb[1] as u64, b: rgb[2] as u64,
                        sq: (rgb[0] as u64).pow(2) + (rgb[1] as u64).pow(2) + (rgb[2] as u64).pow(2)
                    });
                }
            }

            // Calculate Block Sums for Internal Error (Standard Logic)
            let mut block_sum_r = 0u64;
            let mut block_sum_g = 0u64;
            let mut block_sum_b = 0u64;
            let mut block_sum_sq = 0u64;
            for p in &current_block_pixels_rgb {
                block_sum_r += p.r; block_sum_g += p.g; block_sum_b += p.b; block_sum_sq += p.sq;
            }

            // 2. Phase 2: The "Filter" (Edge Matching)
            // We find the Minimum Edge Error first.
            let mut min_edge_error = usize::MAX;
            let mut candidates: Vec<(usize, bool, usize)> = Vec::new(); // (idx, inv, edge_err)

            for (gi, fglyph) in glyphs.iter().enumerate() {
                for inv in [false, true] {
                    // Calculate Edge Error
                    let mut edge_error = 0usize;
                    
                    // Top
                    for i in 0..gw {
                        let g_bit = fglyph.top[i] ^ (inv as u8);
                        if g_bit != src_top[i] { edge_error += 1; }
                    }
                    // Bottom
                    for i in 0..gw {
                        let g_bit = fglyph.bottom[i] ^ (inv as u8);
                        if g_bit != src_bottom[i] { edge_error += 1; }
                    }
                    // Left
                    for i in 0..gh {
                        let g_bit = fglyph.left[i] ^ (inv as u8);
                        if g_bit != src_left[i] { edge_error += 1; }
                    }
                    // Right
                    for i in 0..gh {
                        let g_bit = fglyph.right[i] ^ (inv as u8);
                        if g_bit != src_right[i] { edge_error += 1; }
                    }

                    if edge_error < min_edge_error {
                        min_edge_error = edge_error;
                        candidates.clear();
                        candidates.push((gi, inv, edge_error));
                    } else if edge_error == min_edge_error {
                        candidates.push((gi, inv, edge_error));
                    }
                }
            }

            // 3. Phase 3: The "Selection" (Visual Matching on Candidates)
            let mut best_cost = f64::MAX;
            let mut final_glyph = ' ';
            let mut final_fg = Colour::Index(0);
            let mut final_bg: Option<Colour> = None;
            let mut chosen_glyph_index = 0;
            let mut chosen_glyph_inverted = false;
            let mut best_fg_c = [0u8; 3];
            let mut best_bg_c = [0u8; 3];

            for (gi, inv, _edge_err) in candidates {
                let fglyph = &glyphs[gi];
                
                // Standard Internal Error Calculation
                let mut sum_ones_r = 0u64;
                let mut sum_ones_g = 0u64;
                let mut sum_ones_b = 0u64;
                let mut sum_ones_sq = 0u64;
                let mut count_ones = 0u64;

                for i in 0..bp {
                    if fglyph.bitmap[i] ^ (inv as u8) == 1 {
                        let px = &current_block_pixels_rgb[i];
                        sum_ones_r += px.r; sum_ones_g += px.g; sum_ones_b += px.b; sum_ones_sq += px.sq;
                        count_ones += 1;
                    }
                }
                let count_zeros = bp as u64 - count_ones;
                let sum_zeros_r = block_sum_r - sum_ones_r;
                let sum_zeros_g = block_sum_g - sum_ones_g;
                let sum_zeros_b = block_sum_b - sum_ones_b;
                let sum_zeros_sq = block_sum_sq - sum_ones_sq;

                let avg_ones = if count_ones > 0 { [(sum_ones_r / count_ones) as u8, (sum_ones_g / count_ones) as u8, (sum_ones_b / count_ones) as u8] } else { [0,0,0] };
                let avg_zeros = if count_zeros > 0 { [(sum_zeros_r / count_zeros) as u8, (sum_zeros_g / count_zeros) as u8, (sum_zeros_b / count_zeros) as u8] } else { [0,0,0] };

                let (fg_c, bg_c, fg_col, bg_col) = if render == Render::Ansi24 {
                    let target_bg = if args.fg_only { [0,0,0] } else { avg_zeros };
                    (avg_ones, target_bg, Colour::RGB(avg_ones), if args.fg_only { None } else { Some(Colour::RGB(target_bg)) })
                } else {
                    let palette: &[u32] = if render == Render::Ansi { &ANSI256 } else { &IRC99 };
                    let fgi = nearest_hex_colour_fast(make_rgb_u32(&avg_ones), palette);
                    let bgi = if args.fg_only { 
                        nearest_hex_colour_fast(0x000000, palette) 
                    } else { 
                        nearest_hex_colour_fast(make_rgb_u32(&avg_zeros), palette) 
                    };
                    (unpack_rgb(palette[fgi as usize]), unpack_rgb(palette[bgi as usize]), Colour::Index(fgi), if args.fg_only { None } else { Some(Colour::Index(bgi)) })
                };

                let fg_p = PrePixel { r: fg_c[0] as u64, g: fg_c[1] as u64, b: fg_c[2] as u64, sq: (fg_c[0] as u64).pow(2) + (fg_c[1] as u64).pow(2) + (fg_c[2] as u64).pow(2) };
                let bg_p = PrePixel { r: bg_c[0] as u64, g: bg_c[1] as u64, b: bg_c[2] as u64, sq: (bg_c[0] as u64).pow(2) + (bg_c[1] as u64).pow(2) + (bg_c[2] as u64).pow(2) };

                let c1 = (sum_ones_sq + count_ones * fg_p.sq).saturating_sub(2 * (fg_p.r * sum_ones_r + fg_p.g * sum_ones_g + fg_p.b * sum_ones_b));
                let c0 = (sum_zeros_sq + count_zeros * bg_p.sq).saturating_sub(2 * (bg_p.r * sum_zeros_r + bg_p.g * sum_zeros_g + bg_p.b * sum_zeros_b));
                
                let coverage = if inv { count_zeros } else { count_ones } as f64 / bp as f64;
                let cost = if args.fg_only {
                    (c1 as f64) + (count_zeros as f64 * 1000.0) - (coverage * 0.001)
                } else {
                    (c1 + c0) as f64 - (coverage * 0.001)
                };

                if cost < best_cost {
                    best_cost = cost;
                    chosen_glyph_index = gi;
                    chosen_glyph_inverted = inv;
                    final_glyph = fglyph.ch;
                    if inv && !args.fg_only {
                        final_fg = bg_col.unwrap();
                        final_bg = Some(fg_col);
                    } else {
                        final_fg = fg_col;
                        final_bg = bg_col;
                    }
                    best_fg_c = fg_c;
                    best_bg_c = bg_c;
                }
            }

            // Error Calculation (Dithering)
            let fglyph = &glyphs[chosen_glyph_index];
            let inv = chosen_glyph_inverted;
            let mut block_error = 0u64;
            let fg_p = PrePixel { r: best_fg_c[0] as u64, g: best_fg_c[1] as u64, b: best_fg_c[2] as u64, sq: (best_fg_c[0] as u64).pow(2) + (best_fg_c[1] as u64).pow(2) + (best_fg_c[2] as u64).pow(2) };
            let bg_p = PrePixel { r: best_bg_c[0] as u64, g: best_bg_c[1] as u64, b: best_bg_c[2] as u64, sq: (best_bg_c[0] as u64).pow(2) + (best_bg_c[1] as u64).pow(2) + (best_bg_c[2] as u64).pow(2) };

            for i in 0..bp {
                let px = &current_block_pixels_rgb[i];
                let dist_fg = (px.sq + fg_p.sq).saturating_sub(2 * (px.r * fg_p.r + px.g * fg_p.g + px.b * fg_p.b));
                let dist_bg = (px.sq + bg_p.sq).saturating_sub(2 * (px.r * bg_p.r + px.g * bg_p.g + px.b * bg_p.b));
                let bit = fglyph.bitmap[i] ^ (inv as u8);
                if bit == 1 { if dist_bg < dist_fg { block_error += 1; } }
                else { if !args.fg_only && dist_fg < dist_bg { block_error += 1; } }
            }
            total_error += block_error;

            grid_row.push(RenderedCell { char: final_glyph, fg: best_fg_c, bg: if args.fg_only { None } else { Some(best_bg_c) } });
            emit_colourized(&mut out, render, final_fg, final_bg, final_glyph, &mut first, &mut last_fg, &mut last_bg);
        }
        if args.fg_only {
            out.push_str(match render { Render::Ansi | Render::Ansi24 => "\x1b[39m\n", Render::Irc => "\x03\n" });
        } else {
            out.push_str(match render { Render::Ansi | Render::Ansi24 => "\x1b[0m\n", Render::Irc => "\x0f\n" });
        }
        grid.push(grid_row);
    }
    
    RenderResult { content: out.trim_end_matches('\n').into(), error_count: total_error, total_pixels, dynamic_scaling_count: 0, grid }
}

pub fn render_blocks(
    image: &AnsiImage,
    args: &RenderArgs,
    render: Render,
    glyphs: &[FastGlyph],
) -> RenderResult {
    if args.connected {
        return render_blocks_connected(image, args, render, glyphs);
    }

    if args.smooth.is_some() {
        return render_blocks_smooth(image, args, render, glyphs);
    }

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
                };
                let working_pixels = &working_pixels_buffer;

                if render == Render::Ansi24 {
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
                        
                        // Derive Zeros
                        let sum_zeros_r = block_sum_r - sum_ones_r;
                        let sum_zeros_g = block_sum_g - sum_ones_g;
                        let sum_zeros_b = block_sum_b - sum_ones_b;
                        let sum_zeros_sq = block_sum_sq - sum_ones_sq;

                        let avg_ones = if count_ones > 0 {
                            [((sum_ones_r / count_ones) as u8), ((sum_ones_g / count_ones) as u8), ((sum_ones_b / count_ones) as u8)]
                        } else { [0,0,0] };
                        let avg_zeros = if count_zeros > 0 {
                            [((sum_zeros_r / count_ones) as u8), ((sum_zeros_g / count_zeros) as u8), ((sum_zeros_b / count_zeros) as u8)]
                        } else { [0,0,0] };
                        
                        let use_coverage_heuristic = args.fg_only;
                        let penalty_per_miss = 1000u64;
                        
                        // Cost Calculation (Normal)
                        let cost_no_invert = if use_coverage_heuristic {
                             let term1 = sum_ones_sq;
                             let term2 = 2 * (avg_ones[0] as u64 * sum_ones_r + avg_ones[1] as u64 * sum_ones_g + avg_ones[2] as u64 * sum_ones_b);
                             let term3 = count_ones * (avg_ones[0] as u64 * avg_ones[0] as u64 + avg_ones[1] as u64 * avg_ones[1] as u64 + avg_ones[2] as u64 * avg_ones[2] as u64);
                             (term1 + term3).saturating_sub(term2) + (count_zeros * penalty_per_miss)
                        } else {
                            let target_bg = avg_zeros;
                            let t1_1 = sum_ones_sq;
                            let t2_1 = 2 * (avg_ones[0] as u64 * sum_ones_r + avg_ones[1] as u64 * sum_ones_g + avg_ones[2] as u64 * sum_ones_b);
                            let t3_1 = count_ones * (avg_ones[0] as u64 * avg_ones[0] as u64 + avg_ones[1] as u64 * avg_ones[1] as u64 + avg_ones[2] as u64 * avg_ones[2] as u64);
                            let c1 = (t1_1 + t3_1).saturating_sub(t2_1);
                            let t1_0 = sum_zeros_sq;
                            let t2_0 = 2 * (target_bg[0] as u64 * sum_zeros_r + target_bg[1] as u64 * sum_zeros_g + target_bg[2] as u64 * sum_zeros_b);
                            let t3_0 = count_zeros * (target_bg[0] as u64 * target_bg[0] as u64 + target_bg[1] as u64 * target_bg[1] as u64 + target_bg[2] as u64 * target_bg[2] as u64);
                            let c0 = (t1_0 + t3_0).saturating_sub(t2_0);
                            c1 + c0
                        };

                        if cost_no_invert < best_cost {
                            best_cost = cost_no_invert;
                            best_glyph_idx = gi;
                            best_chosen_fg_color = avg_ones;
                            best_chosen_bg_color = if use_coverage_heuristic { [0,0,0] } else { avg_zeros };
                            best_inv = false;
                        }

                        // Cost Calculation (Inverted)
                        let cost_invert = if use_coverage_heuristic {
                             let t1_0 = sum_zeros_sq;
                             let t2_0 = 2 * (avg_zeros[0] as u64 * sum_zeros_r + avg_zeros[1] as u64 * sum_zeros_g + avg_zeros[2] as u64 * sum_zeros_b);
                             let t3_0 = count_zeros * (avg_zeros[0] as u64 * avg_zeros[0] as u64 + avg_zeros[1] as u64 * avg_zeros[1] as u64 + avg_zeros[2] as u64 * avg_zeros[2] as u64);
                             (t1_0 + t3_0).saturating_sub(t2_0) + (count_ones * penalty_per_miss)
                        } else {
                             let target_bg = avg_ones;
                             let t1_0 = sum_zeros_sq;
                             let t2_0 = 2 * (avg_zeros[0] as u64 * sum_zeros_r + avg_zeros[1] as u64 * sum_zeros_g + avg_zeros[2] as u64 * sum_zeros_b);
                             let t3_0 = count_zeros * (avg_zeros[0] as u64 * avg_zeros[0] as u64 + avg_zeros[1] as u64 * avg_zeros[1] as u64 + avg_zeros[2] as u64 * avg_zeros[2] as u64);
                             let c_fg = (t1_0 + t3_0).saturating_sub(t2_0);
                             let t1_1 = sum_ones_sq;
                             let t2_1 = 2 * (target_bg[0] as u64 * sum_ones_r + target_bg[1] as u64 * sum_ones_g + target_bg[2] as u64 * sum_ones_b);
                             let t3_1 = count_ones * (target_bg[0] as u64 * target_bg[0] as u64 + target_bg[1] as u64 * target_bg[1] as u64 + target_bg[2] as u64 * target_bg[2] as u64);
                             let c_bg = (t1_1 + t3_1).saturating_sub(t2_1);
                             c_fg + c_bg
                        };

                        if cost_invert < best_cost {
                            best_cost = cost_invert;
                            best_glyph_idx = gi;
                            best_chosen_fg_color = avg_zeros;
                            best_chosen_bg_color = if use_coverage_heuristic { [0,0,0] } else { avg_ones };
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

                            let cost_base = if args.fg_only {
                                let total = bp;
                                let score = (2 * fgm_val as isize) - (fg_tot as isize);
                                ((total as isize) - score) as usize
                            } else {
                                (fg_tot.saturating_sub(fgm_val)) + (bg_tot.saturating_sub(bgm_effective))
                            };

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
                
                for i in 0..bp {
                    let px_rgb = &original_pixels_rgb[i];
                    let rgb_u32 = ((px_rgb.r as u32) << 16) | ((px_rgb.g as u32) << 8) | (px_rgb.b as u32);
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
        if args.fg_only {
            out.push_str(match render { Render::Ansi | Render::Ansi24 => "\x1b[39m\n", Render::Irc => "\x03\n" });
        } else {
            out.push_str(match render { Render::Ansi | Render::Ansi24 => "\x1b[0m\n", Render::Irc => "\x0f\n" });
        }
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
        if args.fg_only {
            out.push_str(match render { Render::Ansi | Render::Ansi24 => "\x1b[39m\n", Render::Irc => "\x03\n" });
        } else {
            out.push_str(match render { Render::Ansi | Render::Ansi24 => "\x1b[0m\n", Render::Irc => "\x0f\n" });
        }
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

pub fn render_blocks_smooth(
    image: &AnsiImage,
    args: &RenderArgs,
    render: Render,
    glyphs: &[FastGlyph],
) -> RenderResult {
    if glyphs.is_empty() { return "Error: no glyphs available".into(); }

    let (gh, gw) = image.glyphs.metrics;
    let bp = gh * gw;
    let blocks_width = if !image.block.is_empty() { image.block[0].len() } else { 0 };

    let mut out = String::new();
    let mut total_error = 0u64;
    let mut total_pixels = 0u64;
    let mut dynamic_scaling_count = 0u64;

    let smooth_weight = args.smooth.unwrap_or(1.0).clamp(0.0, 1.0); 
    let continuity_scalar = 10_000.0; 

    let mut prev_row_bottom_bits: Vec<Vec<u8>> = vec![vec![0u8; gw]; blocks_width];
    
    let fuzz = args.fuzz.max(0.0);
    let scales = if fuzz > 0.0001 {
        let f = fuzz.max(0.001);
        vec![(1.0, 1.0), (1.0 - f, 1.0), (1.0 + f, 1.0), (1.0, 1.0 - f), (1.0, 1.0 + f)]
    } else { vec![(1.0, 1.0)] };

    let mut grid: Vec<Vec<RenderedCell>> = Vec::new();
    let mut working_pixels_buffer = Vec::with_capacity(bp);
    
    let mut cost_cache_fg_no_inv = vec![0u64; bp];
    let mut cost_cache_bg_no_inv = vec![0u64; bp];
    let mut cost_cache_fg_inv = vec![0u64; bp];
    let mut cost_cache_bg_inv = vec![0u64; bp];

    for (row_idx, block_row) in image.block.iter().enumerate() {
        let mut grid_row = Vec::new();
        let mut first = true;
        let mut last_fg: Option<Colour> = None;
        let mut last_bg: Option<Colour> = None;
        let mut left_neighbor_right_bits: Vec<u8> = vec![0u8; gh];

        for (col_idx, blk) in block_row.iter().enumerate() {
            total_pixels += bp as u64;
            let mut best_overall_cost = f64::MAX;
            let mut best_match_data = (0usize, false, [0u8;3], [0u8;3]); 
            let mut best_scale_is_identity = true;

            let mut original_pixels_rgb: Vec<PrePixel> = Vec::with_capacity(bp);
            for y_idx in 0..gh {
                for x_idx in 0..gw {
                    let [r, g, b] = unpack_rgb(blk.pixels[y_idx][x_idx].orig);
                    original_pixels_rgb.push(PrePixel { r: r as u64, g: g as u64, b: b as u64, sq: (r*r + g*g + b*b) as u64 });
                }
            }
            
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

                let mut std_best_cost = f64::MAX;
                let mut locked_fg = [0u8; 3];
                let mut locked_bg = [0u8; 3];
                
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
                    let sum_zeros_r = block_sum_r - sum_ones_r;
                    let sum_zeros_g = block_sum_g - sum_ones_g;
                    let sum_zeros_b = block_sum_b - sum_ones_b;
                    let sum_zeros_sq = block_sum_sq - sum_ones_sq;

                    let avg_ones = if count_ones > 0 { [(sum_ones_r / count_ones) as u8, (sum_ones_g / count_ones) as u8, (sum_ones_b / count_ones) as u8] } else { [0,0,0] };
                    let avg_zeros = if count_zeros > 0 { [(sum_zeros_r / count_zeros) as u8, (sum_zeros_g / count_zeros) as u8, (sum_zeros_b / count_zeros) as u8] } else { [0,0,0] };

                    // Normal
                    let (fg_c, bg_c) = if render == Render::Ansi24 {
                        let target_bg = if args.fg_only { [0,0,0] } else { avg_zeros };
                        (avg_ones, target_bg)
                    } else {
                        let palette: &[u32] = if render == Render::Ansi { &ANSI256 } else { &IRC99 };
                        let fgi = nearest_hex_colour_fast(make_rgb_u32(&avg_ones), palette);
                        let bgi = if args.fg_only { 
                            nearest_hex_colour_fast(0x000000, palette) 
                        } else { nearest_hex_colour_fast(make_rgb_u32(&avg_zeros), palette) };
                        (unpack_rgb(palette[fgi as usize]), unpack_rgb(palette[bgi as usize]))
                    };

                    let fg_p = PrePixel { r: fg_c[0] as u64, g: fg_c[1] as u64, b: fg_c[2] as u64, sq: (fg_c[0] as u64).pow(2) + (fg_c[1] as u64).pow(2) + (fg_c[2] as u64).pow(2) };
                    let bg_p = PrePixel { r: bg_c[0] as u64, g: bg_c[1] as u64, b: bg_c[2] as u64, sq: (bg_c[0] as u64).pow(2) + (bg_c[1] as u64).pow(2) + (bg_c[2] as u64).pow(2) };
                    let c1 = (sum_ones_sq + count_ones * fg_p.sq).saturating_sub(2 * (fg_p.r * sum_ones_r + fg_p.g * sum_ones_g + fg_p.b * sum_ones_b));
                    let c0 = (sum_zeros_sq + count_zeros * bg_p.sq).saturating_sub(2 * (bg_p.r * sum_zeros_r + bg_p.g * sum_zeros_g + bg_p.b * sum_zeros_b));
                    
                    let coverage = count_ones as f64 / bp as f64;
                    let cost_no_inv = if args.fg_only {
                        (c1 as f64) + (count_zeros as f64 * 1000.0) - (coverage * 0.001)
                    } else {
                        (c1 + c0) as f64 - (coverage * 0.001)
                    };

                    if cost_no_inv < std_best_cost {
                        std_best_cost = cost_no_inv; locked_fg = fg_c; locked_bg = bg_c;
                    }

                    // Inverted
                    let (fg_c_inv, bg_c_inv) = if render == Render::Ansi24 {
                        let target_bg = if args.fg_only { [0,0,0] } else { avg_ones };
                        (avg_zeros, target_bg)
                    } else {
                        let palette: &[u32] = if render == Render::Ansi { &ANSI256 } else { &IRC99 };
                        let fgi = nearest_hex_colour_fast(make_rgb_u32(&avg_zeros), palette);
                        let bgi = if args.fg_only { 
                            nearest_hex_colour_fast(0x000000, palette) 
                        } else { nearest_hex_colour_fast(make_rgb_u32(&avg_ones), palette) };
                        (unpack_rgb(palette[fgi as usize]), unpack_rgb(palette[bgi as usize]))
                    };

                    let fg_p_inv = PrePixel { r: fg_c_inv[0] as u64, g: fg_c_inv[1] as u64, b: fg_c_inv[2] as u64, sq: (fg_c_inv[0] as u64).pow(2) + (fg_c_inv[1] as u64).pow(2) + (fg_c_inv[2] as u64).pow(2) };
                    let bg_p_inv = PrePixel { r: bg_c_inv[0] as u64, g: bg_c_inv[1] as u64, b: bg_c_inv[2] as u64, sq: (bg_c_inv[0] as u64).pow(2) + (bg_c_inv[1] as u64).pow(2) + (bg_c_inv[2] as u64).pow(2) };
                    let c1_inv = (sum_ones_sq + count_ones * bg_p_inv.sq).saturating_sub(2 * (bg_p_inv.r * sum_ones_r + bg_p_inv.g * sum_ones_g + bg_p_inv.b * sum_ones_b));
                    let c0_inv = (sum_zeros_sq + count_zeros * fg_p_inv.sq).saturating_sub(2 * (fg_p_inv.r * sum_zeros_r + fg_p_inv.g * sum_zeros_g + fg_p_inv.b * sum_zeros_b));
                    
                    let coverage_inv = count_zeros as f64 / bp as f64;
                    let cost_inv = if args.fg_only {
                        (c1_inv as f64) + (count_ones as f64 * 1000.0) - (coverage_inv * 0.001)
                    } else {
                        (c1_inv + c0_inv) as f64 - (coverage_inv * 0.001)
                    };

                    if cost_inv < std_best_cost {
                        std_best_cost = cost_inv; locked_fg = fg_c_inv; locked_bg = bg_c_inv;
                    }
                }

                let mut best_cost = f64::MAX;
                let mut best_glyph_idx = 0;
                let mut best_chosen_inv = false;
                
                let fg_p = PrePixel { r: locked_fg[0] as u64, g: locked_fg[1] as u64, b: locked_fg[2] as u64, sq: (locked_fg[0] as u64).pow(2) + (locked_fg[1] as u64).pow(2) + (locked_fg[2] as u64).pow(2) };
                let bg_p = PrePixel { r: locked_bg[0] as u64, g: locked_bg[1] as u64, b: locked_bg[2] as u64, sq: (locked_bg[0] as u64).pow(2) + (locked_bg[1] as u64).pow(2) + (locked_bg[2] as u64).pow(2) };

                for i in 0..bp {
                    let px = &current_block_pixels_rgb[i];
                    cost_cache_fg_no_inv[i] = (px.sq + fg_p.sq).saturating_sub(2 * (px.r * fg_p.r + px.g * fg_p.g + px.b * fg_p.b));
                    if args.fg_only {
                        cost_cache_bg_no_inv[i] = 1000;
                    } else {
                        cost_cache_bg_no_inv[i] = (px.sq + bg_p.sq).saturating_sub(2 * (px.r * bg_p.r + px.g * bg_p.g + px.b * bg_p.b));
                    }
                    cost_cache_fg_inv[i] = cost_cache_bg_no_inv[i];
                    cost_cache_bg_inv[i] = cost_cache_fg_no_inv[i];
                }

                for (gi, fglyph) in glyphs.iter().enumerate() {
                    let mut cost_internal_no_inv = 0u64;
                    let mut cost_internal_inv = 0u64;
                    for i in 0..bp {
                        if fglyph.bitmap[i] == 1 {
                            cost_internal_no_inv += cost_cache_fg_no_inv[i];
                            cost_internal_inv += cost_cache_fg_inv[i];
                        } else {
                            cost_internal_no_inv += cost_cache_bg_no_inv[i];
                            cost_internal_inv += cost_cache_bg_inv[i];
                        }
                    }
                    
                    let mut boundary_cost_no_inv = 0u64;
                    if smooth_weight > 0.0 {
                        if row_idx > 0 {
                            for x in 0..gw { if fglyph.top[x] != top_neighbor_bits[x] { boundary_cost_no_inv += 1; } }
                        }
                        if col_idx > 0 {
                             for y in 0..gh { if fglyph.left[y] != left_neighbor_bits[y] { boundary_cost_no_inv += 1; } }
                        }
                    }
                    let total_no_inv = (cost_internal_no_inv as f64 + (boundary_cost_no_inv as f64 * continuity_scalar * smooth_weight as f64)) - (fglyph.coverage as f64 * 0.001);
                    if total_no_inv < best_cost { best_cost = total_no_inv; best_glyph_idx = gi; best_chosen_inv = false; }
                    
                    if !args.fg_only {
                        let mut boundary_cost_inv = 0u64;
                        if smooth_weight > 0.0 {
                            if row_idx > 0 {
                                for x in 0..gw { if (1 - fglyph.top[x]) != top_neighbor_bits[x] { boundary_cost_inv += 1; } }
                            }
                            if col_idx > 0 {
                                 for y in 0..gh { if (1 - fglyph.left[y]) != left_neighbor_bits[y] { boundary_cost_inv += 1; } }
                            }
                        }
                        let total_inv = (cost_internal_inv as f64 + (boundary_cost_inv as f64 * continuity_scalar * smooth_weight as f64)) - ((1.0 - fglyph.coverage as f64) * 0.001);
                        if total_inv < best_cost { best_cost = total_inv; best_glyph_idx = gi; best_chosen_inv = true; }
                    }
                }

                if best_cost < best_overall_cost {
                    best_overall_cost = best_cost;
                    best_match_data = (best_glyph_idx, best_chosen_inv, locked_fg, locked_bg);
                    best_scale_is_identity = *sx == 1.0 && *sy == 1.0;
                }
            }

            if !best_scale_is_identity { dynamic_scaling_count += 1; }
            let (best_gi, best_inv, best_fg_c, best_bg_c) = best_match_data;
            let fglyph = &glyphs[best_gi];
            left_neighbor_right_bits.clear();
            for y in 0..gh { left_neighbor_right_bits.push(if best_inv { 1 - fglyph.right[y] } else { fglyph.right[y] }); }
            let mut bottom_edge = Vec::with_capacity(gw);
            for x in 0..gw { bottom_edge.push(if best_inv { 1 - fglyph.bottom[x] } else { fglyph.bottom[x] }); }
            prev_row_bottom_bits[col_idx] = bottom_edge;

            let (fg_col, bg_col) = match render {
                Render::Ansi24 => (Colour::RGB(best_fg_c), if args.fg_only { None } else { Some(Colour::RGB(best_bg_c)) }),
                _ => {
                    let palette: &[u32] = if render == Render::Ansi { &ANSI256 } else { &IRC99 };
                    (Colour::Index(nearest_hex_colour_fast(make_rgb_u32(&best_fg_c), palette)), if args.fg_only { None } else { Some(Colour::Index(nearest_hex_colour_fast(make_rgb_u32(&best_bg_c), palette))) })
                }
            };

            grid_row.push(RenderedCell { char: fglyph.ch, fg: best_fg_c, bg: if args.fg_only { None } else { Some(best_bg_c) } });
            emit_colourized(&mut out, render, fg_col, bg_col, fglyph.ch, &mut first, &mut last_fg, &mut last_bg);
        }
        if args.fg_only {
            out.push_str(match render { Render::Ansi | Render::Ansi24 => "\x1b[39m\n", Render::Irc => "\x03\n" });
        } else {
            out.push_str(match render { Render::Ansi | Render::Ansi24 => "\x1b[0m\n", Render::Irc => "\x0f\n" });
        }
        grid.push(grid_row);
    }
    RenderResult { content: out.trim_end_matches('\n').into(), error_count: 0, total_pixels, dynamic_scaling_count, grid }
}

pub fn calculate_smoothness(res: &RenderResult, _glyphs: &GlyphStore) -> f64 {
    let mut transitions = 0u64;
    for row in &res.grid {
        for i in 0..row.len().saturating_sub(1) {
            if row[i].char != row[i+1].char || row[i].fg != row[i+1].fg || row[i].bg != row[i+1].bg { transitions += 1; }
        }
    }
    transitions as f64
}

pub fn calculate_smoothness_relative(res: &RenderResult, glyphs: &GlyphStore) -> f64 {
    calculate_smoothness(res, glyphs)
}

pub fn calculate_frequency_metric(res: &RenderResult, _image: &AnsiImage, _glyphs: &GlyphStore) -> f64 {
    res.error_count as f64
}

pub fn save_as_png(res: &RenderResult, glyphs: &GlyphStore, path: &str, _metadata: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (gh, gw) = glyphs.metrics;
    let rows = res.grid.len();
    let cols = if rows > 0 { res.grid[0].len() } else { 0 };
    let mut img = image::RgbImage::new((cols * gw) as u32, (rows * gh) as u32);

    let glyph_map: HashMap<char, &Vec<Vec<u8>>> = glyphs.glyphs.iter().map(|(c, b)| (*c, b)).collect();

    for (ry, row) in res.grid.iter().enumerate() {
        for (rx, cell) in row.iter().enumerate() {
            let bmp = glyph_map.get(&cell.char).unwrap();
            for y in 0..gh {
                for x in 0..gw {
                    let is_fg = bmp[y][x] == 1;
                    let color = if is_fg { cell.fg } else { cell.bg.unwrap_or([0,0,0]) };
                    img.put_pixel((rx * gw + x) as u32, (ry * gh + y) as u32, image::Rgb(color));
                }
            }
        }
    }
    img.save(path)?;
    Ok(())
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
            let lerp = |a: u64, b: u64, t: f32| (a as f32 * (1.0 - t) + b as f32 * t);
            let r = lerp(lerp(p00.r, p10.r, wx) as u64, lerp(p01.r, p11.r, wx) as u64, wy) as u64;
            let g = lerp(lerp(p00.g, p10.g, wx) as u64, lerp(p01.g, p11.g, wx) as u64, wy) as u64;
            let b = lerp(lerp(p00.b, p10.b, wx) as u64, lerp(p01.b, p11.b, wx) as u64, wy) as u64;
            out.push(PrePixel { r, g, b, sq: r*r + g*g + b*b });
        }
    }
}