use crate::generator;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;

const EMBEDDED_FONT: &[u8] = include_bytes!("../static/CascadiaCode-Regular.ttf");

#[derive(Serialize)]
struct PythonCell {
    character: char,
    foreground: [u8; 3],
    background: Option<[u8; 3]>,
    inverted: bool,
    bold: bool,
    italic: bool,
    underline: bool,
}

#[derive(Serialize)]
struct PythonTimings {
    prepare_canvas_ms: f64,
    glyph_match_ms: f64,
    smoothing_prepare_ms: f64,
    smoothing_search_ms: f64,
    shape_refine_ms: f64,
    final_score_ms: f64,
    encode_ms: f64,
    contour_cache_hits: u64,
    contour_cache_misses: u64,
}

#[derive(Serialize)]
struct PythonOcrDetection {
    text: String,
    confidence: f32,
    polygon: Vec<[f32; 2]>,
}

#[derive(Serialize)]
struct PythonResult {
    content: String,
    save_content: Option<String>,
    columns: usize,
    rows: usize,
    cells: Option<Vec<Vec<PythonCell>>>,
    error_count: u64,
    total_pixels: u64,
    dynamic_scaling_count: u64,
    score: f64,
    longest_line_bytes: usize,
    preview_width: u32,
    preview_height: u32,
    font_size: f32,
    cell_width: usize,
    cell_height: usize,
    cell_advance: f32,
    line_height: f32,
    timings: PythonTimings,
    ocr_detections: Vec<PythonOcrDetection>,
    options: crate::args::RenderArgs,
}

#[derive(Serialize)]
struct PythonFontInfo {
    glyph_count: usize,
    selected_count: usize,
    cell_width: usize,
    cell_height: usize,
    cell_advance: f32,
    line_height: f32,
    font_names: Vec<String>,
    groups: BTreeMap<String, String>,
    characters: String,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct PythonContourWeights {
    bending: f64,
    endpoints: f64,
    junctions: f64,
    fragments: f64,
    fidelity: f64,
    boundary_balance: f64,
    peak_sensitivity: f64,
}

impl Default for PythonContourWeights {
    fn default() -> Self {
        let weights = crate::contour_score::ContourScoreWeights::default();
        Self {
            bending: weights.bending,
            endpoints: weights.endpoints,
            junctions: weights.junctions,
            fragments: weights.fragments,
            fidelity: weights.fidelity,
            boundary_balance: weights.boundary_balance,
            peak_sensitivity: weights.peak_sensitivity,
        }
    }
}

impl PythonContourWeights {
    fn validate(self) -> Result<crate::contour_score::ContourScoreWeights, String> {
        let values = [
            self.bending,
            self.endpoints,
            self.junctions,
            self.fragments,
            self.fidelity,
            self.boundary_balance,
            self.peak_sensitivity,
        ];
        if values
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
        {
            return Err("contour weights must be finite, non-negative numbers".to_string());
        }
        if self.peak_sensitivity > 1.0 {
            return Err("peak_sensitivity must be between 0 and 1".to_string());
        }
        Ok(crate::contour_score::ContourScoreWeights {
            bending: self.bending,
            endpoints: self.endpoints,
            junctions: self.junctions,
            fragments: self.fragments,
            fidelity: self.fidelity,
            boundary_balance: self.boundary_balance,
            peak_sensitivity: self.peak_sensitivity,
        })
    }
}

struct PythonBuffers {
    metadata: String,
    preview_rgba: Vec<u8>,
    preview_png: Vec<u8>,
    encoded: Vec<u8>,
}

fn milliseconds(duration: std::time::Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

#[pyfunction]
fn default_options_json() -> PyResult<String> {
    serde_json::to_string(&generator::default_options())
        .map_err(|error| PyRuntimeError::new_err(error.to_string()))
}

#[pyfunction]
fn resolved_options_json(options_json: &str) -> PyResult<String> {
    let options = generator::options_from_json(options_json).map_err(PyValueError::new_err)?;
    serde_json::to_string(&options).map_err(|error| PyRuntimeError::new_err(error.to_string()))
}

fn palette_rgb(palette: &[u32]) -> Vec<(u8, u8, u8)> {
    palette
        .iter()
        .map(|colour| {
            (
                ((colour >> 16) & 0xff) as u8,
                ((colour >> 8) & 0xff) as u8,
                (colour & 0xff) as u8,
            )
        })
        .collect()
}

fn named_palette(name: &str) -> PyResult<&'static [u32]> {
    match name.trim().to_ascii_lowercase().as_str() {
        "ansi" | "ansi256" | "ansi-256" => Ok(&crate::palette::ANSI256),
        "irc" | "irc99" | "irc-99" => Ok(&crate::palette::IRC99),
        _ => Err(PyValueError::new_err(
            "palette must be 'ansi256' or 'irc99'",
        )),
    }
}

#[pyfunction]
#[pyo3(signature = (rgb, palette="ansi256", preserve_grayscale=false, grayscale_tolerance=0))]
fn nearest_palette_colour(
    rgb: (u8, u8, u8),
    palette: &str,
    preserve_grayscale: bool,
    grayscale_tolerance: u8,
) -> PyResult<(u8, (u8, u8, u8))> {
    let palette = named_palette(palette)?;
    let packed = (u32::from(rgb.0) << 16) | (u32::from(rgb.1) << 8) | u32::from(rgb.2);
    let index = if preserve_grayscale {
        crate::draw::nearest_strict_partitioned_hex_colour(packed, palette, grayscale_tolerance)
    } else {
        crate::draw::nearest_hex_colour_fast(packed, palette)
    };
    let colour = palette[index as usize];
    Ok((
        index,
        (
            ((colour >> 16) & 0xff) as u8,
            ((colour >> 8) & 0xff) as u8,
            (colour & 0xff) as u8,
        ),
    ))
}

#[pyfunction]
fn image_dimensions(py: Python<'_>, image_data: Vec<u8>) -> PyResult<(u32, u32)> {
    py.detach(move || {
        let image = generator::decode_image(&image_data).map_err(PyValueError::new_err)?;
        Ok((image.get_width(), image.get_height()))
    })
}

#[pyfunction]
fn validate_font(py: Python<'_>, font_data: Vec<u8>) -> PyResult<()> {
    py.detach(move || {
        crate::font::validate_monospace_font_bytes(&font_data).map_err(PyValueError::new_err)
    })
}

fn glyph_store_for_python(
    options: &crate::args::RenderArgs,
    font_data: Option<&[u8]>,
    system_fonts: bool,
) -> Result<crate::font::GlyphStore, String> {
    let font_size = if options.font_size == 0.0 {
        24.0
    } else {
        options.font_size
    };
    if !system_fonts || font_data.is_some() {
        crate::font::glyph_store_from_font_bytes(
            font_data.unwrap_or(EMBEDDED_FONT),
            &options.blocks,
            &options.exclude_range,
            &options.exclude,
            &options.include_range,
            options.include.as_ref(),
            font_size,
            false,
        )
    } else {
        Ok(crate::font::glyph_store(
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

#[pyfunction]
fn glyph_groups_json() -> PyResult<String> {
    let (groups, _) = crate::font::load_blocks_config_unfiltered();
    let groups = groups
        .into_iter()
        .map(|(name, codes)| {
            let characters = codes
                .into_iter()
                .filter_map(char::from_u32)
                .collect::<String>();
            (name, characters)
        })
        .collect::<BTreeMap<_, _>>();
    serde_json::to_string(&groups).map_err(|error| PyRuntimeError::new_err(error.to_string()))
}

#[pyfunction]
#[pyo3(signature = (options_json="{}", font_data=None, system_fonts=false))]
fn font_info_json(
    py: Python<'_>,
    options_json: &str,
    font_data: Option<Vec<u8>>,
    system_fonts: bool,
) -> PyResult<String> {
    let options = generator::options_from_json(options_json).map_err(PyValueError::new_err)?;
    py.detach(move || {
        let store = glyph_store_for_python(&options, font_data.as_deref(), system_fonts)
            .map_err(PyValueError::new_err)?;
        let mut characters = store.selected.iter().copied().collect::<Vec<_>>();
        characters.sort_unstable();
        let groups = store
            .groups
            .iter()
            .map(|(name, members)| {
                let mut members = members
                    .iter()
                    .filter(|character| store.selected.contains(character))
                    .copied()
                    .collect::<Vec<_>>();
                members.sort_unstable();
                (name.clone(), members.into_iter().collect())
            })
            .collect();
        let font_names = store
            .glyphs
            .iter()
            .map(|(_, _, name)| name.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let info = PythonFontInfo {
            glyph_count: store.glyphs.len(),
            selected_count: store.selected.len(),
            cell_width: store.metrics.0,
            cell_height: store.metrics.1,
            cell_advance: store.float_metrics.0,
            line_height: store.float_metrics.1,
            font_names,
            groups,
            characters: characters.into_iter().collect(),
        };
        serde_json::to_string(&info).map_err(|error| PyRuntimeError::new_err(error.to_string()))
    })
}

fn photon_png(image: &photon_rs::PhotonImage) -> Result<Vec<u8>, String> {
    let rgba = image::RgbaImage::from_raw(
        image.get_width(),
        image.get_height(),
        image.get_raw_pixels(),
    )
    .ok_or_else(|| "processed image has an invalid RGBA buffer".to_string())?;
    let mut output = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(rgba)
        .write_to(&mut output, image::ImageFormat::Png)
        .map_err(|error| format!("could not encode processed PNG: {error}"))?;
    Ok(output.into_inner())
}

#[pyfunction]
#[pyo3(signature = (image_data, options_json="{}", include_png=true))]
fn process_json<'py>(
    py: Python<'py>,
    image_data: Vec<u8>,
    options_json: &str,
    include_png: bool,
) -> PyResult<(u32, u32, Bound<'py, PyBytes>, Bound<'py, PyBytes>)> {
    let options = generator::options_from_json(options_json).map_err(PyValueError::new_err)?;
    let (width, height, rgba, png) = py
        .detach(move || {
            let image = generator::decode_image(&image_data)?;
            let pipeline = crate::effects::default_pipeline(&options);
            let image = crate::effects::apply_pipeline(&options, image, &pipeline);
            let png = if include_png {
                photon_png(&image)?
            } else {
                Vec::new()
            };
            Ok::<_, String>((
                image.get_width(),
                image.get_height(),
                image.get_raw_pixels(),
                png,
            ))
        })
        .map_err(PyRuntimeError::new_err)?;
    Ok((
        width,
        height,
        PyBytes::new(py, &rgba),
        PyBytes::new(py, &png),
    ))
}

fn rgb_pixels(image: &photon_rs::PhotonImage) -> Vec<[u8; 3]> {
    image
        .get_raw_pixels()
        .chunks_exact(4)
        .map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .collect()
}

fn contour_json(
    score: crate::contour_score::ContourScore,
    reference: Option<crate::contour_score::ReferenceContourScore>,
) -> Result<String, String> {
    let mut value = serde_json::json!({
        "total": score.total,
        "bending": score.bending,
        "endpoints": score.endpoints,
        "junctions": score.junctions,
        "fragments": score.fragments,
        "boundary_length": score.boundary_length,
        "endpoint_count": score.endpoint_count,
        "junction_count": score.junction_count,
        "component_count": score.component_count,
        "fidelity": 0.0,
        "boundary_balance": 0.0,
    });
    if let Some(reference) = reference {
        value["total"] = serde_json::json!(reference.total);
        value["fidelity"] = serde_json::json!(reference.fidelity);
        value["boundary_balance"] = serde_json::json!(reference.boundary_balance);
    }
    serde_json::to_string(&value).map_err(|error| error.to_string())
}

#[pyfunction]
#[pyo3(signature = (image_data, cell_scale=8, reference_data=None, weights_json="{}"))]
fn contour_score_json(
    py: Python<'_>,
    image_data: Vec<u8>,
    cell_scale: usize,
    reference_data: Option<Vec<u8>>,
    weights_json: &str,
) -> PyResult<String> {
    if cell_scale == 0 {
        return Err(PyValueError::new_err(
            "cell_scale must be greater than zero",
        ));
    }
    let weights: PythonContourWeights = serde_json::from_str(weights_json)
        .map_err(|error| PyValueError::new_err(format!("invalid contour weights: {error}")))?;
    let weights = weights.validate().map_err(PyValueError::new_err)?;
    py.detach(move || {
        let output = generator::decode_image(&image_data).map_err(PyValueError::new_err)?;
        let width = output.get_width() as usize;
        let height = output.get_height() as usize;
        let output_pixels = rgb_pixels(&output);
        if let Some(reference_data) = reference_data {
            let reference =
                generator::decode_image(&reference_data).map_err(PyValueError::new_err)?;
            if reference.get_width() != output.get_width()
                || reference.get_height() != output.get_height()
            {
                return Err(PyValueError::new_err(
                    "reference and output images must have identical dimensions",
                ));
            }
            let reference_pixels = rgb_pixels(&reference);
            let score = crate::contour_score::score_rgb_against_reference_with_weights(
                &reference_pixels,
                &output_pixels,
                width,
                height,
                cell_scale,
                &weights,
            );
            contour_json(score.contour, Some(score)).map_err(PyRuntimeError::new_err)
        } else {
            let score = crate::contour_score::score_rgb_with_weights(
                &output_pixels,
                width,
                height,
                cell_scale,
                &weights,
            );
            contour_json(score, None).map_err(PyRuntimeError::new_err)
        }
    })
}

#[pyfunction]
#[pyo3(signature = (image_data, options_json="{}"))]
fn detect_text_json(py: Python<'_>, image_data: Vec<u8>, options_json: &str) -> PyResult<String> {
    let options = generator::options_from_json(options_json).map_err(PyValueError::new_err)?;
    py.detach(move || {
        let image = generator::decode_image(&image_data).map_err(PyValueError::new_err)?;
        let detections = crate::ocr::detect_text(&image, &options)
            .map_err(|error| PyRuntimeError::new_err(error.to_string()))?;
        let detections = detections
            .into_iter()
            .map(|detection| PythonOcrDetection {
                text: detection.text,
                confidence: detection.confidence,
                polygon: detection.polygon,
            })
            .collect::<Vec<_>>();
        serde_json::to_string(&detections)
            .map_err(|error| PyRuntimeError::new_err(error.to_string()))
    })
}

#[pyfunction]
fn contour_cache_stats() -> (u64, u64) {
    crate::contour_score::cache_stats()
}

#[pyfunction]
fn clear_contour_cache() {
    crate::contour_score::clear_cache();
}

fn generate_buffers(
    image_data: Vec<u8>,
    options: crate::args::RenderArgs,
    font_data: Option<Vec<u8>>,
    system_fonts: bool,
    include_cells: bool,
    include_preview: bool,
) -> Result<PythonBuffers, String> {
    let font_data = if system_fonts {
        font_data.as_deref()
    } else {
        Some(font_data.as_deref().unwrap_or(EMBEDDED_FONT))
    };
    let output = generator::generate(&image_data, options, font_data)?;
    let preview = include_preview
        .then(|| generator::preview_rgba(&output))
        .flatten();
    let preview_png = if include_preview {
        generator::preview_png(&output)?
    } else {
        Vec::new()
    };
    let (preview_width, preview_height, preview_rgba) =
        preview.unwrap_or_else(|| (0, 0, Vec::new()));
    let profile = &output.result.profile;
    let mut encoded = Vec::new();
    crate::write_encoded_to(
        &output.result.content,
        &output.options.encoding,
        &mut encoded,
    )
    .map_err(|error| error.to_string())?;
    let cells = include_cells.then(|| {
        output
            .result
            .grid
            .iter()
            .map(|row| {
                row.iter()
                    .map(|cell| PythonCell {
                        character: cell.char,
                        foreground: cell.fg,
                        background: cell.bg,
                        inverted: cell.inverted,
                        bold: cell.bold,
                        italic: cell.italic,
                        underline: cell.underline,
                    })
                    .collect()
            })
            .collect()
    });
    let metadata = PythonResult {
        content: output.result.content.clone(),
        save_content: output.result.save_content.clone(),
        columns: output.result.grid.first().map_or(0, Vec::len),
        rows: output.result.grid.len(),
        cells,
        error_count: output.result.error_count,
        total_pixels: output.result.total_pixels,
        dynamic_scaling_count: output.result.dynamic_scaling_count,
        score: output.result.score,
        longest_line_bytes: output.result.longest_line_bytes,
        preview_width,
        preview_height,
        font_size: output.options.font_size,
        cell_width: output.glyph_store.metrics.0,
        cell_height: output.glyph_store.metrics.1,
        cell_advance: output.glyph_store.float_metrics.0,
        line_height: output.glyph_store.float_metrics.1,
        timings: PythonTimings {
            prepare_canvas_ms: milliseconds(profile.prepare_canvas),
            glyph_match_ms: milliseconds(profile.glyph_match),
            smoothing_prepare_ms: milliseconds(profile.smoothing_prepare),
            smoothing_search_ms: milliseconds(profile.smoothing_search),
            shape_refine_ms: milliseconds(profile.shape_refine),
            final_score_ms: milliseconds(profile.final_score),
            encode_ms: milliseconds(profile.encode),
            contour_cache_hits: profile.contour_cache_hits,
            contour_cache_misses: profile.contour_cache_misses,
        },
        ocr_detections: output
            .ocr_detections
            .into_iter()
            .map(|detection| PythonOcrDetection {
                text: detection.text,
                confidence: detection.confidence,
                polygon: detection.polygon,
            })
            .collect(),
        options: output.options,
    };
    let metadata = serde_json::to_string(&metadata).map_err(|error| error.to_string())?;
    Ok(PythonBuffers {
        metadata,
        preview_rgba,
        preview_png,
        encoded,
    })
}

#[pyfunction]
#[pyo3(signature = (image_data, options_json="{}", font_data=None, system_fonts=false, include_cells=false, include_preview=true))]
fn generate_json<'py>(
    py: Python<'py>,
    image_data: Vec<u8>,
    options_json: &str,
    font_data: Option<Vec<u8>>,
    system_fonts: bool,
    include_cells: bool,
    include_preview: bool,
) -> PyResult<(
    String,
    Bound<'py, PyBytes>,
    Bound<'py, PyBytes>,
    Bound<'py, PyBytes>,
)> {
    let options = generator::options_from_json(options_json).map_err(PyValueError::new_err)?;
    // Rendering can take long enough that holding the interpreter lock would
    // stall unrelated Python threads. Everything in this closure is Rust-only.
    let buffers = py
        .detach(move || {
            generate_buffers(
                image_data,
                options,
                font_data,
                system_fonts,
                include_cells,
                include_preview,
            )
        })
        .map_err(PyRuntimeError::new_err)?;
    Ok((
        buffers.metadata,
        PyBytes::new(py, &buffers.preview_rgba),
        PyBytes::new(py, &buffers.preview_png),
        PyBytes::new(py, &buffers.encoded),
    ))
}

#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add("__version__", crate::VERSION)?;
    module.add("__build__", crate::BUILD_NUMBER)?;
    module.add("OCR_ENABLED", cfg!(feature = "ocr"))?;
    module.add("ANSI256", palette_rgb(&crate::palette::ANSI256))?;
    module.add("IRC99", palette_rgb(&crate::palette::IRC99))?;
    module.add_function(wrap_pyfunction!(default_options_json, module)?)?;
    module.add_function(wrap_pyfunction!(resolved_options_json, module)?)?;
    module.add_function(wrap_pyfunction!(generate_json, module)?)?;
    module.add_function(wrap_pyfunction!(process_json, module)?)?;
    module.add_function(wrap_pyfunction!(image_dimensions, module)?)?;
    module.add_function(wrap_pyfunction!(validate_font, module)?)?;
    module.add_function(wrap_pyfunction!(font_info_json, module)?)?;
    module.add_function(wrap_pyfunction!(glyph_groups_json, module)?)?;
    module.add_function(wrap_pyfunction!(nearest_palette_colour, module)?)?;
    module.add_function(wrap_pyfunction!(contour_score_json, module)?)?;
    module.add_function(wrap_pyfunction!(detect_text_json, module)?)?;
    module.add_function(wrap_pyfunction!(contour_cache_stats, module)?)?;
    module.add_function(wrap_pyfunction!(clear_contour_cache, module)?)?;
    Ok(())
}
