mod args;
mod draw;
mod palette;
mod effects;
mod font;

use reqwest;
use url::Url;
use photon_rs::PhotonImage;
use std::{error::Error, io::Cursor, process::exit};

#[tokio::main]
async fn main() {
    let args = args::parse_args();

    let glyphs = font::glyph_store(&args.blocks, &args.exclude_range, &args.exclude);
    eprintln!("Loaded {} glyphs", glyphs.glyphs.len());
    eprintln!("Cell size: {}x{}", glyphs.metrics.0, glyphs.metrics.1);

    if args.list_blocks {
        eprintln!("Available blocks:");
        font::print_blocks_config();
        exit(0);
    }



    match load_image_from_url_or_path(&args.clone().image.unwrap().as_str()).await {
        Ok(image) => {
            let image = effects::apply_effects(&args, &glyphs, image.clone());
            let canvas = draw::AnsiImage::new(image.clone(), glyphs.clone());

            eprintln!("Loaded canvases");

            if args.braille {
                let image_luma = effects::apply_luma_effects(&args, &glyphs, image.clone());
                let canvas_luma = draw::AnsiImage::new(image_luma.clone(), glyphs.clone());
                match args.render {
                    args::Render::Irc => println!("{}", draw::render_braille(&canvas_luma, &canvas, &args, args::Render::Irc)),
                    args::Render::Ansi => println!("{}", draw::render_braille(&canvas_luma, &canvas, &args, args::Render::Ansi)),
                    args::Render::Ansi24 => println!("{}", draw::render_braille(&canvas_luma, &canvas, &args, args::Render::Ansi24)),
                }
            } else {
                match args.render {
                    args::Render::Irc => println!("{}", draw::render_blocks(&canvas, &args, args::Render::Irc)),
                    args::Render::Ansi => println!("{}", draw::render_blocks(&canvas, &args, args::Render::Ansi)),
                    args::Render::Ansi24 => println!("{}", draw::render_blocks(&canvas, &args, args::Render::Ansi24)),
                }
            }
        }

        Err(e) => {
            eprintln!("Error: {}", e);
            exit(1);
        }
    }
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
