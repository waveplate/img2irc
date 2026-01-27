mod args;
mod draw;
mod palette;
mod effects;
mod font;

use reqwest;
use url::Url;
use photon_rs::PhotonImage;
use std::{error::Error, io::{Cursor, Write}, process::exit};
use crate::args::Encoding;

#[tokio::main]
async fn main() {
    let args = args::parse_args();

    if args.clear_cache {
        font::clear_cache();
        eprintln!("Cache cleared.");
        if args.image.is_none() && !args.list_blocks && !args.show_block_bitmaps {
            exit(0);
        }
    }

    if args.list_blocks {
        eprintln!("Available blocks:");
        write_encoded_output(&font::print_blocks_config(), &args.encoding);
        exit(0);
    }

    if args.list_ranges {
        write_encoded_output(&font::list_ranges(), &args.encoding);
        exit(0);
    }

    if args.print_ranges {
        write_encoded_output(&font::print_ranges(), &args.encoding);
        exit(0);
    }

    if args.show_block_bitmaps {
        let glyphs = font::glyph_store(
            &args.blocks,
            &args.exclude_range,
            &args.exclude,
            &args.include_range,
                        args.include.as_ref(),
                        &args.font,
                        args.y_offset,
                        args.font_size
                    );
        write_encoded_output(&font::print_block_bitmaps(&glyphs), &args.encoding);
        exit(0);
    }

    if let Some(font) = &args.font.first() {
        std::env::set_var("IMG2IRC_FONT", font);
    }

    match load_image_from_url_or_path(&args.clone().image.unwrap().as_str()).await {
        Ok(image) => {
            let glyphs = font::glyph_store(
                &args.blocks, 
                &args.exclude_range, 
                &args.exclude, 
                &args.include_range,
                            args.include.as_ref(),
                            &args.font,
                            args.y_offset,
                            args.font_size
                        );            eprintln!("Loaded {} glyphs", glyphs.glyphs.len());
            eprintln!("Cell size: {}x{}", glyphs.metrics.0, glyphs.metrics.1);

            if let Some(mode) = &args.optimize {
                let (base_sx, base_sy) = args.scale.unwrap_or((1.0, 1.0));
                let range_pct = args.scale_range.abs();
                let start_mult = 1.0 - (range_pct / 100.0);
                let end_mult = 1.0 + (range_pct / 100.0);
                
                let step = args.optimize_step / 100.0;
                if step <= 0.0001 {
                    eprintln!("Error: Step size must be positive.");
                    exit(1);
                }

                eprintln!("Optimizing {:?} on {:?}-axis with scale range +/-{:.1}% (step {:.1}%)", 
                    mode, args.optimize_axis, range_pct, args.optimize_step);

                let mut best_metric = f64::MAX;
                let mut best_content = String::new();
                let mut best_scale = if args.optimize_axis == args::OptimizeAxis::X { base_sx } else { base_sy };

                let mut current_mult = start_mult;
                let total_steps = ((end_mult - start_mult) / step + 1e-5).floor() as usize + 1;
                let mut step_count = 0;

                while current_mult <= end_mult + 0.00001 {
                    step_count += 1;
                    let mut loop_args = args.clone();
                    let (new_sx, new_sy) = match args.optimize_axis {
                        args::OptimizeAxis::X => (base_sx * current_mult, base_sy),
                        args::OptimizeAxis::Y => (base_sx, base_sy * current_mult),
                    };
                    loop_args.scale = Some((new_sx, new_sy));

                    eprint!("\r[{}/{}] Testing scale {:.3}:{}... ", step_count, total_steps, new_sx, new_sy);
                    let _ = std::io::stderr().flush();

                    let result = render_output(&loop_args, &glyphs, image.clone());
                    
                    let metric = match mode {
                        args::OptimizationMode::TotalBytes => result.content.len() as f64,
                        args::OptimizationMode::LongestLine => result.content.lines().map(|l| l.len()).max().unwrap_or(0) as f64,
                        args::OptimizationMode::BitmapError => {
                            if result.total_pixels > 0 {
                                result.error_count as f64 / result.total_pixels as f64
                            } else {
                                1.0 // Maximum error if no pixels
                            }
                        }
                    };

                    if metric < best_metric {
                        eprintln!("New best! ({:.4} -> {:.4})", if best_metric == f64::MAX { 0.0 } else { best_metric }, metric);
                        best_metric = metric;
                        best_content = result.content;
                        best_scale = if args.optimize_axis == args::OptimizeAxis::X { new_sx } else { new_sy };
                    }

                    current_mult += step;
                }
                
                let (final_sx, final_sy) = match args.optimize_axis {
                    args::OptimizeAxis::X => (best_scale, base_sy),
                    args::OptimizeAxis::Y => (base_sx, best_scale),
                };
                
                eprintln!("\nOptimization complete. Best scale: {:.3}:{:.3} with metric: {:.4}", final_sx, final_sy, best_metric);
                write_encoded_output(&best_content, &args.encoding);

            } else {
                let result = render_output(&args, &glyphs, image);
                write_encoded_output(&result.content, &args.encoding);
            }
        }

        Err(e) => {
            eprintln!("Error: {}", e);
            exit(1);
        }
    }
}

fn render_output(args: &args::Args, glyphs: &font::GlyphStore, image: PhotonImage) -> draw::RenderResult {
    let image = effects::apply_effects(args, glyphs, image);
    let canvas = draw::AnsiImage::new(image.clone(), glyphs.clone(), args.grayscale_tolerance);

    if args.braille {
        let image_luma = effects::apply_luma_effects(args, glyphs, image);
        let canvas_luma = draw::AnsiImage::new(image_luma, glyphs.clone(), args.grayscale_tolerance);
        match args.render {
            args::Render::Irc => draw::render_braille(&canvas_luma, &canvas, args, args::Render::Irc),
            args::Render::Ansi => draw::render_braille(&canvas_luma, &canvas, args, args::Render::Ansi),
            args::Render::Ansi24 => draw::render_braille(&canvas_luma, &canvas, args, args::Render::Ansi24),
        }
    } else {
        match args.render {
            args::Render::Irc => draw::render_blocks(&canvas, args, args::Render::Irc),
            args::Render::Ansi => draw::render_blocks(&canvas, args, args::Render::Ansi),
            args::Render::Ansi24 => draw::render_blocks(&canvas, args, args::Render::Ansi24),
        }
    }
}

fn write_encoded_output(content: &str, encoding: &Encoding) {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    let _ = write_encoded_to(content, encoding, &mut handle);
    let _ = handle.flush();
}

fn write_encoded_to<W: Write>(content: &str, encoding: &Encoding, handle: &mut W) -> std::io::Result<()> {
    match encoding {
        Encoding::Utf8 => {
            writeln!(handle, "{}", content)?;
        }
        Encoding::Utf16 => {
            // Default to native endian with BOM
            let bom = 0xFEFFu16;
            #[cfg(target_endian = "little")]
            {
                handle.write_all(&bom.to_le_bytes())?;
                write_encoded_to(content, &Encoding::Utf16le, handle)?;
            }
            #[cfg(target_endian = "big")]
            {
                handle.write_all(&bom.to_be_bytes())?;
                write_encoded_to(content, &Encoding::Utf16be, handle)?;
            }
        }
        Encoding::Utf16be => {
            for c in content.encode_utf16() {
                handle.write_all(&c.to_be_bytes())?;
            }
            handle.write_all(&('\n' as u16).to_be_bytes())?;
        }
        Encoding::Utf16le => {
            for c in content.encode_utf16() {
                handle.write_all(&c.to_le_bytes())?;
            }
            handle.write_all(&('\n' as u16).to_le_bytes())?;
        }
        Encoding::Cesu8 => {
            for u in content.encode_utf16() {
                if u <= 0x7F {
                    handle.write_all(&[u as u8])?;
                } else if u <= 0x7FF {
                    handle.write_all(&[
                        0xC0 | ((u >> 6) as u8),
                        0x80 | ((u & 0x3F) as u8),
                    ])?;
                } else {
                    handle.write_all(&[
                        0xE0 | ((u >> 12) as u8),
                        0x80 | (((u >> 6) & 0x3F) as u8),
                        0x80 | ((u & 0x3F) as u8),
                    ])?;
                }
            }
            handle.write_all(b"\n")?;
        }
    }
    Ok(())
}

async fn load_image_from_url_or_path(image: &str) -> Result<PhotonImage, Box<dyn Error>> {
    match Url::parse(image) {
        Ok(url) => {
            let response = reqwest::get(url).await?;
            let bytes = response.bytes().await?;
            let image_data = Cursor::new(bytes);
            match photon_rs::native::open_image_from_bytes(image_data.into_inner().as_ref()) {
                Ok(image) => Ok(image),
                Err(e) => Err(Box::new(e)),
            }
        }
        Err(_) => {
            match photon_rs::native::open_image(image) {
                Ok(image) => Ok(image),
                Err(e) => Err(Box::new(e)),
            }
        }
    }
}
