mod args;
mod draw;
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

            match (args.irc, args.ansi, args.ansi24, args.qb, args.braille) {
                (true, _, _, true, false) => println!("{}", draw::irc_draw_qb(&canvas_chroma, &args)),
                (true, _, _, false, false) => println!("{}", draw::irc_draw(&canvas_chroma, &args)),
                (true, _, _, _, true) => println!("{}", draw::irc_draw_braille(&canvas_luma, &canvas_chroma, &args)),
                (_, true, _, true, false) => println!("{}", draw::ansi_draw_8bit_qb(&canvas_chroma, &args)),
                (_, true, _, false, false) => println!("{}", draw::ansi_draw_8bit(&canvas_chroma, &args)),
                (_, true, _, _, true) => println!("{}", draw::ansi_draw_braille_8bit(&canvas_luma, &canvas_chroma, &args)),
                (_, _, true, true, false) => println!("{}", draw::ansi_draw_24bit_qb(&canvas_chroma)),
                (_, _, true, false, false) => println!("{}", draw::ansi_draw_24bit(&canvas_chroma)),
                (_, _, true, _, true) => println!("{}", draw::ansi_draw_braille_24bit(&canvas_luma, &canvas_chroma)),
                (_, _, _, true, false) => println!("{}", draw::irc_draw_qb(&canvas_chroma, &args)),
                (_, _, _, _, true) => println!("{}", draw::irc_draw_braille(&canvas_luma, &canvas_chroma, &args)),
                _ => println!("{}", draw::irc_draw(&canvas_chroma, &args)),
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
