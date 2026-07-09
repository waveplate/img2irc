use img2irc_rs::{args, draw, font};
use simplelog::{Config, LevelFilter, WriteLogger};
use std::fs::File;
use std::process::exit;

#[tokio::main]
async fn main() {
    let mut args = args::parse_args();

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
        if args.image.is_none() && !args.list_blocks && !args.show_block_bitmaps {
            exit(0);
        }
    }

    if args.list_blocks {
        log::info!("Available blocks:");
        img2irc_rs::write_encoded_output(&font::print_blocks_config(), &args.encoding);
        exit(0);
    }

    if args.list_ranges {
        img2irc_rs::write_encoded_output(&font::list_ranges(), &args.encoding);
        exit(0);
    }

    if args.print_ranges {
        img2irc_rs::write_encoded_output(&font::print_ranges(), &args.encoding);
        exit(0);
    }

    if args.show_block_bitmaps {
        let render_args = img2irc_rs::args_to_render_args(&args);
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
        img2irc_rs::write_encoded_output(&font::print_block_bitmaps(&glyphs), &args.encoding);
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
        Ok(image) => {
            let ocr_detections = if render_args.ocr {
                match img2irc_rs::ocr::detect_text(&image, &render_args) {
                    Ok(detections) => {
                        img2irc_rs::ocr::log_detections(&detections, &render_args);
                        if !img2irc_rs::ocr::has_renderable_detections(&detections, &render_args, image.get_width(), image.get_height()) {
                            log::info!("No renderable text detected via OCR. Falling back to standard render.");
                            render_args.ocr = false;
                            None
                        } else {
                            img2irc_rs::ocr::choose_text_font_size(
                                &mut render_args,
                                &detections,
                                image.get_height(),
                            );
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

            // 2. Resolve auto font size based on loaded image unless OCR chose it.
            if !render_args.ocr {
                img2irc_rs::resolve_auto_font_size(&mut render_args, &image, &initial_glyph_store);
            }

            // 3. Final glyph store with resolved font size
            let mut glyph_store = font::glyph_store(
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
                img2irc_rs::ocr::choose_text_render_geometry(
                    &mut render_args,
                    &image,
                    &glyph_store,
                    detections,
                );
                glyph_store = font::glyph_store(
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
            }

            let render_image = if let Some(detections) = &ocr_detections {
                img2irc_rs::ocr::remove_detected_text(&image, &render_args, detections)
            } else {
                image.clone()
            };

            if let Some(detections) = &ocr_detections {
                let overlays = img2irc_rs::ocr::detections_to_overlays(
                    &image,
                    &render_image,
                    &render_args,
                    &glyph_store,
                    detections,
                );
                log::info!("Created {} OCR text overlays", overlays.len());
                render_args.overlays.extend(overlays);
            }

            log::info!("Loaded {} glyphs", glyph_store.glyphs.len());
            log::info!(
                "Cell size: {}x{}",
                glyph_store.metrics.0,
                glyph_store.metrics.1
            );

            // Pre-calculate FastGlyphs once
            let glyphs = draw::prepare_glyphs(&glyph_store, &render_args);
            log::info!("Prepared {} optimized glyphs", glyphs.len());

            let result = img2irc_rs::render_output(
                render_args.clone(),
                &glyph_store,
                render_image,
                &glyphs,
                None,
            );

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
