use img2irc_rs::{args, draw, font};
use clap::CommandFactory;
use simplelog::{LevelFilter, WriteLogger};
use std::fs::File;
use std::process::exit;

fn glyph_bitmap_font_size(requested: f32) -> f32 {
    if requested > 0.0 {
        requested
    } else {
        16.0
    }
}

#[tokio::main]
async fn main() {
    let no_arguments = std::env::args_os().len() == 1;
    let mut args = args::parse_args();

    if no_arguments {
        args::Args::command().print_help().unwrap();
        println!();
        exit(0);
    }

    let log_config = simplelog::ConfigBuilder::new()
        .set_time_format_rfc3339()
        .build();
    let _ = WriteLogger::init(
        LevelFilter::Info,
        log_config,
        File::create("img2irc.log").unwrap(),
    );

    // Load config
    if let Some(config) = img2irc_rs::config::Config::load() {
        if args.font.is_empty() {
            if let Some(f) = config.font {
                args.font = vec![f];
            }
        }
        if args.blocks.is_empty() {
            if let Some(b) = config.blocks {
                args.blocks = b;
            }
        }
        if args.ocr_figlet_fonts.is_empty() {
            if let Some(fonts_by_height) = config.ocr_figlet_fonts {
                args.ocr_figlet_fonts = fonts_by_height
                    .into_iter()
                    .filter_map(|(height, fonts)| match height.parse::<u32>() {
                        Ok(height) if height > 0 => {
                            Some(args::OcrFigletFontList { height, fonts })
                        }
                        _ => {
                            log::warn!(
                                "Ignoring config ocr_figlet_fonts key {height:?}; expected a positive line count"
                            );
                            None
                        }
                    })
                    .collect();
            }
        }
    }

    if args.tui {
        let render_args = img2irc_rs::args_to_render_args(&args);
        if let Err(e) = img2irc_rs::tui::run(render_args).await {
            log::error!("Error running TUI: {}", e);
            std::process::exit(1);
        }
        std::process::exit(0);
    }

    if args.clear_cache {
        font::clear_cache();
        log::info!("Cache cleared.");
        if args.image.is_none() && !args.print_glyph_chars && !args.print_glyph_bitmaps {
            exit(0);
        }
    }

    if args.print_glyph_chars {
        img2irc_rs::write_encoded_output(&font::print_glyph_chars(), &args.encoding);
        exit(0);
    }

    if args.print_glyph_bitmaps {
        let mut render_args = img2irc_rs::args_to_render_args(&args);
        render_args.font_size = glyph_bitmap_font_size(render_args.font_size);
        let glyphs = font::glyph_store(
            &render_args.blocks,
            &render_args.exclude_range,
            &render_args.exclude,
            &render_args.include_range,
            render_args.include.as_ref(),
            &render_args.font,
            render_args.font_size,
            false,
            None,
        );
        img2irc_rs::write_encoded_output(&font::print_glyph_bitmaps(&glyphs), &args.encoding);
        exit(0);
    }

    if let Some(font) = &args.font.first() {
        std::env::set_var("IMG2IRC_FONT", font);
    }

    // Move args preparation before image loading to support SVG dimension calculation
    let mut render_args = img2irc_rs::args_to_render_args(&args);

    let img_path = match &args.image {
        Some(p) => p,
        None => {
            log::info!("No image specified. Use --help for usage or launch `tui` bin.");
            exit(0);
        }
    };

    // 1. Initial glyph store for loading image (e.g. SVG dimension calculation)
    let initial_glyph_store = font::glyph_store(
        &render_args.blocks,
        &render_args.exclude_range,
        &render_args.exclude,
        &render_args.include_range,
        render_args.include.as_ref(),
        &render_args.font,
        if (render_args.font_size - 0.0).abs() < f32::EPSILON {
            24.0
        } else {
            render_args.font_size
        },
        false,
        None,
    );

    match img2irc_rs::load_image_from_url_or_path(img_path, &render_args, &initial_glyph_store)
        .await
    {
        Ok(loaded_image) => {
            let ocr_detections = if render_args.ocr {
                let ocr_image = match render_args.ocr_width {
                    Some(w) if w > 0 && w != loaded_image.get_width() => {
                        let h = ((w as f32 * loaded_image.get_height() as f32) / loaded_image.get_width() as f32).round().max(1.0) as u32;
                        photon_rs::transform::resize(&loaded_image, w, h, photon_rs::transform::SamplingFilter::Lanczos3)
                    }
                    _ => loaded_image.clone(),
                };
                let orig_w = loaded_image.get_width() as f32;
                let orig_h = loaded_image.get_height() as f32;
                let ocr_w = ocr_image.get_width() as f32;
                let ocr_h = ocr_image.get_height() as f32;
                match img2irc_rs::ocr::detect_text(&ocr_image, &render_args) {
                    Ok(raw_detections) => {
                        let scaled_detections: Vec<img2irc_rs::ocr::OcrDetection> = if (ocr_w - orig_w).abs() > 0.5 || (ocr_h - orig_h).abs() > 0.5 {
                            raw_detections
                                .into_iter()
                                .map(|mut d| {
                                    for pt in &mut d.polygon {
                                        pt[0] = pt[0] * orig_w / ocr_w;
                                        pt[1] = pt[1] * orig_h / ocr_h;
                                    }
                                    d
                                })
                                .collect()
                        } else {
                            raw_detections
                        };
                        let detections = img2irc_rs::ocr::merge_same_line_detections(
                            scaled_detections,
                            loaded_image.get_width(),
                            loaded_image.get_height(),
                        );
                        img2irc_rs::ocr::log_detections(&detections, &render_args);
                        if !img2irc_rs::ocr::has_renderable_detections(&detections, &render_args, loaded_image.get_width(), loaded_image.get_height()) {
                            log::info!("No renderable text detected via OCR. Falling back to standard render.");
                            render_args.ocr = false;
                            None
                        } else {
                            Some(detections)
                        }
                    }
                    Err(e) => {
                        log::error!("OCR text detection failed: {}", e);
                        std::process::exit(1);
                    }
                }
            } else {
                None
            };

            let base_image = if let Some(detections) = &ocr_detections {
                img2irc_rs::ocr::remove_detected_text(&loaded_image, &render_args, detections)
            } else {
                loaded_image.clone()
            };

            // 2. Resolve auto font size based on loaded image if not explicitly set (> 0)
            if render_args.font_size == 0.0 {
                img2irc_rs::resolve_auto_font_size(&mut render_args, &loaded_image, &initial_glyph_store);
            }

            // 3. Final glyph store with resolved font size
            let glyph_store = font::glyph_store(
                &render_args.blocks,
                &render_args.exclude_range,
                &render_args.exclude,
                &render_args.include_range,
                render_args.include.as_ref(),
                &render_args.font,
                render_args.font_size,
                false,
                None,
            );

            if let Some(detections) = &ocr_detections {
                let overlays = img2irc_rs::ocr::detections_to_overlays(
                    &loaded_image,
                    &base_image,
                    &render_args,
                    &glyph_store,
                    detections,
                );
                log::info!("Created {} OCR text overlays", overlays.len());
                render_args.overlays.extend(overlays);
            }

            let effect_image = img2irc_rs::effects::apply_effects(&render_args, &glyph_store, base_image);

            log::info!("Loaded {} glyphs", glyph_store.glyphs.len());
            log::info!(
                "Cell size: {}x{}",
                glyph_store.metrics.0,
                glyph_store.metrics.1
            );

            // Pre-calculate FastGlyphs once
            let glyphs = draw::prepare_glyphs(&glyph_store, &render_args);
            log::info!("Prepared {} optimized glyphs", glyphs.len());

            let mut output_args = render_args.clone();
            output_args.pipeline.clear();
            output_args.rotate = 0.0;
            output_args.fliph = false;
            output_args.flipv = false;
            output_args.scale = None;
            output_args.brightness = 0.0;
            output_args.contrast = 0.0;
            output_args.gamma = 0.0;
            output_args.saturation = 0.0;
            output_args.hue = 0.0;
            output_args.invert = false;
            output_args.dither = 0;
            output_args.grayscale = false;
            output_args.nograyscale = false;
            output_args.pixelize = 0;
            output_args.box_blur = false;
            output_args.gaussian_blur = 0;

            let result = img2irc_rs::render_output(
                output_args,
                &glyph_store,
                render_image,
                &glyphs,
                None,
            );

            if args.profile {
                eprintln!("img2irc profile: {}", result.profile.summary());
            }

            img2irc_rs::write_encoded_output(&result.content, &args.encoding);
            if let Some(path) = &args.save {
                let metadata = format!("{:?}", args);
                if let Err(e) = draw::save_as_png(&result, &glyph_store, path, &metadata) {
                    log::info!("Failed to save PNG: {}", e);
                } else {
                    log::info!("Saved image to {}", path);
                }
            }
        }

        Err(e) => {
            log::info!("Error: {}", e);
            exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::glyph_bitmap_font_size;

    #[test]
    fn glyph_bitmaps_default_to_sixteen_pixel_font() {
        assert_eq!(glyph_bitmap_font_size(0.0), 16.0);
        assert_eq!(glyph_bitmap_font_size(24.0), 24.0);
    }
}
