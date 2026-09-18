use crate::args;
use crate::font::GlyphStore;
use image::Rgba;
use imageproc::geometric_transformations::rotate_about_center;
use imageproc::geometric_transformations::Interpolation;
use once_cell::sync::Lazy;
use photon_rs::transform::{crop, fliph, flipv, resize, SamplingFilter};
use photon_rs::PhotonImage;
use photon_rs::{channels, colour_spaces, conv, effects, filters, monochrome, noise};
use std::hash::Hash;
use std::sync::Mutex;

#[derive(Hash, PartialEq, Eq, Clone)]
struct GeometryKey {
    src_hash: u64,
    width: Option<u32>,
    height: Option<u32>,
    scale: Option<(u32, u32)>,
    crop: Option<(u32, u32, u32, u32)>,
    trim: Option<(u32, u32, u32, u32)>,
    fliph: bool,
    flipv: bool,
    rotate: u32,
    filter: args::SamplingFilter,
    font: Vec<String>,
    font_size: u32,
    blocks: Vec<String>,
    exclude: Vec<char>,
    exclude_range: Vec<String>,
    include: Option<String>,
    include_range: Vec<String>,
    braille: bool,
}

fn hash_image(img: &PhotonImage) -> u64 {
    use std::hash::Hasher;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    img.get_width().hash(&mut hasher);
    img.get_height().hash(&mut hasher);
    let raw = img.get_raw_pixels();
    // Rotation commonly creates a large transparent border, so hashing only a
    // prefix can make visibly different processed images share a cache key.
    raw.hash(&mut hasher);
    hasher.finish()
}

static RESIZED_CACHE: Lazy<Mutex<Option<(GeometryKey, PhotonImage)>>> =
    Lazy::new(|| Mutex::new(None));

pub fn calculate_dimensions(
    args: &args::RenderArgs,
    glyphs: &GlyphStore,
    width: f32,
    height: f32,
) -> (u32, u32) {
    let orig_w = width;
    let orig_h = height;

    let (factor_x, factor_y) = if args.braille {
        (2.0, 4.0)
    } else {
        let (gw, gh) = glyphs.metrics;
        (gw as f32, gh as f32)
    };

    let (float_x, float_y) = if args.braille {
        (2.0, 4.0)
    } else {
        glyphs.float_metrics
    };
    // The integer bitmap dimensions necessarily jump at different font sizes
    // (width is rounded while line height is ceiled). Terminals still lay out
    // their grid from the font's continuous advance and line metrics, so using
    // the bitmap ratio here makes the output alternate between aspect ratios as
    // those two dimensions cross pixel boundaries independently.
    let cell_aspect =
        if float_x.is_finite() && float_y.is_finite() && float_x > 0.0 && float_y > 0.0 {
            float_x / float_y
        } else {
            factor_x / factor_y
        };
    let image_aspect = (orig_w / orig_h).max(f32::MIN_POSITIVE);

    log::info!("factor_x: {}, float_x: {}", factor_x, float_x);
    log::info!("factor_y: {}, float_y: {}", factor_y, float_y);

    log::info!("img width: {}", orig_w);
    log::info!("img height: {}", orig_h);

    let (mut pw, mut ph) = match (args.width, args.height) {
        (Some(wc), Some(hc)) => (wc as f32 * factor_x, hc as f32 * factor_y),

        (Some(wc), None) => {
            let ch = wc as f32 * cell_aspect / image_aspect;
            let pw = wc as f32 * factor_x;
            let ph = (ch.round() * factor_y).max(factor_y);
            (pw, ph)
        }

        (None, Some(hc)) => {
            let cw = hc as f32 * image_aspect / cell_aspect;
            let pw = (cw.round() * factor_x).max(factor_x);
            let ph = hc as f32 * factor_y;
            (pw, ph)
        }

        (None, None) => {
            let cols = (orig_w / factor_x).round().max(1.0);
            let ch = cols * cell_aspect / image_aspect;
            let pw = cols * factor_x;
            let ph = (ch.round() * factor_y).max(factor_y);
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

    let mut out_w = pw.round().max(1.0) as u32;
    let mut out_h = ph.round().max(1.0) as u32;

    // Safety cap: prevent absurd allocations (e.g. 46TB VRAM) from stale glyph metrics
    const MAX_TOTAL_PIXELS: u64 = 100_000_000; // 100M pixels
    let total = out_w as u64 * out_h as u64;
    if total > MAX_TOTAL_PIXELS {
        let scale = (MAX_TOTAL_PIXELS as f64 / total as f64).sqrt();
        out_w = (out_w as f64 * scale).round().max(1.0) as u32;
        out_h = (out_h as f64 * scale).round().max(1.0) as u32;
        log::warn!(
            "Clamped dimensions from {}x{} to {}x{} (exceeded {}M pixel cap)",
            pw.round() as u32,
            ph.round() as u32,
            out_w,
            out_h,
            MAX_TOTAL_PIXELS / 1_000_000
        );
    }

    (out_w, out_h)
}

fn fast_crop_and_nearest_resize(
    img: &PhotonImage,
    crop_coords: Option<(u32, u32, u32, u32)>, // (top, right, bottom, left)
    dest_w: u32,
    dest_h: u32,
    fliph: bool,
    flipv: bool,
) -> PhotonImage {
    let raw_w = img.get_width();
    let raw_h = img.get_height();
    let raw = img.get_raw_pixels();

    let (left, top, src_w, src_h) =
        if let Some((crop_top, crop_right, crop_bottom, crop_left)) = crop_coords {
            let l = crop_left.min(raw_w);
            let t = crop_top.min(raw_h);
            let r = crop_right.min(raw_w - l);
            let b = crop_bottom.min(raw_h - t);
            (l, t, raw_w - l - r, raw_h - t - b)
        } else {
            (0, 0, raw_w, raw_h)
        };

    let mut dest_pixels = vec![0u8; (dest_w * dest_h * 4) as usize];

    if src_w == 0 || src_h == 0 {
        return PhotonImage::new(dest_pixels, dest_w, dest_h);
    }

    for y in 0..dest_h {
        let mapped_y = if flipv { dest_h - 1 - y } else { y };
        let src_y = top + (mapped_y * src_h) / dest_h;
        let src_row_offset = (src_y * raw_w * 4) as usize;
        let dest_row_offset = (y * dest_w * 4) as usize;

        for x in 0..dest_w {
            let mapped_x = if fliph { dest_w - 1 - x } else { x };
            let src_x = left + (mapped_x * src_w) / dest_w;
            let src_idx = src_row_offset + (src_x * 4) as usize;
            let dest_idx = dest_row_offset + (x * 4) as usize;

            dest_pixels[dest_idx..dest_idx + 4].copy_from_slice(&raw[src_idx..src_idx + 4]);
        }
    }

    PhotonImage::new(dest_pixels, dest_w, dest_h)
}

fn resize_for_output(
    args: &args::RenderArgs,
    glyphs: &GlyphStore,
    mut photon_image: PhotonImage,
) -> PhotonImage {
    let src_hash = hash_image(&photon_image);

    let geometry_key = GeometryKey {
        src_hash,
        width: args.width,
        height: args.height,
        scale: args.scale.map(|(sx, sy)| (sx.to_bits(), sy.to_bits())),
        crop: args.crop,
        trim: args.trim,
        fliph: args.fliph,
        flipv: args.flipv,
        rotate: args.rotate.to_bits(),
        filter: args.filter,
        font: args.font.clone(),
        font_size: args.font_size.to_bits(),
        blocks: args.blocks.clone(),
        exclude: args.exclude.clone(),
        exclude_range: args.exclude_range.clone(),
        include: args.include.clone(),
        include_range: args.include_range.clone(),
        braille: args.braille,
    };

    let mut cache = RESIZED_CACHE.lock().unwrap();
    let mut base_image = if let Some((ref key, ref img)) = *cache {
        if key == &geometry_key {
            Some(img.clone())
        } else {
            None
        }
    } else {
        None
    };

    if base_image.is_none() {
        let mut resized = if args.filter == args::SamplingFilter::Nearest {
            let (width, height) = calculate_dimensions(
                args,
                glyphs,
                if let Some((top, right, bottom, left)) = args.trim {
                    let w = photon_image.get_width();
                    let h = photon_image.get_height();
                    if left + right < w && top + bottom < h {
                        (w - right - left) as f32
                    } else {
                        w as f32
                    }
                } else {
                    photon_image.get_width() as f32
                },
                if let Some((top, right, bottom, left)) = args.trim {
                    let w = photon_image.get_width();
                    let h = photon_image.get_height();
                    if left + right < w && top + bottom < h {
                        (h - bottom - top) as f32
                    } else {
                        h as f32
                    }
                } else {
                    photon_image.get_height() as f32
                },
            );

            fast_crop_and_nearest_resize(
                &photon_image,
                args.trim,
                width,
                height,
                args.fliph,
                args.flipv,
            )
        } else {
            if let Some((top, right, bottom, left)) = args.trim {
                let w = photon_image.get_width();
                let h = photon_image.get_height();
                if left + right < w && top + bottom < h {
                    photon_image = crop(&photon_image, left, top, w - right, h - bottom);
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

            let filter = match args.filter {
                args::SamplingFilter::Nearest => SamplingFilter::Nearest,
                args::SamplingFilter::Triangle => SamplingFilter::Triangle,
                args::SamplingFilter::CatmullRom => SamplingFilter::CatmullRom,
                args::SamplingFilter::Gaussian => SamplingFilter::Gaussian,
                args::SamplingFilter::Lanczos3 => SamplingFilter::Lanczos3,
            };

            resize(&photon_image, width, height, filter)
        };

        let width = resized.get_width();
        let height = resized.get_height();

        if args.rotate != 0.00 {
            let angle_radians = args.rotate.to_radians();
            let image_buffer =
                image::ImageBuffer::from_raw(width, height, resized.get_raw_pixels()).unwrap();
            let rotated = rotate_about_center(
                &image_buffer,
                angle_radians,
                Interpolation::Bilinear,
                Rgba([0, 0, 0, 0]),
            );
            let rotated_buffer = rotated.into_raw();
            resized = PhotonImage::new(rotated_buffer, width, height);
        }

        *cache = Some((geometry_key, resized.clone()));
        base_image = Some(resized);
    }

    base_image.unwrap()
}

pub fn default_pipeline(args: &args::RenderArgs) -> Vec<crate::pipeline::ImageEffect> {
    if !args.pipeline.is_empty() {
        args.pipeline.clone()
    } else {
        use crate::pipeline::ImageEffect;
        let mut p = Vec::new();
        if args.dither > 0 {
            p.push(ImageEffect::Dither(args.dither));
        }
        if args.brightness != 0.0 {
            p.push(ImageEffect::Brightness(args.brightness));
        }
        if args.saturation != 0.0 {
            p.push(ImageEffect::Saturation(args.saturation));
        }
        if args.contrast != 0.0 {
            p.push(ImageEffect::Contrast(args.contrast));
        }
        if args.hue != 0.0 {
            p.push(ImageEffect::Hue(args.hue));
        }
        if args.gamma != 0.0 {
            p.push(ImageEffect::Gamma(args.gamma));
        }
        if args.gaussian_blur > 0 {
            p.push(ImageEffect::GaussBlur(args.gaussian_blur));
        }
        if args.pixelize > 0 {
            p.push(ImageEffect::Pixelize(args.pixelize));
        }
        if args.halftone {
            p.push(ImageEffect::Halftone);
        }
        if args.invert {
            p.push(ImageEffect::Invert);
        }
        if args.sepia {
            p.push(ImageEffect::Sepia);
        }
        if args.solarize {
            p.push(ImageEffect::Solarize);
        }
        if args.normalize {
            p.push(ImageEffect::Normalize);
        }
        if args.noise {
            p.push(ImageEffect::Noise);
        }
        if args.sharpen {
            p.push(ImageEffect::Sharpen);
        }
        if args.edge_detection {
            p.push(ImageEffect::EdgeDetect);
        }
        if args.emboss {
            p.push(ImageEffect::Emboss);
        }
        if args.frosted_glass {
            p.push(ImageEffect::FrostGlass);
        }
        if args.box_blur {
            p.push(ImageEffect::BoxBlur);
        }
        if args.grayscale {
            p.push(ImageEffect::Grayscale);
        }
        if args.identity {
            p.push(ImageEffect::Identity);
        }
        if args.laplace {
            p.push(ImageEffect::Laplace);
        }
        if args.cali {
            p.push(ImageEffect::Cali);
        }
        if args.dramatic {
            p.push(ImageEffect::Dramatic);
        }
        if args.firenze {
            p.push(ImageEffect::Firenze);
        }
        if args.golden {
            p.push(ImageEffect::Golden);
        }
        if args.lix {
            p.push(ImageEffect::Lix);
        }
        if args.lofi {
            p.push(ImageEffect::Lofi);
        }
        if args.neue {
            p.push(ImageEffect::Neue);
        }
        if args.obsidian {
            p.push(ImageEffect::Obsidian);
        }
        if args.pastel_pink {
            p.push(ImageEffect::PastelPink);
        }
        if args.ryo {
            p.push(ImageEffect::Ryo);
        }
        if let Some(oil) = &args.oil {
            let vals: Vec<&str> = oil.split(",").collect();
            if vals.len() == 2 {
                if let (Ok(r), Ok(i)) = (vals[0].parse::<i32>(), vals[1].parse::<f64>()) {
                    p.push(ImageEffect::Oil(r, i));
                }
            }
        }
        p
    }
}

pub fn apply_effects(
    args: &args::RenderArgs,
    glyphs: &GlyphStore,
    photon_image: PhotonImage,
) -> PhotonImage {
    let pipeline = default_pipeline(args);
    resize_for_output(args, glyphs, apply_pipeline(args, photon_image, &pipeline))
}

/// Apply exactly these nodes, without resizing the output grid or adding defaults.
pub fn apply_pipeline(
    args: &args::RenderArgs,
    mut photon_image: PhotonImage,
    pipeline: &[crate::pipeline::ImageEffect],
) -> PhotonImage {
    // Pipeline nodes operate on the source image. Output sizing is deliberately
    // deferred until every node has run so ScaleX/ScaleY alter the input
    // geometry without multiplying the configured terminal output width.
    type ColourFunc = fn(&mut PhotonImage, &str, f32);

    let colour_func: ColourFunc = match args.colorspace {
        args::ColourSpace::HSL => colour_spaces::hsl,
        args::ColourSpace::HSV => colour_spaces::hsv,
        args::ColourSpace::HSLUV => colour_spaces::hsluv,
        args::ColourSpace::LCH => colour_spaces::lch,
    };

    for effect in pipeline.iter().cloned() {
        use crate::pipeline::ImageEffect::*;
        match effect {
            ReplaceColour {
                from,
                to,
                tolerance,
            } => match args.render {
                args::Render::Irc => crate::adjustments::replace_colour_quantized(
                    &mut photon_image,
                    from,
                    to,
                    tolerance,
                    &crate::palette::IRC99,
                ),
                args::Render::Ansi => crate::adjustments::replace_colour_quantized(
                    &mut photon_image,
                    from,
                    to,
                    tolerance,
                    &crate::palette::ANSI256,
                ),
                args::Render::Ansi24 => {
                    crate::adjustments::replace_colour(&mut photon_image, from, to, tolerance)
                }
            },
            LumaContrast(c) => crate::adjustments::luma_contrast(&mut photon_image, c),
            MedianBlur(r) => crate::adjustments::median_blur(&mut photon_image, r),
            DarkLines(r) => crate::adjustments::line_thickness(&mut photon_image, r, false),
            LightLines(r) => crate::adjustments::line_thickness(&mut photon_image, r, true),
            Dither(d) => {
                if d > 0 {
                    effects::dither(&mut photon_image, d);
                }
            }
            Brightness(b) => match b {
                x if x > 0.0 => colour_func(&mut photon_image, "lighten", b / 100.0),
                x if x < 0.0 => colour_func(&mut photon_image, "darken", b.abs() / 100.0),
                _ => {}
            },
            Saturation(s) => match s {
                x if x > 0.0 => colour_func(&mut photon_image, "saturate", s / 100.0),
                x if x < 0.0 => colour_func(&mut photon_image, "desaturate", s.abs() / 100.0),
                _ => {}
            },
            Contrast(c) => {
                if c != 0.0 {
                    effects::adjust_contrast(&mut photon_image, c);
                }
            }
            Hue(h) => {
                if h > 0.0 {
                    colour_func(&mut photon_image, "shift_hue", h / 360.0);
                }
            }
            Gamma(g_val) => {
                if g_val != 0.0 {
                    let g = 1.0 - g_val / 255.0;
                    colour_spaces::gamma_correction(&mut photon_image, g, g, g);
                }
            }
            GaussBlur(r) => {
                if r > 0 {
                    conv::gaussian_blur(&mut photon_image, r.try_into().unwrap());
                }
            }
            LumaBlur(_) => { /* not supported here natively but space for it */ }
            Pixelize(p) => {
                if p > 0 {
                    effects::pixelize(&mut photon_image, p);
                }
            }
            Halftone => {
                effects::halftone(&mut photon_image);
            }
            Invert => {
                channels::invert(&mut photon_image);
            }
            Sepia => {
                monochrome::sepia(&mut photon_image);
            }
            Solarize => {
                effects::solarize(&mut photon_image);
            }
            Normalize => {
                effects::normalize(&mut photon_image);
            }
            Noise => {
                noise::add_noise_rand(&mut photon_image);
            }
            Sharpen => {
                conv::sharpen(&mut photon_image);
            }
            EdgeDetect => {
                conv::edge_detection(&mut photon_image);
            }
            Emboss => {
                conv::emboss(&mut photon_image);
            }
            FrostGlass => {
                effects::frosted_glass(&mut photon_image);
            }
            BoxBlur => {
                conv::box_blur(&mut photon_image);
            }
            Grayscale => {
                monochrome::grayscale(&mut photon_image);
            }
            Identity => {
                conv::identity(&mut photon_image);
            }
            Laplace => {
                conv::laplace(&mut photon_image);
            }
            Cali => {
                filters::cali(&mut photon_image);
            }
            Dramatic => {
                filters::dramatic(&mut photon_image);
            }
            Firenze => {
                filters::firenze(&mut photon_image);
            }
            Golden => {
                filters::golden(&mut photon_image);
            }
            Lix => {
                filters::lix(&mut photon_image);
            }
            Lofi => {
                filters::lofi(&mut photon_image);
            }
            Neue => {
                filters::neue(&mut photon_image);
            }
            Obsidian => {
                filters::obsidian(&mut photon_image);
            }
            PastelPink => {
                filters::pastel_pink(&mut photon_image);
            }
            Ryo => {
                filters::ryo(&mut photon_image);
            }
            Oil(r, i) => {
                effects::oil(&mut photon_image, r, i);
            }
            Eyedropper => {} // Implementation comes later
            // Geometry effects applied inline
            CropTop(_) | CropBottom(_) | CropLeft(_) | CropRight(_) => {
                let w = photon_image.get_width();
                let h = photon_image.get_height();
                let (top, right, bottom, left) = match &effect {
                    CropTop(v) => (*v as u32, 0, 0, 0),
                    CropBottom(v) => (0, 0, *v as u32, 0),
                    CropLeft(v) => (0, 0, 0, *v as u32),
                    CropRight(v) => (0, *v as u32, 0, 0),
                    _ => unreachable!(),
                };
                let t = top.min(h.saturating_sub(1));
                let b = bottom.min(h.saturating_sub(1 + t));
                let l = left.min(w.saturating_sub(1));
                let r_val = right.min(w.saturating_sub(1 + l));
                if l + r_val < w && t + b < h {
                    photon_image = crop(&photon_image, l, t, w - r_val, h - b);
                }
            }
            FlipH => {
                fliph(&mut photon_image);
            }
            FlipV => {
                flipv(&mut photon_image);
            }
            Rotate(deg) => {
                if deg != 0.0 {
                    use image::Rgba;
                    use imageproc::geometric_transformations::{
                        rotate_about_center, Interpolation,
                    };
                    let angle_rad = deg.to_radians();
                    let w = photon_image.get_width();
                    let h = photon_image.get_height();
                    let buf =
                        image::ImageBuffer::from_raw(w, h, photon_image.get_raw_pixels()).unwrap();
                    let rotated = rotate_about_center(
                        &buf,
                        angle_rad,
                        Interpolation::Bilinear,
                        Rgba([0, 0, 0, 0]),
                    );
                    photon_image = PhotonImage::new(rotated.into_raw(), w, h);
                }
            }
            ScaleX(sx) => {
                let w = ((photon_image.get_width() as f32 * sx as f32 / 100.0) as u32).max(1);
                let h = photon_image.get_height();
                photon_image = resize(&photon_image, w, h, SamplingFilter::Lanczos3);
            }
            ScaleY(sy) => {
                let w = photon_image.get_width();
                let h = ((photon_image.get_height() as f32 * sy as f32 / 100.0) as u32).max(1);
                photon_image = resize(&photon_image, w, h, SamplingFilter::Lanczos3);
            }
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
            photon_image = crop(&photon_image, left, top, w - right, h - bottom);
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use std::collections::{HashMap, HashSet};

    fn rotate_test_image(source: &PhotonImage, degrees: f32) -> PhotonImage {
        let width = source.get_width();
        let height = source.get_height();
        let buffer = image::ImageBuffer::from_raw(width, height, source.get_raw_pixels()).unwrap();
        let rotated = rotate_about_center(
            &buffer,
            degrees.to_radians(),
            Interpolation::Bilinear,
            Rgba([0, 0, 0, 0]),
        );
        PhotonImage::new(rotated.into_raw(), width, height)
    }

    #[test]
    fn rotate_cache_identity_changes_after_a_transparent_prefix() {
        let width = 64_u32;
        let height = 64_u32;
        let mut pixels = vec![0_u8; (width * height * 4) as usize];
        for y in 22..40 {
            for x in 18..43 {
                let offset = ((y * width + x) * 4) as usize;
                pixels[offset..offset + 4].copy_from_slice(&[
                    (x * 4) as u8,
                    (y * 5) as u8,
                    ((x + y) * 2) as u8,
                    255,
                ]);
            }
        }
        let source = PhotonImage::new(pixels, width, height);
        let first = rotate_test_image(&source, 12.0);
        let second = rotate_test_image(&source, 28.0);

        assert_ne!(first.get_raw_pixels(), second.get_raw_pixels());
        assert_ne!(hash_image(&first), hash_image(&second));
    }

    #[test]
    fn pipeline_scale_changes_source_not_configured_output_size() {
        let cli = crate::args::Args::try_parse_from(["img2irc", "--width", "10"]).unwrap();
        let mut args = crate::args_to_render_args(&cli);
        let unscaled_args = args.clone();
        args.pipeline = vec![
            crate::pipeline::ImageEffect::ScaleX(200),
            crate::pipeline::ImageEffect::ScaleY(200),
        ];

        let glyphs = GlyphStore {
            glyphs: Vec::new(),
            groups: HashMap::new(),
            selected: HashSet::new(),
            metrics: (1, 1),
            float_metrics: (1.0, 1.0),
        };
        let source = PhotonImage::new(vec![128; 100 * 50 * 4], 100, 50);

        let unscaled_output = apply_effects(&unscaled_args, &glyphs, source.clone());
        let output = apply_effects(&args, &glyphs, source);

        assert_eq!(output.get_width(), unscaled_output.get_width());
        assert_eq!(output.get_height(), unscaled_output.get_height());
        assert_eq!((output.get_width(), output.get_height()), (10, 5));
    }
}
