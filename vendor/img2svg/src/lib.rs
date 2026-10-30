//! img2svg - A high-quality image to SVG converter library
//!
//! This library provides functionality to convert raster images (PNG, JPEG, etc.)
//! into scalable vector graphics (SVG) format.
//!
//! ## Features
//!
//! - **Color quantization** using median-cut algorithm
//! - **Marching squares** contour tracing for accurate shape detection
//! - **Ramer-Douglas-Peucker** simplification for clean paths
//! - **Gaussian smoothing** for natural curves
//!
//! ## Example
//!
//! ```rust,no_run
//! use img2svg::{convert, ConversionOptions};
//! use std::path::Path;
//!
//! let options = ConversionOptions {
//!     num_colors: 16,
//!     smooth_level: 5,
//!     ..Default::default()
//! };
//!
//! convert(Path::new("input.png"), Path::new("output.svg"), &options)
//!     .expect("Conversion failed");
//! ```

/// Cubic Bézier curve fitting with Newton-Raphson reparameterization.
pub mod bezier_fitter;
pub use bezier_fitter::{
    clamp_precision, optimize_bezier_curves_with_tolerance, DEFAULT_COORDINATE_PRECISION,
    DEFAULT_OPTIMIZE_TOLERANCE,
};
/// Sobel edge detection for gradient-based edge maps.
pub mod edge_detector;
/// K-means++ initialization and edge-aware color quantization.
pub mod enhanced_quantizer;
/// Enhanced vectorization pipeline: marching squares → Bézier curves.
pub mod enhanced_vectorizer;
/// Potrace-style despeckle: remove small color islands before contour extraction.
pub mod despeckle;
/// Centerline tracing for line art mode.
pub mod centerline_tracer;
/// EPS/PDF/AI vector export.
pub mod eps_generator;
/// DXF/GeoJSON/Gimppath vector export.
pub mod extra_backends;
/// Logo fill + outline separation.
pub mod logo_pipeline;
/// HTML parameter tuning grid generator.
pub mod tune;
/// Linear and radial gradient detection for smoother SVG gradient fills.
pub mod gradient_detector;
/// Comprehensive image processing filters (noise reduction, edge enhancement,
/// contrast adjustment, thresholding, morphology, color operations, effects).
pub mod image_filters;
/// Image loading, auto-resize, and median-cut color quantization.
pub mod image_processor;
/// Visvalingam-Whyatt simplification with corner preservation.
pub mod path_simplifier;
/// LUT bilateral filter and color reduction for photo preprocessing.
pub mod preprocessor;
/// Background color detection via border-pixel frequency.
pub mod region_extractor;
/// SVG generation with compact output and alpha channel support.
pub mod svg_generator;
/// Original pipeline: marching squares, Gaussian smoothing, RDP simplification.
pub mod vectorizer;

/// Re-export anyhow Result for convenience.
pub use anyhow::Result;
/// Re-export enhanced vectorization pipeline types and functions.
pub use enhanced_vectorizer::{
    generate_enhanced_svg, generate_enhanced_svg_prec, generate_enhanced_svg_to,
    generate_enhanced_svg_to_prec, vectorize_enhanced, write_enhanced_svg, write_enhanced_svg_prec,
    EnhancedOptions, EnhancedPath, EnhancedVectorData, RecolorMethod,
};
/// Re-export image filter functions and types.
pub use gradient_detector::{
    detect_gradient, detect_linear_gradient, detect_radial_gradient, DetectedGradient,
    LinearGradient, RadialGradient,
};
pub use image_filters::{
    mean_shift_filter, median_filter, prewitt_edge_detection, quantize_from_labels,
    roberts_edge_detection, slic_superpixels, watershed_segment, weighted_median_filter,
    AdaptiveThreshold, AdaptiveToneMapper, CannyEdgeDetector, ColorOps, ColorTemperature,
    EmbossFilter, GammaCorrection, HighBoostFilter, HistogramEqualizer, LaplacianSharpen,
    MorphologyKind, MorphologyOp, NonLocalMeans, OtsuThreshold, PeronaMalik, Saturation,
    SepiaFilter, SmartThreshold, UnsharpMask, VignetteFilter, VintageFilter, CLAHE, HSL, LAB,
};
pub use eps_generator::{
    write_enhanced_ai, write_enhanced_eps, write_enhanced_output, write_enhanced_output_prec,
    write_enhanced_pdf, write_multi_page_pdf, write_vectorized_eps, EnhancedOutputFormat,
};
pub use extra_backends::{
    write_enhanced_dxf, write_enhanced_geojson, write_enhanced_gimppath,
};
/// Re-export image loading and quantization functions.
pub use image_processor::{
    cmyk_to_rgb, detect_color_space, image_data_from_cmyk, load_animated_gif, load_image,
    quantize_colors, ColorSpace, GifFrame, ImageData,
};
/// Re-export photo preprocessing functions.
pub use despeckle::despeckle;
/// Re-export SVG generation functions.
pub use svg_generator::{
    generate_svg, generate_svg_advanced, generate_svg_to, write_multi_path_to, write_subpath_to,
};
/// Re-export original vectorization pipeline types and functions.
pub use vectorizer::{vectorize, Curve, Point, VectorizedData};

/// Options for image to SVG conversion
#[derive(Debug, Clone)]
pub struct ConversionOptions {
    /// Number of colors for quantization (default: 16)
    pub num_colors: usize,
    /// Edge detection threshold 0.0-1.0 (default: 0.1)
    pub threshold: f64,
    /// Path smoothing level 0-10 (default: 5)
    pub smooth_level: u8,
    /// Enable hierarchical decomposition (default: false)
    pub hierarchical: bool,
    /// Use advanced SVG generation (default: false)
    pub advanced: bool,
}

impl Default for ConversionOptions {
    fn default() -> Self {
        Self {
            num_colors: 16,
            threshold: 0.1,
            smooth_level: 5,
            hierarchical: false,
            advanced: false,
        }
    }
}

/// Convert an image file to SVG
///
/// # Arguments
///
/// * `input_path` - Path to the input image file
/// * `output_path` - Path to the output SVG file
/// * `options` - Conversion options
///
/// # Example
///
/// ```rust,no_run
/// use img2svg::{convert, ConversionOptions};
/// use std::path::Path;
///
/// let options = ConversionOptions::default();
/// convert(Path::new("input.png"), Path::new("output.svg"), &options)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn convert(
    input_path: &std::path::Path,
    output_path: &std::path::Path,
    options: &ConversionOptions,
) -> Result<()> {
    // Load the image
    let image_data = load_image(input_path)?;

    // Vectorize the image
    let vectorized_data = vectorize(
        &image_data,
        options.num_colors,
        options.threshold,
        options.smooth_level,
        options.hierarchical,
    )?;

    // Generate SVG output
    if options.advanced {
        generate_svg_advanced(&vectorized_data, output_path)?;
    } else {
        generate_svg(&vectorized_data, output_path)?;
    }

    Ok(())
}

/// Convert image data directly to SVG string
///
/// This is useful when you have image data in memory and want to get
/// the SVG content as a string without writing to a file.
///
/// # Arguments
///
/// * `image_data` - The image data to convert
/// * `options` - Conversion options
///
/// # Returns
///
/// A String containing the SVG content
pub fn convert_to_svg_string(
    image_data: &ImageData,
    options: &ConversionOptions,
) -> Result<String> {
    let vectorized_data = vectorize(
        image_data,
        options.num_colors,
        options.threshold,
        options.smooth_level,
        options.hierarchical,
    )?;

    let mut buffer = Vec::new();
    svg_generator::generate_svg_to(&vectorized_data, &mut buffer)?;
    Ok(String::from_utf8(buffer)?)
}

/// Convert an animated GIF to a vector of per-frame SVG strings.
///
/// Each frame is processed independently using the original pipeline.
/// Returns `(svg_strings, delays_ms)` so callers can assemble an
/// animated SVG or output individual files.
pub fn convert_animated_gif(
    frames: &[GifFrame],
    options: &ConversionOptions,
) -> Result<(Vec<String>, Vec<u32>)> {
    let mut svgs = Vec::with_capacity(frames.len());
    let mut delays = Vec::with_capacity(frames.len());

    for frame in frames {
        let svg = convert_to_svg_string(&frame.image_data, options)?;
        svgs.push(svg);
        delays.push(frame.delay_ms);
    }

    Ok((svgs, delays))
}

/// Check whether a file path has a supported raster image extension.
pub fn is_supported_image(path: &std::path::Path) -> bool {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        matches!(
            ext.to_lowercase().as_str(),
            "bmp" | "png" | "jpg" | "jpeg" | "gif" | "ico" | "tiff" | "tif" | "webp" | "pnm"
                | "tga" | "dds" | "farbfeld"
        )
    } else {
        false
    }
}

/// Result of converting one file in a batch run.
#[derive(Debug, Clone)]
pub struct BatchConvertEntry {
    pub input: std::path::PathBuf,
    pub output: std::path::PathBuf,
    pub success: bool,
    pub error: Option<String>,
}

/// Summary of a batch directory conversion.
#[derive(Debug, Clone)]
pub struct BatchConvertSummary {
    pub total: usize,
    pub converted: usize,
    pub errors: usize,
    pub entries: Vec<BatchConvertEntry>,
}

/// Convert all supported images in `input_dir` to vector files in `output_dir`.
///
/// Matches CLI batch mode (`img2svg -i dir/ -o out/`). Creates `output_dir` if missing.
pub fn batch_convert_enhanced(
    input_dir: &std::path::Path,
    output_dir: &std::path::Path,
    options: &EnhancedOptions,
    format: EnhancedOutputFormat,
    max_size: u32,
) -> Result<BatchConvertSummary> {
    std::fs::create_dir_all(output_dir)?;

    let entries: Vec<_> = std::fs::read_dir(input_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file() && is_supported_image(&e.path()))
        .collect();

    let total = entries.len();
    let mut converted = 0usize;
    let mut errors = 0usize;
    let mut results = Vec::with_capacity(total);

    for entry in entries {
        let input_path = entry.path();
        let stem = input_path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let mut output_path = output_dir.join(&stem);
        output_path.set_extension(format.extension());

        let outcome = (|| -> Result<()> {
            let image_data = load_image(&input_path)?;
            let image_data = image_processor::resize_if_needed(image_data, max_size);
            convert_enhanced_data(&image_data, &output_path, options, Some(format))?;
            Ok(())
        })();

        match outcome {
            Ok(()) => {
                converted += 1;
                results.push(BatchConvertEntry {
                    input: input_path,
                    output: output_path,
                    success: true,
                    error: None,
                });
            }
            Err(e) => {
                errors += 1;
                results.push(BatchConvertEntry {
                    input: input_path,
                    output: output_path,
                    success: false,
                    error: Some(e.to_string()),
                });
            }
        }
    }

    Ok(BatchConvertSummary {
        total,
        converted,
        errors,
        entries: results,
    })
}

/// Convert all supported images in `input_dir` into a single multi-page PDF.
///
/// Each image is vectorized with the enhanced Bézier pipeline and becomes one
/// page sized to its own dimensions. This is the library counterpart of
/// `img2svg -i dir/ -o combined.pdf`.
pub fn batch_convert_to_pdf(
    input_dir: &std::path::Path,
    output_path: &std::path::Path,
    options: &EnhancedOptions,
    max_size: u32,
) -> Result<BatchConvertSummary> {
    let entries: Vec<_> = std::fs::read_dir(input_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file() && is_supported_image(&e.path()))
        .collect();

    let total = entries.len();
    let mut converted = 0usize;
    let mut errors = 0usize;
    let mut results = Vec::with_capacity(total);
    let mut pages: Vec<EnhancedVectorData> = Vec::with_capacity(total);

    for entry in entries {
        let input_path = entry.path();
        let outcome = (|| -> Result<()> {
            let image_data = load_image(&input_path)?;
            let image_data = image_processor::resize_if_needed(image_data, max_size);
            let data = vectorize_enhanced(&image_data, options)?;
            pages.push(data);
            Ok(())
        })();
        match outcome {
            Ok(()) => {
                converted += 1;
                results.push(BatchConvertEntry {
                    input: input_path,
                    output: output_path.to_path_buf(),
                    success: true,
                    error: None,
                });
            }
            Err(e) => {
                errors += 1;
                results.push(BatchConvertEntry {
                    input: input_path,
                    output: output_path.to_path_buf(),
                    success: false,
                    error: Some(e.to_string()),
                });
            }
        }
    }

    if pages.is_empty() {
        anyhow::bail!("no images converted; PDF not written");
    }
    eps_generator::write_multi_page_pdf(&pages, output_path)?;

    Ok(BatchConvertSummary {
        total,
        converted,
        errors,
        entries: results,
    })
}

/// Convert in-memory image data with the enhanced Bézier pipeline.
///
/// Output format defaults to the `output_path` extension (`.svg` default).
/// Pass `format` to override (e.g. write EPS to a path ending in `.svg`).
/// Returns vectorization metadata after writing the output file.
pub fn convert_enhanced_data(
    image_data: &ImageData,
    output_path: &std::path::Path,
    options: &EnhancedOptions,
    format: Option<EnhancedOutputFormat>,
) -> Result<EnhancedVectorData> {
    let fmt = format.unwrap_or_else(|| EnhancedOutputFormat::from_path(output_path));
    let data = vectorize_enhanced(image_data, options)?;
    write_enhanced_output_prec(
        &data,
        output_path,
        fmt,
        clamp_precision(options.coordinate_precision),
    )?;
    Ok(data)
}

/// Convert an image file with the enhanced Bézier pipeline (CLI default).
///
/// This is the high-level counterpart to [`convert`] for the default pipeline.
/// Output format is inferred from `output_path` extension (`.svg` default).
///
/// # Example
///
/// ```rust,no_run
/// use img2svg::{convert_enhanced, EnhancedOptions};
/// use std::path::Path;
///
/// let options = EnhancedOptions {
///     num_colors: 16,
///     ..Default::default()
/// };
/// convert_enhanced(Path::new("input.png"), Path::new("output.svg"), &options)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn convert_enhanced(
    input_path: &std::path::Path,
    output_path: &std::path::Path,
    options: &EnhancedOptions,
) -> Result<()> {
    let image_data = load_image(input_path)?;
    convert_enhanced_data(&image_data, output_path, options, None)?;
    Ok(())
}

/// Convert in-memory image data to an enhanced SVG string.
///
/// Same pipeline as [`convert_enhanced`] but returns SVG markup instead of writing a file.
pub fn convert_enhanced_to_svg_string(
    image_data: &ImageData,
    options: &EnhancedOptions,
) -> Result<String> {
    let data = vectorize_enhanced(image_data, options)?;
    Ok(generate_enhanced_svg_prec(
        &data,
        clamp_precision(options.coordinate_precision),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_conversion_options_default() {
        let options = ConversionOptions::default();
        assert_eq!(options.num_colors, 16);
        assert_eq!(options.threshold, 0.1);
        assert_eq!(options.smooth_level, 5);
        assert!(!options.hierarchical);
        assert!(!options.advanced);
    }

    #[test]
    fn test_is_supported_image() {
        assert!(is_supported_image(std::path::Path::new("a.png")));
        assert!(is_supported_image(std::path::Path::new("a.JPG")));
        assert!(!is_supported_image(std::path::Path::new("readme.txt")));
    }

    #[test]
    fn test_convert_enhanced_to_svg_string() {
        let mut pixels = Vec::new();
        for _ in 0..(20 * 20) {
            pixels.push(rgb::RGBA8::new(200, 50, 50, 255));
        }
        let image = ImageData {
            width: 20,
            height: 20,
            pixels,
        };
        let opts = EnhancedOptions {
            num_colors: 4,
            preprocess: false,
            ..Default::default()
        };
        let svg = convert_enhanced_to_svg_string(&image, &opts).expect("convert");
        assert!(svg.contains("<svg"));
        assert!(svg.contains("<path") || svg.contains("<rect"));
    }
}
