pub mod args;
pub mod config;
pub mod draw;
pub mod effects;
pub mod font;
pub mod gpu;
pub mod palette;
pub mod pipeline;

pub mod colorpicker;
pub mod tui;

use crate::args::{Args, Param, RenderArgs};
use photon_rs::PhotonImage;
use std::{
    error::Error,
    io::{Cursor, Write},
};
use url::Url;

// Main execution logic moved to binary files

pub fn args_to_render_args(args: &Args) -> RenderArgs {
    let (mut sx, mut sy) = args.scale.unwrap_or((1.0, 1.0));
    if let Some(p) = &args.scalex {
        sx = param_base(p);
    }
    if let Some(p) = &args.scaley {
        sy = param_base(p);
    }

    let mut crop = args.crop;
    if args.crop_x1.is_some()
        || args.crop_y1.is_some()
        || args.crop_x2.is_some()
        || args.crop_y2.is_some()
    {
        let (mut x1, mut y1, mut x2, mut y2) = crop.unwrap_or((0, 0, 0, 0));
        if let Some(p) = &args.crop_x1 {
            x1 = param_base(p) as u32;
        }
        if let Some(p) = &args.crop_y1 {
            y1 = param_base(p) as u32;
        }
        if let Some(p) = &args.crop_x2 {
            x2 = param_base(p) as u32;
        }
        if let Some(p) = &args.crop_y2 {
            y2 = param_base(p) as u32;
        }
        crop = Some((x1, y1, x2, y2));
    }

    RenderArgs {
        as_preview: false,
        list_blocks: args.list_blocks,
        list_ranges: args.list_ranges,
        print_ranges: args.print_ranges,
        show_block_bitmaps: args.show_block_bitmaps,
        clear_cache: args.clear_cache,
        auto_optimize_batch_size: args.auto_optimize_batch_size,

        image: args.image.clone(),
        width: args.width.as_ref().map(|p| param_base(p) as u32),
        height: args.height.as_ref().map(|p| param_base(p) as u32),
        scale: Some((sx, sy)),
        crop,
        trim: args.trim,
        filter: args.filter.clone(),
        rotate: param_base(&args.rotate),
        fliph: args.fliph,
        flipv: args.flipv,
        render: args.render,
        braille: args.braille,
        blocks: args.blocks.clone(),
        include_range: args.include_range.clone(),
        include: args.include.clone(),
        exclude_range: args.exclude_range.clone(),
        exclude: args.exclude.clone(),
        font: args.font.clone(),
        font_size: param_base(&args.font_size),

        grayscale_tolerance: param_base(&args.grayscale_tolerance) as u8,

        brightness: param_base(&args.brightness),
        contrast: param_base(&args.contrast),
        gamma: param_base(&args.gamma),
        saturation: param_base(&args.saturation),
        hue: param_base(&args.hue),
        invert: args.invert,
        dither: args.dither,
        luma_brightness: param_base(&args.luma_brightness),
        luma_contrast: param_base(&args.luma_contrast),
        luma_gamma: param_base(&args.luma_gamma),
        luma_saturation: param_base(&args.luma_saturation),
        luma_invert: args.luma_invert,
        colorspace: args.colorspace.clone(),
        encoding: args.encoding.clone(),
        grayscale: args.grayscale,
        nograyscale: args.nograyscale,
        pixelize: param_base(&args.pixelize) as i32,
        box_blur: args.box_blur,
        gaussian_blur: param_base(&args.gaussian_blur) as i32,
        luma_blur: param_base(&args.luma_blur) as i32,
        oil: args.oil.clone(),
        halftone: args.halftone,
        sepia: args.sepia,
        normalize: args.normalize,
        noise: args.noise,
        emboss: args.emboss,
        identity: args.identity,
        laplace: args.laplace,
        noise_reduction: args.noise_reduction,
        sharpen: args.sharpen,
        cali: args.cali,
        dramatic: args.dramatic,
        firenze: args.firenze,
        golden: args.golden,
        lix: args.lix,
        lofi: args.lofi,
        neue: args.neue,
        obsidian: args.obsidian,
        pastel_pink: args.pastel_pink,
        ryo: args.ryo,
        frosted_glass: args.frosted_glass,
        solarize: args.solarize,
        edge_detection: args.edge_detection,
        show_discontinuities: args.show_discontinuities,
        discontinuity_threshold: args.discontinuity_threshold,
        fix_disc: args.fix_disc,
        fix_disc_passes: args.fix_disc_passes,
        fix_disc_shift_radius: args.fix_disc_shift_radius,
        fix_disc_ref_tolerance_ratio: args.fix_disc_ref_tolerance_ratio,
        fix_disc_ref_tolerance_pixels: args.fix_disc_ref_tolerance_pixels,
        fix_disc_pool_size: args.fix_disc_pool_size,
        misc3: args.misc3,
        misc4: args.misc4,
        misc5: args.misc5,
        misc6: args.misc6,
        misc7: args.misc7,
        misc8: args.misc8,
        gpu: args.gpu,
        save: args.save.clone(),
        fft_debug_dir: args.fft_debug_dir.clone(),

        score: args.score,
        pipeline: Vec::new(),
        overlays: Vec::new(),
    }
}

pub fn param_base(p: &Param) -> f32 {
    match p {
        Param::Fixed(v) => *v,
        Param::Range { start, .. } => *start,
    }
}

pub fn prepare_canvas(
    args: &RenderArgs,
    glyphs: &font::GlyphStore,
    image: PhotonImage,
) -> (draw::AnsiImage, Option<draw::AnsiImage>) {
    let image = effects::apply_effects(args, glyphs, image);

    if let Some(path) = &args.save {
        let _ = photon_rs::native::save_image(image.clone(), path);
    }

    let canvas = draw::AnsiImage::new(image.clone(), glyphs.clone(), args.grayscale_tolerance);

    let luma_canvas = if args.braille {
        let image_luma = effects::apply_luma_effects(args, glyphs, image);
        Some(draw::AnsiImage::new(
            image_luma,
            glyphs.clone(),
            args.grayscale_tolerance,
        ))
    } else {
        None
    };
    (canvas, luma_canvas)
}

pub fn render_from_canvas(
    args: &RenderArgs,
    canvas: &draw::AnsiImage,
    luma_canvas: Option<&draw::AnsiImage>,
    glyphs: &[draw::FastGlyph],
    _abort_flag: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> draw::RenderResult {
    if args.braille {
        if let Some(lc) = luma_canvas {
            match args.render {
                crate::args::Render::Irc => {
                    draw::render_braille(lc, canvas, args, crate::args::Render::Irc)
                }
                crate::args::Render::Ansi => {
                    draw::render_braille(lc, canvas, args, crate::args::Render::Ansi)
                }
                crate::args::Render::Ansi24 => {
                    draw::render_braille(lc, canvas, args, crate::args::Render::Ansi24)
                }
            }
        } else {
            draw::RenderResult::from("Error: Braille mode without luma canvas")
        }
    } else {
        match args.render {
            crate::args::Render::Irc => {
                draw::render_blocks(canvas, args, crate::args::Render::Irc, glyphs)
            }
            crate::args::Render::Ansi => {
                draw::render_blocks(canvas, args, crate::args::Render::Ansi, glyphs)
            }
            crate::args::Render::Ansi24 => {
                draw::render_blocks(canvas, args, crate::args::Render::Ansi24, glyphs)
            }
        }
    }
}

pub fn resolve_auto_font_size(
    args: &mut RenderArgs,
    image: &PhotonImage,
    _glyphs: &font::GlyphStore,
) {
    if (args.font_size - 0.0).abs() < f32::EPSILON {
        let font_ratio = 0.5;
        let img_w = image.get_width() as f32;
        let target_char_w = args.width.unwrap_or(80) as f32;

        let target_gw = img_w / target_char_w;
        let target_gh = target_gw / font_ratio;

        args.font_size = target_gh.round().clamp(6.0, 256.0);
        log::info!(
            "Auto-calculated font size: {} (based on ratio {:.3})",
            args.font_size,
            font_ratio
        );
    }
}

pub fn render_output(
    mut args: RenderArgs,
    glyph_store: &font::GlyphStore,
    image: PhotonImage,
    glyphs: &[draw::FastGlyph],
    abort_flag: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> draw::RenderResult {
    if let Some(flag) = &abort_flag {
        if flag.load(std::sync::atomic::Ordering::Relaxed) {
            return draw::RenderResult::from("Cancelled");
        }
    }

    resolve_auto_font_size(&mut args, &image, glyph_store);

    // If font_size was resolved/changed, we might need a new glyph_store and new FastGlyphs.
    // However, for simplicity in this pass, we'll use the provided ones if they match.
    // In TUI, this will be handled by the refresh loop.
    let (canvas, luma_canvas) = prepare_canvas(&args, glyph_store, image);
    render_from_canvas(&args, &canvas, luma_canvas.as_ref(), glyphs, abort_flag)
}

pub fn write_encoded_output(content: &str, encoding: &crate::args::Encoding) {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    let _ = write_encoded_to(content, encoding, &mut handle);
    let _ = handle.flush();
}

pub fn write_encoded_to<W: Write>(
    content: &str,
    encoding: &crate::args::Encoding,
    handle: &mut W,
) -> std::io::Result<()> {
    match encoding {
        crate::args::Encoding::Utf8 => {
            writeln!(handle, "{}", content)?;
        }
        crate::args::Encoding::Utf16 => {
            let bom = 0xFEFFu16;
            #[cfg(target_endian = "little")]
            {
                handle.write_all(&bom.to_le_bytes())?;
                write_encoded_to(content, &crate::args::Encoding::Utf16le, handle)?;
            }
            #[cfg(target_endian = "big")]
            {
                handle.write_all(&bom.to_be_bytes())?;
                write_encoded_to(content, &crate::args::Encoding::Utf16be, handle)?;
            }
        }
        crate::args::Encoding::Utf16be => {
            for c in content.encode_utf16() {
                handle.write_all(&c.to_be_bytes())?;
            }
            handle.write_all(&('\n' as u16).to_be_bytes())?;
        }
        crate::args::Encoding::Utf16le => {
            for c in content.encode_utf16() {
                handle.write_all(&c.to_le_bytes())?;
            }
            handle.write_all(&('\n' as u16).to_le_bytes())?;
        }
        crate::args::Encoding::Cesu8 => {
            for u in content.encode_utf16() {
                if u <= 0x7F {
                    handle.write_all(&[u as u8])?;
                } else if u <= 0x7FF {
                    handle.write_all(&[0xC0 | ((u >> 6) as u8), 0x80 | ((u & 0x3F) as u8)])?;
                } else {
                    handle.write_all(&[
                        0xE0 | ((u >> 12) as u8),
                        0x80 | (((u >> 6) & 0x3F) as u8),
                        0x80 | ((u & 0x3F) as u8),
                    ])?;
                }
            }
            handle.write_all(b"\n")?;
        }
    }
    Ok(())
}

pub async fn load_image_from_url_or_path(
    image: &str,
    args: &RenderArgs,
    glyphs: &font::GlyphStore,
) -> Result<PhotonImage, Box<dyn Error>> {
    let bytes = match Url::parse(image) {
        Ok(url) => {
            let response = reqwest::get(url).await?;
            response.bytes().await?.to_vec()
        }
        Err(_) => std::fs::read(image)?,
    };

    // Try detecting SVG
    let opt = resvg::usvg::Options::default();
    let mut fontdb = resvg::usvg::fontdb::Database::new();
    fontdb.load_system_fonts(); // Load system fonts to support text in SVGs

    if let Ok(tree) = resvg::usvg::Tree::from_data(&bytes, &opt, &fontdb) {
        let size = tree.size();
        let (w, h) = effects::calculate_dimensions(args, glyphs, size.width(), size.height());

        log::info!("Detected SVG. Rendering at {}x{}", w, h);

        let mut pixmap = tiny_skia::Pixmap::new(w, h).ok_or("Failed to create SVG pixmap")?;
        let render_ts =
            tiny_skia::Transform::from_scale(w as f32 / size.width(), h as f32 / size.height());

        resvg::render(&tree, render_ts, &mut pixmap.as_mut());

        // Convert RGBA buffer to PhotonImage
        // tiny-skia uses premultiplied alpha. We might need to demultiply if strict correctness is required, but for
        // terminal output, it's often negligible. However, let's demultiply to be safe for effects.
        let mut data = pixmap.data().to_vec();
        for chunk in data.chunks_mut(4) {
            let a = chunk[3];
            if a > 0 && a < 255 {
                let scale = 255.0 / a as f32;
                chunk[0] = (chunk[0] as f32 * scale).min(255.0) as u8;
                chunk[1] = (chunk[1] as f32 * scale).min(255.0) as u8;
                chunk[2] = (chunk[2] as f32 * scale).min(255.0) as u8;
            }
        }

        Ok(PhotonImage::new(data, w, h))
    } else {
        // Fallback to standard image loading
        let image_data = Cursor::new(bytes);
        match photon_rs::native::open_image_from_bytes(image_data.into_inner().as_ref()) {
            Ok(image) => Ok(image),
            Err(e) => Err(Box::new(e)),
        }
    }
}
