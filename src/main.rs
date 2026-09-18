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

    if args.font_licenses {
        print!("{}", font::bundled_font_licenses());
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
    let blocks_from_cli = !args.blocks.is_empty();

    if let Some(config_dir) = &args.config_dir {
        std::env::set_var("IMG2IRC_CONFIG_DIR", config_dir);
    }
    if let Some(figlet_dir) = &args.figlet_dir {
        std::env::set_var("IMG2IRC_FIGLET_FONT_DIR", figlet_dir);
    }

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
        if !blocks_from_cli {
            if let Some(persisted) = img2irc_rs::font::load_enabled_glyph_sets() {
                let (catalog, _) = img2irc_rs::font::load_blocks_config_unfiltered();
                let known: std::collections::HashSet<&str> =
                    catalog.keys().map(String::as_str).collect();
                let restored: Vec<String> = persisted
                    .into_iter()
                    .filter(|name| known.contains(name.as_str()))
                    .collect();
                if !restored.is_empty() {
                    log::info!(
                        "Restoring {} enabled glyph set(s) from glyphs.toml",
                        restored.len()
                    );
                    args.blocks = restored;
                }
            }
        }
        let render_args = img2irc_rs::args_to_tui_render_args(&args);
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
                match img2irc_rs::ocr::detect_text(&loaded_image, &render_args) {
                    Ok(detections) => {
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
            if render_args.ocr && render_args.ocr_auto_width {
                render_args.width = None;
                render_args.height = None;
            }
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
                img2irc_rs::ocr::choose_text_render_geometry(
                    &mut render_args, &loaded_image, &glyph_store, detections,
                );
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

            let render_image = img2irc_rs::effects::apply_effects(&render_args, &glyph_store, base_image);

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
