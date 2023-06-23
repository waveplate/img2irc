use crate::palette::{RGB99, ANSI256, nearest_hex_color};

use photon_rs::PhotonImage;

const CHAR: &str ="\u{2580}";

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
    pub irc: u8,
}

impl AnsiPixel {
    pub fn new(pixel: &u32) -> AnsiPixel {
        let irc = nearest_hex_color(*pixel, RGB99.to_vec());
        let ansi = nearest_hex_color(*pixel, ANSI256.to_vec());
        AnsiPixel {
            orig: *pixel,
            ansi: ansi,
            irc: irc,
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
    // convert u32 to r,g,b
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

            out.push_str(format!("\x1b[38;2;{}m\x1b[48;2;{}m{}", fg.join(";"), bg.join(";"), CHAR).as_str());
        }
        out.push_str("\x1b[0m");

        if y != image.halfblock.len() - 1 {
            out.push_str("\n");
        }
    }
    return out
}

pub fn ansi_draw_8bit(image: AnsiImage) -> String {
    let mut out: String = String::new();
    for (y, row) in image.halfblock.iter().enumerate() {
        for pixel_pair in row.iter() {
            let fg = pixel_pair.top.ansi;
            let bg = pixel_pair.bottom.ansi;

            out.push_str(format!("\x1b[38;5;{}m\x1b[48;5;{}m{}", fg, bg, CHAR).as_str());
        }
        out.push_str("\x1b[0m");

        if y != image.halfblock.len() - 1 {
            out.push_str("\n");
        }
    }
    return out
}

pub fn irc_draw(image: AnsiImage) -> String {
    let mut out: String = String::new();
    for (y, row) in image.halfblock.iter().enumerate() {
        let mut last_fg: u8 = 0;
        let mut last_bg: u8 = 0;
        for (x, pixel_pair) in row.iter().enumerate() {
            let fg = pixel_pair.top.irc;
            let bg = pixel_pair.bottom.irc;

            if x != 0 {
                if fg == last_fg && bg == last_bg {
                    out.push_str(&format!("{}", CHAR));
                } else if bg == last_bg {
                    out.push_str(&format!("\x03{}{}", fg, CHAR));
                } else {
                    out.push_str(&format!("\x03{},{}{}", fg, bg, CHAR));
                }
            } else {
                out.push_str(&format!("\x03{},{}{}", fg, bg, CHAR));
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