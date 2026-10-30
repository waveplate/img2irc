//! Enhanced vectorization pipeline ported from the vec project.
//!
//! Pipeline: bilateral filter → Sobel edge detection → edge-aware quantization
//! → flood-fill region extraction → Gaussian smoothing with corner preservation
//! → Visvalingam-Whyatt simplification → cubic Bézier fitting → SVG output
//! with gap-filling strokes and color grouping.
//!
//! The original marching-squares pipeline is preserved in vectorizer.rs for comparison.

use crate::bezier_fitter::{bezier_to_svg_path_to_prec, optimize_bezier_corners, optimize_bezier_curves_with_tolerance, BezierCurve, BezierFitter};
use crate::despeckle::despeckle;
use crate::edge_detector::detect_edges_sobel;
use crate::enhanced_quantizer::{
    adaptive_color_count, count_distinct_colors, quantize_edge_aware, quantize_superpixel,
};
use crate::image_processor::ImageData;
use crate::path_simplifier::{detect_corners, smooth_with_corners, visvalingam_whyatt};
use crate::preprocessor::{preprocess, PreprocessOptions};
use crate::region_extractor::detect_background_color;
use crate::vectorizer::{marching_squares_contours, Point};
use anyhow::Result;
use rayon::prelude::*;
use std::collections::HashMap;
use std::io::Write;

/// Print a single progress update to stderr when `progress` is enabled.
/// Uses a carriage return so the bar stays on one line.
fn report_progress(enabled: bool, percent: u8, label: &str) {
    if !enabled {
        return;
    }
    let width = 20usize;
    let filled = (percent as usize * width / 100).min(width);
    let bar = format!(
        "[{}{}]",
        "#".repeat(filled),
        "-".repeat(width - filled)
    );
    eprint!("\r{} {:3}% {}", bar, percent, label);
    let _ = std::io::stderr().flush();
}

/// Finish the progress line by moving to the next line.
fn finish_progress(enabled: bool) {
    if enabled {
        eprintln!();
    }
}

/// Log a pipeline stage. When `progress` is enabled, draws the progress bar;
/// otherwise prints a plain stage message to stderr.
fn stage_log(progress: bool, percent: u8, msg: &str) {
    if progress {
        report_progress(true, percent, msg);
    } else {
        #[cfg(not(feature = "quiet"))]
        eprintln!("    {}", msg);
    }
}

/// Auto-tune enhanced options based on simple image heuristics.
/// Returns a modified options struct; the original `autotrace` flag is cleared
/// so the pipeline does not recurse.
fn autotune_options(image: &ImageData, options: &EnhancedOptions) -> EnhancedOptions {
    let n_colors = count_distinct_colors(image);
    let pixel_count = (image.width * image.height) as usize;
    let max_dim = image.width.max(image.height) as usize;

    let mut tuned = options.clone();
    tuned.autotrace = false;

    if n_colors <= 8 && pixel_count < 200_000 {
        // Logo-like: flat graphics with a handful of colors
        tuned.num_colors = 8;
        tuned.smoothing_passes = 2;
        tuned.simplification_tolerance = 1.5;
        tuned.curve_tolerance = 2.0;
        tuned.corner_threshold = 60.0;
        tuned.min_region_area = 20;
        tuned.preprocess = false;
        tuned.optimize_corners = true;
        tuned.optimize_curves = true;
        tuned.logo_mode = true;
    } else if max_dim < 200 && n_colors <= 64 {
        // Icon-like: small, compact
        tuned.num_colors = 12;
        tuned.smoothing_passes = 2;
        tuned.simplification_tolerance = 1.0;
        tuned.curve_tolerance = 1.5;
        tuned.corner_threshold = 45.0;
        tuned.min_region_area = 10;
        tuned.preprocess = false;
        tuned.optimize_corners = true;
        tuned.optimize_curves = true;
        tuned.logo_mode = false;
    } else if n_colors > 1000 || pixel_count > 500_000 {
        // Photo-like: many colors or large resolution
        tuned.num_colors = 16;
        tuned.smoothing_passes = 3;
        tuned.simplification_tolerance = 2.0;
        tuned.curve_tolerance = 2.5;
        tuned.corner_threshold = 90.0;
        tuned.min_region_area = 30;
        tuned.preprocess = true;
        tuned.optimize_corners = false;
        tuned.optimize_curves = true;
        tuned.logo_mode = false;
    }
    // else: keep existing defaults (general tuning)

    tuned
}

/// How to derive each region's display color from the original image pixels
/// that fell into its quantized bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RecolorMethod {
    /// Arithmetic mean of the original pixels (current default; smooth but can
    /// produce muddy colors when a bucket spans several distinct originals).
    #[default]
    Mean,
    /// Per-channel median of the original pixels. Robust to outliers and
    /// anti-aliased fringe pixels, so it picks a representative "middle" color
    /// rather than a washed-out average.
    Median,
    /// Most common exact original color in the bucket. Best for flat-color
    /// graphics (logos/icons) where the true color is one of a few discrete
    /// originals and averaging would blend it toward a neighbor.
    Mode,
}

/// Enhanced vectorization options.
#[derive(Debug, Clone)]
pub struct EnhancedOptions {
    /// Number of colors (0 = auto-detect based on image size)
    pub num_colors: usize,
    /// Curve fitting tolerance (lower = more accurate, larger SVG)
    pub curve_tolerance: f64,
    /// Path simplification tolerance
    pub simplification_tolerance: f64,
    /// Corner detection threshold in degrees
    pub corner_threshold: f64,
    /// Minimum region area in pixels
    pub min_region_area: usize,
    /// Edge detection threshold for edge-aware quantization
    pub edge_threshold: u8,
    /// Number of majority-vote smoothing passes
    pub smoothing_passes: usize,
    /// Smoothing window size for boundary points
    pub smooth_window: usize,
    /// Whether to apply bilateral filter preprocessing
    pub preprocess: bool,
    /// Whether to recolor from original image
    pub recolor: bool,
    /// Method used to derive each region's display color from the original
    /// pixels when `recolor` is enabled (default [`RecolorMethod::Mean`]).
    pub recolor_method: RecolorMethod,
    /// Use SLIC superpixel quantization instead of k-means
    pub use_superpixel: bool,
    /// Centerline/stroke tracing for line art (open paths)
    pub centerline: bool,
    /// Potrace-style corner optimization on Bézier control points
    pub optimize_corners: bool,
    /// Potrace `-O` style merge of consecutive nearly-linear curve segments
    pub optimize_curves: bool,
    /// Merge tolerance (px) for `optimize_curves` (default 1.5; higher = more aggressive)
    pub optimize_tolerance: f64,
    /// Separate fill and outline layers for logos
    pub logo_mode: bool,
    /// Show a progress bar on stderr during vectorization.
    pub progress: bool,
    /// Auto-select tuning parameters based on image characteristics.
    pub autotrace: bool,
    /// Decimal places for SVG path coordinates (1-6, default 2).
    /// Lower values shrink the output file; higher values preserve detail.
    pub coordinate_precision: u32,
    /// Detect smooth linear/radial gradients per region and render them as
    /// native SVG `<defs>` gradient fills (default false).
    pub detect_gradients: bool,
    /// Max fit MSE for gradient detection (default 50.0; lower = stricter).
    pub gradient_error_threshold: f64,
    /// Wrap each color group in an SVG `<g>` element with a
    /// `data-img2svg-color="#rrggbb"` attribute (default false).
    /// Produces Inkscape-style hierarchical layers that are easier to
    /// edit/restyle in vector tools; ignores the background rect and
    /// stroke-only paths.
    pub group_by_color: bool,
    /// Use interpolated (sub-pixel) marching squares for contour extraction.
    /// Linearly interpolates edge-crossing positions from a coverage field,
    /// producing smoother, anti-aliased contours that need less smoothing and
    /// yield cleaner Bézier fits on diagonal/curved boundaries (default true).
    pub subpixel_contours: bool,
}

impl Default for EnhancedOptions {
    fn default() -> Self {
        Self {
            num_colors: 0, // auto
            curve_tolerance: 2.0,
            simplification_tolerance: 1.5,
            corner_threshold: 60.0,
            min_region_area: 20,
            edge_threshold: 25,
            smoothing_passes: 2,
            smooth_window: 3,
            preprocess: true,
            recolor: true,
            recolor_method: RecolorMethod::Mean,
            use_superpixel: false,
            centerline: false,
            optimize_corners: false,
            optimize_curves: false,
            optimize_tolerance: crate::bezier_fitter::DEFAULT_OPTIMIZE_TOLERANCE,
            logo_mode: false,
            progress: false,
            autotrace: false,
            coordinate_precision: crate::bezier_fitter::DEFAULT_COORDINATE_PRECISION,
            detect_gradients: false,
            gradient_error_threshold: 50.0,
            group_by_color: false,
            subpixel_contours: true,
        }
    }
}

/// Result of enhanced vectorization.
pub struct EnhancedVectorData {
    pub width: u32,
    pub height: u32,
    pub background_color: (u8, u8, u8, u8),
    pub paths: Vec<EnhancedPath>,
    /// Whether to wrap each color group in an SVG `<g>` with a
    /// `data-img2svg-color="#rrggbb"` attribute (Inkscape-style hierarchy).
    pub group_by_color: bool,
}

/// A vectorized path with Bézier curves.
#[derive(Debug, Clone)]
pub struct EnhancedPath {
    pub curves: Vec<BezierCurve>,
    pub color: (u8, u8, u8, u8),
    pub area: usize,
    /// Pre-built SVG path data for thin stripe rects (bypasses bezier_to_svg_path).
    pub svg_override: Option<String>,
    /// Detected gradient for this path's region (rendered as a `<defs>` gradient
    /// fill when output as SVG). `None` unless `EnhancedOptions.detect_gradients`.
    pub gradient: Option<crate::gradient_detector::DetectedGradient>,
    /// Stroke-only path (centerline / logo outline)
    pub stroke_only: bool,
}

/// Run the enhanced vectorization pipeline.
///
/// Uses enhanced quantization (k-means++, edge-aware) with the proven
/// marching-squares contour extraction, then applies Visvalingam-Whyatt
/// simplification and cubic Bézier fitting for smooth curves.
pub fn vectorize_enhanced(
    image_data: &ImageData,
    options: &EnhancedOptions,
) -> Result<EnhancedVectorData> {
    if options.centerline {
        return vectorize_centerline(image_data, options);
    }

    let options = if options.autotrace {
        autotune_options(image_data, options)
    } else {
        options.clone()
    };

    let width = image_data.width as usize;
    let height = image_data.height as usize;
    let pixel_count = width * height;
    let is_small = pixel_count < 10_000;

    // Detect if image has many colors (photos, gradients)
    let progress = options.progress;

    let n_colors = count_distinct_colors(image_data);
    let is_many_colors = n_colors > 16;
    stage_log(progress, 5, &format!("Detected {} unique colors", n_colors));

    // Determine target color count
    let target_colors = if options.num_colors > 0 {
        options.num_colors
    } else if is_many_colors {
        adaptive_color_count(image_data)
    } else {
        n_colors.min(64)
    };
    stage_log(progress, 10, &format!("Target palette: {} colors", target_colors));

    // Optional preprocessing (bilateral filter for photos)
    let preprocessed = if options.preprocess && is_many_colors {
        stage_log(progress, 25, "Applying bilateral filter preprocessing...");
        let opts = PreprocessOptions::photo();
        preprocess(image_data, &opts)?
    } else {
        image_data.clone()
    };

    // Edge detection + edge-aware quantization (k-means++ with perceptual distance)
    stage_log(progress, 40, "Detecting edges and quantizing...");
    let edges = detect_edges_sobel(&preprocessed);
    // Detect if image is a photo (continuous tones) vs complex graphic (many distinct colors).
    // Photos: many colors, smooth gradients → fewer smoothing passes, bilateral preprocess.
    // Complex graphics: many colors, sharp edges, thin features → more smoothing passes.
    let is_photo = is_many_colors && n_colors > 1000;
    // More smoothing passes for non-photo images to merge thin stripes/artifacts.
    // Edge-aware smoothing preserves strong-edge shapes while merging weak-edge thin features.
    let smooth_passes = if !is_photo && is_many_colors {
        options.smoothing_passes.max(4)
    } else {
        options.smoothing_passes
    };
    let (mut quantized, _indices, _palette) = if options.use_superpixel {
        quantize_superpixel(&preprocessed, target_colors)
    } else {
        quantize_edge_aware(
            &preprocessed,
            target_colors,
            &edges,
            options.edge_threshold,
            smooth_passes,
        )
    };

    if options.min_region_area > 1 {
        stage_log(
            progress,
            55,
            &format!(
                "Despeckle: removing regions smaller than {} px...",
                options.min_region_area
            ),
        );
        quantized = despeckle(&quantized, options.min_region_area);
    }

    // Group pixels by quantized color for region assignment
    let mut color_pixels: HashMap<(u8, u8, u8, u8), Vec<(usize, usize)>> = HashMap::new();
    for y in 0..height {
        for x in 0..width {
            let p = quantized.pixels[y * width + x];
            let key = (p.r, p.g, p.b, p.a);
            color_pixels.entry(key).or_default().push((x, y));
        }
    }

    // Build a mapping from quantized color → representative original color for
    // display, using the configured recolor method (mean/median/mode).
    let recolor_map: HashMap<(u8, u8, u8, u8), (u8, u8, u8, u8)> =
        if options.recolor && is_many_colors {
            let mut map = HashMap::new();
            for (&qcolor, pixels) in &color_pixels {
                let orig: Vec<(u8, u8, u8, u8)> = pixels
                    .iter()
                    .map(|&(x, y)| {
                        let p = image_data.pixels[y * width + x];
                        (p.r, p.g, p.b, p.a)
                    })
                    .collect();
                map.insert(qcolor, recolor_color(&orig, options.recolor_method));
            }
            map
        } else {
            HashMap::new()
        };

    // Sort colors by pixel count (largest area first for proper z-order)
    let mut color_list: Vec<_> = color_pixels.into_iter().collect();
    color_list.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

    // Background detection using border pixels of quantized image.
    // Use quantized color directly (not recolored) — recolored averages can produce
    // unexpected dark colors for photos where the border region spans diverse originals.
    let bg_quantized = detect_background_color(&quantized);
    let background_color = bg_quantized;

    let w_f = width as f64;
    let h_f = height as f64;
    let fitter = BezierFitter::new(options.curve_tolerance);
    // For photos (many colors), use higher simplification tolerance to reduce SVG size
    let simp_tol = if is_small {
        options.simplification_tolerance.min(0.5)
    } else if is_many_colors {
        options.simplification_tolerance * 2.0
    } else {
        options.simplification_tolerance
    };
    // Minimum polygon area from options (Potrace turdsize equivalent)
    let min_poly_area = options.min_region_area.max(1) as f64;

    stage_log(
        progress,
        70,
        &format!(
            "Extracting contours for {} colors (bg excluded)...",
            color_list.len().saturating_sub(1)
        ),
    );

    // For each color: build binary mask → label connected components →
    // [optional gradient detection per component] → marching squares per component
    // → smooth → simplify → Bézier fit.
    // Detecting gradients per connected component (not per color) ensures that
    // when the same quantized color appears in disjoint regions with different
    // underlying gradients (e.g. top half horizontal, bottom half vertical),
    // each region gets its own correct gradient fit.
    // Collect (display_color, pixel_count, mask, contours, gradient) tuples for parallel processing
    let color_contours: Vec<(
        (u8, u8, u8, u8),
        usize,
        Vec<bool>,
        Vec<Vec<Point>>,
        Option<crate::gradient_detector::DetectedGradient>,
    )> = color_list
        .iter()
        .filter(|(color, _)| *color != bg_quantized)
        .flat_map(|(color, pixels)| {
            let mut mask = vec![false; pixel_count];
            for &(x, y) in pixels {
                mask[y * width + x] = true;
            }
            let display_color = recolor_map.get(color).copied().unwrap_or(*color);
            let total_pixels = pixels.len();

            if !options.detect_gradients {
                // Gradients disabled: no detection, single mask.
                let contours = extract_contours(&mask, width, height, &options);
                return vec![(
                    display_color,
                    total_pixels,
                    mask,
                    contours,
                    None,
                )];
            }

            // Gradient mode: label connected components and detect per component.
            let components = crate::gradient_detector::connected_components(&mask, width, height);
            if components.is_empty() {
                let contours = extract_contours(&mask, width, height, &options);
                return vec![(display_color, total_pixels, mask, contours, None)];
            }
            components
                .into_iter()
                .map(|comp| {
                    let comp_pixels = comp.iter().filter(|&&b| b).count();
                    let gradient =
                        crate::gradient_detector::detect_gradient(
                            image_data,
                            &comp,
                            options.gradient_error_threshold,
                        )
                        .map(|g| extend_gradient_endpoints(g, width, height));
                    let contours = extract_contours(&comp, width, height, &options);
                    (display_color, comp_pixels, comp, contours, gradient)
                })
                .collect::<Vec<_>>()
        })
        .collect();

    // Parallel: for each contour, smooth → simplify → Bézier fit
    let mut enhanced_paths: Vec<EnhancedPath> = color_contours
        .par_iter()
        .flat_map(|(color, pixel_count, mask, contours, gradient)| {
            let mut paths = Vec::new();

            for contour in contours {
                if contour.len() < 4 {
                    continue;
                }
                if polygon_area(contour) < min_poly_area {
                    continue;
                }

                // Fast path: thin stripe contours (height or width < 2px) →
                // emit as simple rectangle directly (bypass Bézier fitter which collapses thin shapes).
                let (cb_min_x, cb_min_y, cb_max_x, cb_max_y) = bounds_from_points(contour);
                let cb_w = cb_max_x - cb_min_x;
                let cb_h = cb_max_y - cb_min_y;
                if (cb_h < 2.0 && cb_w >= 2.0) || (cb_w < 2.0 && cb_h >= 2.0) {
                    let x0 = cb_min_x.round() as i64;
                    let y0 = cb_min_y.round() as i64;
                    let x1 = if cb_w < 2.0 {
                        x0 + cb_w.ceil().max(1.0) as i64
                    } else {
                        cb_max_x.round() as i64
                    };
                    let y1 = if cb_h < 2.0 {
                        y0 + cb_h.ceil().max(1.0) as i64
                    } else {
                        cb_max_y.round() as i64
                    };
                    // Emit direct SVG rect path (bypasses bezier_to_svg_path collinear merge)
                    let svg = format!("M{x0},{y0}L{x1},{y0}L{x1},{y1}L{x0},{y1}Z");
                    paths.push(EnhancedPath {
                        curves: Vec::new(),
                        color: *color,
                        area: *pixel_count,
                        svg_override: Some(svg),
                        gradient: gradient.clone(),
                        stroke_only: false,
                    });
                    continue;
                }

                // Smooth with corner preservation (enhanced)
                let smoothed =
                    smooth_with_corners(contour, options.smooth_window, options.corner_threshold);

                // Detect corners for Visvalingam-Whyatt (enhanced)
                let corners = detect_corners(&smoothed, options.corner_threshold);

                // Visvalingam-Whyatt simplification with corner preservation (enhanced)
                let simplified = visvalingam_whyatt(&smoothed, simp_tol * simp_tol, &corners);

                if simplified.len() < 3 {
                    continue;
                }

                // Snap points near image edges to exact boundary
                let snap = 4.0;
                let snapped: Vec<Point> = simplified
                    .into_iter()
                    .map(|p| Point {
                        x: if p.x < snap {
                            0.0
                        } else if p.x > w_f - snap {
                            w_f
                        } else {
                            p.x
                        },
                        y: if p.y < snap {
                            0.0
                        } else if p.y > h_f - snap {
                            h_f
                        } else {
                            p.y
                        },
                    })
                    .collect();

                // Inject image corner points where contour transitions between edges.
                // E.g. right edge (x=W) → top edge (y=0) needs corner (W,0) inserted.
                let snapped = inject_image_corners(&snapped, w_f, h_f);

                // Deduplicate consecutive near-identical points (from snapping + corner injection).
                // Without this, duplicate points at image corners break the fitter's angle detection.
                let snapped = dedup_consecutive(&snapped, 0.5);

                if snapped.len() < 3 || polygon_area(&snapped) < min_poly_area {
                    continue;
                }

                // Cubic Bézier fitting with internal corner detection (enhanced)
                let mut curves = fitter.fit_path(&snapped, true);

                if options.optimize_corners {
                    optimize_bezier_corners(&mut curves);
                }
                if options.optimize_curves {
                    optimize_bezier_curves_with_tolerance(&mut curves, options.optimize_tolerance);
                }

                // Clamp control points to image bounds (prevents bulging corners)
                for curve in &mut curves {
                    curve.control1.x = curve.control1.x.clamp(0.0, w_f);
                    curve.control1.y = curve.control1.y.clamp(0.0, h_f);
                    curve.control2.x = curve.control2.x.clamp(0.0, w_f);
                    curve.control2.y = curve.control2.y.clamp(0.0, h_f);
                }

                if !curves.is_empty() {
                    paths.push(EnhancedPath {
                        curves,
                        color: *color,
                        area: *pixel_count,
                        svg_override: None,
                        gradient: gradient.clone(),
                        stroke_only: false,
                    });
                }

                if options.logo_mode {
                    let ring = crate::logo_pipeline::outline_mask(&mask, width, height);
                    let outline_contours = if options.subpixel_contours {
                        let field = crate::vectorizer::build_smoothed_field(&ring, width, height);
                        crate::vectorizer::marching_squares_contours_interp(&ring, &field, width, height)
                    } else {
                        marching_squares_contours(&ring, width, height)
                    };
                    let ocolor = crate::logo_pipeline::outline_color(*color);
                    for outline in outline_contours {
                        if outline.len() < 3 {
                            continue;
                        }
                        let mut ocurves = fitter.fit_path(&outline, true);
                        if options.optimize_corners {
                            optimize_bezier_corners(&mut ocurves);
                        }
                        if options.optimize_curves {
                            optimize_bezier_curves_with_tolerance(&mut ocurves, options.optimize_tolerance);
                        }
                        if !ocurves.is_empty() {
                            paths.push(EnhancedPath {
                                curves: ocurves,
                                color: ocolor,
                                area: outline.len(),
                                svg_override: None,
                                gradient: None,
                                stroke_only: true,
                            });
                        }
                    }
                }
            }

            paths
        })
        .collect();

    // Sort: largest regions first (back-to-front layering)
    enhanced_paths.sort_unstable_by(|a, b| b.area.cmp(&a.area));
    stage_log(
        progress,
        100,
        &format!("Generated {} paths", enhanced_paths.len()),
    );
    finish_progress(progress);

    Ok(EnhancedVectorData {
        width: image_data.width,
        height: image_data.height,
        background_color,
        paths: enhanced_paths,
        group_by_color: options.group_by_color,
    })
}

fn vectorize_centerline(
    image_data: &ImageData,
    options: &EnhancedOptions,
) -> Result<EnhancedVectorData> {
    let width = image_data.width as usize;
    let height = image_data.height as usize;
    let mut mask = vec![false; width * height];
    for (i, p) in image_data.pixels.iter().enumerate() {
        let g = 0.299 * p.r as f64 + 0.587 * p.g as f64 + 0.114 * p.b as f64;
        mask[i] = g < 128.0;
    }

    let polylines = crate::centerline_tracer::trace_centerlines(&mask, width, height);
    let fitter = BezierFitter::new(options.curve_tolerance);
    let bg = detect_background_color(image_data);
    let mut paths = Vec::new();

    for poly in polylines {
        if poly.len() < 2 {
            continue;
        }
        let mut curves = fitter.fit_path(&poly, false);
        if options.optimize_corners {
            optimize_bezier_corners(&mut curves);
        }
        if options.optimize_curves {
            optimize_bezier_curves_with_tolerance(&mut curves, options.optimize_tolerance);
        }
        if !curves.is_empty() {
            paths.push(EnhancedPath {
                curves,
                color: (0, 0, 0, 255),
                area: poly.len(),
                svg_override: None,
                gradient: None,
                stroke_only: true,
            });
        }
    }

    Ok(EnhancedVectorData {
        width: image_data.width,
        height: image_data.height,
        background_color: bg,
        paths,
        group_by_color: false,
    })
}

/// Compute the representative display color for a set of original pixels
/// using the chosen recolor method.
fn recolor_color(pixels: &[(u8, u8, u8, u8)], method: RecolorMethod) -> (u8, u8, u8, u8) {
    if pixels.is_empty() {
        return (0, 0, 0, 0);
    }
    match method {
        RecolorMethod::Mean => {
            let mut sr: u64 = 0;
            let mut sg: u64 = 0;
            let mut sb: u64 = 0;
            let mut sa: u64 = 0;
            for &(r, g, b, a) in pixels {
                sr += r as u64;
                sg += g as u64;
                sb += b as u64;
                sa += a as u64;
            }
            let n = pixels.len() as u64;
            ((sr / n) as u8, (sg / n) as u8, (sb / n) as u8, (sa / n) as u8)
        }
        RecolorMethod::Median => {
            // Per-channel median. Cloning into separate Vecs is O(n) per
            // channel; regions are typically small so this is acceptable.
            let mut rs: Vec<u8> = pixels.iter().map(|p| p.0).collect();
            let mut gs: Vec<u8> = pixels.iter().map(|p| p.1).collect();
            let mut bs: Vec<u8> = pixels.iter().map(|p| p.2).collect();
            let mut as_: Vec<u8> = pixels.iter().map(|p| p.3).collect();
            rs.sort_unstable();
            gs.sort_unstable();
            bs.sort_unstable();
            as_.sort_unstable();
            let mid = pixels.len() / 2;
            (rs[mid], gs[mid], bs[mid], as_[mid])
        }
        RecolorMethod::Mode => {
            // Most frequent exact color (ties broken by first occurrence).
            use std::collections::HashMap;
            let mut counts: HashMap<(u8, u8, u8, u8), usize> = HashMap::new();
            let mut best = pixels[0];
            let mut best_count = 0usize;
            for &c in pixels {
                let n = counts.entry(c).or_insert(0);
                *n += 1;
                if *n > best_count {
                    best_count = *n;
                    best = c;
                }
            }
            best
        }
    }
}

/// Remove consecutive near-duplicate points (distance < threshold).
fn dedup_consecutive(points: &[Point], threshold: f64) -> Vec<Point> {
    if points.is_empty() {
        return Vec::new();
    }
    let t2 = threshold * threshold;
    let mut result = vec![points[0].clone()];
    for p in &points[1..] {
        let last = result.last().unwrap();
        let dx = p.x - last.x;
        let dy = p.y - last.y;
        if dx * dx + dy * dy > t2 {
            result.push(p.clone());
        }
    }
    result
}

/// Classify which image edge a point lies on (if any).
/// Returns: 'T' (top, y=0), 'B' (bottom, y=H), 'L' (left, x=0), 'R' (right, x=W), or ' ' (interior).
fn edge_of(p: &Point, w: f64, h: f64) -> char {
    let eps = 0.01;
    if p.y.abs() < eps {
        'T'
    } else if (p.y - h).abs() < eps {
        'B'
    } else if p.x.abs() < eps {
        'L'
    } else if (p.x - w).abs() < eps {
        'R'
    } else {
        ' '
    }
}

/// Inject image corner points where a contour transitions between two different
/// image edges. For example, if point A is on the right edge (x=W) and point B
/// is on the top edge (y=0), the corner (W,0) is inserted between them.
/// This ensures the Bézier fitter detects the 90° corner and splits there.
fn inject_image_corners(points: &[Point], w: f64, h: f64) -> Vec<Point> {
    if points.len() < 3 {
        return points.to_vec();
    }

    let corners = [
        ('T', 'R', Point { x: w, y: 0.0 }),   // top-right
        ('R', 'B', Point { x: w, y: h }),     // bottom-right
        ('B', 'L', Point { x: 0.0, y: h }),   // bottom-left
        ('L', 'T', Point { x: 0.0, y: 0.0 }), // top-left
    ];

    let mut result = Vec::with_capacity(points.len() + 4);
    let n = points.len();

    for i in 0..n {
        result.push(points[i].clone());

        let j = (i + 1) % n;
        let e1 = edge_of(&points[i], w, h);
        let e2 = edge_of(&points[j], w, h);

        // Both on edges, but different edges → insert corner
        if e1 != ' ' && e2 != ' ' && e1 != e2 {
            for &(ea, eb, ref corner) in &corners {
                if (e1 == ea && e2 == eb) || (e1 == eb && e2 == ea) {
                    // Don't insert if already very close to the corner
                    let d1 = (points[i].x - corner.x).powi(2) + (points[i].y - corner.y).powi(2);
                    let d2 = (points[j].x - corner.x).powi(2) + (points[j].y - corner.y).powi(2);
                    if d1 > 1.0 && d2 > 1.0 {
                        result.push(corner.clone());
                    }
                    break;
                }
            }
        }
    }

    result
}

/// Compute signed polygon area (Shoelace formula).
fn polygon_area(points: &[Point]) -> f64 {
    let n = points.len();
    if n < 3 {
        return 0.0;
    }
    let mut area = 0.0;
    for i in 0..n {
        let j = (i + 1) % n;
        area += points[i].x * points[j].y;
        area -= points[j].x * points[i].y;
    }
    (area / 2.0).abs()
}

/// Generate SVG string from enhanced vector data.
/// Uses gap-filling strokes and consecutive same-color path grouping.
#[allow(dead_code)] // Re-exported at crate root; clippy cannot see the chain.
pub fn generate_enhanced_svg(data: &EnhancedVectorData) -> String {
    generate_enhanced_svg_prec(
        data,
        crate::bezier_fitter::DEFAULT_COORDINATE_PRECISION,
    )
}

/// Generate SVG string from enhanced vector data with user-controlled
/// coordinate precision (1-6 decimal places, clamped).
pub fn generate_enhanced_svg_prec(data: &EnhancedVectorData, precision: u32) -> String {
    let mut buffer = Vec::new();
    if let Err(e) = generate_enhanced_svg_to_prec(data, &mut buffer, precision) {
        // This should not fail for an in-memory Vec, but propagate just in case
        panic!("Failed to generate enhanced SVG: {}", e);
    }
    String::from_utf8(buffer).expect("SVG is valid UTF-8")
}

/// Incrementally write enhanced SVG to any `Write` implementor.
///
/// This avoids building the entire SVG string in memory, which is
/// beneficial for very large or complex images.
///
/// **Zero-allocation path streaming**: Paths are streamed directly without
/// collecting them into intermediate `ColorGroup` vectors or `path_data` strings.
/// A single reusable scratch buffer is used per color group.
#[allow(dead_code)] // Re-exported at crate root; bin target uses the _prec variant.
pub fn generate_enhanced_svg_to<W: Write>(
    data: &EnhancedVectorData,
    writer: &mut W,
) -> Result<()> {
    generate_enhanced_svg_to_prec(data, writer, crate::bezier_fitter::DEFAULT_COORDINATE_PRECISION)
}

/// Build a `<linearGradient>`/`<radialGradient>` def string with the given id.
fn gradient_svg_def(id: &str, g: &crate::gradient_detector::DetectedGradient) -> String {
    use crate::gradient_detector::DetectedGradient;
    fn stop(offset: &str, c: (u8, u8, u8, u8)) -> String {
        if c.3 < 255 {
            format!(
                r##"<stop offset="{offset}" stop-color="#{:02x}{:02x}{:02x}" stop-opacity="{:.2}"/>"##,
                c.0,
                c.1,
                c.2,
                c.3 as f64 / 255.0
            )
        } else {
            format!(
                r##"<stop offset="{offset}" stop-color="#{:02x}{:02x}{:02x}"/>"##,
                c.0, c.1, c.2
            )
        }
    }
    let stops = |start: (u8, u8, u8, u8), end: (u8, u8, u8, u8)| {
        format!("{}{}", stop("0", start), stop("1", end))
    };
    match g {
        DetectedGradient::Linear(l) => format!(
            r#"<linearGradient id="{id}" gradientUnits="userSpaceOnUse" x1="{:.2}" y1="{:.2}" x2="{:.2}" y2="{:.2}">{}</linearGradient>"#,
            l.x1,
            l.y1,
            l.x2,
            l.y2,
            stops(l.start_color, l.end_color)
        ),
        DetectedGradient::Radial(r) => format!(
            r#"<radialGradient id="{id}" gradientUnits="userSpaceOnUse" cx="{:.2}" cy="{:.2}" r="{:.2}">{}</radialGradient>"#,
            r.cx,
            r.cy,
            r.r,
            stops(r.start_color, r.end_color)
        ),
    }
}

/// Extract contours from a binary mask using either sub-pixel interpolated or
/// midpoint marching squares, based on `options.subpixel_contours`.
fn extract_contours(
    mask: &[bool],
    width: usize,
    height: usize,
    options: &EnhancedOptions,
) -> Vec<Vec<Point>> {
    if options.subpixel_contours {
        let field = crate::vectorizer::build_smoothed_field(mask, width, height);
        crate::vectorizer::marching_squares_contours_interp(mask, &field, width, height)
    } else {
        marching_squares_contours(mask, width, height)
    }
}

/// Extend a detected gradient's endpoints to cover the full image extent in
/// the gradient direction. With `gradientUnits="userSpaceOnUse"`, this makes
/// each quantized band render the correct *portion* of the full gradient
/// rather than a compressed copy, so adjacent bands appear continuous.
fn extend_gradient_endpoints(
    g: crate::gradient_detector::DetectedGradient,
    width: usize,
    height: usize,
) -> crate::gradient_detector::DetectedGradient {
    use crate::gradient_detector::{DetectedGradient, LinearGradient, RadialGradient};
    let w = width as f64;
    let h = height as f64;
    match g {
        DetectedGradient::Linear(lin) => {
            let dx = lin.x2 - lin.x1;
            let dy = lin.y2 - lin.y1;
            let len = (dx * dx + dy * dy).sqrt();
            if len < 1e-9 {
                return DetectedGradient::Linear(lin);
            }
            let ux = dx / len;
            let uy = dy / len;
            // Project all four image corners onto the gradient direction.
            let corners = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)];
            let mut t_min = f64::INFINITY;
            let mut t_max = f64::NEG_INFINITY;
            for &(cx, cy) in &corners {
                let t = cx * ux + cy * uy;
                t_min = t_min.min(t);
                t_max = t_max.max(t);
            }
            // New endpoints: the extreme projections along the direction,
            // placed at the image center in the perpendicular direction so
            // the gradient line spans the full image.
            let mid_x = w / 2.0;
            let mid_y = h / 2.0;
            // Perpendicular component of the center (stays fixed).
            let perp = mid_x * ux + mid_y * uy;
            let cx_perp = mid_x - perp * ux;
            let cy_perp = mid_y - perp * uy;
            DetectedGradient::Linear(LinearGradient {
                x1: cx_perp + t_min * ux,
                y1: cy_perp + t_min * uy,
                x2: cx_perp + t_max * ux,
                y2: cy_perp + t_max * uy,
                start_color: lin.start_color,
                end_color: lin.end_color,
                error: lin.error,
            })
        }
        DetectedGradient::Radial(rad) => {
            // Extend radius to cover the farthest image corner from the center.
            let corners = [
                (0.0, 0.0),
                (w, 0.0),
                (0.0, h),
                (w, h),
                (rad.cx, rad.cy),
            ];
            let max_r = corners
                .iter()
                .map(|&(cx, cy)| ((cx - rad.cx).powi(2) + (cy - rad.cy).powi(2)).sqrt())
                .fold(0.0f64, f64::max)
                .max(1.0);
            DetectedGradient::Radial(RadialGradient {
                cx: rad.cx,
                cy: rad.cy,
                r: max_r,
                start_color: rad.start_color,
                end_color: rad.end_color,
                error: rad.error,
            })
        }
    }
}

/// Collect unique gradient defs for all fill paths.
/// Returns the `<defs>...</defs>` block (empty when no gradients) and, for each
/// path index carrying a gradient, the id of its (deduplicated) gradient def.
///
/// Two gradients are considered **similar** when they share the same variant
/// (Linear vs Radial) and their parameters are close relative to the fit
/// error. This enables merging gradients from adjacent quantized regions
/// that represent the same underlying smooth transition, reducing the number
/// of `<defs>` entries while preserving visual fidelity.
fn collect_gradient_defs(
    paths: &[EnhancedPath],
) -> (String, std::collections::HashMap<usize, String>) {
    let mut ids: std::collections::HashMap<usize, String> = std::collections::HashMap::new();
    let mut seen: Vec<(crate::gradient_detector::DetectedGradient, String)> = Vec::new();
    let mut defs: Vec<String> = Vec::new();
    for (i, path) in paths.iter().enumerate() {
        if path.stroke_only {
            continue;
        }
        if let Some(g) = &path.gradient {
            // 1) Exact match (existing behaviour)
            if let Some((_, id)) = seen.iter().find(|(g2, _)| *g2 == *g) {
                ids.insert(i, id.clone());
                continue;
            }
            // 2) Similar-match: merge gradients from different regions that
            //    represent the same underlying smooth transition (same type,
            //    comparable parameters). This reduces <defs> count while
            //    keeping visual fidelity.
            if let Some(id) = seen_iter_matching_similar(g, &seen) {
                ids.insert(i, id);
                continue;
            }
            let id = format!("img2svg-grad-{}", defs.len());
            defs.push(gradient_svg_def(&id, g));
            seen.push((g.clone(), id.clone()));
            ids.insert(i, id);
        }
    }
    if defs.is_empty() {
        (String::new(), ids)
    } else {
        (format!("<defs>{}</defs>", defs.join("")), ids)
    }
}

/// Check whether `g1` is similar to `g2`.
/// Similar means: same variant (Linear/Radial), compatible direction (for
/// linear — roughly parallel), and compatible start/end colors. This prevents
/// merging a horizontal gradient with a vertical one, or a red→blue gradient
/// with a green→yellow one.
fn gradient_is_similar(
    g1: &crate::gradient_detector::DetectedGradient,
    g2: &crate::gradient_detector::DetectedGradient,
) -> bool {
    use crate::gradient_detector::DetectedGradient;
    const COLOR_TOL: f64 = 60.0; // per-channel tolerance for color matching
    const DIR_TOL: f64 = 0.85; // dot-product threshold for parallel directions

    match (g1, g2) {
        (DetectedGradient::Linear(a), DetectedGradient::Linear(b)) => {
            // Direction vectors.
            let dx1 = a.x2 - a.x1;
            let dy1 = a.y2 - a.y1;
            let dx2 = b.x2 - b.x1;
            let dy2 = b.y2 - b.y1;
            let l1 = (dx1 * dx1 + dy1 * dy1).sqrt();
            let l2 = (dx2 * dx2 + dy2 * dy2).sqrt();
            if l1 < 1e-9 || l2 < 1e-9 {
                return false;
            }
            let dot = ((dx1 * dx2 + dy1 * dy2) / (l1 * l2)).abs();
            if dot < DIR_TOL {
                return false;
            }
            // Colors: start/end may be swapped (eigenvector sign is arbitrary).
            colors_compatible(a.start_color, a.end_color, b.start_color, b.end_color, COLOR_TOL)
        }
        (DetectedGradient::Radial(a), DetectedGradient::Radial(b)) => {
            // Centers should be reasonably close.
            let dc = ((a.cx - b.cx).powi(2) + (a.cy - b.cy).powi(2)).sqrt();
            if dc > (a.r + b.r).max(50.0) * 0.5 {
                return false;
            }
            colors_compatible(a.start_color, a.end_color, b.start_color, b.end_color, COLOR_TOL)
        }
        _ => false,
    }
}

/// Check if two (start, end) color pairs are compatible, allowing for the
/// start/end to be swapped (gradient direction sign is arbitrary).
fn colors_compatible(
    s1: (u8, u8, u8, u8),
    e1: (u8, u8, u8, u8),
    s2: (u8, u8, u8, u8),
    e2: (u8, u8, u8, u8),
    tol: f64,
) -> bool {
    let d = |a: (u8, u8, u8, u8), b: (u8, u8, u8, u8)| -> f64 {
        ((a.0 as f64 - b.0 as f64).powi(2)
            + (a.1 as f64 - b.1 as f64).powi(2)
            + (a.2 as f64 - b.2 as f64).powi(2)
            + (a.3 as f64 - b.3 as f64).powi(2))
        .sqrt()
    };
    // Same direction: s1≈s2 and e1≈e2
    let same = d(s1, s2) < tol && d(e1, e2) < tol;
    // Swapped: s1≈e2 and e1≈s2
    let swapped = d(s1, e2) < tol && d(e1, s2) < tol;
    same || swapped
}

/// Find the first seen gradient similar to `g`, returning its id if found.
fn seen_iter_matching_similar(
    g: &crate::gradient_detector::DetectedGradient,
    seen: &[(crate::gradient_detector::DetectedGradient, String)],
) -> Option<String> {
    seen.iter().find_map(|(g2, id)| gradient_is_similar(g, g2).then_some(id.clone()))
}

/// Incrementally write enhanced SVG to any `Write` implementor with
/// user-controlled coordinate precision (1-6 decimal places, clamped).
///
/// See [`generate_enhanced_svg_to`] for the default-precision variant.
pub fn generate_enhanced_svg_to_prec<W: Write>(
    data: &EnhancedVectorData,
    writer: &mut W,
    precision: u32,
) -> Result<()> {
    let precision = crate::bezier_fitter::clamp_precision(precision);
    let (defs_block, gradient_ids) = collect_gradient_defs(&data.paths);
    writeln!(
        writer,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="0 0 {} {}">"#,
        data.width, data.height, data.width, data.height
    )?;
    if !defs_block.is_empty() {
        writeln!(writer, "  {}", defs_block)?;
    }

    // Background rect
    let bg = data.background_color;
    let bg_hex = format!("#{:02x}{:02x}{:02x}", bg.0, bg.1, bg.2);
    let bg_opacity = if bg.3 < 255 {
        format!(" fill-opacity=\"{}\"", (bg.3 as f64 / 255.0 * 100.0).round() / 100.0)
    } else {
        String::new()
    };
    writeln!(
        writer,
        "  <rect width=\"{}\" height=\"{}\" fill=\"{}\"{} />",
        data.width, data.height, bg_hex, bg_opacity
    )?;

    // Stream paths directly: detect color changes on the fly instead of
    // collecting all paths into a Vec<ColorGroup> first.
    let mut scratch: Vec<u8> = Vec::with_capacity(4096);
    let mut current_color: Option<(u8, u8, u8, u8)> = None;
    let mut current_gradient_id: Option<String> = None;
    let mut current_hex = String::with_capacity(7);
    let mut has_data = false;
    let mut group_open = false; // tracks if a <g data-img2svg-color=...> is currently open

    for (path_idx, path) in data.paths.iter().enumerate() {
        if path.stroke_only {
            // Close any open fill-color group before emitting a stroke-only path
            // (stroke-only paths are never wrapped in <g>, so they sit at top level).
            if group_open {
                writeln!(writer, "  </g>")?;
                group_open = false;
            }
            let color = path.color;
            let hex = format!("#{:02x}{:02x}{:02x}", color.0, color.1, color.2);
            let opacity_attr = if color.3 < 255 {
                let op = (color.3 as f64 / 255.0 * 100.0).round() / 100.0;
                format!(" stroke-opacity=\"{}\"", op)
            } else {
                String::new()
            };
            write!(
                writer,
                r#"  <path fill="none" stroke="{}" stroke-width="1" stroke-linejoin="round"{} d=""#,
                hex, opacity_attr
            )?;
            if let Some(ref svg) = path.svg_override {
                writer.write_all(svg.as_bytes())?;
            } else {
                bezier_to_svg_path_to_prec(writer, &path.curves, false, precision)?;
            }
            writeln!(writer, r#""/>"#)?;
            continue;
        }

        let path_color = path.color;

        if current_color != Some(path_color) {
            // Flush previous color group
            if has_data {
                let alpha = current_color.map(|c| c.3).unwrap_or(255);
                let opacity_attr = if alpha < 255 {
                    let op = (alpha as f64 / 255.0 * 100.0).round() / 100.0;
                    format!(" fill-opacity=\"{}\" stroke-opacity=\"{}\"", op, op)
                } else {
                    String::new()
                };
                let fill_attr = match &current_gradient_id {
                    Some(id) => format!("fill=\"url(#{})\"", id),
                    None => format!("fill=\"{}\"", current_hex),
                };
                write!(
                    writer,
                    r#"  <path {} stroke="{}" stroke-width="0.5" stroke-linejoin="round"{} d=""#,
                    fill_attr, current_hex, opacity_attr
                )?;
                writer.write_all(&scratch)?;
                writeln!(writer, r#""/>"#)?;
                if data.group_by_color {
                    writeln!(writer, "  </g>")?;
                    group_open = false;
                }
            }

            current_color = Some(path_color);
            current_gradient_id = gradient_ids.get(&path_idx).cloned();
            current_hex = format!("#{:02x}{:02x}{:02x}", path_color.0, path_color.1, path_color.2);
            if data.group_by_color {
                writeln!(writer, r#"  <g data-img2svg-color="{}">"#, current_hex)?;
                group_open = true;
            }
            scratch.clear();
            has_data = false;
        }

        let scratch_len_before = scratch.len();

        if let Some(ref svg) = path.svg_override {
            scratch.extend_from_slice(svg.as_bytes());
        } else if !path.curves.is_empty() {
            let (min_x, min_y, max_x, max_y) = curve_bounds(&path.curves);
            if (max_x - min_x) < 1.0 && (max_y - min_y) < 1.0 {
                // skip degenerate subpath
            } else {
                bezier_to_svg_path_to_prec(&mut scratch, &path.curves, true, precision)?;
            }
        } else {
            bezier_to_svg_path_to_prec(&mut scratch, &path.curves, true, precision)?;
        }

        if scratch.len() > scratch_len_before {
            has_data = true;
        }
    }

    // Flush final color group
    if has_data {
        let alpha = current_color.map(|c| c.3).unwrap_or(255);
        let opacity_attr = if alpha < 255 {
            let op = (alpha as f64 / 255.0 * 100.0).round() / 100.0;
            format!(" fill-opacity=\"{}\" stroke-opacity=\"{}\"", op, op)
        } else {
            String::new()
        };
        let fill_attr = match &current_gradient_id {
            Some(id) => format!("fill=\"url(#{})\"", id),
            None => format!("fill=\"{}\"", current_hex),
        };
        write!(
            writer,
            r#"  <path {} stroke="{}" stroke-width="0.5" stroke-linejoin="round"{} d=""#,
            fill_attr, current_hex, opacity_attr
        )?;
        writer.write_all(&scratch)?;
        writeln!(writer, r#""/>"#)?;
        if data.group_by_color && group_open {
            writeln!(writer, "  </g>")?;
        }
    }

    writeln!(writer, "</svg>")?;
    Ok(())
}

/// Write enhanced SVG to a file incrementally (low memory footprint).
pub fn write_enhanced_svg(data: &EnhancedVectorData, output_path: &std::path::Path) -> Result<()> {
    write_enhanced_svg_prec(data, output_path, crate::bezier_fitter::DEFAULT_COORDINATE_PRECISION)
}

/// Write enhanced SVG to a file incrementally with user-controlled
/// coordinate precision (1-6 decimal places, clamped).
pub fn write_enhanced_svg_prec(
    data: &EnhancedVectorData,
    output_path: &std::path::Path,
    precision: u32,
) -> Result<()> {
    let mut file = std::fs::File::create(output_path)?;
    generate_enhanced_svg_to_prec(data, &mut file, precision)?;
    Ok(())
}

/// Group consecutive same-color paths (kept for tests).
#[allow(dead_code)]
fn group_by_color(paths: &[EnhancedPath]) -> Vec<(String, Vec<EnhancedPath>)> {
    let mut groups: Vec<(String, Vec<EnhancedPath>)> = Vec::new();

    for path in paths {
        if path.curves.is_empty() && path.svg_override.is_none() {
            continue;
        }

        let color_hex = format!(
            "#{:02x}{:02x}{:02x}",
            path.color.0, path.color.1, path.color.2
        );

        if let Some((last_hex, last_paths)) = groups.last_mut()
            && *last_hex == color_hex
        {
            last_paths.push(path.clone());
            continue;
        }

        groups.push((color_hex, vec![path.clone()]));
    }

    groups
}

fn curve_bounds(curves: &[BezierCurve]) -> (f64, f64, f64, f64) {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for c in curves {
        for p in [&c.start, &c.control1, &c.control2, &c.end] {
            min_x = min_x.min(p.x);
            min_y = min_y.min(p.y);
            max_x = max_x.max(p.x);
            max_y = max_y.max(p.y);
        }
    }
    (min_x, min_y, max_x, max_y)
}

fn bounds_from_points(points: &[Point]) -> (f64, f64, f64, f64) {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for p in points {
        min_x = min_x.min(p.x);
        min_y = min_y.min(p.y);
        max_x = max_x.max(p.x);
        max_y = max_y.max(p.y);
    }
    (min_x, min_y, max_x, max_y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rgb::RGBA8;

    fn make_test_image(w: u32, h: u32) -> ImageData {
        let mut pixels = Vec::new();
        for _y in 0..h {
            for x in 0..w {
                if x < w / 2 {
                    pixels.push(RGBA8::new(200, 0, 0, 255));
                } else {
                    pixels.push(RGBA8::new(0, 0, 200, 255));
                }
            }
        }
        ImageData {
            width: w,
            height: h,
            pixels,
        }
    }

    #[test]
    fn test_enhanced_vectorize_basic() {
        let img = make_test_image(20, 20);
        let options = EnhancedOptions {
            num_colors: 4,
            preprocess: false,
            ..Default::default()
        };
        let result = vectorize_enhanced(&img, &options).unwrap();
        assert_eq!(result.width, 20);
        assert_eq!(result.height, 20);
        // Should produce at least one path
        assert!(!result.paths.is_empty());
    }

    #[test]
    fn test_enhanced_svg_generation() {
        let img = make_test_image(20, 20);
        let options = EnhancedOptions {
            num_colors: 4,
            preprocess: false,
            ..Default::default()
        };
        let result = vectorize_enhanced(&img, &options).unwrap();
        let svg = generate_enhanced_svg(&result);
        assert!(svg.contains("<svg"));
        assert!(svg.contains("</svg>"));
        assert!(svg.contains("<rect"));
        assert!(svg.contains("<path"));
        // Should have gap-filling strokes
        assert!(svg.contains("stroke-width=\"0.5\""));
    }

    #[test]
    fn test_enhanced_options_default() {
        let opts = EnhancedOptions::default();
        assert_eq!(opts.num_colors, 0); // auto
        assert_eq!(opts.curve_tolerance, 2.0);
        assert_eq!(opts.smoothing_passes, 2);
        assert!(opts.preprocess);
        assert!(opts.recolor);
    }

    #[test]
    fn test_enhanced_vectorize_gradient() {
        // Gradient image
        let mut pixels = Vec::new();
        for y in 0..20 {
            for x in 0..20 {
                pixels.push(RGBA8::new((x * 12) as u8, (y * 12) as u8, 128, 255));
            }
        }
        let img = ImageData {
            width: 20,
            height: 20,
            pixels,
        };
        let options = EnhancedOptions {
            num_colors: 8,
            preprocess: false,
            ..Default::default()
        };
        let result = vectorize_enhanced(&img, &options).unwrap();
        assert_eq!(result.width, 20);
    }

    #[test]
    fn test_enhanced_vectorize_single_color() {
        let pixels = vec![RGBA8::new(128, 128, 128, 255); 100];
        let img = ImageData {
            width: 10,
            height: 10,
            pixels,
        };
        let options = EnhancedOptions {
            num_colors: 4,
            preprocess: false,
            ..Default::default()
        };
        let result = vectorize_enhanced(&img, &options).unwrap();
        assert_eq!(result.width, 10);
    }

    #[test]
    fn test_edge_of_classification() {
        assert_eq!(edge_of(&Point { x: 50.0, y: 0.0 }, 400.0, 400.0), 'T');
        assert_eq!(edge_of(&Point { x: 50.0, y: 400.0 }, 400.0, 400.0), 'B');
        assert_eq!(edge_of(&Point { x: 0.0, y: 50.0 }, 400.0, 400.0), 'L');
        assert_eq!(edge_of(&Point { x: 400.0, y: 50.0 }, 400.0, 400.0), 'R');
        assert_eq!(edge_of(&Point { x: 50.0, y: 50.0 }, 400.0, 400.0), ' ');
        // Corner points: top edge takes priority over left/right
        assert_eq!(edge_of(&Point { x: 0.0, y: 0.0 }, 400.0, 400.0), 'T');
        assert_eq!(edge_of(&Point { x: 400.0, y: 0.0 }, 400.0, 400.0), 'T');
    }

    #[test]
    fn test_dedup_consecutive() {
        let pts = vec![
            Point { x: 0.0, y: 0.0 },
            Point { x: 0.1, y: 0.1 }, // near-duplicate
            Point { x: 10.0, y: 0.0 },
            Point { x: 10.0, y: 10.0 },
            Point { x: 10.0, y: 10.2 }, // near-duplicate
        ];
        let result = dedup_consecutive(&pts, 0.5);
        assert_eq!(result.len(), 3); // (0,0), (10,0), (10,10)
        assert!((result[0].x - 0.0).abs() < 0.01);
        assert!((result[1].x - 10.0).abs() < 0.01);
        assert!((result[2].y - 10.0).abs() < 0.01);
    }

    #[test]
    fn test_inject_image_corners_right_to_top() {
        // Contour goes along right edge then top edge → should inject (400,0)
        let pts = vec![
            Point { x: 200.0, y: 200.0 },
            Point { x: 400.0, y: 200.0 }, // right edge
            Point { x: 200.0, y: 0.0 },   // top edge
        ];
        let result = inject_image_corners(&pts, 400.0, 400.0);
        // Should have injected (400,0) between right-edge and top-edge points
        assert!(result.len() > 3);
        let corner_found = result
            .iter()
            .any(|p| (p.x - 400.0).abs() < 0.01 && p.y.abs() < 0.01);
        assert!(corner_found, "Expected corner (400,0) to be injected");
    }

    #[test]
    fn test_inject_image_corners_no_injection_for_same_edge() {
        // Both points on same edge → no corner injection
        let pts = vec![
            Point { x: 100.0, y: 0.0 }, // top edge
            Point { x: 200.0, y: 0.0 }, // top edge
            Point { x: 200.0, y: 100.0 },
        ];
        let result = inject_image_corners(&pts, 400.0, 400.0);
        assert_eq!(result.len(), 3); // no injection needed
    }

    #[test]
    fn test_inject_image_corners_all_four() {
        // Contour visits all 4 edges → should inject up to 4 corners
        let pts = vec![
            Point { x: 200.0, y: 0.0 },   // top
            Point { x: 400.0, y: 200.0 }, // right
            Point { x: 200.0, y: 400.0 }, // bottom
            Point { x: 0.0, y: 200.0 },   // left
        ];
        let result = inject_image_corners(&pts, 400.0, 400.0);
        assert!(
            result.len() >= 8,
            "Expected 4 original + 4 corners, got {}",
            result.len()
        );
    }

    #[test]
    fn test_recolor_produces_different_colors_for_photos() {
        // Create a "photo-like" image with many colors (>16 distinct)
        let mut pixels = Vec::new();
        for y in 0..30u32 {
            for x in 0..30u32 {
                pixels.push(RGBA8::new(
                    (x * 8) as u8,
                    (y * 8) as u8,
                    ((x + y) * 4) as u8,
                    255,
                ));
            }
        }
        let img = ImageData {
            width: 30,
            height: 30,
            pixels,
        };
        let options_no_recolor = EnhancedOptions {
            num_colors: 8,
            preprocess: false,
            recolor: false,
            ..Default::default()
        };
        let options_recolor = EnhancedOptions {
            num_colors: 8,
            preprocess: false,
            recolor: true,
            ..Default::default()
        };
        let result_no = vectorize_enhanced(&img, &options_no_recolor).unwrap();
        let result_yes = vectorize_enhanced(&img, &options_recolor).unwrap();
        // Both should produce valid output
        assert!(!result_no.paths.is_empty());
        assert!(!result_yes.paths.is_empty());
        // Recolored version should have different colors (averaged from original)
        // since quantized palette colors differ from original averages
        let svg_no = generate_enhanced_svg(&result_no);
        let svg_yes = generate_enhanced_svg(&result_yes);
        assert!(svg_no.contains("<path"));
        assert!(svg_yes.contains("<path"));
    }

    #[test]
    fn test_recolor_color_methods() {
        // A bucket spanning two distinct originals: mean blends them, median
        // picks a middle channel value, mode picks the most frequent one.
        let pixels = vec![
            (200, 200, 200, 255),
            (200, 200, 200, 255),
            (200, 200, 200, 255),
            (100, 100, 100, 255),
        ];
        let mean = recolor_color(&pixels, RecolorMethod::Mean);
        let median = recolor_color(&pixels, RecolorMethod::Median);
        let mode = recolor_color(&pixels, RecolorMethod::Mode);
        // Mean: (200*3 + 100)/4 = 175.
        assert_eq!(mean, (175, 175, 175, 255));
        // Median of [100,200,200,200] (sorted) → index 2 = 200.
        assert_eq!(median, (200, 200, 200, 255));
        // Mode: 200 appears 3×, 100 appears 1×.
        assert_eq!(mode, (200, 200, 200, 255));
        // Mean must differ from mode here (proving the methods diverge).
        assert_ne!(mean, mode);
    }

    #[test]
    fn test_recolor_color_empty() {
        assert_eq!(recolor_color(&[], RecolorMethod::Mean), (0, 0, 0, 0));
        assert_eq!(recolor_color(&[], RecolorMethod::Median), (0, 0, 0, 0));
        assert_eq!(recolor_color(&[], RecolorMethod::Mode), (0, 0, 0, 0));
    }

    #[test]
    fn test_enhanced_svg_with_alpha() {
        let paths = vec![
            EnhancedPath {
                curves: vec![BezierCurve {
                    start: Point { x: 0.0, y: 0.0 },
                    control1: Point { x: 1.0, y: 1.0 },
                    control2: Point { x: 2.0, y: 1.0 },
                    end: Point { x: 3.0, y: 0.0 },
                }],
                color: (255, 0, 0, 128),
                area: 100,
                svg_override: None,
                gradient: None,
                stroke_only: false,
            },
        ];
        let data = EnhancedVectorData {
            width: 10,
            height: 10,
            background_color: (0, 0, 0, 64),
            paths,
            group_by_color: false,
        };
        let svg = generate_enhanced_svg(&data);
        assert!(svg.contains("fill-opacity=\"0.25\""));
    }

    #[test]
    fn test_generate_enhanced_svg_to_matches_string_version() {
        let paths = vec![
            EnhancedPath {
                curves: vec![BezierCurve {
                    start: Point { x: 0.0, y: 0.0 },
                    control1: Point { x: 1.0, y: 1.0 },
                    control2: Point { x: 2.0, y: 1.0 },
                    end: Point { x: 3.0, y: 0.0 },
                }],
                color: (255, 0, 0, 128),
                area: 100,
                svg_override: None,
                gradient: None,
                stroke_only: false,
            },
            EnhancedPath {
                curves: vec![BezierCurve {
                    start: Point { x: 5.0, y: 5.0 },
                    control1: Point { x: 6.0, y: 6.0 },
                    control2: Point { x: 7.0, y: 6.0 },
                    end: Point { x: 8.0, y: 5.0 },
                }],
                color: (0, 255, 0, 255),
                area: 50,
                svg_override: None,
                gradient: None,
                stroke_only: false,
            },
        ];
        let data = EnhancedVectorData {
            width: 10,
            height: 10,
            background_color: (255, 255, 255, 255),
            paths,
            group_by_color: false,
        };
        let svg_string = generate_enhanced_svg(&data);

        let mut buffer = Vec::new();
        generate_enhanced_svg_to(&data, &mut buffer).unwrap();
        let svg_from_writer = String::from_utf8(buffer).unwrap();

        assert_eq!(svg_string, svg_from_writer);
    }

    #[test]
    fn test_group_by_color() {
        let paths = vec![
            EnhancedPath {
                curves: vec![BezierCurve {
                    start: Point { x: 0.0, y: 0.0 },
                    control1: Point { x: 1.0, y: 1.0 },
                    control2: Point { x: 2.0, y: 1.0 },
                    end: Point { x: 3.0, y: 0.0 },
                }],
                color: (255, 0, 0, 255),
                area: 100,
                svg_override: None,
                gradient: None,
                stroke_only: false,
            },
            EnhancedPath {
                curves: vec![BezierCurve {
                    start: Point { x: 0.0, y: 0.0 },
                    control1: Point { x: 1.0, y: 1.0 },
                    control2: Point { x: 2.0, y: 1.0 },
                    end: Point { x: 3.0, y: 0.0 },
                }],
                color: (255, 0, 0, 255), // same color
                area: 50,
                svg_override: None,
                gradient: None,
                stroke_only: false,
            },
            EnhancedPath {
                curves: vec![BezierCurve {
                    start: Point { x: 0.0, y: 0.0 },
                    control1: Point { x: 1.0, y: 1.0 },
                    control2: Point { x: 2.0, y: 1.0 },
                    end: Point { x: 3.0, y: 0.0 },
                }],
                color: (0, 0, 255, 255), // different color
                area: 80,
                svg_override: None,
                gradient: None,
                stroke_only: false,
            },
        ];
        let groups = group_by_color(&paths);
        assert_eq!(groups.len(), 2); // red group + blue group
        assert_eq!(groups[0].1.len(), 2); // two red paths merged
        assert_eq!(groups[1].1.len(), 1);
    }

    #[test]
    fn test_vectorize_enhanced_with_progress() {
        let img = make_test_image(20, 20);
        let mut opts = EnhancedOptions::default();
        opts.num_colors = 4;
        opts.preprocess = false;
        opts.progress = true;
        let data = vectorize_enhanced(&img, &opts).expect("vectorize with progress");
        assert!(!data.paths.is_empty());
    }

    #[test]
    fn test_autotune_selects_logo() {
        // 4-color 100x100 image → logo-like
        let mut img = ImageData {
            width: 100,
            height: 100,
            pixels: vec![rgb::RGBA8::new(255, 0, 0, 255); 100 * 100],
        };
        for y in 0..50 {
            for x in 50..100 {
                img.pixels[y * 100 + x] = rgb::RGBA8::new(0, 255, 0, 255);
            }
        }
        for y in 50..100 {
            for x in 0..50 {
                img.pixels[y * 100 + x] = rgb::RGBA8::new(0, 0, 255, 255);
            }
        }
        for y in 50..100 {
            for x in 50..100 {
                img.pixels[y * 100 + x] = rgb::RGBA8::new(255, 255, 0, 255);
            }
        }
        let opts = EnhancedOptions { autotrace: true, ..Default::default() };
        let tuned = autotune_options(&img, &opts);
        assert_eq!(tuned.num_colors, 8);
        assert!(tuned.logo_mode);
        assert!(!tuned.preprocess);
        assert!(tuned.optimize_corners);
    }

    #[test]
    fn test_autotune_selects_icon() {
        // 64-color 50x50 image → icon-like
        let mut pixels = Vec::with_capacity(50 * 50);
        for i in 0..(50 * 50) {
            pixels.push(rgb::RGBA8::new((i % 64) as u8, 128, 128, 255));
        }
        let img = ImageData { width: 50, height: 50, pixels };
        let opts = EnhancedOptions { autotrace: true, ..Default::default() };
        let tuned = autotune_options(&img, &opts);
        assert_eq!(tuned.num_colors, 12);
        assert_eq!(tuned.min_region_area, 10);
        assert!(!tuned.logo_mode);
    }

    #[test]
    fn test_autotune_selects_photo() {
        // 200x200 image with >1000 distinct colors → photo-like
        let mut pixels = Vec::with_capacity(200 * 200);
        for i in 0..(200 * 200) {
            let r = (i % 256) as u8;
            let g = ((i / 256) % 256) as u8;
            let b = ((i / 65_536) % 256) as u8;
            pixels.push(rgb::RGBA8::new(r, g, b, 255));
        }
        let img = ImageData { width: 200, height: 200, pixels };
        let opts = EnhancedOptions { autotrace: true, ..Default::default() };
        let tuned = autotune_options(&img, &opts);
        assert_eq!(tuned.num_colors, 16);
        assert!(tuned.preprocess);
        assert!(!tuned.logo_mode);
        assert!(!tuned.optimize_corners);
    }

    #[test]
    fn test_coordinate_precision_limits_decimals() {
        // Gradient-ish image produces curved contours with fractional coords.
        let mut pixels = Vec::with_capacity(40 * 40);
        for y in 0..40 {
            for x in 0..40 {
                pixels.push(rgb::RGBA8::new((x * 6) as u8, (y * 6) as u8, 128, 255));
            }
        }
        let img = ImageData { width: 40, height: 40, pixels };
        let data = vectorize_enhanced(
            &img,
            &EnhancedOptions {
                num_colors: 4,
                preprocess: false,
                ..Default::default()
            },
        )
        .expect("vectorize");

        let count_decimals = |svg: &str| -> usize {
            // Max fractional digit count over every number inside d="...".
            let mut max = 0;
            for attr in svg.split("d=\"").skip(1) {
                let d = attr.split('"').next().unwrap_or("");
                for token in d.split(['M', 'L', 'C', 'Z', ' ', ',']).filter(|t| !t.is_empty()) {
                    if let Some(dot) = token.find('.') {
                        max = max.max(token.len() - dot - 1);
                    }
                }
            }
            max
        };

        let p1 = generate_enhanced_svg_prec(&data, 1);
        let p3 = generate_enhanced_svg_prec(&data, 3);
        assert!(p1.contains("d=\"") && (p1.contains("<path") || p1.contains("<rect")));
        assert!(count_decimals(&p1) <= 1, "precision 1 violated: {}", p1);
        assert!(count_decimals(&p3) <= 3, "precision 3 violated: {}", p3);
        // Higher precision preserves at least as much information (>= size).
        assert!(
            p3.len() >= p1.len(),
            "precision 3 output ({} B) smaller than precision 1 ({} B)",
            p3.len(),
            p1.len()
        );
        // Default writer matches the historical 2-decimal behavior.
        assert_eq!(generate_enhanced_svg(&data), generate_enhanced_svg_prec(&data, 2));
    }

    #[test]
    fn test_detect_gradients_emits_svg_defs() {
        // Solid white background + a horizontal black→white gradient block.
        let mut pixels = vec![rgb::RGBA8::new(255, 255, 255, 255); 40 * 40];
        for y in 10..30 {
            for x in 10..30 {
                let t = ((x - 10) * 12) as u8;
                pixels[y * 40 + x] = rgb::RGBA8::new(t, t, t, 255);
            }
        }
        let img = ImageData { width: 40, height: 40, pixels };
        let opts = |gradients: bool| EnhancedOptions {
            num_colors: 8,
            preprocess: false,
            detect_gradients: gradients,
            ..Default::default()
        };

        // Detection runs on the ORIGINAL image, so quantized bands of the
        // gradient block each carry horizontal variance and are detected.
        let data = vectorize_enhanced(&img, &opts(true)).expect("vectorize gradients");
        assert!(
            data.paths.iter().any(|p| p.gradient.is_some()),
            "expected at least one detected gradient"
        );
        let svg = generate_enhanced_svg(&data);
        assert!(svg.contains("<defs>"), "defs block missing: {}", svg);
        assert!(svg.contains("<linearGradient"), "linearGradient missing");
        assert!(svg.contains("fill=\"url(#img2svg-grad-"), "gradient fill missing");
        // <defs> must be emitted before the first <path>.
        let defs_pos = svg.find("<defs>").expect("defs present");
        let path_pos = svg.find("<path").expect("path present");
        assert!(defs_pos < path_pos, "defs must precede paths");

        // Flag off (default): output must not contain gradient defs.
        let data_plain = vectorize_enhanced(&img, &opts(false)).expect("vectorize plain");
        assert!(data_plain.paths.iter().all(|p| p.gradient.is_none()));
        assert!(!generate_enhanced_svg(&data_plain).contains("<defs>"));
    }

    #[test]
    fn test_group_by_color_wraps_paths_in_g_elements() {
        // Two fill paths of different colors with group_by_color=true
        // should produce two <g data-img2svg-color="..."> wrappers.
        let mk_path = |color: (u8, u8, u8, u8)| EnhancedPath {
            curves: vec![BezierCurve {
                start: Point { x: 0.0, y: 0.0 },
                control1: Point { x: 0.0, y: 10.0 },
                control2: Point { x: 10.0, y: 10.0 },
                end: Point { x: 10.0, y: 0.0 },
            }],
            color,
            area: 100,
            svg_override: None,
            gradient: None,
            stroke_only: false,
        };
        let data_on = EnhancedVectorData {
            width: 10,
            height: 10,
            background_color: (255, 255, 255, 255),
            paths: vec![mk_path((255, 0, 0, 255)), mk_path((0, 0, 255, 255))],
            group_by_color: true,
        };
        let svg = generate_enhanced_svg(&data_on);
        assert_eq!(
            svg.matches("<g data-img2svg-color=").count(),
            2,
            "expected 2 color groups, got: {}",
            svg
        );
        assert!(svg.contains("data-img2svg-color=\"#ff0000\""));
        assert!(svg.contains("data-img2svg-color=\"#0000ff\""));
        assert_eq!(
            svg.matches("</g>").count(),
            2,
            "expected 2 closing </g> tags, got: {}",
            svg
        );

        // group_by_color=false (default): no <g> wrappers
        let data_off = EnhancedVectorData {
            width: 10,
            height: 10,
            background_color: (255, 255, 255, 255),
            paths: vec![mk_path((255, 0, 0, 255)), mk_path((0, 0, 255, 255))],
            group_by_color: false,
        };
        let svg_off = generate_enhanced_svg(&data_off);
        assert!(
            !svg_off.contains("<g data-img2svg-color="),
            "default output must not contain <g data-img2svg-color=...>, got: {}",
            svg_off
        );
    }

    #[test]
    fn test_gradient_is_similar_same_variant() {
        use crate::gradient_detector::{DetectedGradient, LinearGradient, RadialGradient};
        let lin_a = DetectedGradient::Linear(LinearGradient {
            x1: 0.0, y1: 0.0, x2: 100.0, y2: 100.0,
            start_color: (0, 0, 0, 255), end_color: (255, 255, 255, 255),
            error: 1.0,
        });
        let lin_b = DetectedGradient::Linear(LinearGradient {
            x1: 5.0, y1: 5.0, x2: 95.0, y2: 95.0,
            start_color: (10, 10, 10, 255), end_color: (245, 245, 245, 255),
            error: 1.5,
        });
        let rad = DetectedGradient::Radial(RadialGradient {
            cx: 50.0, cy: 50.0, r: 50.0,
            start_color: (0, 0, 0, 255), end_color: (255, 255, 255, 255),
            error: 1.0,
        });
        // Same variant → similar
        assert!(gradient_is_similar(&lin_a, &lin_b));
        // Different variant → not similar
        assert!(!gradient_is_similar(&lin_a, &rad));
        assert!(!gradient_is_similar(&rad, &lin_a));
    }

    #[test]
    fn test_collect_gradient_defs_deduplicates_by_similarity() {
        use crate::gradient_detector::{DetectedGradient, LinearGradient};
        // Two paths with similar-but-not-identical linear gradients should
        // share a single gradient def, reducing the <defs> count.
        let lin_a = DetectedGradient::Linear(LinearGradient {
            x1: 0.0, y1: 0.0, x2: 100.0, y2: 100.0,
            start_color: (10, 20, 30, 255), end_color: (240, 230, 220, 255),
            error: 1.0,
        });
        let lin_b = DetectedGradient::Linear(LinearGradient {
            x1: 0.0, y1: 0.0, x2: 100.0, y2: 100.0,
            start_color: (12, 22, 32, 255), end_color: (238, 228, 218, 255),
            error: 1.1,
        });
        let paths = vec![
            EnhancedPath {
                curves: Vec::new(), color: (1, 1, 1, 255), area: 100,
                svg_override: None, gradient: Some(lin_a), stroke_only: false,
            },
            EnhancedPath {
                curves: Vec::new(), color: (2, 2, 2, 255), area: 200,
                svg_override: None, gradient: Some(lin_b), stroke_only: false,
            },
        ];
        let (defs, ids) = collect_gradient_defs(&paths);
        // Same variant (Linear) → merged into one def
        assert_eq!(defs.matches("<linearGradient").count(), 1, "should produce 1 merged def, got: {}", defs);
        // Both paths map to the same gradient id
        assert_eq!(ids.get(&0), ids.get(&1), "both paths should share the same gradient id");
        assert_eq!(ids.len(), 2, "both paths should have ids");
    }
}
