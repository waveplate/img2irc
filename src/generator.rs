//! Reusable, non-interactive image generation API.
//!
//! This module owns the orchestration which embedding APIs need around the
//! lower-level glyph, OCR, effects, and drawing modules.

use crate::args::{Args, RenderArgs};
use crate::{draw, effects, font};
use clap::Parser;
use photon_rs::PhotonImage;
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

/// Apply a partial JSON object to the normal generation defaults.
pub fn options_from_json(options_json: &str) -> Result<RenderArgs, String> {
    let mut defaults =
        serde_json::to_value(default_options()).map_err(|error| error.to_string())?;
    let patch: serde_json::Value = serde_json::from_str(options_json)
        .map_err(|error| format!("invalid options JSON: {error}"))?;
    let patch_object = patch
        .as_object()
        .ok_or_else(|| "generation options must be a JSON object".to_string())?;
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
    mut options: RenderArgs,
    font_data: Option<&[u8]>,
) -> Result<GeneratedOutput, String> {
    validate_options(&options)?;
    let initial_font_size = if options.font_size == 0.0 {
        24.0
    } else {
        options.font_size
    };
    let initial_glyph_store = glyph_store(&options, initial_font_size, font_data)?;

    let ocr_detections = if options.ocr {
        let ocr_image = match options.ocr_width {
            Some(width) if width > 0 && width != loaded_image.get_width() => {
                let height = ((width as f32 * loaded_image.get_height() as f32)
                    / loaded_image.get_width() as f32)
                    .round()
                    .max(1.0) as u32;
                photon_rs::transform::resize(
                    &loaded_image,
                    width,
                    height,
                    photon_rs::transform::SamplingFilter::Lanczos3,
                )
            }
            _ => loaded_image.clone(),
        };
        let original_width = loaded_image.get_width() as f32;
        let original_height = loaded_image.get_height() as f32;
        let ocr_width = ocr_image.get_width() as f32;
        let ocr_height = ocr_image.get_height() as f32;
        let detections = crate::ocr::detect_text(&ocr_image, &options)
            .map_err(|error| format!("OCR text detection failed: {error}"))?
            .into_iter()
            .map(|mut detection| {
                if (ocr_width - original_width).abs() > 0.5
                    || (ocr_height - original_height).abs() > 0.5
                {
                    for point in &mut detection.polygon {
                        point[0] = point[0] * original_width / ocr_width;
                        point[1] = point[1] * original_height / ocr_height;
                    }
                }
                detection
            })
            .collect::<Vec<_>>();
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

    if options.font_size == 0.0 {
        crate::resolve_auto_font_size(&mut options, &loaded_image, &initial_glyph_store);
    }
    let glyph_store = glyph_store(&options, options.font_size, font_data)?;

    if let Some(detections) = &ocr_detections {
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
    output_options.pipeline.clear();
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
}
