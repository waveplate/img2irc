mod args;
mod draw;
mod chars;
mod palette;
mod effects;

use reqwest;
use url::Url;
use photon_rs::PhotonImage;
use std::{error::Error, io::Cursor, process::exit};

#[tokio::main]
async fn main() {
    let args = args::parse_args();

    match load_image_from_url_or_path(args.image.as_str()).await {
        Ok(image) => {
            let image_luma = effects::apply_luma_effects(&args, image.clone());
            let image_chroma = effects::apply_effects(&args, image.clone());

            let canvas_luma = draw::AnsiImage::new(image_luma.clone());
            let canvas_chroma = draw::AnsiImage::new(image_chroma.clone());

            eprintln!("Render mode: {:?}", args.render);

            if args.braille {
                // Braille rendering
                match args.render {
                    args::Render::Irc => println!("{}", draw::render_braille(&canvas_luma, &canvas_chroma, &args, args::Render::Irc)),
                    args::Render::Ansi => println!("{}", draw::render_braille(&canvas_luma, &canvas_chroma, &args, args::Render::Ansi)),
                    args::Render::Ansi24 => println!("{}", draw::render_braille(&canvas_luma, &canvas_chroma, &args, args::Render::Ansi24)),
                }
            } else {
                // Block rendering
                match args.render {
                    args::Render::Irc => println!("{}", draw::render_blocks(&canvas_chroma, &args, args::Render::Irc)),
                    args::Render::Ansi => println!("{}", draw::render_blocks(&canvas_chroma, &args, args::Render::Ansi)),
                    args::Render::Ansi24 => println!("{}", draw::render_blocks(&canvas_chroma, &args, args::Render::Ansi24)),
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
