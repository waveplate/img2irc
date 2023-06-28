use crate::args;
use crate::palette::{RGB99, RGB88, ANSI232, ANSI256, nearest_hex_color};
use photon_rs::PhotonImage;

// █ full
const FULL: &str = "\u{2588}";

// ▄ down
const UP: &str = "\u{2580}";

// ▀ up
const DOWN: &str = "\u{2584}";

// ▌ left
const LEFT: &str = "\u{258C}";

// ▐ right
const RIGHT: &str = "\u{2590}";

// ▞ diag_right
const DIAG_RIGHT: &str = "\u{259E}";

// ▚ diag_left
const DIAG_LEFT: &str = "\u{259A}";

// ▙ down_left (2596 prev)
const DOWN_LEFT: &str = "\u{2599}";

// ▟ down_right
const DOWN_RIGHT: &str = "\u{259F}";

// ▛ top_left
const UP_LEFT: &str = "\u{259B}";

// ▜ top_right
const UP_RIGHT: &str = "\u{259C}";

#[derive(Debug, Clone)]
pub struct AnsiImage {
    pub image: PhotonImage,
    pub bitmap: Vec<Vec<u32>>,
    pub halfblock: Vec<Vec<AnsiPixelPair>>,
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

impl AnsiPixel {
    pub fn new(pixel: &u32) -> AnsiPixel {
        let irc = nearest_hex_color(*pixel, RGB99.to_vec());
        let irc88 = nearest_hex_color(*pixel, RGB88.to_vec());
        let ansi = nearest_hex_color(*pixel, ANSI256.to_vec());
        let ansi232 = nearest_hex_color(*pixel, ANSI232.to_vec());
        AnsiPixel {
            orig: *pixel,
            ansi: ansi,
            ansi232: ansi232,
            irc: irc,
            irc88: irc88,
        }
    }
}

impl AnsiImage {
    pub fn new(image: PhotonImage) -> AnsiImage {
        let mut bitmap = image.get_raw_pixels()
            .chunks(4)
            .map(|x| make_rgb_u32(x.to_vec()))
            .collect::<Vec<u32>>()
            .chunks(image.get_width() as usize)
            .map(|x| x.to_vec())
            .collect::<Vec<Vec<u32>>>();

        if bitmap.len() % 2 != 0 {
            bitmap.push(vec![0; image.get_width() as usize]);
        }

        let halfblock = halfblock_bitmap(&bitmap);
 
        return AnsiImage {
            image: image,
            bitmap: bitmap,
            halfblock: halfblock,
        }
    }
}

pub fn make_rgb_u8(rgb: u32) -> [u8; 3] {
    let r = (rgb >> 16) as u8;
    let g = (rgb >> 8) as u8;
    let b = rgb as u8;

    return [r, g, b]
}

pub fn make_rgb_u32(rgb: Vec<u8>) -> u32 {
    let r = *rgb.get(0).unwrap() as u32;
    let g = *rgb.get(1).unwrap() as u32;
    let b = *rgb.get(2).unwrap() as u32;

    let rgb = (r << 16) + (g << 8) + b;

    return rgb
}

pub fn halfblock_bitmap(bitmap: &Vec<Vec<u32>>) -> Vec<Vec<AnsiPixelPair>> {
    let ansi_bitmap = bitmap
    .iter()
    .map(|x| {
       x.iter().map(|y| AnsiPixel::new(y)).collect::<Vec<AnsiPixel>>() 
    })
    .collect::<Vec<Vec<AnsiPixel>>>();

    let mut ansi_canvas: Vec<Vec<AnsiPixelPair>> = Vec::new();

    for two_rows in ansi_bitmap.chunks(2) {
        let rows = two_rows.to_vec();
        let top_row = rows.get(0).unwrap();
        let bottom_row = rows.get(1).unwrap();

        let mut ansi_row: Vec<AnsiPixelPair> = Vec::new();

        for i in 0..bitmap.get(0).unwrap().len() {
            let top_pixel = top_row.get(i as usize).unwrap();
            let bottom_pixel = bottom_row.get(i as usize).unwrap();

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

fn get_qb_char(pixel_pairs: &[AnsiPixelPair]) -> &str {
    let (pair0_top, pair0_bottom) = (&pixel_pairs[0].top.irc, &pixel_pairs[0].bottom.irc);
    let (pair1_top, pair1_bottom) = (&pixel_pairs[1].top.irc, &pixel_pairs[1].bottom.irc);

    let ups_equal = pair0_top == pair1_top;
    let downs_equal = pair0_bottom == pair1_bottom;
    let lefts_equal = pair0_top == pair0_bottom;
    let rights_equal = pair1_top == pair1_bottom;
    let left_diag = pair0_top == pair1_bottom;
    let right_diag = pair1_top == pair0_bottom;

    match (ups_equal, downs_equal, lefts_equal, rights_equal, left_diag, right_diag) {
        (true, _, true, true, _, _) => FULL,
        (true, _, true, _, _, _) => UP_LEFT,
        (true, _, _, true, _, _) => UP_RIGHT,
        (true, _, _, _, _, _) => UP,
        (_, true, true, _, _, _) => DOWN_LEFT,
        (_, true, _, true, _, _) => DOWN_RIGHT,
        (_, true, _, _, _, _) => DOWN,
        (_, _, true, false, _, _) => LEFT,
        (_, _, false, true, _, _) => RIGHT,
        (_, _, _, _, true, _) => DIAG_LEFT,
        (_, _, _, _, _, true) => DIAG_RIGHT,
        _ => UP,
    }
}

pub fn ansi_draw_24bit(image: AnsiImage) -> String {
    let mut out: String = String::new();
    for (y, row) in image.halfblock.iter().enumerate() {
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

            out.push_str(format!("\x1b[38;2;{}m\x1b[48;2;{}m{}", fg.join(";"), bg.join(";"), UP).as_str());
        }
        out.push_str("\x1b[0m");

        if y != image.halfblock.len() - 1 {
            out.push_str("\n");
        }
    }
    return out
}

pub fn ansi_draw_24bit_qb(image: AnsiImage) -> String {
    let mut out: String = String::new();
    for (y, row) in image.halfblock.iter().enumerate() {
        for pixel_pairs in row.chunks(2) {
            let fg = make_rgb_u8(pixel_pairs[0].top.orig)
                .to_vec()
                .iter()
                .map(|x| x.to_string())
                .collect::<Vec<String>>();
            
            let bg = make_rgb_u8(pixel_pairs[0].bottom.orig)
                .to_vec()
                .iter()
                .map(|x| x.to_string())
                .collect::<Vec<String>>();

            let char = match y {
                _ if y == image.halfblock.len() - 1 => UP,
                _ => get_qb_char(pixel_pairs),
            };

            out.push_str(format!("\x1b[38;2;{}m\x1b[48;2;{}m{}", fg.join(";"), bg.join(";"), char).as_str());
        }
        out.push_str("\x1b[0m");

        if y != image.halfblock.len() - 1 {
            out.push_str("\n");
        }
    }
    return out
}

pub fn ansi_draw_8bit(image: AnsiImage, args: &args::Args) -> String {
    let mut out: String = String::new();
    for (y, row) in image.halfblock.iter().enumerate() {
        for pixel_pair in row.iter() {
    
            let fg = match args.nograyscale {
                true => pixel_pair.top.ansi232,
                false => pixel_pair.top.ansi,
            };

            let bg = match args.nograyscale {
                true => pixel_pair.bottom.ansi232,
                false => pixel_pair.bottom.ansi,
            };

            out.push_str(format!("\x1b[38;5;{}m\x1b[48;5;{}m{}", fg, bg, UP).as_str());
        }
        out.push_str("\x1b[0m");

        if y != image.halfblock.len() - 1 {
            out.push_str("\n");
        }
    }
    return out
}

pub fn ansi_draw_8bit_qb(image: AnsiImage, args: &args::Args) -> String {
    let mut out: String = String::new();
    for (y, row) in image.halfblock.iter().enumerate() {
        for pixel_pairs in row.chunks(2) {
            let fg = match args.nograyscale {
                true => pixel_pairs[0].top.ansi232,
                false => pixel_pairs[0].top.ansi,
            };

            let bg = match args.nograyscale {
                true => pixel_pairs[0].bottom.ansi232,
                false => pixel_pairs[0].bottom.ansi,
            };

            let char = match y {
                _ if y == image.halfblock.len() - 1 => UP,
                _ => get_qb_char(pixel_pairs),
            };

            out.push_str(format!("\x1b[38;5;{}m\x1b[48;5;{}m{}", fg, bg, char).as_str());
        }
        out.push_str("\x1b[0m");

        if y != image.halfblock.len() - 1 {
            out.push_str("\n");
        }
    }
    return out
}

pub fn irc_draw(image: AnsiImage, args: &args::Args) -> String {
    let mut out: String = String::new();
    for (y, row) in image.halfblock.iter().enumerate() {
        let mut last_fg: u8 = 0;
        let mut last_bg: u8 = 0;
        for (x, pixel_pair) in row.iter().enumerate() {
            let fg = match args.nograyscale {
                true => pixel_pair.top.irc88,
                false => pixel_pair.top.irc,
            };

            let bg = match args.nograyscale {
                true => pixel_pair.bottom.irc88,
                false => pixel_pair.bottom.irc,
            };

            if x != 0 {
                if fg == last_fg && bg == last_bg {
                    out.push_str(&format!("{}", UP));
                } else if bg == last_bg {
                    out.push_str(&format!("\x03{}{}", fg, UP));
                } else {
                    out.push_str(&format!("\x03{},{}{}", fg, bg, UP));
                }
            } else {
                out.push_str(&format!("\x03{},{}{}", fg, bg, UP));
            }

            last_fg = fg;
            last_bg = bg;
        }

        out.push_str("\x0f");

        if y != image.halfblock.len() - 1 {
            out.push_str("\n");
        }
    }
    return out
}

pub fn irc_draw_qb(image: AnsiImage, args: &args::Args) -> String {
    let mut out: String = String::new();
    for (y, row) in image.halfblock.iter().enumerate() {
        let mut last_fg: u8 = 0;
        let mut last_bg: u8 = 0;
        for (x, pixel_pairs) in row.chunks(2).enumerate() {
            let fg = match args.nograyscale {
                true => pixel_pairs[0].top.irc88,
                false => pixel_pairs[0].top.irc,
            };

            let bg = match args.nograyscale {
                true => pixel_pairs[0].bottom.irc88,
                false => pixel_pairs[0].bottom.irc,
            };

            let char = match y {
                _ if y == image.halfblock.len() - 1 => UP,
                _ => get_qb_char(pixel_pairs),
            };

            if x == 0 {
                out.push_str(&format!("\x03{},{}{}", fg, bg, char));
            } else {
                if fg == last_fg && bg == last_bg {
                    out.push_str(&format!("{}", char));
                } else if bg == last_bg {
                    out.push_str(&format!("\x03{}{}", fg, char));
                } else {
                    out.push_str(&format!("\x03{},{}{}", fg, bg, char));
                }
            }

            last_fg = fg;
            last_bg = bg;
        }

        out.push_str("\x0f");

        if y != image.halfblock.len() - 1 {
            out.push_str("\n");
        }
    }
    return out
}