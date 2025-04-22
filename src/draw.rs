use crate::args;
use crate::palette::{RGB99, RGB88, ANSI232, ANSI256, nearest_hex_color};
use photon_rs::PhotonImage;
use std::collections::HashMap;

const UP: &str = "\u{2580}";

static QUARTER_BLOCKS: [&str; 16] = [
    " ",          // 0b0000
    "\u{2597}",   // ▗ Quadrant lower right
    "\u{2596}",   // ▖ Quadrant lower left
    "\u{2584}",   // ▄ Lower half block
    "\u{259D}",   // ▝ Quadrant upper right
    "\u{2590}",   // ▐ Right half block
    "\u{259E}",   // ▞ Quadrant upper right and lower left
    "\u{259F}",   // ▟ Quadrant upper right, lower left, lower right
    "\u{2598}",   // ▘ Quadrant upper left
    "\u{259A}",   // ▚ Quadrant upper left and lower right
    "\u{258C}",   // ▌ Left half block
    "\u{2599}",   // ▙ Quadrant upper left, lower left, lower right
    "\u{2580}",   // ▀ Upper half block
    "\u{259C}",   // ▜ Quadrant upper left, upper right, lower right
    "\u{259B}",   // ▛ Quadrant upper left, upper right, lower left
    "\u{2588}",   // █ Full block
];

const POSITIONS: [(usize, usize, u32); 8] = [
    (0, 0, 0x01), // dot 1
    (0, 1, 0x02), // dot 2
    (0, 2, 0x04), // dot 3
    (1, 0, 0x08), // dot 4
    (1, 1, 0x10), // dot 5
    (1, 2, 0x20), // dot 6
    (0, 3, 0x40), // dot 7
    (1, 3, 0x80), // dot 8
];

#[derive(Debug, Clone)]
pub struct AnsiImage {
    pub bitmap: Vec<Vec<u32>>,
    pub halfblock: Vec<Vec<AnsiPixelPair>>,
    pub quarterblock: Vec<Vec<AnsiPixelQuad>>,
}

#[derive(Debug, Clone, Copy)]
pub struct AnsiPixelPair {
    pub top: AnsiPixel,
    pub bottom: AnsiPixel,
}

#[derive(Debug, Clone, Copy)]
pub struct AnsiPixel {
    pub orig: u32,
    pub ansi: u8,
    pub ansi232: u8,
    pub irc: u8,
    pub irc88: u8,
}

#[derive(Debug, Clone, Copy)]
pub struct AnsiPixelQuad {
    pub top_left: AnsiPixel,
    pub top_right: AnsiPixel,
    pub bottom_left: AnsiPixel,
    pub bottom_right: AnsiPixel,
}

impl AnsiPixel {
    pub fn new(pixel: &u32) -> AnsiPixel {
        let irc = nearest_hex_color(*pixel, RGB99.to_vec());
        let irc88 = nearest_hex_color(*pixel, RGB88.to_vec());
        let ansi = nearest_hex_color(*pixel, ANSI256.to_vec());
        let ansi232 = nearest_hex_color(*pixel, ANSI232.to_vec());
        AnsiPixel {
            orig: *pixel,
            ansi,
            ansi232,
            irc,
            irc88,
        }
    }
}

impl AnsiImage {
    pub fn new(image: PhotonImage) -> AnsiImage {
        let mut bitmap = image
            .get_raw_pixels()
            .chunks(4)
            .map(|x| make_rgb_u32(x.to_vec()))
            .collect::<Vec<u32>>()
            .chunks(image.get_width() as usize)
            .map(|x| x.to_vec())
            .collect::<Vec<Vec<u32>>>();

        if bitmap.len() % 2 != 0 {
            bitmap.push(vec![0; image.get_width() as usize]);
        }

        for row in &mut bitmap {
            if row.len() % 2 != 0 {
                row.push(0);
            }
        }

        let halfblock = halfblock_bitmap(&bitmap);
        let quarterblock = quarterblock_bitmap(&bitmap);

        AnsiImage {                                          
            bitmap,
            halfblock,
            quarterblock,
        }
    }
}

pub fn make_rgb_u8(rgb: u32) -> [u8; 3] {
    let r = (rgb >> 16) as u8;
    let g = (rgb >> 8) as u8;
    let b = rgb as u8;

    [r, g, b]
}

pub fn make_rgb_u32(rgb: Vec<u8>) -> u32 {
    let r = rgb[0] as u32;
    let g = rgb[1] as u32;
    let b = rgb[2] as u32;

    (r << 16) + (g << 8) + b
}

pub fn quarterblock_bitmap(bitmap: &Vec<Vec<u32>>) -> Vec<Vec<AnsiPixelQuad>> {
    let ansi_bitmap = bitmap
        .iter()
        .map(|x| x.iter().map(|y| AnsiPixel::new(y)).collect::<Vec<AnsiPixel>>())
        .collect::<Vec<Vec<AnsiPixel>>>();

    let mut ansi_canvas: Vec<Vec<AnsiPixelQuad>> = Vec::new();

    for two_rows in ansi_bitmap.chunks(2) {
        let top_row = &two_rows[0];
        let bottom_row = &two_rows[1];

        let mut ansi_row: Vec<AnsiPixelQuad> = Vec::new();

        for i in (0..top_row.len()).step_by(2) {
            let default_pixel = AnsiPixel::new(&0);
            let top_left_pixel = top_row.get(i).unwrap_or(&default_pixel);
            let top_right_pixel = top_row.get(i + 1).unwrap_or(&default_pixel);
            let bottom_left_pixel = bottom_row.get(i).unwrap_or(&default_pixel);
            let bottom_right_pixel = bottom_row.get(i + 1).unwrap_or(&default_pixel);

            let pixel_quad = AnsiPixelQuad {
                top_left: *top_left_pixel,
                top_right: *top_right_pixel,
                bottom_left: *bottom_left_pixel,
                bottom_right: *bottom_right_pixel,
            };

            ansi_row.push(pixel_quad);
        }

        ansi_canvas.push(ansi_row);
    }

    ansi_canvas
}

pub fn halfblock_bitmap(bitmap: &Vec<Vec<u32>>) -> Vec<Vec<AnsiPixelPair>> {
    let ansi_bitmap = bitmap
        .iter()
        .map(|x| x.iter().map(|y| AnsiPixel::new(y)).collect::<Vec<AnsiPixel>>())
        .collect::<Vec<Vec<AnsiPixel>>>();

    let mut ansi_canvas: Vec<Vec<AnsiPixelPair>> = Vec::new();

    for two_rows in ansi_bitmap.chunks(2) {
        let top_row = &two_rows[0];
        let bottom_row = &two_rows[1];

        let mut ansi_row: Vec<AnsiPixelPair> = Vec::new();

        for i in 0..top_row.len() {
            let default_pixel = AnsiPixel::new(&0);
            let top_pixel = top_row.get(i).unwrap_or(&default_pixel);
            let bottom_pixel = bottom_row.get(i).unwrap_or(&default_pixel);

            let pixel_pair = AnsiPixelPair {
                top: *top_pixel,
                bottom: *bottom_pixel,
            };

            ansi_row.push(pixel_pair);
        }

        ansi_canvas.push(ansi_row);
    }

    ansi_canvas
}

fn get_qb_char<T: PartialEq>(pixels: &[T; 4], fg_color: &T) -> &'static str {
    let mut pattern = 0;
    if pixels[2] == *fg_color {
        pattern |= 1 << 0; // bit 0 (bottom-left)
    }
    if pixels[3] == *fg_color {
        pattern |= 1 << 1; // bit 1 (bottom-right)
    }
    if pixels[0] == *fg_color {
        pattern |= 1 << 2; // bit 2 (top-left)
    }
    if pixels[1] == *fg_color {
        pattern |= 1 << 3; // bit 3 (top-right)
    }
    QUARTER_BLOCKS[pattern as usize]
}

pub fn irc_draw_qb(image: &AnsiImage, args: &args::Args) -> String {
    let mut out: String = String::new();
    for (y, row) in image.quarterblock.iter().enumerate() {
        let mut last_fg: u8 = 255;
        let mut last_bg: u8 = 255;
        for (x, pixel_quad) in row.iter().enumerate() {

            let c0 = if args.nograyscale {
                pixel_quad.top_right.irc88
            } else {
                pixel_quad.top_right.irc
            };
            let c1 = if args.nograyscale {
                pixel_quad.top_left.irc88
            } else {
                pixel_quad.top_left.irc
            };

            let c2 = if args.nograyscale {
                pixel_quad.bottom_right.irc88
            } else {
                pixel_quad.bottom_right.irc
            };

            let c3 = if args.nograyscale {
                pixel_quad.bottom_left.irc88
            } else {
                pixel_quad.bottom_left.irc
            };

            let mut color_counts = HashMap::new();
            for &color in &[c0, c1, c2, c3] {
                *color_counts.entry(color).or_insert(0) += 1;
            }

            let bg_color = *color_counts
                .iter()
                .max_by_key(|entry| entry.1)
                .map(|(color, _)| color)
                .unwrap_or(&0);
            let fg_color = *color_counts
                .iter()
                .filter(|&(color, _)| color != &bg_color)
                .map(|(color, _)| color)
                .next()
                .unwrap_or(&bg_color);

            let pixels = [c0, c1, c2, c3];
            let char = get_qb_char(&pixels, &fg_color);

            if x == 0 || fg_color != last_fg || bg_color != last_bg {
                if fg_color == bg_color {
                    out.push_str(&format!("\x03{}{}", fg_color, char));
                } else {
                    out.push_str(&format!("\x03{},{}{}", fg_color, bg_color, char));
                }
            } else {
                out.push_str(&char);
            }

            last_fg = fg_color;
            last_bg = bg_color;
        }

        out.push_str("\x0f");

        if y != image.quarterblock.len() - 1 {
            out.push_str("\n");
        }
    }
    out.trim_end().to_string()
}

pub fn ansi_draw_24bit_qb(image: &AnsiImage) -> String {
    let mut out: String = String::new();
    for row in &image.quarterblock {
        for pixel_quad in row.iter() {

            let c0_rgb = make_rgb_u8(pixel_quad.top_right.orig);
            let c1_rgb = make_rgb_u8(pixel_quad.top_left.orig);
            let c2_rgb = make_rgb_u8(pixel_quad.bottom_right.orig);
            let c3_rgb = make_rgb_u8(pixel_quad.bottom_left.orig);

            let mut color_counts = HashMap::new();
            for &rgb in &[c0_rgb, c1_rgb, c2_rgb, c3_rgb] {
                *color_counts.entry(rgb).or_insert(0) += 1;
            }

            let bg_color = *color_counts
                .iter()
                .max_by_key(|entry| entry.1)
                .map(|(rgb, _)| rgb)
                .unwrap_or(&[0, 0, 0]);
            let fg_color = *color_counts
                .iter()
                .filter(|&(rgb, _)| rgb != &bg_color)
                .map(|(rgb, _)| rgb)
                .next()
                .unwrap_or(&bg_color);

            let pixels = [c0_rgb, c1_rgb, c2_rgb, c3_rgb];
            let char = get_qb_char(&pixels, &fg_color);

            out.push_str(&format!(
                "\x1b[38;2;{};{};{}m\x1b[48;2;{};{};{}m{}",
                fg_color[0],
                fg_color[1],
                fg_color[2],
                bg_color[0],
                bg_color[1],
                bg_color[2],
                char
            ));
        }
        out.push_str("\x1b[0m\n");
    }
    out.trim_end().to_string()
}

pub fn ansi_draw_8bit_qb(image: &AnsiImage, args: &args::Args) -> String {
    let mut out: String = String::new();
    for row in &image.quarterblock {
        for pixel_quad in row.iter() {

            let c0 = if args.nograyscale {
                pixel_quad.top_right.ansi232
            } else {
                pixel_quad.top_right.ansi
            };
            let c1 = if args.nograyscale {
                pixel_quad.top_left.ansi232
            } else {
                pixel_quad.top_left.ansi
            };
            let c2 = if args.nograyscale {
                pixel_quad.bottom_right.ansi232
            } else {
                pixel_quad.bottom_right.ansi
            };
            let c3 = if args.nograyscale {
                pixel_quad.bottom_left.ansi232
            } else {
                pixel_quad.bottom_left.ansi
            };

            let mut color_counts = HashMap::new();
            for &color in &[c0, c1, c2, c3] {
                *color_counts.entry(color).or_insert(0) += 1;
            }

            let bg_color = *color_counts
                .iter()
                .max_by_key(|entry| entry.1)
                .map(|(color, _)| color)
                .unwrap_or(&0);
            let fg_color = *color_counts
                .iter()
                .filter(|&(color, _)| color != &bg_color)
                .map(|(color, _)| color)
                .next()
                .unwrap_or(&bg_color);

            let pixels = [c0, c1, c2, c3];
            let char = get_qb_char(&pixels, &fg_color);

            out.push_str(&format!(
                "\x1b[38;5;{}m\x1b[48;5;{}m{}",
                fg_color,
                bg_color,
                char
            ));
        }
        out.push_str("\x1b[0m\n");
    }
    out.trim_end().to_string()
}

pub fn ansi_draw_24bit(image: &AnsiImage) -> String {
    let mut out: String = String::new();
    for row in &image.halfblock {
        for pixel_pair in row.iter() {
            let fg = make_rgb_u8(pixel_pair.top.orig)
                .to_vec()
                .iter()
                .map(|x| x.to_string())
                .collect::<Vec<String>>();

            let bg = make_rgb_u8(pixel_pair.bottom.orig)
                .to_vec()
                .iter()
                .map(|x| x.to_string())
                .collect::<Vec<String>>();

            out.push_str(
                format!(
                    "\x1b[38;2;{}m\x1b[48;2;{}m{}",
                    fg.join(";"),
                    bg.join(";"),
                    UP
                )
                .as_str(),
            );
        }
        out.push_str("\x1b[0m\n");
    }
    out.trim_end().to_string()
}

pub fn ansi_draw_8bit(image: &AnsiImage, args: &args::Args) -> String {
    let mut out: String = String::new();
    for row in &image.halfblock {
        for pixel_pair in row.iter() {

            let fg = if args.nograyscale {
                pixel_pair.top.ansi232
            } else {
                pixel_pair.top.ansi
            };

            let bg = if args.nograyscale {
                pixel_pair.bottom.ansi232
            } else {
                pixel_pair.bottom.ansi
            };

            out.push_str(format!("\x1b[38;5;{}m\x1b[48;5;{}m{}", fg, bg, UP).as_str());
        }
        out.push_str("\x1b[0m\n");
    }
    out.trim_end().to_string()
}

pub fn irc_draw(image: &AnsiImage, args: &args::Args) -> String {
    let mut out: String = String::new();
    for row in &image.halfblock {
        let mut last_fg: u8 = 255;
        let mut last_bg: u8 = 255;
        for (x, pixel_pair) in row.iter().enumerate() {

            let fg = if args.nograyscale {
                pixel_pair.top.irc88
            } else {
                pixel_pair.top.irc
            };

            let bg = if args.nograyscale {
                pixel_pair.bottom.irc88
            } else {
                pixel_pair.bottom.irc
            };

            if x == 0 || fg != last_fg || bg != last_bg {
                if last_bg == bg {
                    out.push_str(&format!("\x03{}{}", fg, UP));
                } else {
                    out.push_str(&format!("\x03{},{}{}", fg, bg, UP));
                }
            } else {
                out.push_str(&UP);
            }

            last_fg = fg;
            last_bg = bg;
        }

        out.push_str("\x0f\n");
    }
    out.trim_end().to_string()
}

pub fn luma(rgb: &[u8; 3]) -> u8 {
    let r = rgb[0] as u16;
    let g = rgb[1] as u16;
    let b = rgb[2] as u16;
    ((0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32).round()) as u8
}

pub fn ansi_draw_braille_24bit(image_luma: &AnsiImage, image_chroma: &AnsiImage) -> String {
    let mut out: String = String::new();
    let bitmap_luma = &image_luma.bitmap;
    let bitmap_chroma = &image_chroma.bitmap;
    let height = bitmap_luma.len();
    let width = bitmap_luma[0].len();

    let mut error_matrix = vec![vec![0.0; width]; height];
    
    let mut min_luma = 255u32;
    let mut max_luma = 0u32;
    for row in bitmap_luma.iter() {
        for &pixel in row.iter() {
            let l = luma(&make_rgb_u8(pixel)) as u32;
            min_luma = min_luma.min(l);
            max_luma = max_luma.max(l);
        }
    }

    let luma_range = max_luma.saturating_sub(min_luma).max(1);

    let scaled_threshold = min_luma + (luma_range / 2);

    for y in (0..height).step_by(4) {
        let mut last_fg = [0u8; 3];
        let mut first = true;
        for x in (0..width).step_by(2) {
            let mut braille_char = 0x2800;
            let mut color_counts: HashMap<[u8; 3], u32> = HashMap::new();

            for &(dx, dy, bit) in &POSITIONS {
                let current_y = y + dy;
                let current_x = x + dx;
                if current_y < height && current_x < width {
                    let original_pixel = bitmap_luma[current_y][current_x];
                    let rgb = make_rgb_u8(original_pixel);
                    let current_luma = luma(&rgb) as f64;

                    let mut pixel_luma = current_luma + error_matrix[current_y][current_x];
                    pixel_luma = pixel_luma.clamp(0.0, 255.0);

                    let new_pixel = if pixel_luma > scaled_threshold as f64 {
                        255.0
                    } else {
                        0.0
                    };

                    let error = pixel_luma - new_pixel;

                    if new_pixel == 255.0 {
                        braille_char |= bit;

                        let px_chroma = bitmap_chroma[current_y][current_x];
                        let rgb_chroma = make_rgb_u8(px_chroma);
                        *color_counts.entry(rgb_chroma).or_insert(0) += 1;
                    }

                    if current_x + 1 < width {
                        error_matrix[current_y][current_x + 1] += error * 7.0 / 16.0;
                    }
                    if current_y + 1 < height {
                        if current_x > 0 {
                            error_matrix[current_y + 1][current_x - 1] += error * 3.0 / 16.0;
                        }
                        error_matrix[current_y + 1][current_x] += error * 5.0 / 16.0;
                        if current_x + 1 < width {
                            error_matrix[current_y + 1][current_x + 1] += error * 1.0 / 16.0;
                        }
                    }
                }
            }

            let fg_color = color_counts
                .iter()
                .max_by_key(|entry| entry.1)
                .map(|(color, _)| *color)
                .unwrap_or([0, 0, 0]);
            let braille_char = char::from_u32(braille_char).unwrap_or(' ');

            if first || last_fg != fg_color {
                out.push_str(&format!(
                    "\x1b[38;2;{};{};{}m{}",
                    fg_color[0], fg_color[1], fg_color[2], braille_char
                ));
            } else {
                out.push(braille_char);
            }
            last_fg = fg_color;
            first = false;
        }
        out.push_str("\x1b[0m\n");
    }
    out.trim_end().to_string()
}


pub fn ansi_draw_braille_8bit(
    image_luma: &AnsiImage,
    image_chroma: &AnsiImage,
    args: &args::Args,
) -> String {
    let mut out: String = String::new();
    let bitmap_luma = &image_luma.bitmap;
    let bitmap_chroma = &image_chroma.bitmap;
    let height = bitmap_luma.len();
    let width = bitmap_luma[0].len();

    let mut error_matrix = vec![vec![0.0; width]; height];
    
    let mut min_luma = 255u32;
    let mut max_luma = 0u32;
    for row in bitmap_luma.iter() {
        for &pixel in row.iter() {
            let l = luma(&make_rgb_u8(pixel)) as u32;
            min_luma = min_luma.min(l);
            max_luma = max_luma.max(l);
        }
    }

    let luma_range = max_luma.saturating_sub(min_luma).max(1);

    let scaled_threshold = min_luma + (luma_range / 2);

    for y in (0..height).step_by(4) {
        let mut last_fg = 255u8; // Initialize with a default color
        let mut first = true;
        for x in (0..width).step_by(2) {
            let mut braille_char = 0x2800; // Base Unicode braille character
            let mut color_counts: HashMap<u8, usize> = HashMap::new(); // Color counting

            for &(dx, dy, bit) in &POSITIONS {
                let current_y = y + dy;
                let current_x = x + dx;
                if current_y < height && current_x < width {
                    let original_pixel = bitmap_luma[current_y][current_x];
                    let rgb = make_rgb_u8(original_pixel);
                    let current_luma = luma(&rgb) as f64;

                    let mut pixel_luma = current_luma + error_matrix[current_y][current_x];
                    pixel_luma = pixel_luma.clamp(0.0, 255.0);

                    let new_pixel = if pixel_luma > scaled_threshold as f64 {
                        255.0
                    } else {
                        0.0
                    };

                    let error = pixel_luma - new_pixel;

                    if new_pixel == 255.0 {
                        braille_char |= bit;

                        let px_chroma = bitmap_chroma[current_y][current_x];
                        let color = if args.nograyscale {
                            nearest_hex_color(px_chroma, ANSI232.to_vec())
                        } else {
                            nearest_hex_color(px_chroma, ANSI256.to_vec())
                        };
                        *color_counts.entry(color).or_insert(0) += 1;
                    }

                    if current_x + 1 < width {
                        error_matrix[current_y][current_x + 1] += error * 7.0 / 16.0;
                    }
                    if current_y + 1 < height {
                        if current_x > 0 {
                            error_matrix[current_y + 1][current_x - 1] += error * 3.0 / 16.0;
                        }
                        error_matrix[current_y + 1][current_x] += error * 5.0 / 16.0;
                        if current_x + 1 < width {
                            error_matrix[current_y + 1][current_x + 1] += error * 1.0 / 16.0;
                        }
                    }
                }
            }

            let fg_color = *color_counts
                .iter()
                .max_by_key(|entry| entry.1)
                .map(|(color, _)| color)
                .unwrap_or(&last_fg); // Default to last_fg if no colors are counted

            let braille_char = char::from_u32(braille_char).unwrap_or(' ');

            if first || last_fg != fg_color {
                out.push_str(&format!("\x1b[38;5;{}m{}", fg_color, braille_char));
            } else {
                out.push(braille_char);
            }
            last_fg = fg_color;
            first = false;
        }
        out.push_str("\x1b[0m\n"); // Reset colors and move to the next line
    }
    out.trim_end().to_string()
}

pub fn irc_draw_braille(image_luma: &AnsiImage, image_chroma: &AnsiImage, args: &args::Args) -> String {
    let mut out: String = String::new();
    let bitmap_luma = &image_luma.bitmap;
    let bitmap_chroma = &image_chroma.bitmap;
    let height = bitmap_luma.len();
    let width = bitmap_luma[0].len();

    let mut error_matrix = vec![vec![0.0; width]; height];
    
    let mut min_luma = 255u32;
    let mut max_luma = 0u32;

    for row in bitmap_luma.iter() {
        for &pixel in row.iter() {
            let l = luma(&make_rgb_u8(pixel)) as u32;
            min_luma = min_luma.min(l);
            max_luma = max_luma.max(l);
        }
    }

    let luma_range = max_luma.saturating_sub(min_luma).max(1);

    let scaled_threshold = min_luma + (luma_range / 2);

    for y in (0..height).step_by(4) {
        let mut last_fg = 1u8;
        let mut first = true;
        for x in (0..width).step_by(2) {
            let mut braille_char = 0x2800;
            let mut color_counts = HashMap::new();

            for &(dx, dy, bit) in &POSITIONS {
                let current_y = y + dy;
                let current_x = x + dx;
                if current_y < height && current_x < width {
                    let original_pixel = bitmap_luma[current_y][current_x];
                    let rgb = make_rgb_u8(original_pixel);
                    let current_luma = luma(&rgb) as f64;

                    let mut pixel_luma = current_luma + error_matrix[current_y][current_x];
                    pixel_luma = pixel_luma.clamp(0.0, 255.0);

                    let new_pixel = if pixel_luma > scaled_threshold as f64 {
                        255.0
                    } else {
                        0.0
                    };

                    let error = pixel_luma - new_pixel;

                    if new_pixel == 255.0 {
                        braille_char |= bit;

                        let px_chroma = bitmap_chroma[current_y][current_x];
                        let color = if args.nograyscale {
                            nearest_hex_color(px_chroma, RGB88.to_vec())
                        } else {
                            nearest_hex_color(px_chroma, RGB99.to_vec())
                        };
                        *color_counts.entry(color).or_insert(0) += 1;
                    }

                    if current_x + 1 < width {
                        error_matrix[current_y][current_x + 1] += error * 7.0 / 16.0;
                    }
                    if current_y + 1 < height {
                        if current_x > 0 {
                            error_matrix[current_y + 1][current_x - 1] += error * 3.0 / 16.0;
                        }
                        error_matrix[current_y + 1][current_x] += error * 5.0 / 16.0;
                        if current_x + 1 < width {
                            error_matrix[current_y + 1][current_x + 1] += error * 1.0 / 16.0;
                        }
                    }
                }
            }

            let fg_color = *color_counts
                .iter()
                .max_by_key(|entry| entry.1)
                .map(|(color, _)| color)
                .unwrap_or(&last_fg);

            let braille_char = char::from_u32(braille_char).unwrap_or(' ');

            if first || fg_color != last_fg {
                out.push_str(&format!("\x03{}{}", fg_color, braille_char));
            } else {
                out.push(braille_char);
            }
            last_fg = fg_color;
            first = false;
        }
        out.push_str("\x0f\n");
    }
    out.trim_end().to_string()
}
