use crate::args;
use crate::font::GlyphStore;
use photon_rs::{colour_spaces, channels, conv, effects, filters, monochrome, noise};
use photon_rs::transform::{SamplingFilter, resize, crop, rotate, flipv, fliph};
use photon_rs::PhotonImage;

fn calculate_dimensions(args: &args::Args, glyphs: &GlyphStore, image: &PhotonImage) -> (u32, u32) {
    let orig_w = image.get_width()  as f32;
    let orig_h = image.get_height() as f32;

    // determine per‐cell pixel factor:
    // braille cells cover 2×4 pixels; block cells use glyph bitmap size
    let (factor_x, factor_y) = if args.braille {
        (2.0, 4.0)
    } else {
        let (gh, gw) = glyphs.metrics;
        (gw as f32, gh as f32)
    };

    let (mut pw, mut ph) = match (args.width, args.height) {
        (Some(wc), Some(hc)) => (
            wc as f32 * factor_x,
            hc as f32 * factor_y,
        ),

        // only width in characters → width*factor_x pixels, height by original aspect
        (Some(wc), None) => {
            let pw = wc as f32 * factor_x;
            let ph = pw * (orig_h / orig_w);
            (pw, ph)
        }

        // only height in characters → height*factor_y pixels, width by original aspect
        (None, Some(hc)) => {
            let ph = hc as f32 * factor_y;
            let pw = ph * (orig_w / orig_h);
            (pw, ph)
        }

        // neither specified → keep original pixel dimensions
        (None, None) => (orig_w, orig_h),
    };

    if let Some((sx, sy)) = args.scale {
        pw *= sx;
        ph *= sy;
    }

    // braille aspect tweak: if only one dimension was specified,
    // shrink the other by 10% to compensate cell shape
    if args.braille {
        match (args.width.is_some(), args.height.is_some()) {
            (true, false) => {
                // width fixed → reduce height
                ph *= 0.9;
            }
            (false, true) => {
                // height fixed → reduce width
                pw *= 1.11;
            }
            _ => {}
        }
    }

    let out_w = pw.round().max(1.0) as u32;
    let out_h = ph.round().max(1.0) as u32;

    (out_w, out_h)
}

pub fn apply_effects(
    args: &args::Args,
    glyphs: &GlyphStore,
    mut photon_image: PhotonImage,
) -> PhotonImage {
    let (width, height) = calculate_dimensions(args, glyphs, &photon_image);
        
    if args.rotate != 0.00 {
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

    if args.luma_blur > 0 {
        apply_luma_blur(&mut photon_image, args.luma_blur);
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

pub fn apply_luma_effects(args: &args::Args, glyphs: &GlyphStore, mut photon_image: PhotonImage) -> PhotonImage {
    let (width, height) = calculate_dimensions(args, glyphs, &photon_image);

    if args.rotate != 0.00 {
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

fn apply_luma_blur(img: &mut PhotonImage, radius: i32) {
    let mut luma = img.clone();
    monochrome::grayscale(&mut luma);
    conv::gaussian_blur(&mut luma, radius);
    
    let raw_orig = img.get_raw_pixels();
    let raw_luma = luma.get_raw_pixels();
    let mut new_raw = raw_orig.clone();
    
    // YUV coefficients (Rec. 601)
    for i in (0..raw_orig.len()).step_by(4) {
        let r = raw_orig[i] as f32;
        let g = raw_orig[i+1] as f32;
        let b = raw_orig[i+2] as f32;
        
        // Calculate original Chroma (U, V)
        let y_old = 0.299 * r + 0.587 * g + 0.114 * b;
        let u = 0.492 * (b - y_old);
        let v = 0.877 * (r - y_old);
        
        // Use new Luma (Y) from blurred grayscale image
        let y_new = raw_luma[i] as f32;
        
        // Convert back to RGB
        let new_r = (y_new + 1.140 * v).clamp(0.0, 255.0) as u8;
        let new_g = (y_new - 0.395 * u - 0.581 * v).clamp(0.0, 255.0) as u8;
        let new_b = (y_new + 2.032 * u).clamp(0.0, 255.0) as u8;
        
        new_raw[i] = new_r;
        new_raw[i+1] = new_g;
        new_raw[i+2] = new_b;
        // Alpha remains unchanged
    }
    
    *img = PhotonImage::new(new_raw, img.get_width(), img.get_height());
}
