use clap::parser::ValueSource;
use clap::{ArgMatches, CommandFactory, FromArgMatches, Parser, ValueEnum};
use std::path::PathBuf;

/// Tuned parameter bundles for common conversion scenarios.
#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
pub enum Preset {
    /// Flat logos and brand marks: fewer colors, light smoothing, no photo preprocessing.
    Logo,
    /// Photographs: preprocessing enabled, moderate palette, tuned threshold.
    Photo,
    /// Small UI icons: compact palette, light smoothing.
    Icon,
}

impl Preset {
    fn defaults(self) -> PresetDefaults {
        match self {
            Preset::Logo => PresetDefaults {
                colors: 8,
                smooth: 2,
                threshold: 0.15,
                preprocess: false,
            },
            Preset::Photo => PresetDefaults {
                colors: 12,
                smooth: 3,
                threshold: 0.15,
                preprocess: true,
            },
            Preset::Icon => PresetDefaults {
                colors: 12,
                smooth: 2,
                threshold: 0.1,
                preprocess: false,
            },
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct PresetDefaults {
    colors: usize,
    smooth: u8,
    threshold: f64,
    preprocess: bool,
}

#[derive(Parser)]
#[command(name = "img2svg")]
#[command(about = "A high-quality image to SVG converter with Bézier curves")]
#[command(version)]
pub struct Cli {
    /// Input image file or directory (batch mode)
    #[arg(short, long)]
    pub input: PathBuf,

    /// Output SVG file or directory (batch mode)
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Maximum image dimension (auto-resize larger images to prevent OOM)
    #[arg(long, default_value = "4096")]
    pub max_size: u32,

    /// Number of colors to quantize (default: 16)
    #[arg(short, long, default_value = "16")]
    pub colors: usize,

    /// Edge detection threshold (0.0-1.0, default: 0.1)
    #[arg(short, long, default_value = "0.1")]
    pub threshold: f64,

    /// Path smoothing level (0-10, default: 5)
    #[arg(short = 's', long, default_value = "5")]
    pub smooth: u8,

    /// Enable hierarchical decomposition for better quality
    #[arg(long)]
    pub hierarchical: bool,

    /// Use advanced SVG generation with layers
    #[arg(short, long)]
    pub advanced: bool,

    /// Apply preprocessing (edge-preserving smoothing + color reduction) for photos
    #[arg(short = 'p', long)]
    pub preprocess: bool,

    /// Use original pipeline (line segments, RDP simplification) instead of default Bézier
    #[arg(long)]
    pub original: bool,

    /// Preset tuned for common use cases (explicit -c/-s/-t/-p override preset values)
    #[arg(long, value_enum)]
    pub preset: Option<Preset>,

    /// Use SLIC superpixel segmentation instead of k-means quantization
    #[arg(long)]
    pub superpixel: bool,

    /// Centerline/stroke tracing mode for line art
    #[arg(long)]
    pub centerline: bool,

    /// Potrace-style Bézier corner optimization
    #[arg(long)]
    pub optimize_corners: bool,

    /// Potrace `-O` style merge of consecutive nearly-linear curve segments
    #[arg(long)]
    pub optimize_curves: bool,

    /// Merge tolerance in pixels for --optimize-curves (default: 1.5; higher merges more)
    #[arg(long, default_value = "1.5")]
    pub optimize_tolerance: f64,

    /// Logo mode: separate fill regions and outline strokes
    #[arg(long)]
    pub logo_mode: bool,

    /// Minimum connected region area in pixels; smaller islands are merged (Potrace turdsize). Set to 0 or use --no-despeckle to disable.
    #[arg(long, default_value = "20")]
    pub min_region_area: usize,

    /// Disable despeckle pass (equivalent to --min-region-area 0)
    #[arg(long)]
    pub no_despeckle: bool,

    /// Generate HTML parameter comparison grid instead of converting
    #[arg(long)]
    pub tune: bool,

    /// Show a progress bar during vectorization
    #[arg(long)]
    pub progress: bool,

    /// Auto-select tuning parameters based on image characteristics
    #[arg(long)]
    pub autotrace: bool,

    /// Output format (default: svg, or inferred from file extension)
    #[arg(long, value_enum, default_value = "svg")]
    pub format: OutputFormat,

    /// Decimal places for SVG path coordinates (1-6, default: 2)
    #[arg(long, default_value = "2", value_parser = clap::value_parser!(u32).range(1..=6))]
    pub precision: u32,

    /// Precision preset for SVG path coordinates (1=low, 2=medium/default, 6=high).
    /// Overrides --precision when set.
    #[arg(long = "quality-preset")]
    pub precision_preset: Option<String>,

    /// Curve merge tolerance in pixels for --optimize-curves.
    /// Higher values merge more consecutive nearly-linear segments (default: 2.0).
    /// Only effective when --optimize-curves is also set.
    #[arg(long, default_value = "2.0")]
    pub curve_tolerance: f64,

    /// Print conversion stats (dimensions, paths, colors, output size, time) after conversion
    #[arg(long)]
    pub stats: bool,

    /// Detect smooth gradients per region and render them as native SVG gradient fills
    #[arg(long)]
    pub gradients: bool,

    /// Wrap each color group in an SVG `<g>` element with a
    /// `data-img2svg-color="#hex"` attribute. Produces Inkscape-style
    /// hierarchical layers that are easier to edit/restyle in vector tools.
    #[arg(long)]
    pub group_by_color: bool,

    /// Disable interpolated (sub-pixel) marching squares and fall back to plain
    /// midpoint contour placement. Sub-pixel contours are on by default and
    /// produce smoother boundaries on diagonal/curved edges.
    #[arg(long = "no-subpixel-contours")]
    pub no_subpixel_contours: bool,

    /// Method used to derive each region's display color from the original
    /// image pixels: mean (default), median, or mode. Median/mode avoid the
    /// muddy averaged colors mean produces when a quantized bucket spans
    /// several distinct originals (common in flat-color logos/icons).
    #[arg(long = "recolor-mode", default_value = "mean")]
    pub recolor_mode: String,
}

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq, Default)]
pub enum OutputFormat {
    #[default]
    Svg,
    Eps,
    Pdf,
    Ai,
    Dxf,
    GeoJson,
    Gimppath,
}

impl OutputFormat {
    pub fn from_path(path: &std::path::Path) -> Self {
        match path.extension().and_then(|e| e.to_str()).map(|s| s.to_lowercase()) {
            Some(ext) if ext == "eps" => OutputFormat::Eps,
            Some(ext) if ext == "pdf" => OutputFormat::Pdf,
            Some(ext) if ext == "ai" => OutputFormat::Ai,
            Some(ext) if ext == "dxf" => OutputFormat::Dxf,
            Some(ext) if ext == "json" => OutputFormat::GeoJson,
            Some(ext) if ext == "gimppath" => OutputFormat::Gimppath,
            _ => OutputFormat::Svg,
        }
    }
}

/// Parse CLI arguments and apply preset defaults where flags were not explicitly set.
pub fn parse_args<I, T>(args: I) -> Cli
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let matches = Cli::command().get_matches_from(args);
    let mut cli = Cli::from_arg_matches(&matches).expect("validated matches");
    if let Some(preset) = cli.preset {
        apply_preset(&mut cli, preset, &matches);
    }
    cli
}

fn apply_preset(cli: &mut Cli, preset: Preset, matches: &ArgMatches) {
    let d = preset.defaults();
    if !arg_from_command_line(matches, "colors") {
        cli.colors = d.colors;
    }
    if !arg_from_command_line(matches, "smooth") {
        cli.smooth = d.smooth;
    }
    if !arg_from_command_line(matches, "threshold") {
        cli.threshold = d.threshold;
    }
    if !arg_from_command_line(matches, "preprocess") {
        cli.preprocess = d.preprocess;
    }
    if preset == Preset::Logo && !arg_from_command_line(matches, "logo_mode") {
        cli.logo_mode = true;
    }
}

fn arg_from_command_line(matches: &ArgMatches, id: &str) -> bool {
    matches
        .value_source(id)
        .is_some_and(|s| s == ValueSource::CommandLine)
}

/// Enhanced pipeline preprocess flag: honor preset, explicit `-p`, or default auto-preprocess.
pub fn enhanced_preprocess(cli: &Cli) -> bool {
    if cli.preprocess {
        return true;
    }
    match cli.preset {
        Some(Preset::Logo | Preset::Icon) => false,
        Some(Preset::Photo) => true,
        None => true,
    }
}

/// Check if a file extension is a supported image format.
pub fn is_supported_image(path: &std::path::Path) -> bool {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        matches!(
            ext.to_lowercase().as_str(),
            "bmp" | "png" | "jpg" | "jpeg" | "gif" | "ico" | "tiff" | "tif" | "webp" | "pnm" | "tga" | "dds" | "farbfeld"
        )
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_logo_sets_recommended_defaults() {
        let cli = parse_args(["img2svg", "-i", "logo.png", "--preset", "logo"]);
        assert_eq!(cli.colors, 8);
        assert_eq!(cli.smooth, 2);
        assert!((cli.threshold - 0.15).abs() < f64::EPSILON);
        assert!(!cli.preprocess);
        assert!(cli.logo_mode);
    }

    #[test]
    fn preset_photo_enables_preprocess() {
        let cli = parse_args(["img2svg", "-i", "photo.jpg", "--preset", "photo"]);
        assert_eq!(cli.colors, 12);
        assert_eq!(cli.smooth, 3);
        assert!(cli.preprocess);
    }

    #[test]
    fn preset_icon_settings() {
        let cli = parse_args(["img2svg", "-i", "icon.png", "--preset", "icon"]);
        assert_eq!(cli.colors, 12);
        assert_eq!(cli.smooth, 2);
        assert!(!cli.preprocess);
    }

    #[test]
    fn explicit_colors_override_preset() {
        let cli = parse_args([
            "img2svg",
            "-i",
            "logo.png",
            "--preset",
            "logo",
            "-c",
            "32",
        ]);
        assert_eq!(cli.colors, 32);
        assert_eq!(cli.smooth, 2);
    }

    #[test]
    fn enhanced_preprocess_respects_logo_preset() {
        let cli = parse_args(["img2svg", "-i", "logo.png", "--preset", "logo"]);
        assert!(!enhanced_preprocess(&cli));
    }

    #[test]
    fn subpixel_contours_default_on_negation_flag_off() {
        let cli = parse_args(["img2svg", "-i", "x.png"]);
        assert!(!cli.no_subpixel_contours);
        let cli = parse_args(["img2svg", "-i", "x.png", "--no-subpixel-contours"]);
        assert!(cli.no_subpixel_contours);
    }

    #[test]
    fn recolor_mode_default_and_overrides() {
        let cli = parse_args(["img2svg", "-i", "x.png"]);
        assert_eq!(cli.recolor_mode, "mean");
        let cli = parse_args(["img2svg", "-i", "x.png", "--recolor-mode", "mode"]);
        assert_eq!(cli.recolor_mode, "mode");
        let cli = parse_args(["img2svg", "-i", "x.png", "--recolor-mode", "median"]);
        assert_eq!(cli.recolor_mode, "median");
    }

    #[test]
    fn supported_extensions_lowercase() {
        assert!(is_supported_image(std::path::Path::new("x.png")));
        assert!(is_supported_image(std::path::Path::new("x.jpeg")));
        assert!(is_supported_image(std::path::Path::new("x.webp")));
        assert!(is_supported_image(std::path::Path::new("x.tiff")));
    }

    #[test]
    fn supported_extensions_uppercase() {
        assert!(is_supported_image(std::path::Path::new("photo.PNG")));
        assert!(is_supported_image(std::path::Path::new("photo.JPG")));
    }

    #[test]
    fn unsupported_or_missing_extension() {
        assert!(!is_supported_image(std::path::Path::new("readme")));
        assert!(!is_supported_image(std::path::Path::new("file.txt")));
        assert!(!is_supported_image(std::path::Path::new("noext")));
    }

    #[test]
    fn progress_flag_parses() {
        let cli = parse_args(["img2svg", "-i", "logo.png", "--progress"]);
        assert!(cli.progress);
    }

    #[test]
    fn autotrace_flag_parses() {
        let cli = parse_args(["img2svg", "-i", "logo.png", "--autotrace"]);
        assert!(cli.autotrace);
    }

    #[test]
    fn format_geojson_parses() {
        let cli = parse_args(["img2svg", "-i", "map.png", "-o", "map.json", "--format", "geo-json"]);
        assert_eq!(cli.format, OutputFormat::GeoJson);
    }

    #[test]
    fn format_dxf_inferred_from_output_path() {
        let cli = parse_args(["img2svg", "-i", "drawing.png", "-o", "drawing.dxf"]);
        assert_eq!(OutputFormat::from_path(&cli.output.clone().unwrap()), OutputFormat::Dxf);
    }

    #[test]
    fn format_gimppath_parses() {
        let cli = parse_args(["img2svg", "-i", "shape.png", "--format", "gimppath"]);
        assert_eq!(cli.format, OutputFormat::Gimppath);
    }

    #[test]
    fn precision_defaults_to_2() {
        let cli = parse_args(["img2svg", "-i", "logo.png"]);
        assert_eq!(cli.precision, 2);
    }

    #[test]
    fn precision_flag_parses() {
        let cli = parse_args(["img2svg", "-i", "logo.png", "--precision", "1"]);
        assert_eq!(cli.precision, 1);
    }

    #[test]
    fn precision_out_of_range_rejected() {
        let result = Cli::command().try_get_matches_from([
            "img2svg",
            "-i",
            "logo.png",
            "--precision",
            "7",
        ]);
        assert!(result.is_err(), "precision 7 should be rejected");
    }

    #[test]
    fn stats_flag_parses() {
        let cli = parse_args(["img2svg", "-i", "logo.png", "--stats"]);
        assert!(cli.stats);
        let default = parse_args(["img2svg", "-i", "logo.png"]);
        assert!(!default.stats);
    }

    #[test]
    fn optimize_tolerance_defaults_and_parses() {
        let default = parse_args(["img2svg", "-i", "logo.png"]);
        assert!((default.optimize_tolerance - 1.5).abs() < f64::EPSILON);
        let custom = parse_args([
            "img2svg",
            "-i",
            "logo.png",
            "--optimize-curves",
            "--optimize-tolerance",
            "3",
        ]);
        assert!(custom.optimize_curves);
        assert!((custom.optimize_tolerance - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn gradients_flag_parses() {
        let cli = parse_args(["img2svg", "-i", "gradient.png", "--gradients"]);
        assert!(cli.gradients);
        let default = parse_args(["img2svg", "-i", "gradient.png"]);
        assert!(!default.gradients);
    }
}
