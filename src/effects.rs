use crate::args;
use crate::font::GlyphStore;
use image::Rgba;
use imageproc::geometric_transformations::rotate_about_center;
use imageproc::geometric_transformations::Interpolation;
use photon_rs::transform::{crop, fliph, flipv, resize, SamplingFilter};
use photon_rs::PhotonImage;
use photon_rs::{channels, colour_spaces, conv, effects, filters, monochrome, noise};

pub fn calculate_dimensions(
    args: &args::RenderArgs,
    glyphs: &GlyphStore,
    width: f32,
    height: f32,
) -> (u32, u32) {
    let orig_w = width;
    let orig_h = height;

    // determine per‐cell pixel factor:
    // braille cells cover 2×4 pixels; block cells use glyph bitmap size
    let (factor_x, factor_y) = if args.braille {
        (2.0, 4.0)
    } else {
        let (gw, gh) = glyphs.metrics;
        (gw as f32, gh as f32)
    };

    log::info!("factor_x: {}", factor_x);
    log::info!("factor_y: {}", factor_y);

    log::info!("img width: {}", orig_w);
    log::info!("img height: {}", orig_h);

    let (mut pw, mut ph) = match (args.width, args.height) {
        (Some(wc), Some(hc)) => (wc as f32 * factor_x, hc as f32 * factor_y),

        (Some(wc), None) => {
            let pw = wc as f32 * factor_x;
            let ph = pw * (orig_h / orig_w);
            (pw, ph)
        }

        // "if the user suppplied onyl the heighT: scale the width according to the origina laspec tratio"
        (None, Some(hc)) => {
            let ph = hc as f32 * factor_y;
            let pw = ph * (orig_w / orig_h);
            (pw, ph)
        }

        // "if neither width nor height are supplied, then divide the height of the image by the heiht of the glyph to get the output height, use the image aspect ratio to get thew idth"
        (None, None) => {
            // Height in chars = Image Height / Glyph Height
            // Output Pixel Height = Height in Chars * Glyph Height = Image Height
            let ph = orig_h;

            // Output Pixel Width based on visual aspect ratio
            // Visual Aspect = pw / ph = orig_w / orig_h
            // pw = ph * (orig_w / orig_h)
            let pw = ph * (orig_w / orig_h);
            (pw, ph)
        }
    };

    log::info!("pw: {}", pw);
    log::info!("ph: {}", ph);

    if let Some((sx, sy)) = args.scale {
        pw *= sx;
        ph *= sy;
    }

    log::info!("Final target dimensions (pixels): {}x{}", pw, ph);

    let out_w = pw.round().max(1.0) as u32;
    let out_h = ph.round().max(1.0) as u32;

    (out_w, out_h)
}

pub fn apply_effects(
    args: &args::RenderArgs,
    glyphs: &GlyphStore,
    mut photon_image: PhotonImage,
) -> PhotonImage {
    if let Some((top, right, bottom, left)) = args.trim {
        let w = photon_image.get_width();
        let h = photon_image.get_height();
        if left + right < w && top + bottom < h {
            // crop takes x1, y1, x2, y2 (coordinates)
            photon_image = crop(&mut photon_image, left, top, w - right, h - bottom);
        }
    }

    let (width, height) = calculate_dimensions(
        args,
        glyphs,
        photon_image.get_width() as f32,
        photon_image.get_height() as f32,
    );

    if args.fliph {
        fliph(&mut photon_image);
    }

    if args.flipv {
        flipv(&mut photon_image);
    }

    let filter = if args.render == crate::args::Render::Ansi24 {
        SamplingFilter::Nearest
    } else {
        match args.filter {
            args::SamplingFilter::Nearest => SamplingFilter::Nearest,
            args::SamplingFilter::Triangle => SamplingFilter::Triangle,
            args::SamplingFilter::CatmullRom => SamplingFilter::CatmullRom,
            args::SamplingFilter::Gaussian => SamplingFilter::Gaussian,
            args::SamplingFilter::Lanczos3 => SamplingFilter::Lanczos3,
        }
    };

    let mut photon_image = resize(&photon_image, width, height, filter);

    if args.rotate != 0.00 {
        let angle_radians = args.rotate.to_radians();
        let image_buffer = image::ImageBuffer::from_raw(
            photon_image.get_width(),
            photon_image.get_height(),
            photon_image.get_raw_pixels(),
        )
        .unwrap();
        let rotated = rotate_about_center(
            &image_buffer,
            angle_radians,
            Interpolation::Bilinear,
            Rgba([0, 0, 0, 0]), // Fill empty space with transparency
        );
        let rotated_buffer = rotated.into_raw();
        photon_image = PhotonImage::new(rotated_buffer, width, height);
    }

    type ColourFunc = fn(&mut PhotonImage, &str, f32);

    let colour_func: ColourFunc = match args.colorspace {
        args::ColourSpace::HSL => colour_spaces::hsl,
        args::ColourSpace::HSV => colour_spaces::hsv,
        args::ColourSpace::HSLUV => colour_spaces::hsluv,
        args::ColourSpace::LCH => colour_spaces::lch,
    };

    let pipeline = if !args.pipeline.is_empty() {
        args.pipeline.clone()
    } else {
        vec![
            "Dither".to_string(),
            "Brightness".to_string(),
            "Saturation".to_string(),
            "Contrast".to_string(),
            "Hue".to_string(),
            "Gamma".to_string(),
            "Gauss Blur".to_string(),
            "Pixelize".to_string(),
            "Halftone".to_string(),
            "Invert".to_string(),
            "Sepia".to_string(),
            "Solarize".to_string(),
            "Normalize".to_string(),
            "Noise".to_string(),
            "Sharpen".to_string(),
            "EdgeDetect".to_string(),
            "Emboss".to_string(),
            "FrostGlass".to_string(),
            "Box Blur".to_string(),
            "Grayscale".to_string(),
            "Identity".to_string(),
            "Laplace".to_string(),
            "Cali".to_string(),
            "Dramatic".to_string(),
            "Firenze".to_string(),
            "Golden".to_string(),
            "Lix".to_string(),
            "Lofi".to_string(),
            "Neue".to_string(),
            "Obsidian".to_string(),
            "PastelPink".to_string(),
            "Ryo".to_string(),
            "Oil".to_string(),
        ]
    };

    for effect in pipeline {
        match effect.as_str() {
            "Dither" => {
                if args.dither > 0 {
                    effects::dither(&mut photon_image, args.dither);
                }
            }
            "Brightness" => match args.brightness {
                x if x > 0.0 => colour_func(&mut photon_image, "lighten", args.brightness / 100.0),
                x if x < 0.0 => {
                    colour_func(&mut photon_image, "darken", args.brightness.abs() / 100.0)
                }
                _ => {}
            },
            "Saturation" => match args.saturation {
                x if x > 0.0 => colour_func(&mut photon_image, "saturate", args.saturation / 100.0),
                x if x < 0.0 => colour_func(
                    &mut photon_image,
                    "desaturate",
                    args.saturation.abs() / 100.0,
                ),
                _ => {}
            },
            "Contrast" => {
                if args.contrast != 0.0 {
                    effects::adjust_contrast(&mut photon_image, args.contrast);
                }
            }
            "Hue" => {
                if args.hue > 0.0 {
                    colour_func(&mut photon_image, "shift_hue", args.hue / 360.0);
                }
            }
            "Gamma" => {
                if args.gamma != 0.0 {
                    let g = 1.0 - args.gamma / 255.0;
                    colour_spaces::gamma_correction(&mut photon_image, g, g, g);
                }
            }
            "Gauss Blur" => {
                if args.gaussian_blur > 0 {
                    conv::gaussian_blur(
                        &mut photon_image,
                        (args.gaussian_blur as u32).try_into().unwrap(),
                    );
                }
            }
            "Pixelize" => {
                if args.pixelize > 0 {
                    effects::pixelize(&mut photon_image, args.pixelize as i32);
                }
            }
            "Halftone" => {
                if args.halftone {
                    effects::halftone(&mut photon_image);
                }
            }
            "Invert" => {
                if args.invert {
                    channels::invert(&mut photon_image);
                }
            }
            "Sepia" => {
                if args.sepia {
                    monochrome::sepia(&mut photon_image);
                }
            }
            "Solarize" => {
                if args.solarize {
                    effects::solarize(&mut photon_image);
                }
            }
            "Normalize" => {
                if args.normalize {
                    effects::normalize(&mut photon_image);
                }
            }
            "Noise" => {
                if args.noise {
                    noise::add_noise_rand(&mut photon_image);
                }
            }
            "Sharpen" => {
                if args.sharpen {
                    conv::sharpen(&mut photon_image);
                }
            }
            "EdgeDetect" => {
                if args.edge_detection {
                    conv::edge_detection(&mut photon_image);
                }
            }
            "Emboss" => {
                if args.emboss {
                    conv::emboss(&mut photon_image);
                }
            }
            "FrostGlass" => {
                if args.frosted_glass {
                    effects::frosted_glass(&mut photon_image);
                }
            }
            "Box Blur" => {
                if args.box_blur {
                    conv::box_blur(&mut photon_image);
                }
            }
            "Grayscale" => {
                if args.grayscale {
                    monochrome::grayscale(&mut photon_image);
                }
            }
            "Identity" => {
                if args.identity {
                    conv::identity(&mut photon_image);
                }
            }
            "Laplace" => {
                if args.laplace {
                    conv::laplace(&mut photon_image);
                }
            }
            "Cali" => {
                if args.cali {
                    filters::cali(&mut photon_image);
                }
            }
            "Dramatic" => {
                if args.dramatic {
                    filters::dramatic(&mut photon_image);
                }
            }
            "Firenze" => {
                if args.firenze {
                    filters::firenze(&mut photon_image);
                }
            }
            "Golden" => {
                if args.golden {
                    filters::golden(&mut photon_image);
                }
            }
            "Lix" => {
                if args.lix {
                    filters::lix(&mut photon_image);
                }
            }
            "Lofi" => {
                if args.lofi {
                    filters::lofi(&mut photon_image);
                }
            }
            "Neue" => {
                if args.neue {
                    filters::neue(&mut photon_image);
                }
            }
            "Obsidian" => {
                if args.obsidian {
                    filters::obsidian(&mut photon_image);
                }
            }
            "PastelPink" => {
                if args.pastel_pink {
                    filters::pastel_pink(&mut photon_image);
                }
            }
            "Ryo" => {
                if args.ryo {
                    filters::ryo(&mut photon_image);
                }
            }
            "Oil" => {
                if let Some(oil) = &args.oil {
                    let vals: Vec<&str> = oil.split(",").collect();
                    if vals.len() == 2 {
                        if let (Ok(r), Ok(i)) = (vals[0].parse::<i32>(), vals[1].parse::<f64>()) {
                            effects::oil(&mut photon_image, r, i);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    photon_image
}

pub fn apply_luma_effects(
    args: &args::RenderArgs,
    glyphs: &GlyphStore,
    mut photon_image: PhotonImage,
) -> PhotonImage {
    if let Some((top, right, bottom, left)) = args.trim {
        let w = photon_image.get_width();
        let h = photon_image.get_height();
        if left + right < w && top + bottom < h {
            photon_image = crop(&mut photon_image, left, top, w - right, h - bottom);
        }
    }

    let (width, height) = calculate_dimensions(
        args,
        glyphs,
        photon_image.get_width() as f32,
        photon_image.get_height() as f32,
    );

    if args.fliph {
        fliph(&mut photon_image);
    }

    if args.flipv {
        flipv(&mut photon_image);
    }

    photon_image = resize(
        &photon_image,
        width,
        height,
        match args.filter {
            args::SamplingFilter::Nearest => SamplingFilter::Nearest,
            args::SamplingFilter::Triangle => SamplingFilter::Triangle,
            args::SamplingFilter::CatmullRom => SamplingFilter::CatmullRom,
            args::SamplingFilter::Gaussian => SamplingFilter::Gaussian,
            args::SamplingFilter::Lanczos3 => SamplingFilter::Lanczos3,
        },
    );

    if args.rotate != 0.00 {
        let angle_radians = args.rotate.to_radians();
        let image_buffer = image::ImageBuffer::from_raw(
            photon_image.get_width(),
            photon_image.get_height(),
            photon_image.get_raw_pixels(),
        )
        .unwrap();
        let rotated = rotate_about_center(
            &image_buffer,
            angle_radians,
            Interpolation::Bilinear,
            Rgba([0, 0, 0, 0]), // Fill empty space with transparency
        );
        let rotated_buffer = rotated.into_raw();
        photon_image = PhotonImage::new(rotated_buffer, width, height);
    }

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
        let gamma_value = 1.0 - args.luma_gamma / 255.0;
        colour_spaces::gamma_correction(&mut photon_image, gamma_value, gamma_value, gamma_value);
    }

    if args.luma_brightness > 0.0 {
        colour_func(&mut photon_image, "lighten", args.luma_brightness / 100.0);
    } else if args.luma_brightness < 0.0 {
        colour_func(
            &mut photon_image,
            "darken",
            args.luma_brightness.abs() / 100.0,
        );
    }

    if args.luma_saturation < 0.0 {
        colour_func(
            &mut photon_image,
            "saturate",
            args.luma_saturation.abs() / 100.0,
        );
    } else if args.luma_saturation > 0.0 {
        colour_func(
            &mut photon_image,
            "desaturate",
            args.luma_saturation / 100.0,
        );
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
        let g = raw_orig[i + 1] as f32;
        let b = raw_orig[i + 2] as f32;

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
        new_raw[i + 1] = new_g;
        new_raw[i + 2] = new_b;
        // Alpha remains unchanged
    }

    *img = PhotonImage::new(new_raw, img.get_width(), img.get_height());
}
