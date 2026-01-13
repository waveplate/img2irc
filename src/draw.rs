use crate::args::{Args, Render};
use crate::font::{GlyphBitmap, GlyphStore};
use crate::palette::{IRC99, ANSI256};
use photon_rs::PhotonImage;

use std::collections::HashMap;
use std::iter::repeat;
use std::sync::Mutex;
use once_cell::sync::Lazy;

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

/// Helper to calculate the average RGB color from a slice of u32 RGB values.
#[inline]
fn calculate_average_rgb(colors: &[u32]) -> [u8; 3] {
    if colors.is_empty() {
        return [0, 0, 0]; // Return black if no colors to average
    }
    let mut sum_r: u64 = 0;
    let mut sum_g: u64 = 0;
    let mut sum_b: u64 = 0;
    for &col_u32 in colors {
        sum_r += ((col_u32 >> 16) & 0xFF) as u64;
        sum_g += ((col_u32 >> 8) & 0xFF) as u64;
        sum_b += (col_u32 & 0xFF) as u64;
    }
    let count = colors.len() as u64;
    [
        (sum_r / count) as u8,
        (sum_g / count) as u8,
        (sum_b / count) as u8,
    ]
}

#[inline(always)]
fn diff_rgb(p1: &[u8; 3], p2: &[u8; 3]) -> u64 {
    let dr = p1[0] as i32 - p2[0] as i32;
    let dg = p1[1] as i32 - p2[1] as i32;
    let db = p1[2] as i32 - p2[2] as i32;
    (dr * dr + dg * dg + db * db) as u64
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
    if image.glyphs.glyphs.is_empty() {
        return "Error: no glyphs available".into();
    }

    let bg_opt_rgb = args.bg_opt.as_ref().map(|s| unpack_rgb(parse_hex_color(s)));

    let (gh, gw) = image.glyphs.metrics;
    let bp = gh * gw;

    let glyphs: Vec<(char, &GlyphBitmap)> = image
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
        .map(|(ch, bmp)| (*ch, bmp))
        .collect();

    if glyphs.is_empty() {
        return "Error: no glyphs selected".into();
    }
    let mut out = String::new();
    for block_row in &image.block {
        let mut first = true;
        let mut last_fg: Option<Colour> = None; // Assuming Colour is in parent or use crate::image::Colour
        let mut last_bg: Option<Colour> = None;

        for blk in block_row {
            let final_glyph: char;
            let final_fg: Colour;
            let final_bg: Option<Colour>;

            if render == Render::Ansi24 {
                // Ansi24 specific glyph matching using direct RGB values
                let mut current_block_pixels_rgb: Vec<u32> = Vec::with_capacity(bp);
                for y_idx in 0..gh {
                    for x_idx in 0..gw {
                        current_block_pixels_rgb.push(blk.pixels[y_idx][x_idx].orig);
                    }
                }

                let mut best_cost = u64::MAX;
                let mut best_glyph_idx = 0;
                let mut best_chosen_fg_color = [0u8; 3];
                let mut best_chosen_bg_color = [0u8; 3];

                for (gi, (_, bmp)) in glyphs.iter().enumerate() {
                    let mut pixels_under_glyph_ones: Vec<u32> = Vec::new();
                    let mut pixels_under_glyph_zeros: Vec<u32> = Vec::new();

                    for row in 0..gh {
                        for col in 0..gw {
                            let idx = row * gw + col;
                            let pixel_original_rgb = current_block_pixels_rgb[idx];
                            if bmp[row][col] == 1 {
                                pixels_under_glyph_ones.push(pixel_original_rgb);
                            } else {
                                pixels_under_glyph_zeros.push(pixel_original_rgb);
                            }
                        }
                    }

                    let avg_ones = calculate_average_rgb(&pixels_under_glyph_ones);
                    let avg_zeros = calculate_average_rgb(&pixels_under_glyph_zeros);
                    
                    let bg_ref = if let Some(bg_c) = bg_opt_rgb { Some(bg_c) } else { None };
                    let use_coverage_heuristic = args.fg_only && bg_ref.is_none();
                    let penalty_per_miss = 1000u64;

                    // --- Cost Calculation (Normal: 1=FG, 0=BG) ---
                    let mut cost_no_invert = 0u64;
                    if use_coverage_heuristic {
                        // Heuristic: FG(1) matches avg_ones. BG(0) is ignored but penalized.
                        for row in 0..gh {
                            for col in 0..gw {
                                if bmp[row][col] == 1 {
                                    let idx = row * gw + col;
                                    let px = unpack_rgb(current_block_pixels_rgb[idx]);
                                    let d = diff_rgb(&px, &avg_ones);
                                    cost_no_invert += d;
                                } else {
                                    cost_no_invert += penalty_per_miss;
                                }
                            }
                        }
                    } else {
                        // Standard: FG(1)=avg_ones, BG(0)=avg_zeros (or bg_ref)
                        let target_bg = bg_ref.unwrap_or(avg_zeros);
                        for row in 0..gh {
                            for col in 0..gw {
                                let idx = row * gw + col;
                                let px = unpack_rgb(current_block_pixels_rgb[idx]);
                                let target = if bmp[row][col] == 1 { avg_ones } else { target_bg };
                                cost_no_invert += diff_rgb(&px, &target);
                            }
                        }
                    }

                    if cost_no_invert < best_cost {
                        best_cost = cost_no_invert;
                        best_glyph_idx = gi;
                        best_chosen_fg_color = avg_ones;
                        best_chosen_bg_color = if use_coverage_heuristic { [0,0,0] } else { bg_ref.unwrap_or(avg_zeros) };
                    }

                    // --- Cost Calculation (Inverted: 0=FG, 1=BG) ---
                    let mut cost_invert = 0u64;
                    if use_coverage_heuristic {
                        // Heuristic: FG(0) matches avg_zeros. BG(1) is ignored but penalized.
                        for row in 0..gh {
                            for col in 0..gw {
                                if bmp[row][col] == 0 {
                                    let idx = row * gw + col;
                                    let px = unpack_rgb(current_block_pixels_rgb[idx]);
                                    let d = diff_rgb(&px, &avg_zeros);
                                    cost_invert += d;
                                } else {
                                    cost_invert += penalty_per_miss;
                                }
                            }
                        }
                    } else {
                        // Standard: FG(0)=avg_zeros, BG(1)=avg_ones (or bg_ref)
                        // Note: bg_ref replaces the Background role.
                        // Inverted: 1 is Background. So 1 uses bg_ref.
                        let target_bg = bg_ref.unwrap_or(avg_ones);
                        for row in 0..gh {
                            for col in 0..gw {
                                let idx = row * gw + col;
                                let px = unpack_rgb(current_block_pixels_rgb[idx]);
                                // Inverted map: 0->FG(avg_zeros), 1->BG(target_bg)
                                let target = if bmp[row][col] == 0 { avg_zeros } else { target_bg };
                                cost_invert += diff_rgb(&px, &target);
                            }
                        }
                    }

                    if cost_invert < best_cost {
                        best_cost = cost_invert;
                        best_glyph_idx = gi;
                        best_chosen_fg_color = avg_zeros;
                        best_chosen_bg_color = if use_coverage_heuristic { [0,0,0] } else { bg_ref.unwrap_or(avg_ones) };
                    }

                    if best_cost == 0 {
                        break;
                    }
                }

                let glyph_char = glyphs[best_glyph_idx].0;
                final_glyph = glyph_char;
                final_fg = Colour::RGB(best_chosen_fg_color);
                final_bg = if args.fg_only { None } else { Some(Colour::RGB(best_chosen_bg_color)) };

            } else {
                // Palette-based glyph matching logic for Render::Ansi and Render::Irc
                let mut code = vec![0u8; bp];
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
                        code[y_idx * gw + x_idx] = idx;
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

                for (gi, (_, bmp)) in glyphs.iter().enumerate() {
                    for &inv in &[false, true] {
                        let mut fg_tot: usize = 0;
                        let mut bg_tot: usize = 0;
                        let mut fg_cnt = [0usize; 256];
                        let mut bg_cnt = [0usize; 256];

                        for row in 0..gh {
                            for col in 0..gw {
                                let idx = row * gw + col;
                                let col_idx = code[idx] as usize;
                                if bmp[row][col] ^ (inv as u8) == 1 {
                                    fg_tot += 1;
                                    fg_cnt[col_idx] += 1;
                                } else {
                                    bg_tot += 1;
                                    bg_cnt[col_idx] += 1;
                                }
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

                        let cost = if args.fg_only && bg_opt_idx.is_none() {
                            // Heuristic: Score = 2 * Matches - Total_FG_Pixels
                            // Cost = Total_Pixels - Score
                            let total_pixels = bp;
                            let score = (2 * fgm_val as isize) - (fg_tot as isize);
                            ((total_pixels as isize) - score) as usize
                        } else {
                            (fg_tot.saturating_sub(fgm_val)) + (bg_tot.saturating_sub(bgm_effective))
                        };

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

                let glyph_entry = glyphs[best.1].0;
                let (fg_idx, bg_idx) = if best.4 { (best.3, best.2) } else { (best.2, best.3) };

                final_glyph = glyph_entry;
                final_fg = Colour::Index(fg_idx);
                final_bg = if args.fg_only { None } else { Some(Colour::Index(bg_idx)) };
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
