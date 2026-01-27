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

use rayon::prelude::*;
use std::sync::atomic::{AtomicUsize, Ordering};

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

            if let Some(mode) = &args.opt {
                let (base_sx, base_sy) = args.scale.unwrap_or((1.0, 1.0));
                let base_fuzz = args.fuzz;
                let range_pct = args.range.abs();
                let start_mult = 1.0 - (range_pct / 100.0);
                let end_mult = 1.0 + (range_pct / 100.0);
                
                let step = args.step / 100.0;
                if step <= 0.0001 {
                    eprintln!("Error: Step size must be positive.");
                    exit(1);
                }

                eprintln!("Optimizing {:?} on {:?}-axis with scale range +/-{:.1}% (step {:.1}%)", 
                    mode, args.axis, range_pct, args.step);

                // Generate steps
                let mut scale_steps = Vec::new();
                let mut current_mult = start_mult;
                while current_mult <= end_mult + 0.00001 {
                    scale_steps.push(current_mult);
                    current_mult += step;
                }
                
                let total_steps = scale_steps.len();
                let progress = AtomicUsize::new(0);

                let best_result = scale_steps.par_iter().map(|&mult| {
                    let mut loop_args = args.clone();
                    
                    // Apply multiplier based on axis
                    let (curr_sx, curr_sy, curr_fuzz) = match args.axis {
                        args::OptimizeAxis::X => (base_sx * mult, base_sy, base_fuzz),
                        args::OptimizeAxis::Y => (base_sx, base_sy * mult, base_fuzz),
                        args::OptimizeAxis::Fuzz => (base_sx, base_sy, base_fuzz * mult),
                    };

                    loop_args.scale = Some((curr_sx, curr_sy));
                    loop_args.fuzz = curr_fuzz;

                    let result = render_output(&loop_args, &glyphs, image.clone());
                    
                    let metric = match mode {
                        args::OptimizationMode::TotalBytes => result.content.len() as f64,
                        args::OptimizationMode::LongestLine => result.content.lines().map(|l| l.len()).max().unwrap_or(0) as f64,
                        args::OptimizationMode::BitmapError => {
                            if result.total_pixels > 0 {
                                result.error_count as f64 / result.total_pixels as f64
                            } else {
                                1.0 
                            }
                        }
                    };

                    let p = progress.fetch_add(1, Ordering::Relaxed) + 1;
                    if total_steps < 20 || p % (total_steps / 10).max(1) == 0 {
                         eprint!("\r[{}/{}] Processed...", p, total_steps);
                         let _ = std::io::stderr().flush();
                    }

                    (metric, result, curr_sx, curr_sy, curr_fuzz)
                }).min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

                if let Some((best_metric, best_res, final_sx, final_sy, final_fuzz)) = best_result {
                    eprintln!("\nOptimization complete. Best scale: {:.3}:{:.3}, Best fuzz: {:.3}, Metric: {:.4}", final_sx, final_sy, final_fuzz, best_metric);
                    if best_res.dynamic_scaling_count > 0 {
                        eprintln!("Dynamic scaling applied to {} blocks.", best_res.dynamic_scaling_count);
                    }
                    write_encoded_output(&best_res.content, &args.encoding);
                } else {
                    eprintln!("Optimization failed to produce results.");
                }

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
