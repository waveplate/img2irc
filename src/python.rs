use crate::generator;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use serde::Serialize;

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
    options: crate::args::RenderArgs,
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
    let font_data = if system_fonts {
        font_data.as_deref()
    } else {
        Some(font_data.as_deref().unwrap_or(EMBEDDED_FONT))
    };
    let output =
        generator::generate(&image_data, options, font_data).map_err(PyRuntimeError::new_err)?;
    let preview = if include_preview {
        generator::preview_rgba(&output)
    } else {
        None
    };
    let preview_png = if include_preview {
        generator::preview_png(&output).map_err(PyRuntimeError::new_err)?
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
    .map_err(|error| PyRuntimeError::new_err(error.to_string()))?;
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
        options: output.options,
    };
    let metadata = serde_json::to_string(&metadata)
        .map_err(|error| PyRuntimeError::new_err(error.to_string()))?;
    Ok((
        metadata,
        PyBytes::new(py, &preview_rgba),
        PyBytes::new(py, &preview_png),
        PyBytes::new(py, &encoded),
    ))
}

#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add("__version__", crate::VERSION)?;
    module.add_function(wrap_pyfunction!(default_options_json, module)?)?;
    module.add_function(wrap_pyfunction!(generate_json, module)?)?;
    Ok(())
}
