use crate::args;
use photon_rs::{channels, channels::alter_channels, conv, effects, filters, monochrome, noise};
use photon_rs::transform::{resize, SamplingFilter};
use photon_rs::PhotonImage;

pub fn apply_effects(
    args: &args::Args,
    mut photon_image: PhotonImage,
) -> PhotonImage {

    // Resize to width
    let height =
        (args.width as f32 / photon_image.get_width() as f32 * photon_image.get_height() as f32) as u32;

    photon_image = resize(&mut photon_image, args.width, height, SamplingFilter::Lanczos3);

    // Adjust brightness
    if args.brightness != 0 {
        alter_channels(&mut photon_image, args.brightness, args.brightness, args.brightness);
    }

    // Adjust hue
    if args.hue != 0 {
        alter_channels(&mut photon_image, args.hue, args.hue, args.hue);
    }

    // Adjust contrast
    if args.contrast != 0 {
        alter_channels(&mut photon_image, args.contrast, args.contrast, args.contrast);
    }

    // Adjust saturation
    if args.saturation != 0 {
        alter_channels(&mut photon_image, args.saturation, args.saturation, args.saturation);
    }

    // Adjust opacity
    if args.opacity != 0 {
        alter_channels(&mut photon_image, args.opacity, args.opacity, args.opacity);
    }

    // Adjust gamma
    if args.gamma != 0 {
        alter_channels(&mut photon_image, args.gamma, args.gamma, args.gamma);
    }

    // Adjust dither
    if args.dither > 0 {
        effects::dither(&mut photon_image, args.dither);
    }

    // Adjust gaussian_blur
    if args.gaussian_blur > 0 {
        
        conv::gaussian_blur(&mut photon_image, args.gaussian_blur);
    }

    // Adjust pixelize
    if args.pixelize > 0 {
        effects::pixelize(&mut photon_image, args.pixelize);
    }

    // Adjust halftone
    if args.halftone {
        effects::halftone(&mut photon_image);
    }

    // Adjust invert
    if args.invert {
        channels::invert(&mut photon_image);
    }

    // Adjust sepia
    if args.sepia {
        monochrome::sepia(&mut photon_image);
    }

    // Adjust solarize
    if args.solarize {
        effects::solarize(&mut photon_image);
    }

    // Adjust normalize
    if args.normalize {
        effects::normalize(&mut photon_image);
    }

    // Adjust noise
    if args.noise {
        noise::add_noise_rand(photon_image.clone());
    }

    // Adjust sharpen
    if args.sharpen {
        conv::sharpen(&mut photon_image);
    }

    // Adjust edge_detection
    if args.edge_detection {
        conv::edge_detection(&mut photon_image);
    }

    // Adjust emboss
    if args.emboss {
        conv::emboss(&mut photon_image);
    }

    // Adjust frosted_glass
    if args.frosted_glass {
        effects::frosted_glass(&mut photon_image);
    }

    // Adjust box_blur
    if args.box_blur {
        conv::box_blur(&mut photon_image);
    }

    // Adjust grayscale
    if args.grayscale {
        monochrome::grayscale(&mut photon_image);
    }

    // Adjust identity
    if args.identity {
        conv::identity(&mut photon_image);
    }

    // Adjust laplace
    if args.laplace {
        conv::laplace(&mut photon_image);
    }

    // Adjust cali
    if args.cali {
        filters::cali(&mut photon_image);
    }

    // Adjust dramatic
    if args.dramatic {
        filters::dramatic(&mut photon_image);
    }

    // Adjust firenze
    if args.firenze {
        filters::firenze(&mut photon_image);
    }

    // Adjust golden
    if args.golden {
        filters::golden(&mut photon_image);
    }

    // Adjust lix
    if args.lix {
        filters::lix(&mut photon_image);
    }

    // Adjust lofi
    if args.lofi {
        filters::lofi(&mut photon_image);
    }

    // Adjust neue
    if args.neue {
        filters::neue(&mut photon_image);
    }

    // Adjust obsidian
    if args.obsidian {
        filters::obsidian(&mut photon_image);
    }

    // Adjust pastel_pink
    if args.pastel_pink {
        filters::pastel_pink(&mut photon_image);
    }

    // Adjust ryo
    if args.ryo {
        filters::ryo(&mut photon_image);
    }

    // Adjust oil
    match &args.oil {
        Some(oil) => {
            // split oil at comma
            let vals: Vec<&str> = oil.split(",").collect();

            // check if args.oil has 2 values
            if vals.len() == 2 {
                // convert oil values to i32 and f64
                let radius: i32 = vals.get(0).unwrap().parse::<i32>().unwrap();
                let intensity: f64 = vals.get(1).unwrap().parse::<f64>().unwrap();

                effects::oil(&mut photon_image, radius, intensity);
            }
        }
        None => {}
    }

    photon_image
}
