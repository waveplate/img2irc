use img2irc_rs::{args, draw, font};
use std::process::exit;

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
            render_args.font_size
        );
        img2irc_rs::write_encoded_output(&font::print_block_bitmaps(&glyphs), &args.encoding);
        exit(0);
    }

    if let Some(font) = &args.font.first() {
        std::env::set_var("IMG2IRC_FONT", font);
    }

    // Move args preparation before image loading to support SVG dimension calculation
    let render_args = img2irc_rs::args_to_render_args(&args);
    
    let glyph_store = font::glyph_store(
        &render_args.blocks, 
        &render_args.exclude_range, 
        &render_args.exclude, 
        &render_args.include_range,
        render_args.include.as_ref(),
        &render_args.font,
        render_args.font_size
    );            
    eprintln!("Loaded {} glyphs", glyph_store.glyphs.len());
    eprintln!("Cell size: {}x{}", glyph_store.metrics.0, glyph_store.metrics.1);

    let img_path = match &args.image {
        Some(p) => p,
        None => { exit(0); }
    };

    match img2irc_rs::load_image_from_url_or_path(img_path, &render_args, &glyph_store).await {
        Ok(image) => {
            // Pre-calculate FastGlyphs once
            let glyphs = draw::prepare_glyphs(&glyph_store, &render_args);
            eprintln!("Prepared {} optimized glyphs", glyphs.len());

            let result = img2irc_rs::render_output(&render_args, &glyph_store, image, &glyphs);
            if args.score {
                let smoothness = draw::calculate_smoothness(&result, &glyphs, &render_args);
                eprintln!("Smoothness Score: {:.4} (Lower is smoother)", smoothness);
            }
            img2irc_rs::write_encoded_output(&result.content, &args.encoding);
            if let Some(path) = &args.save {
                let smoothness = draw::calculate_smoothness(&result, &glyphs, &render_args);
                let final_path = path.replace("%s", &format!("{:.4}", smoothness));

                let metadata = format!("{:?}", args);
                if let Err(e) = draw::save_as_png(&result, &glyph_store, &final_path, &metadata) {
                    eprintln!("Failed to save PNG: {}", e);
                } else {
                    eprintln!("Saved image to {}", final_path);
                }
            }
        }

        Err(e) => {
            eprintln!("Error: {}", e);
            exit(1);
        }
    }
}
