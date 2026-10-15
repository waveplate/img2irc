//! Reusable, non-interactive image generation API.
//!
//! This module owns the orchestration which embedding APIs need around the
//! lower-level glyph, OCR, effects, and drawing modules.

use crate::args::{Args, RenderArgs};
use crate::{draw, effects, font};
use clap::{CommandFactory, Parser};
use photon_rs::PhotonImage;
use std::collections::BTreeMap;
use std::io::Cursor;

const MAX_INPUT_PIXELS: u64 = 256 * 1024 * 1024;

pub struct GeneratedOutput {
    pub result: draw::RenderResult,
    pub glyph_store: font::GlyphStore,
    pub options: RenderArgs,
    /// OCR detections which were accepted for text replacement during this
    /// render. Empty when OCR is disabled or found no renderable text.
    pub ocr_detections: Vec<crate::ocr::OcrDetection>,
}

/// Return the same resolved defaults as the CLI, with its normal default glyph
/// set selected explicitly so byte-backed fonts work without a config file.
pub fn default_options() -> RenderArgs {
    let mut options = crate::args_to_render_args(&Args::parse_from(["img2irc"]));
    if options.blocks.is_empty() {
        options.blocks.push("default".to_string());
    }
    options
}

/// Python spellings of CLI render settings, derived from clap so long-option
/// aliases do not need a second, independently maintained catalog.
pub fn render_option_aliases() -> BTreeMap<String, String> {
    let defaults = serde_json::to_value(default_options()).expect("serializable render defaults");
    let mut aliases = BTreeMap::new();
    for arg in Args::command().get_arguments() {
        let id = arg.get_id().as_str();
        let target = match id {
            "blocks" => "glyphs",
            _ => id,
        };
        let component = matches!(
            id,
            "scalex" | "scaley" | "crop_x1" | "crop_y1" | "crop_x2" | "crop_y2"
        );
        if defaults.get(target).is_none() && !component {
            continue;
        }
        aliases.insert(id.to_string(), target.to_string());
        if let Some(names) = arg.get_long_and_visible_aliases() {
            for name in names {
                aliases.insert(name.replace('-', "_"), target.to_string());
            }
        }
        // Include compatibility aliases which clap hides from --help as well.
        if let Some(names) = arg.get_all_aliases() {
            for name in names {
                aliases.insert(name.replace('-', "_"), target.to_string());
            }
        }
    }
    aliases
}

/// Apply a partial JSON object to the normal generation defaults.
pub fn options_from_json(options_json: &str) -> Result<RenderArgs, String> {
    let mut defaults =
        serde_json::to_value(default_options()).map_err(|error| error.to_string())?;
    let mut patch: serde_json::Value = serde_json::from_str(options_json)
        .map_err(|error| format!("invalid options JSON: {error}"))?;
    let patch_object = patch
        .as_object_mut()
        .ok_or_else(|| "generation options must be a JSON object".to_string())?;
    if let Some(blocks) = patch_object.remove("blocks") {
        if patch_object.contains_key("glyphs") {
            return Err("specify glyphs or its deprecated blocks alias, not both".into());
        }
        patch_object.insert("glyphs".into(), blocks);
    }
    if let Some(value) = patch_object.get_mut("glyphs") {
        if let Some(name) = value.as_str() {
            *value = serde_json::json!([name]);
        }
    }
    let known = defaults
        .as_object()
        .expect("serialized RenderArgs is an object");
    if let Some(key) = patch_object.keys().find(|key| !known.contains_key(*key)) {
        return Err(format!("unknown generation option {key:?}"));
    }
    merge_json(&mut defaults, patch);
    let mut options: RenderArgs = serde_json::from_value(defaults)
        .map_err(|error| format!("invalid generation options: {error}"))?;
    options.image = None;
    options.save = None;
    options.fft_debug_dir = None;
    validate_options(&options)?;
    Ok(options)
}

fn merge_json(base: &mut serde_json::Value, patch: serde_json::Value) {
    match (base, patch) {
        (serde_json::Value::Object(base), serde_json::Value::Object(patch)) => {
            for (key, value) in patch {
                match base.get_mut(&key) {
                    Some(base_value) => merge_json(base_value, value),
                    None => {
                        base.insert(key, value);
                    }
                }
            }
        }
        (base, patch) => *base = patch,
    }
}

fn validate_options(options: &RenderArgs) -> Result<(), String> {
    if options.width == Some(0) || options.height == Some(0) {
        return Err("width and height must be greater than zero when supplied".to_string());
    }
    if !options.font_size.is_finite() || options.font_size < 0.0 || options.font_size > 256.0 {
        return Err("font_size must be 0 (automatic) or between 6 and 256".to_string());
    }
    if options.font_size > 0.0 && options.font_size < 6.0 {
        return Err("font_size must be 0 (automatic) or between 6 and 256".to_string());
    }
    let (scale_x, scale_y) = options.scale.unwrap_or((1.0, 1.0));
    if !scale_x.is_finite() || !scale_y.is_finite() || scale_x <= 0.0 || scale_y <= 0.0 {
        return Err("scale values must be positive finite numbers".to_string());
    }
    if options.ocr_threads == 0 {
        return Err("ocr_threads must be greater than zero".to_string());
    }
    if !(0..=10).contains(&options.median_blur) {
        return Err("median_blur must be between 0 and 10".to_string());
    }
    if options
        .ocr_megapixels
        .is_some_and(|value| !value.is_finite() || !(0.1..=16.0).contains(&value))
    {
        return Err("ocr_megapixels must be between 0.1 and 16".to_string());
    }
    if options.ocr_figlet_fonts.iter().any(|list| {
        list.height == 0
            || list.fonts.is_empty()
            || list.fonts.iter().any(|font| font.trim().is_empty())
    }) {
        return Err(
            "ocr_figlet_fonts requires positive line heights and non-empty font names".to_string(),
        );
    }
    if !(1..=5).contains(&options.smooth_orders) {
        return Err("smooth_orders must be between 1 and 5".to_string());
    }
    if options.superpixel_warp > crate::superpixels::MAX_STRENGTH {
        return Err("superpixel_warp must be between 0 and 100".to_string());
    }
    for effect in &options.pipeline {
        if let crate::pipeline::ImageEffect::QuantizePalette { colours, threshold, palette, .. } = effect {
            if *colours > 256 || !threshold.is_finite() || !(0.0..=100.0).contains(threshold) || palette.len() > 256 {
                return Err("quantization requires 0–256 colours, a threshold between 0 and 100, and at most 256 palette entries".into());
            }
        }
    }
    Ok(())
}

fn glyph_store(
    options: &RenderArgs,
    font_size: f32,
    font_data: Option<&[u8]>,
) -> Result<font::GlyphStore, String> {
    if let Some(font_data) = font_data {
        font::glyph_store_from_font_bytes(
            font_data,
            &options.blocks,
            &options.exclude_range,
            &options.exclude,
            &options.include_range,
            options.include.as_ref(),
            font_size,
            false,
        )
    } else {
        Ok(font::glyph_store(
            &options.blocks,
            &options.exclude_range,
            &options.exclude,
            &options.include_range,
            options.include.as_ref(),
            &options.font,
            font_size,
            false,
            None,
        ))
    }
}

pub fn decode_image(image_data: &[u8]) -> Result<PhotonImage, String> {
    let decoded = image::load_from_memory(image_data)
        .map_err(|error| format!("could not decode image: {error}"))?;
    let pixel_count = u64::from(decoded.width()) * u64::from(decoded.height());
    if pixel_count > MAX_INPUT_PIXELS {
        return Err(format!(
            "image has {pixel_count} pixels; the limit is {MAX_INPUT_PIXELS}"
        ));
    }
    let width = decoded.width();
    let height = decoded.height();
    Ok(PhotonImage::new(
        decoded.into_rgba8().into_raw(),
        width,
        height,
    ))
}

/// Generate encoded terminal output from already-encoded image bytes.
///
/// Supplying `font_data` makes generation independent of installed fonts.
/// Passing `None` enables the same system-font selection as the CLI/TUI.
pub fn generate(
    image_data: &[u8],
    options: RenderArgs,
    font_data: Option<&[u8]>,
) -> Result<GeneratedOutput, String> {
    validate_options(&options)?;
    let image = decode_image(image_data)?;
    generate_image(image, options, font_data)
}

pub fn generate_image(
    loaded_image: PhotonImage,
    options: RenderArgs,
    font_data: Option<&[u8]>,
) -> Result<GeneratedOutput, String> {
    generate_image_with_detector(loaded_image, options, font_data, |image, options| {
        crate::ocr::detect_text(image, options)
            .map_err(|error| format!("OCR text detection failed: {error}"))
    })
}

fn generate_image_with_detector(
    loaded_image: PhotonImage,
    mut options: RenderArgs,
    font_data: Option<&[u8]>,
    detect: impl FnOnce(&PhotonImage, &RenderArgs) -> Result<Vec<crate::ocr::OcrDetection>, String>,
) -> Result<GeneratedOutput, String> {
    validate_options(&options)?;
    let initial_font_size = if options.font_size == 0.0 {
        24.0
    } else {
        options.font_size
    };
    let initial_glyph_store = glyph_store(&options, initial_font_size, font_data)?;

    let ocr_detections = if options.ocr {
        // The OCR backend owns detection sizing and returns source coordinates,
        // just as it does for the CLI and standalone Python detect_text().
        let detections = detect(&loaded_image, &options)?;
        crate::ocr::log_detections(&detections, &options);
        if crate::ocr::has_renderable_detections(
            &detections,
            &options,
            loaded_image.get_width(),
            loaded_image.get_height(),
        ) {
            Some(detections)
        } else {
            options.ocr = false;
            None
        }
    } else {
        None
    };

    let base_image = ocr_detections.as_ref().map_or_else(
        || loaded_image.clone(),
        |detections| crate::ocr::remove_detected_text(&loaded_image, &options, detections),
    );

    if options.ocr && options.ocr_auto_width {
        options.width = None;
        options.height = None;
    }
    if options.font_size == 0.0 {
        crate::resolve_auto_font_size(&mut options, &loaded_image, &initial_glyph_store);
    }
    let glyph_store = glyph_store(&options, options.font_size, font_data)?;

    if let Some(detections) = &ocr_detections {
        crate::ocr::choose_text_render_geometry(
            &mut options,
            &loaded_image,
            &glyph_store,
            detections,
        );
        let overlays = crate::ocr::detections_to_overlays(
            &loaded_image,
            &base_image,
            &options,
            &glyph_store,
            detections,
        );
        options.overlays.extend(overlays);
    }

    let render_image = effects::apply_effects(&options, &glyph_store, base_image);
    let fast_glyphs = draw::prepare_glyphs(&glyph_store, &options);
    if fast_glyphs.is_empty() && !options.braille {
        return Err("the selected characters produced no usable glyphs".to_string());
    }

    // The source effects have already been applied. Clear the legacy fields
    // which the CLI and TUI clear before passing the transformed image to the
    // terminal renderer.
    let mut output_options = options.clone();
    output_options
        .pipeline
        .retain(crate::pipeline::ImageEffect::is_output_colour);
    output_options.rotate = 0.0;
    output_options.fliph = false;
    output_options.flipv = false;
    output_options.scale = None;
    output_options.brightness = 0.0;
    output_options.contrast = 0.0;
    output_options.gamma = 0.0;
    output_options.saturation = 0.0;
    output_options.hue = 0.0;
    output_options.invert = false;
    output_options.dither = 0;
    output_options.grayscale = false;
    output_options.nograyscale = false;
    output_options.pixelize = 0;
    output_options.box_blur = false;
    output_options.gaussian_blur = 0;
    output_options.median_blur = 0;

    let result = crate::render_output(
        output_options,
        &glyph_store,
        render_image,
        &fast_glyphs,
        None,
    );
    Ok(GeneratedOutput {
        result,
        glyph_store,
        options,
        ocr_detections: ocr_detections.unwrap_or_default(),
    })
}

pub fn preview_rgba(output: &GeneratedOutput) -> Option<(u32, u32, Vec<u8>)> {
    draw::render_result_rgba(&output.result, &output.glyph_store)
}

pub fn preview_png(output: &GeneratedOutput) -> Result<Vec<u8>, String> {
    let image = draw::render_result_image(&output.result, &output.glyph_store)
        .ok_or_else(|| "render did not produce a preview grid".to_string())?;
    let mut cursor = Cursor::new(Vec::new());
    image
        .write_to(&mut cursor, image::ImageFormat::Png)
        .map_err(|error| format!("could not encode PNG preview: {error}"))?;
    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_options_merge_with_cli_defaults() {
        let options = options_from_json(r#"{"width":42,"render":"Ansi24"}"#).unwrap();
        assert_eq!(options.width, Some(42));
        assert_eq!(options.render, crate::args::Render::Ansi24);
        assert_eq!(options.blocks, ["default"]);
    }

    #[test]
    fn unknown_options_are_rejected() {
        assert!(options_from_json(r#"{"widht":42}"#)
            .unwrap_err()
            .contains("unknown generation option"));
    }

    #[test]
    fn median_blur_setting_matches_cli_and_validates_radius() {
        for flag in ["--median-blur", "--medianblur"] {
            let cli = Args::try_parse_from(["img2irc", flag, "2"]).unwrap();
            let options = options_from_json(r#"{"median_blur":2}"#).unwrap();
            assert_eq!(
                options.median_blur,
                crate::args_to_render_args(&cli).median_blur
            );
        }
        assert_eq!(default_options().median_blur, 0);
        for radius in [-1, 11] {
            assert!(
                Args::try_parse_from(["img2irc", "--median-blur", &radius.to_string()]).is_err()
            );
            assert!(options_from_json(&format!(r#"{{"median_blur":{radius}}}"#)).is_err());
        }
    }

    #[test]
    fn cli_render_settings_all_have_python_names() {
        // These flags run CLI workflows instead of configuring a render.
        let workflows = [
            "font_licenses",
            "print_glyph_chars",
            "print_glyph_bitmaps",
            "tui",
            "image",
            "profile",
            "save",
        ];
        let aliases = render_option_aliases();
        for arg in Args::command().get_arguments() {
            let id = arg.get_id().as_str();
            if workflows.contains(&id) {
                continue;
            }
            assert!(
                aliases.contains_key(id),
                "CLI setting {id} is missing from Python"
            );
            if let Some(long) = arg.get_long() {
                assert!(
                    aliases.contains_key(&long.replace('-', "_")),
                    "CLI flag --{long} is missing from Python"
                );
            }
        }
        assert_eq!(aliases["smooth"], "smooth");
        assert_eq!(aliases["smooth_orders"], "smooth_orders");
        assert_eq!(aliases["boxblur"], "box_blur");
        assert_eq!(aliases["config_path"], "config_dir");
    }

    #[test]
    fn ocr_json_options_match_cli_values() {
        let cli = Args::parse_from([
            "img2irc",
            "--ocr-model-tier",
            "small",
            "--ocr-threads",
            "2",
            "--ocr-auto-width=false",
            "--ocr-megapixels",
            "2.5",
            "--ocr-min-confidence",
            "0.8",
            "--ocr-min-ascii-ratio",
            "0.9",
            "--ocr-box-threshold",
            "0.3",
            "--ocr-box-score-threshold",
            "0.7",
            "--ocr-unclip-ratio",
            "1.8",
            "--ocr-figlet=false",
            "--ocr-figlet-fonts",
            "2=phm-c64,phm-cga",
        ]);
        let options = options_from_json(
            r#"{
            "ocr_model_tier":"Small", "ocr_threads":2,
            "ocr_auto_width":false, "ocr_megapixels":2.5,
            "ocr_min_confidence":0.8, "ocr_min_ascii_ratio":0.9,
            "ocr_box_threshold":0.3, "ocr_box_score_threshold":0.7,
            "ocr_unclip_ratio":1.8, "ocr_figlet":false,
            "ocr_figlet_fonts":[{"height":2,"fonts":["phm-c64","phm-cga"]}]
        }"#,
        )
        .unwrap();
        let cli = serde_json::to_value(crate::args_to_render_args(&cli)).unwrap();
        let options = serde_json::to_value(options).unwrap();
        for (key, value) in cli.as_object().unwrap() {
            if key.starts_with("ocr") {
                assert_eq!(&options[key], value, "OCR setting {key} differs from CLI");
            }
        }
    }

    #[test]
    fn ocr_megapixel_validation_matches_cli() {
        for value in [0.0, 0.09, 16.01, -1.0] {
            assert!(options_from_json(&format!(r#"{{"ocr_megapixels":{value}}}"#)).is_err());
        }
        for value in [0.1, 1.5, 16.0] {
            assert!(options_from_json(&format!(r#"{{"ocr_megapixels":{value}}}"#)).is_ok());
        }
    }

    fn test_render_options() -> RenderArgs {
        let mut options = default_options();
        options.ocr = true;
        options.blocks.clear();
        options.include = Some(" HELLOX".into());
        options.font_size = 24.0;
        options.width = Some(4);
        options.height = Some(2);
        options.render = crate::args::Render::Ansi24;
        options
    }

    #[test]
    fn generator_passes_source_and_resolution_settings_to_detector() {
        let mut options = test_render_options();
        options.ocr_width = Some(1000);
        options.ocr_megapixels = Some(0.5);
        let image = PhotonImage::new(vec![255; 20 * 10 * 4], 20, 10);
        let output = generate_image_with_detector(
            image,
            options,
            Some(include_bytes!("../static/CascadiaCode-Regular.ttf")),
            |source, settings| {
                assert_eq!((source.get_width(), source.get_height()), (20, 10));
                assert_eq!(settings.ocr_width, Some(1000));
                assert_eq!(settings.ocr_megapixels, Some(0.5));
                Ok(Vec::new())
            },
        )
        .unwrap();
        assert!(!output.options.ocr);
        assert_eq!(output.result.grid[0].len(), 4);
        assert_eq!(output.result.grid.len(), 2);
    }

    #[cfg(feature = "ocr")]
    #[test]
    fn generator_ocr_auto_width_fits_text_and_obeys_width_limit() {
        let font = include_bytes!("../static/CascadiaCode-Regular.ttf");
        let detection = crate::ocr::OcrDetection {
            text: "HELLO".into(),
            confidence: 1.0,
            polygon: vec![[10.0, 10.0], [20.0, 10.0], [20.0, 20.0], [10.0, 20.0]],
        };
        for (auto_width, max_width, fits) in
            [(false, 32, false), (true, 64, true), (true, 32, false)]
        {
            let mut options = test_render_options();
            options.ocr_auto_width = auto_width;
            options.ocr_figlet = false;
            options.max_width = Some(max_width);
            let image = PhotonImage::new(vec![255; 100 * 40 * 4], 100, 40);
            let output = generate_image_with_detector(image, options, Some(font), |_, _| {
                Ok(vec![detection.clone()])
            })
            .unwrap();
            assert_eq!(output.ocr_detections.len(), 1);
            if auto_width {
                assert_eq!(!output.options.overlays.is_empty(), fits);
                assert!(output.result.grid[0].len() > 4);
                assert!(output.result.grid[0].len() <= max_width as usize);
                assert_eq!(output.options.height, None);
            } else {
                assert_eq!(output.result.grid[0].len(), 4);
                assert_eq!(output.result.grid.len(), 2);
            }
        }
    }

    #[test]
    fn glyphs_and_legacy_blocks_accept_strings_or_lists() {
        for json in [
            r#"{"glyphs":"smooth"}"#,
            r#"{"glyphs":["smooth"]}"#,
            r#"{"blocks":"smooth"}"#,
            r#"{"blocks":["smooth"]}"#,
        ] {
            let options = options_from_json(json).unwrap();
            assert_eq!(options.blocks, ["smooth"]);
            let encoded = serde_json::to_value(options).unwrap();
            assert_eq!(encoded["glyphs"], serde_json::json!(["smooth"]));
            assert!(encoded.get("blocks").is_none());
        }
        assert!(options_from_json(r#"{"glyphs":"smooth","blocks":"default"}"#).is_err());
    }
}
