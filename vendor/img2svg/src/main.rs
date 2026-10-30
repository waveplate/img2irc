mod cli;
mod image_processor;
mod svg_generator;
mod vectorizer;
mod preprocessor;
mod edge_detector;
mod enhanced_quantizer;
mod region_extractor;
mod path_simplifier;
mod bezier_fitter;
mod despeckle;
mod image_filters;
mod enhanced_vectorizer;
mod centerline_tracer;
mod logo_pipeline;
mod eps_generator;
mod extra_backends;
mod gradient_detector;
mod tune;

use anyhow::Result;
use cli::{enhanced_preprocess, parse_args, is_supported_image, Cli, OutputFormat};
use eps_generator::{write_enhanced_ai, write_enhanced_eps, write_enhanced_pdf, write_vectorized_eps};
use extra_backends::{write_enhanced_dxf, write_enhanced_geojson, write_enhanced_gimppath};
use preprocessor::{preprocess, PreprocessOptions};
use enhanced_vectorizer::{vectorize_enhanced, write_enhanced_svg_prec, EnhancedOptions, EnhancedVectorData};
use std::path::{Path, PathBuf};

fn is_gif(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("gif"))
        .unwrap_or(false)
}

fn enhanced_options(cli: &Cli) -> EnhancedOptions {
    let coordinate_precision = cli.precision_preset.as_deref().and_then(|p| match p {
        "low" => Some(1),
        "medium" => Some(2),
        "high" => Some(6),
        _ => None,
    }).unwrap_or(cli.precision);
    EnhancedOptions {
        num_colors: cli.colors,
        preprocess: enhanced_preprocess(cli),
        use_superpixel: cli.superpixel,
        centerline: cli.centerline,
        optimize_corners: cli.optimize_corners,
        optimize_curves: cli.optimize_curves,
        optimize_tolerance: cli.optimize_tolerance,
        logo_mode: cli.logo_mode,
        min_region_area: if cli.no_despeckle {
            0
        } else {
            cli.min_region_area
        },
        progress: cli.progress,
        autotrace: cli.autotrace,
        coordinate_precision,
        detect_gradients: cli.gradients,
        curve_tolerance: cli.curve_tolerance,
        group_by_color: cli.group_by_color,
        subpixel_contours: !cli.no_subpixel_contours,
        recolor_method: match cli.recolor_mode.to_lowercase().as_str() {
            "median" => enhanced_vectorizer::RecolorMethod::Median,
            "mode" => enhanced_vectorizer::RecolorMethod::Mode,
            _ => enhanced_vectorizer::RecolorMethod::Mean,
        },
        ..Default::default()
    }
}

fn output_extension(format: OutputFormat) -> &'static str {
    match format {
        OutputFormat::Svg => "svg",
        OutputFormat::Eps => "eps",
        OutputFormat::Pdf => "pdf",
        OutputFormat::Ai => "ai",
        OutputFormat::Dxf => "dxf",
        OutputFormat::GeoJson => "json",
        OutputFormat::Gimppath => "gimppath",
    }
}

fn default_output_path(input: &Path, format: OutputFormat) -> PathBuf {
    let mut path = input.to_path_buf();
    path.set_extension(output_extension(format));
    path
}

fn output_format(cli: &Cli, output_path: &Path) -> OutputFormat {
    if cli.format != OutputFormat::Svg {
        cli.format
    } else {
        OutputFormat::from_path(output_path)
    }
}

fn write_enhanced_output(
    data: &EnhancedVectorData,
    path: &Path,
    format: OutputFormat,
    precision: u32,
) -> Result<()> {
    match format {
        OutputFormat::Svg => write_enhanced_svg_prec(data, path, precision),
        OutputFormat::Eps => write_enhanced_eps(data, path),
        OutputFormat::Pdf => write_enhanced_pdf(data, path),
        OutputFormat::Ai => write_enhanced_ai(data, path),
        OutputFormat::Dxf => write_enhanced_dxf(data, path),
        OutputFormat::GeoJson => write_enhanced_geojson(data, path),
        OutputFormat::Gimppath => write_enhanced_gimppath(data, path),
    }
}

fn write_original_output(
    vectorized: &vectorizer::VectorizedData,
    path: &Path,
    format: OutputFormat,
    advanced: bool,
) -> Result<()> {
    match format {
        OutputFormat::Svg if advanced => svg_generator::generate_svg_advanced(vectorized, path),
        OutputFormat::Svg => svg_generator::generate_svg(vectorized, path),
        OutputFormat::Eps => write_vectorized_eps(vectorized, path),
        OutputFormat::Pdf | OutputFormat::Ai | OutputFormat::Dxf | OutputFormat::GeoJson | OutputFormat::Gimppath => {
            let tmp = path.with_extension("svg");
            svg_generator::generate_svg(vectorized, &tmp)?;
            eprintln!("  Note: PDF/AI/DXF/GeoJSON/Gimppath from original pipeline writes SVG to {}", tmp.display());
            Ok(())
        }
    }
}

/// Process an animated GIF file, outputting numbered SVG frames.
fn process_animated_gif(
    input_path: &Path,
    output_prefix: &Path,
    cli: &Cli,
) -> Result<()> {
    eprintln!("  Loading animated GIF...");
    let frames = image_processor::load_animated_gif(input_path)?;
    let total = frames.len();
    eprintln!("  Found {} frame(s)", total);

    let stem = output_prefix
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy();
    let parent = output_prefix.parent().unwrap_or(Path::new("."));

    for (i, frame) in frames.iter().enumerate() {
        let mut image_data = image_processor::resize_if_needed(frame.image_data.clone(), cli.max_size);

        if cli.preprocess {
            eprintln!("  Frame {}/{}: applying preprocessing...", i + 1, total);
            let opts = PreprocessOptions::photo();
            image_data = preprocess(&image_data, &opts)?;
        }

        let frame_name = format!("{}_{:03}.svg", stem, i + 1);
        let out_path = parent.join(&frame_name);

        if cli.original {
            let vectorized_data = vectorizer::vectorize(
                &image_data,
                cli.colors,
                cli.threshold,
                cli.smooth,
                cli.hierarchical,
            )?;
            if cli.advanced {
                svg_generator::generate_svg_advanced(&vectorized_data, &out_path)?;
            } else {
                svg_generator::generate_svg(&vectorized_data, &out_path)?;
            }
        } else {
            let options = enhanced_options(cli);
            let vector_data = vectorize_enhanced(&image_data, &options)?;
            write_enhanced_output(
                &vector_data,
                &out_path,
                output_format(cli, &out_path),
                options.coordinate_precision,
            )?;
        }

        eprintln!("  Frame {}/{} -> {}", i + 1, total, out_path.display());
    }

    Ok(())
}

/// Process a single image file.
fn process_file(
    input_path: &Path,
    output_path: &Path,
    cli: &Cli,
) -> Result<()> {
    let started = std::time::Instant::now();
    eprintln!("  Loading image...");
    let mut image_data = image_processor::load_image(input_path)?;

    // Auto-resize large images to prevent OOM
    image_data = image_processor::resize_if_needed(image_data, cli.max_size);

    // Apply preprocessing if requested
    if cli.preprocess {
        eprintln!("  Applying edge-preserving smoothing and color reduction...");
        let opts = PreprocessOptions::photo();
        image_data = preprocess(&image_data, &opts)?;
    }

    // Provide hints for photographs
    let unique_colors = image_data.pixels.iter()
        .map(|p| (p.r, p.g, p.b))
        .collect::<std::collections::HashSet<_>>()
        .len();

    if unique_colors > 10000 && !cli.preprocess {
        eprintln!("  Note: photo detected ({} colors). Try --preprocess for better results.", unique_colors);
    }

    if cli.original {
        eprintln!("  Using original pipeline (line segments, RDP simplification)...");
        let vectorized_data = vectorizer::vectorize(
            &image_data,
            cli.colors,
            cli.threshold,
            cli.smooth,
            cli.hierarchical,
        )?;

        eprintln!("  Generating SVG...");
        if cli.advanced {
            write_original_output(&vectorized_data, output_path, output_format(cli, output_path), true)?;
        } else {
            write_original_output(&vectorized_data, output_path, output_format(cli, output_path), false)?;
        }

        if cli.stats {
            let mut colors = std::collections::HashSet::new();
            for curve in &vectorized_data.curves {
                colors.insert(curve.color);
            }
            print_stats(
                output_path,
                image_data.width,
                image_data.height,
                vectorized_data.curves.len(),
                colors.len(),
                started,
            );
        }
    } else {
        let options = enhanced_options(cli);
        eprintln!("  Vectorizing with enhanced pipeline...");
        let vector_data = vectorize_enhanced(&image_data, &options)?;
        eprintln!("  Generating output...");
        write_enhanced_output(
            &vector_data,
            output_path,
            output_format(cli, output_path),
            options.coordinate_precision,
        )?;
        eprintln!(
            "  {} paths, background #{:02x}{:02x}{:02x}",
            vector_data.paths.len(),
            vector_data.background_color.0,
            vector_data.background_color.1,
            vector_data.background_color.2,
        );

        if cli.stats {
            let mut colors = std::collections::HashSet::new();
            colors.insert(vector_data.background_color);
            for path in &vector_data.paths {
                colors.insert(path.color);
            }
            print_stats(
                output_path,
                vector_data.width,
                vector_data.height,
                vector_data.paths.len(),
                colors.len(),
                started,
            );
        }
    }

    Ok(())
}

/// Print a one-line conversion summary (file size, dimensions, paths, colors, elapsed time).
fn print_stats(
    output_path: &Path,
    width: u32,
    height: u32,
    paths: usize,
    colors: usize,
    started: std::time::Instant,
) {
    let out_size = std::fs::metadata(output_path).map(|m| m.len()).unwrap_or(0);
    let size_str = if out_size >= 1024 * 1024 {
        format!("{:.1} MB", out_size as f64 / (1024.0 * 1024.0))
    } else if out_size >= 1024 {
        format!("{:.1} KB", out_size as f64 / 1024.0)
    } else {
        format!("{} B", out_size)
    };
    eprintln!(
        "  Stats: {}x{}, {} paths, {} colors, {} output, {} ms",
        width,
        height,
        paths,
        colors,
        size_str,
        started.elapsed().as_millis()
    );
}

/// Combine every supported image in `input_dir` into a single multi-page PDF.
///
/// Triggered by `img2svg -i dir/ -o combined.pdf` (input is a directory, output
/// is a single `.pdf` file). Each image becomes one page using its own dimensions.
fn combine_directory_to_pdf(input_dir: &Path, output_path: &Path, cli: &Cli) -> Result<()> {
    let entries: Vec<_> = std::fs::read_dir(input_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file() && is_supported_image(&e.path()))
        .collect();

    let total = entries.len();
    eprintln!(
        "Combining {} images from {} into multi-page PDF {}...",
        total,
        input_dir.display(),
        output_path.display()
    );

    let options = enhanced_options(cli);
    let mut pages: Vec<EnhancedVectorData> = Vec::with_capacity(total);
    let mut converted = 0u32;
    let mut errors = 0u32;

    for entry in &entries {
        let path = entry.path();
        eprintln!("[{}/{}] {}", converted + errors + 1, total, path.display());
        let result = (|| -> Result<()> {
            let mut image_data = image_processor::load_image(&path)?;
            image_data = image_processor::resize_if_needed(image_data, cli.max_size);
            if cli.preprocess {
                let opts = PreprocessOptions::photo();
                image_data = preprocess(&image_data, &opts)?;
            }
            let data = vectorize_enhanced(&image_data, &options)?;
            pages.push(data);
            Ok(())
        })();
        match result {
            Ok(()) => converted += 1,
            Err(e) => {
                eprintln!("  Error: {}", e);
                errors += 1;
            }
        }
    }

    if pages.is_empty() {
        anyhow::bail!("no images converted; PDF not written");
    }

    eps_generator::write_multi_page_pdf(&pages, output_path)?;
    println!(
        "Multi-page PDF written to {} ({} pages, {} errors).",
        output_path.display(),
        converted,
        errors
    );
    Ok(())
}

fn main() -> Result<()> {
    let cli = parse_args(std::env::args());

    if cli.tune {
        let output_path = cli.output.clone().unwrap_or_else(|| {
            let mut p = cli.input.clone();
            p.set_extension("html");
            if p.extension().is_none() {
                p = PathBuf::from("tune.html");
            }
            p
        });
        eprintln!("Generating tune HTML at {}...", output_path.display());
        tune::generate_tune_html(&cli.input, &output_path)?;
        println!("Tune page written to {}", output_path.display());
        return Ok(());
    }

    if cli.input.is_dir() {
        // Multi-page PDF combine mode: input is a directory and the output path is
        // a single .pdf file (not an existing directory). Combines all images into
        // one multi-page PDF, one image per page. Triggered by an explicit
        // `--format pdf` or by a `.pdf` output extension.
        if let Some(out) = &cli.output {
            let out_ext_is_pdf = out
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("pdf"))
                .unwrap_or(false);
            let effective_format_is_pdf =
                cli.format == OutputFormat::Pdf || out_ext_is_pdf;
            if effective_format_is_pdf && out_ext_is_pdf && !out.is_dir() {
                return combine_directory_to_pdf(&cli.input, out, &cli);
            }
        }

        // Batch mode: process all supported images in directory
        let output_dir = cli.output.clone().unwrap_or_else(|| cli.input.clone());
        if !output_dir.exists() {
            std::fs::create_dir_all(&output_dir)?;
        }

        let mut count = 0u32;
        let mut errors = 0u32;
        let entries: Vec<_> = std::fs::read_dir(&cli.input)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file() && is_supported_image(&e.path()))
            .collect();

        let total = entries.len();
        eprintln!("Batch converting {} images from {}...", total, cli.input.display());

        for entry in &entries {
            let path = entry.path();
            let stem = path.file_stem().unwrap().to_string_lossy();
            let mut out_path = output_dir.join(stem.as_ref());
            out_path.set_extension(output_extension(cli.format));

            eprintln!("[{}/{}] {} -> {}", count + errors + 1, total, path.display(), out_path.display());
            let result = if is_gif(&path) {
                process_animated_gif(&path, &out_path, &cli)
            } else {
                process_file(&path, &out_path, &cli)
            };
            match result {
                Ok(()) => count += 1,
                Err(e) => {
                    eprintln!("  Error: {}", e);
                    errors += 1;
                }
            }
        }

        println!("Batch complete: {} converted, {} errors.", count, errors);
    } else {
        // Single file mode
        let output_path = cli.output.clone().unwrap_or_else(|| default_output_path(&cli.input, cli.format));

        println!("Converting {} to {}...", cli.input.display(), output_path.display());

        if is_gif(&cli.input) {
            process_animated_gif(&cli.input, &output_path, &cli)?;
        } else {
            process_file(&cli.input, &output_path, &cli)?;
        }

        println!("Conversion complete!");
    }

    Ok(())
}
