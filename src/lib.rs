pub const VERSION: &str = env!("IMG2IRC_VERSION");
pub const BUILD_NUMBER: &str = env!("BUILD_NUMBER");

pub mod adjustments;
pub mod args;
#[cfg(not(target_arch = "wasm32"))]
pub mod config;
pub mod contour_score;
pub mod draw;
pub mod effects;
#[cfg(any(feature = "ocr", feature = "web"))]
mod figlet_text;
pub mod font;
#[cfg(all(feature = "ocr", not(target_arch = "wasm32")))]
mod ocr_layout;
#[cfg(not(target_arch = "wasm32"))]
pub mod generator;
#[cfg(all(feature = "ocr", not(target_arch = "wasm32")))]
pub mod ocr;
#[cfg(any(not(feature = "ocr"), target_arch = "wasm32"))]
pub mod ocr {
    use crate::args::{RenderArgs, TextOverlay};
    use crate::font::GlyphStore;
    use photon_rs::PhotonImage;
    use std::error::Error;

    #[derive(Clone, Debug)]
    pub struct OcrDetection {
        pub text: String,
        pub confidence: f32,
        pub polygon: Vec<[f32; 2]>,
    }

    pub fn detect_text(
        _source: &PhotonImage,
        _args: &RenderArgs,
    ) -> Result<Vec<OcrDetection>, Box<dyn Error>> {
        Err("OCR support is not compiled in; rebuild with `--features ocr`".into())
    }

    pub fn valid_detections<'a>(
        detections: &'a [OcrDetection],
        _args: &'a RenderArgs,
    ) -> impl Iterator<Item = &'a OcrDetection> + 'a {
        detections.iter().filter(|_| false)
    }

    pub fn has_renderable_detections(
        _detections: &[OcrDetection],
        _args: &RenderArgs,
        _image_w: u32,
        _image_h: u32,
    ) -> bool {
        false
    }

    pub fn log_detections(_detections: &[OcrDetection], _args: &RenderArgs) {}

    pub fn choose_text_render_geometry(
        _args: &mut RenderArgs,
        _source: &PhotonImage,
        _glyphs: &GlyphStore,
        _detections: &[OcrDetection],
    ) {
    }

    pub fn choose_text_font_size(
        _args: &mut RenderArgs,
        _detections: &[OcrDetection],
        _image_h: u32,
    ) {
    }

    pub fn detections_to_overlays(
        _source: &PhotonImage,
        _background_source: &PhotonImage,
        _args: &RenderArgs,
        _glyphs: &GlyphStore,
        _detections: &[OcrDetection],
    ) -> Vec<TextOverlay> {
        Vec::new()
    }

    pub fn refresh_figlet_overlay(
        overlay: &mut TextOverlay,
        _font_lists: &[crate::args::OcrFigletFontList],
    ) -> bool {
        if let Some(source) = &overlay.source_text {
            overlay.text = source.clone();
            overlay.figlet_font = Some("plain".to_string());
        }
        false
    }

    pub fn remove_detected_text(
        source: &PhotonImage,
        _args: &RenderArgs,
        _detections: &[OcrDetection],
    ) -> PhotonImage {
        source.clone()
    }
}
pub mod palette;
pub mod pipeline;

#[cfg(not(target_arch = "wasm32"))]
pub mod colorpicker;
#[cfg(not(target_arch = "wasm32"))]
pub mod tui;
#[cfg(all(feature = "web", target_arch = "wasm32"))]
mod web;
#[cfg(all(feature = "python", not(target_arch = "wasm32")))]
mod python;

use crate::args::{Args, Param, RenderArgs};
use photon_rs::PhotonImage;
use std::io::Write;

#[cfg(not(target_arch = "wasm32"))]
use std::{error::Error, io::Cursor};
#[cfg(not(target_arch = "wasm32"))]
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
        clear_cache: args.clear_cache,
        auto_optimize_batch_size: args.auto_optimize_batch_size,

        image: args.image.clone(),
        width: args
            .width
            .as_ref()
            .map(|p| param_base(p) as u32)
            .filter(|width| *width > 0),
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
        score_fix: args.smooth,
        score_fix_candidates: args.smooth_candidates,
        score_fix_orderings: args.smooth_orders,
        score_fix_geometry_first: args.smooth_shapes,
        score_fix_neighborhood_guard: args.smooth_neighbors,
        contour_bending_weight: args.contour_bending_weight,
        contour_endpoint_weight: args.contour_endpoint_weight,
        contour_junction_weight: args.contour_junction_weight,
        contour_fragment_weight: args.contour_fragment_weight,
        contour_fidelity_weight: args.contour_fidelity_weight,
        contour_boundary_weight: args.contour_boundary_weight,
        contour_peak_weight: args.contour_peak_weight,
        misc3: args.misc3,
        misc4: args.misc4,
        misc5: args.misc5,
        misc6: args.misc6,
        misc7: args.misc7,
        misc8: args.misc8,
        save: args.save.clone(),
        fft_debug_dir: args.fft_debug_dir.clone(),
        ocr: cfg!(all(feature = "ocr", not(target_arch = "wasm32"))) && args.ocr,
        ocr_min_confidence: args.ocr_min_confidence,
        ocr_min_ascii_ratio: args.ocr_min_ascii_ratio,
        ocr_model_tier: args.ocr_model_tier,
        ocr_model_cache: args.ocr_model_cache.clone(),
        ocr_det_model: args.ocr_det_model.clone(),
        ocr_cls_model: args.ocr_cls_model.clone(),
        ocr_rec_model: args.ocr_rec_model.clone(),
        ocr_dict: args.ocr_dict.clone(),
        ocr_textline_orientation: args.ocr_textline_orientation,
        ocr_most_angle: args.ocr_most_angle,
        ocr_doc_orientation: args.ocr_doc_orientation,
        ocr_doc_orientation_model: args.ocr_doc_orientation_model.clone(),
        ocr_doc_unwarping: args.ocr_doc_unwarping,
        ocr_threads: args.ocr_threads,
        ocr_width: args.ocr_width,
        ocr_megapixels: args.ocr_megapixels,
        ocr_auto_width: args.ocr_auto_width.unwrap_or(false),
        ocr_max_side_len: args.ocr_max_side_len,
        ocr_max_text_height_ratio: args.ocr_max_text_height_ratio,
        ocr_figlet: args.ocr_figlet,
        ocr_figlet_min_height: args.ocr_figlet_min_height,
        ocr_figlet_min_height_ratio: args.ocr_figlet_min_height_ratio,
        ocr_figlet_fill: args.ocr_figlet_fill,
        ocr_figlet_max_width_ratio: args.ocr_figlet_max_width_ratio,
        ocr_figlet_max_height_ratio: args.ocr_figlet_max_height_ratio,
        ocr_figlet_fonts: if args.ocr_figlet_fonts.is_empty() {
            crate::args::default_ocr_figlet_font_lists()
        } else {
            args.ocr_figlet_fonts.clone()
        },
        ocr_box_score_threshold: args.ocr_box_score_threshold,
        ocr_box_threshold: args.ocr_box_threshold,
        ocr_unclip_ratio: args.ocr_unclip_ratio,
        ocr_lock: args.ocr_lock,
        ocr_debug_boxes: args.ocr_debug_boxes,

        min_width: args.min_width,
        max_width: args.max_width,

        score: args.score,
        pipeline: Vec::new(),
        overlays: Vec::new(),
        config_dir: args.config_dir.clone(),
        figlet_dir: args.figlet_dir.clone(),
    }
}

/// Build render arguments for the interactive TUI.
///
/// OCR auto-width is enabled when the option was not supplied, while an
/// explicit `--ocr-auto-width false` remains authoritative.
pub fn args_to_tui_render_args(args: &Args) -> RenderArgs {
    let mut render_args = args_to_render_args(args);
    render_args.ocr_auto_width = args.ocr_auto_width.unwrap_or(true);
    render_args
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

    #[cfg(not(target_arch = "wasm32"))]
    if let Some(path) = &args.save {
        let _ = photon_rs::native::save_image(image.clone(), path);
    }

    let canvas = draw::AnsiImage::new(
        image.clone(),
        glyphs.clone(),
        args.grayscale_tolerance,
        args.nograyscale,
    );

    let luma_canvas = if args.braille {
        let image_luma = effects::apply_luma_effects(args, glyphs, image);
        let canvas_luma = draw::AnsiImage::new(
            image_luma,
            glyphs.clone(),
            args.grayscale_tolerance,
            args.nograyscale,
        );
        Some(canvas_luma)
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
    abort_flag: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> draw::RenderResult {
    if let Some(flag) = &abort_flag {
        if flag.load(std::sync::atomic::Ordering::Relaxed) {
            return draw::RenderResult::from("Cancelled");
        }
    }

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
                draw::render_blocks(canvas, args, crate::args::Render::Irc, glyphs, abort_flag)
            }
            crate::args::Render::Ansi => {
                draw::render_blocks(canvas, args, crate::args::Render::Ansi, glyphs, abort_flag)
            }
            crate::args::Render::Ansi24 => draw::render_blocks(
                canvas,
                args,
                crate::args::Render::Ansi24,
                glyphs,
                abort_flag,
            ),
        }
    }
}

pub fn resolve_auto_font_size(
    args: &mut RenderArgs,
    image: &PhotonImage,
    glyphs: &font::GlyphStore,
) {
    if (args.font_size - 0.0).abs() < f32::EPSILON {
        let img_w = image.get_width() as f32;
        let target_char_w = args.width.unwrap_or(80) as f32;

        let target_gw = img_w / target_char_w;
        let (w, h) = glyphs.float_metrics;

        let target_gh = if w > 1.0 && h > 1.0 {
            target_gw * h / w
        } else {
            target_gw / 0.5
        };

        args.font_size = target_gh.round().clamp(6.0, 256.0);
        log::info!(
            "Auto-calculated font size: {} (based on loaded metrics width {} and height {})",
            args.font_size,
            w,
            h
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
    let prepare_started = draw::RenderInstant::now();
    let (canvas, luma_canvas) = prepare_canvas(&args, glyph_store, image);
    let prepare_time = prepare_started.elapsed();
    let mut result = render_from_canvas(&args, &canvas, luma_canvas.as_ref(), glyphs, abort_flag);
    result.profile.prepare_canvas = prepare_time;
    log::info!("render profile: {}", result.profile.summary());
    result
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

#[cfg(not(target_arch = "wasm32"))]
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

    let is_svg_potential = bytes.starts_with(b"<svg")
        || bytes.starts_with(b"<?xml")
        || (bytes.len() > 10
            && std::str::from_utf8(&bytes[0..10.min(bytes.len())])
                .map_or(false, |s| s.trim_start().starts_with('<')));

    if is_svg_potential {
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

            return Ok(PhotonImage::new(data, w, h));
        }
    }

    // Fallback to standard image loading
    let image_data = Cursor::new(bytes);
    match photon_rs::native::open_image_from_bytes(image_data.into_inner().as_ref()) {
        Ok(image) => Ok(image),
        Err(e) => Err(Box::new(e)),
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    #[test]
    fn zero_cli_width_means_auto() {
        let args = crate::args::Args::parse_from(["img2irc", "--width", "0"]);
        assert_eq!(crate::args_to_render_args(&args).width, None);
    }

    #[test]
    fn cli_and_tui_use_distinct_ocr_auto_width_defaults() {
        let unspecified = crate::args::Args::parse_from(["img2irc"]);
        assert_eq!(unspecified.ocr_auto_width, None);
        assert!(!crate::args_to_render_args(&unspecified).ocr_auto_width);
        assert!(crate::args_to_tui_render_args(&unspecified).ocr_auto_width);

        let enabled = crate::args::Args::parse_from(["img2irc", "--ocr-auto-width"]);
        assert_eq!(enabled.ocr_auto_width, Some(true));
        assert!(crate::args_to_render_args(&enabled).ocr_auto_width);
        assert!(crate::args_to_tui_render_args(&enabled).ocr_auto_width);

        let disabled =
            crate::args::Args::parse_from(["img2irc", "--ocr-auto-width", "false"]);
        assert_eq!(disabled.ocr_auto_width, Some(false));
        assert!(!crate::args_to_render_args(&disabled).ocr_auto_width);
        assert!(!crate::args_to_tui_render_args(&disabled).ocr_auto_width);
    }

    #[test]
    fn bare_boolean_option_does_not_consume_the_image_path() {
        let args = crate::args::parse_args_from([
            "img2irc",
            "--ocr-auto-width",
            "example.png",
        ]);
        assert_eq!(args.ocr_auto_width, Some(true));
        assert_eq!(args.image.as_deref(), Some("example.png"));

        let args = crate::args::parse_args_from([
            "img2irc",
            "--ocr-auto-width",
            "false",
            "example.png",
        ]);
        assert_eq!(args.ocr_auto_width, Some(false));
        assert_eq!(args.image.as_deref(), Some("example.png"));
    }

    #[test]
    fn value_taking_boolean_options_accept_bare_flags() {
        let enabled = crate::args::Args::parse_from([
            "img2irc",
            "--smooth-neighbors",
            "--ocr-auto-width",
            "--ocr-textline-orientation",
            "--ocr-figlet",
        ]);
        assert!(enabled.smooth_neighbors);
        assert_eq!(enabled.ocr_auto_width, Some(true));
        assert!(enabled.ocr_textline_orientation);
        assert!(enabled.ocr_figlet);

        let disabled = crate::args::Args::parse_from([
            "img2irc",
            "--smooth-neighbors",
            "false",
            "--ocr-auto-width",
            "false",
            "--ocr-textline-orientation",
            "false",
            "--ocr-figlet",
            "false",
        ]);
        assert!(!disabled.smooth_neighbors);
        assert_eq!(disabled.ocr_auto_width, Some(false));
        assert!(!disabled.ocr_textline_orientation);
        assert!(!disabled.ocr_figlet);
    }

    #[test]
    fn cli_accepts_figlet_font_lists_by_line_height() {
        let args = crate::args::Args::parse_from([
            "img2irc",
            "--ocr-figlet-fonts",
            "1=plain",
            "--ocr-figlet-fonts",
            "2=phm-minecraft,phm-lcdmatrix",
        ]);
        let lists = crate::args_to_render_args(&args).ocr_figlet_fonts;

        assert_eq!(lists.len(), 2);
        assert_eq!(lists[0].height, 1);
        assert_eq!(lists[1].fonts, ["phm-minecraft", "phm-lcdmatrix"]);
    }

    #[test]
    fn cli_controls_when_figlet_is_allowed_for_ocr() {
        let args = crate::args::Args::parse_from([
            "img2irc",
            "--ocr-figlet",
            "false",
            "--ocr-figlet-min-height",
            "3.5",
            "--ocr-figlet-min-height-ratio",
            "1.75",
        ]);
        let render_args = crate::args_to_render_args(&args);

        assert!(!render_args.ocr_figlet);
        assert_eq!(render_args.ocr_figlet_min_height, 3.5);
        assert_eq!(render_args.ocr_figlet_min_height_ratio, 1.75);
    }

    #[test]
    fn cli_uses_glyph_print_names_and_removes_obsolete_switches() {
        let args = crate::args::Args::parse_from([
            "img2irc",
            "--print-glyph-chars",
            "--print-glyph-bitmaps",
        ]);
        assert!(args.print_glyph_chars);
        assert!(args.print_glyph_bitmaps);

        for obsolete in [
            "--list-blocks",
            "--list-ranges",
            "--print-ranges",
            "--show-block-bitmaps",
            "--gpu",
        ] {
            assert!(crate::args::Args::try_parse_from(["img2irc", obsolete]).is_err());
        }
    }

    #[test]
    fn cli_uses_short_smoothing_option_names() {
        let args = crate::args::Args::parse_from([
            "img2irc",
            "--smooth",
            "--smooth-candidates",
            "24",
            "--smooth-orders",
            "3",
            "--smooth-shapes",
            "--smooth-neighbors",
            "false",
        ]);

        assert!(args.smooth);
        assert_eq!(args.smooth_candidates, 24);
        assert_eq!(args.smooth_orders, 3);
        assert!(args.smooth_shapes);
        assert!(!args.smooth_neighbors);
    }

    #[tokio::test]
    async fn test_consecutive_renders_performance() {
        let img_path = "inputs/lisa.png";
        if !std::path::Path::new(img_path).exists() {
            return;
        }
        let font_path = "/usr/share/fonts/OTF/CascadiaMono-Regular.otf";
        if !std::path::Path::new(font_path).exists() {
            return;
        }

        let args = crate::args::Args::parse_from(&[
            "img2irc",
            "--width",
            "40",
            "--font-size",
            "13",
            "inputs/lisa.png",
        ]);
        let render_args = crate::args_to_render_args(&args);

        let initial_glyph_store = crate::font::glyph_store(
            &render_args.blocks,
            &render_args.exclude_range,
            &render_args.exclude,
            &render_args.include_range,
            render_args.include.as_ref(),
            &render_args.font,
            render_args.font_size,
            false,
            None,
        );

        let image =
            crate::load_image_from_url_or_path(img_path, &render_args, &initial_glyph_store)
                .await
                .unwrap();

        let glyphs = crate::draw::prepare_glyphs(&initial_glyph_store, &render_args);

        // Run 1 (Cold)
        let start1 = std::time::Instant::now();
        let _res1 = crate::render_output(
            render_args.clone(),
            &initial_glyph_store,
            image.clone(),
            &glyphs,
            None,
        );
        let duration1 = start1.elapsed();
        println!("Cold render took: {:?}", duration1);

        // Run 2 (Warm)
        let start2 = std::time::Instant::now();
        let _res2 = crate::render_output(
            render_args.clone(),
            &initial_glyph_store,
            image.clone(),
            &glyphs,
            None,
        );
        let duration2 = start2.elapsed();
        println!("Warm render took: {:?}", duration2);

        // Run 3 (Warm, different contrast)
        let mut render_args_contrast = render_args.clone();
        render_args_contrast.contrast = 0.5;
        let start3 = std::time::Instant::now();
        let _res3 = crate::render_output(
            render_args_contrast,
            &initial_glyph_store,
            image.clone(),
            &glyphs,
            None,
        );
        let duration3 = start3.elapsed();
        println!("Warm render (diff contrast) took: {:?}", duration3);

        // Run 4 (Warm, different crop)
        let mut render_args_crop = render_args.clone();
        render_args_crop.trim = Some((2, 2, 2, 2));
        let start4 = std::time::Instant::now();
        let _res4 = crate::render_output(
            render_args_crop,
            &initial_glyph_store,
            image.clone(),
            &glyphs,
            None,
        );
        let duration4 = start4.elapsed();
        println!("Warm render (diff crop) took: {:?}", duration4);
    }

    #[test]
    fn test_version_contains_build_number() {
        assert!(!crate::VERSION.is_empty());
        assert!(!crate::BUILD_NUMBER.is_empty());
        let build_num: u64 = crate::BUILD_NUMBER
            .parse()
            .expect("BUILD_NUMBER must be a valid u64");
        assert!(build_num > 0);
        assert!(crate::VERSION.contains(&format!("+{}", build_num)));
    }

    #[test]
    fn test_cli_config_dir_and_figlet_dir_args() {
        use clap::Parser;
        let args = crate::args::Args::parse_from([
            "img2irc",
            "--config-dir",
            "/tmp/custom_config",
            "--figlet-dir",
            "/tmp/custom_fonts",
        ]);
        assert_eq!(
            args.config_dir.as_deref(),
            Some(std::path::Path::new("/tmp/custom_config"))
        );
        assert_eq!(
            args.figlet_dir.as_deref(),
            Some(std::path::Path::new("/tmp/custom_fonts"))
        );

        let render_args = crate::args_to_render_args(&args);
        assert_eq!(
            render_args.config_dir.as_deref(),
            Some(std::path::Path::new("/tmp/custom_config"))
        );
        assert_eq!(
            render_args.figlet_dir.as_deref(),
            Some(std::path::Path::new("/tmp/custom_fonts"))
        );
    }
}
