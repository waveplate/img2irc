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
        Ok(mut image) => {
            image = effects::apply_effects(
                &args,
                image,
            );

            let canvas = draw::AnsiImage::new(image);

            match (args.irc, args.ansi, args.ansi24, args.qb) {
                (true, _, _, true) => println!("{}", draw::irc_draw_qb(canvas, &args).as_str()),
                (true, _, _, false) => println!("{}", draw::irc_draw(canvas, &args).as_str()),
                (_, true, _, true) => println!("{}", draw::ansi_draw_8bit_qb(canvas, &args).as_str()),
                (_, true, _, false) => println!("{}", draw::ansi_draw_8bit(canvas, &args).as_str()),
                (_, _, true, true) => println!("{}", draw::ansi_draw_24bit_qb(canvas).as_str()),
                (_, _, true, false) => println!("{}", draw::ansi_draw_24bit(canvas).as_str()),
                (_, _, _, true) => println!("{}", draw::irc_draw_qb(canvas, &args).as_str()),
                _ => println!("{}", draw::irc_draw(canvas, &args).as_str()),
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

