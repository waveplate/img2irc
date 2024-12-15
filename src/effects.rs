use crate::args;
use photon_rs::{colour_spaces, channels, conv, effects, filters, monochrome, noise};
use photon_rs::transform::{SamplingFilter, resize, crop, rotate, flipv, fliph};
use photon_rs::PhotonImage;

fn calculate_dimensions(args: &args::Args, image: &PhotonImage) -> (u32, u32) {
    let original_width = image.get_width() as f32;
    let original_height = image.get_height() as f32;
    let original_aspect = original_width / original_height;

    let base_width;
    let base_height;

    let aspect_ratio = if let Some(aspect) = args.aspect {
        aspect.0 / aspect.1
    } else {
        original_aspect
    };

    if args.width.is_none() && args.height.is_none() {
        base_width = original_width;
        base_height = original_height;
    } else if args.width.is_none() {
        let provided_height = args.height.unwrap();
        base_height = provided_height as f32;
        base_width = base_height * aspect_ratio;
    } else if args.height.is_none() {
        let provided_width = args.width.unwrap();
        base_width = provided_width as f32;
        base_height = base_width / aspect_ratio;
    } else {
        base_width = args.width.unwrap() as f32;
        base_height = args.height.unwrap() as f32;
    }

    let (scaled_width, scaled_height) = if let Some(scale) = args.scale {
        (base_width * scale.0, base_height * scale.1)
    } else {
        (base_width, base_height)
    };

    // Step 4: Round the scaled dimensions to the nearest integer and ensure a minimum size of 1.
    let final_width = scaled_width.round().max(1.0) as u32;
    let final_height = scaled_height.round().max(1.0) as u32;

    (final_width, final_height)
}

pub fn apply_effects(
    args: &args::Args,
    mut photon_image: PhotonImage,
) -> PhotonImage {
    let (width, height) = calculate_dimensions(args, &photon_image);
        
    if args.rotate != 0 {
        photon_image = rotate(&photon_image, args.rotate as i32);
    }

    if args.fliph {
        fliph(&mut photon_image);
    }

    if args.flipv {
        flipv(&mut photon_image);
    }

    photon_image = resize(&photon_image, width, height, match args.filter {
        args::SamplingFilter::Nearest => SamplingFilter::Nearest,
        args::SamplingFilter::Triangle => SamplingFilter::Triangle,
        args::SamplingFilter::CatmullRom => SamplingFilter::CatmullRom,
        args::SamplingFilter::Gaussian => SamplingFilter::Gaussian,
        args::SamplingFilter::Lanczos3 => SamplingFilter::Lanczos3,
    });

    type ColourFunc = fn(&mut PhotonImage, &str, f32);

    let colour_func: ColourFunc = match args.colorspace {
        args::ColourSpace::HSL => colour_spaces::hsl,
        args::ColourSpace::HSV => colour_spaces::hsv,
        args::ColourSpace::HSLUV => colour_spaces::hsluv,
        args::ColourSpace::LCH => colour_spaces::lch,
    };

    if args.dither > 0 {
        effects::dither(&mut photon_image, args.dither);
    }

    match args.brightness {
        x if x > 0.0 => {
            colour_func(&mut photon_image, "lighten", args.brightness / 100.0);
        }
        x if x < 0.0 => {
            colour_func(&mut photon_image, "darken", args.brightness.abs() / 100.0);
        },
        _ => {}
    }

    match args.saturation {
        x if x > 0.0 => {
            colour_func(&mut photon_image, "saturate", args.saturation/100.0);
        }
        x if x < 0.0 => {
            colour_func(&mut photon_image, "desaturate", args.saturation.abs()/100.0);
        }
        _ => {}
    }

    if args.contrast != 0.0 {
        effects::adjust_contrast(&mut photon_image, args.contrast);
    }

    if args.hue > 0.0 {
        colour_func(&mut photon_image, "shift_hue", args.hue/360.0);
    }

    if args.gamma != 0.0 {
        let gamma_value = 1.0 - args.gamma/255.0;
        colour_spaces::gamma_correction(&mut photon_image, gamma_value, gamma_value, gamma_value);
    }

    if args.crop.is_some() {
        let crop_args = args.crop.unwrap();
        photon_image = crop(&mut photon_image, crop_args.0, crop_args.1, crop_args.2, crop_args.3);
    }

    if args.gaussian_blur > 0 {
        conv::gaussian_blur(&mut photon_image, args.gaussian_blur);
    }

    if args.pixelize > 0 {
        effects::pixelize(&mut photon_image, args.pixelize);
    }

    if args.halftone {
        effects::halftone(&mut photon_image);
    }

    if args.invert {
        channels::invert(&mut photon_image);
    }

    if args.sepia {
        monochrome::sepia(&mut photon_image);
    }

    if args.solarize {
        effects::solarize(&mut photon_image);
    }

    if args.normalize {
        effects::normalize(&mut photon_image);
    }

    if args.noise {
        noise::add_noise_rand(&mut photon_image);
    }

    if args.sharpen {
        conv::sharpen(&mut photon_image);
    }

    if args.edge_detection {
        
        conv::edge_detection(&mut photon_image);
    }
    if args.emboss {
        conv::emboss(&mut photon_image);
    }

    if args.frosted_glass {
        effects::frosted_glass(&mut photon_image);
    }

    if args.box_blur {
        conv::box_blur(&mut photon_image);
    }

    if args.grayscale {
        monochrome::grayscale(&mut photon_image);
    }

    if args.identity {
        conv::identity(&mut photon_image);
    }

    if args.laplace {
        conv::laplace(&mut photon_image);
    }

    if args.cali {
        filters::cali(&mut photon_image);
    }

    if args.dramatic {
        filters::dramatic(&mut photon_image);
    }

    if args.firenze {
        filters::firenze(&mut photon_image);
    }

    if args.golden {
        filters::golden(&mut photon_image);
    }

    if args.lix {
        filters::lix(&mut photon_image);
    }

    if args.lofi {
        filters::lofi(&mut photon_image);
    }

    if args.neue {
        filters::neue(&mut photon_image);
    }

    if args.obsidian {
        filters::obsidian(&mut photon_image);
    }

    if args.pastel_pink {
        filters::pastel_pink(&mut photon_image);
    }

    if args.ryo {
        filters::ryo(&mut photon_image);
    }

    match &args.oil {
        Some(oil) => {
            let vals: Vec<&str> = oil.split(",").collect();
            if vals.len() == 2 {
                let radius: i32 = vals.get(0).unwrap().parse::<i32>().unwrap();
                let intensity: f64 = vals.get(1).unwrap().parse::<f64>().unwrap();
                effects::oil(&mut photon_image, radius, intensity);
            }
        }
        None => {}
    }

    photon_image
}

pub fn apply_luma_effects(args: &args::Args, mut photon_image: PhotonImage) -> PhotonImage {
    let (width, height) = calculate_dimensions(args, &photon_image);

    if args.rotate != 0 {
        photon_image = rotate(&photon_image, args.rotate);
    }

    if args.fliph {
        fliph(&mut photon_image);
    }

    if args.flipv {
        flipv(&mut photon_image);
    }

    photon_image = resize(&photon_image, width, height, match args.filter {
        args::SamplingFilter::Nearest => SamplingFilter::Nearest,
        args::SamplingFilter::Triangle => SamplingFilter::Triangle,
        args::SamplingFilter::CatmullRom => SamplingFilter::CatmullRom,
        args::SamplingFilter::Gaussian => SamplingFilter::Gaussian,
        args::SamplingFilter::Lanczos3 => SamplingFilter::Lanczos3,
    });

    type ColourFunc = fn(&mut PhotonImage, &str, f32);

    let colour_func: ColourFunc = match args.colorspace {
        args::ColourSpace::HSL => colour_spaces::hsl,
        args::ColourSpace::HSV => colour_spaces::hsv,
        args::ColourSpace::HSLUV => colour_spaces::hsluv,
        args::ColourSpace::LCH => colour_spaces::lch,
    };


    if args.luma_invert {
        channels::invert(&mut photon_image);
    }

    if args.luma_contrast != 0.0 {
        effects::adjust_contrast(&mut photon_image, args.luma_contrast);
    }

    if args.luma_gamma != 0.0 {
        let gamma_value = 1.0 - args.luma_gamma/255.0;
        colour_spaces::gamma_correction(&mut photon_image, gamma_value, gamma_value, gamma_value);
    }

    if args.luma_brightness > 0.0 {
        colour_func(&mut photon_image, "lighten", args.luma_brightness/100.0);
    } else if args.luma_brightness < 0.0 {
        colour_func(&mut photon_image, "darken", args.luma_brightness.abs()/100.0);
    }

    if args.luma_saturation < 0.0 {
        colour_func(&mut photon_image, "saturate", args.luma_saturation.abs()/100.0);
    } else if args.luma_saturation > 0.0 {
        colour_func(&mut photon_image, "desaturate", args.luma_saturation/100.0);
    }

    photon_image
}
