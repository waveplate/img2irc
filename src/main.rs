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
            match &args.render {
                None => println!("{}", draw::irc_draw(canvas).as_str()),
                Some(ref render) => match render.as_str() {
                    "irc" => println!("{}", draw::irc_draw(canvas).as_str()),
                    "ansi" => println!("{}", draw::ansi_draw_8bit(canvas).as_str()),
                    "ansi24" => println!("{}", draw::ansi_draw_24bit(canvas).as_str()),
                    _ => {
                        eprintln!("Error: invalid render type");
                        exit(1);
                    }
                },
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

