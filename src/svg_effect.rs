//! In-memory img2svg tracing followed by rasterization at the source dimensions.
use crate::pipeline::EffectParameter;
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Preset {
    #[default]
    None,
    Logo,
    Photo,
    Icon,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Recolour {
    #[default]
    Mean,
    Median,
    Mode,
}

/// Rendering options shared by the image CLI, video CLI and pipeline editor.
/// Optional fields preserve explicit overrides of a preset, including values
/// equal to the ordinary defaults.
/// Keep clap's internal IDs namespaced as well as the flags: flattening shares
/// the parent's ID space, which already contains `smooth` with a different type.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, clap::Args)]
#[serde(default)]
#[group(id = "img2svg-options")]
pub struct Options {
    /// img2svg working resolution; larger inputs are downscaled for tracing (1-8192).
    #[arg(id = "img2svg-max-size", long = "img2svg-max-size", default_value_t = 4096, value_parser = clap::value_parser!(u32).range(1..=8192))]
    pub max_size: u32,
    /// img2svg palette size (0=automatic; default 16, or the selected preset).
    #[arg(id = "img2svg-colors", long = "img2svg-colors", visible_alias = "img2svg-colours", value_parser = clap::value_parser!(u32).range(0..=256))]
    pub colors: Option<u32>,
    /// Edge threshold from 0 to 1 (default 0.1, or the selected preset).
    #[arg(id = "img2svg-threshold", long = "img2svg-threshold", value_parser = unit_interval)]
    pub threshold: Option<f32>,
    /// Original-pipeline path smoothing, 0-10 (default 5, or the selected preset).
    #[arg(id = "img2svg-smooth", long = "img2svg-smooth", value_parser = clap::value_parser!(u8).range(0..=10))]
    pub smooth: Option<u8>,
    /// img2svg tuning preset; individual options override it.
    #[arg(
        id = "img2svg-preset",
        long = "img2svg-preset",
        value_enum,
        default_value = "none"
    )]
    pub preset: Preset,
    /// Edge-preserving photo preprocessing (accepts true/false; default on for Bezier).
    #[arg(id = "img2svg-preprocess", long = "img2svg-preprocess", num_args = 0..=1, default_missing_value = "true")]
    pub preprocess: Option<bool>,
    /// Use img2svg's original line-segment tracer instead of Bezier curves.
    #[arg(id = "img2svg-original", long = "img2svg-original")]
    pub original: bool,
    /// Preserve nested contours in the original tracer.
    #[arg(id = "img2svg-hierarchical", long = "img2svg-hierarchical")]
    pub hierarchical: bool,
    /// Use SLIC superpixel quantization instead of edge-aware k-means.
    #[arg(id = "img2svg-superpixel", long = "img2svg-superpixel")]
    pub superpixel: bool,
    /// Trace open centerlines/strokes for line art.
    #[arg(id = "img2svg-centerline", long = "img2svg-centerline")]
    pub centerline: bool,
    /// Optimize Bezier corners.
    #[arg(id = "img2svg-optimize-corners", long = "img2svg-optimize-corners")]
    pub optimize_corners: bool,
    /// Merge consecutive nearly linear Bezier segments.
    #[arg(id = "img2svg-optimize-curves", long = "img2svg-optimize-curves")]
    pub optimize_curves: bool,
    /// Curve-merge tolerance in source pixels.
    #[arg(id = "img2svg-optimize-tolerance", long = "img2svg-optimize-tolerance", default_value_t = 1.5, value_parser = nonnegative)]
    pub optimize_tolerance: f32,
    /// Separate logo fills and outlines (accepts true/false; logo preset enables it).
    #[arg(id = "img2svg-logo-mode", long = "img2svg-logo-mode", num_args = 0..=1, default_missing_value = "true")]
    pub logo_mode: Option<bool>,
    /// Merge colour islands smaller than this many tracing pixels; 0 disables it.
    #[arg(
        id = "img2svg-min-region-area",
        long = "img2svg-min-region-area",
        default_value_t = 20
    )]
    pub min_region_area: u32,
    /// Disable removal of small colour islands.
    #[arg(id = "img2svg-no-despeckle", long = "img2svg-no-despeckle")]
    pub no_despeckle: bool,
    /// Let img2svg override tuning parameters using its image heuristics.
    #[arg(id = "img2svg-autotrace", long = "img2svg-autotrace")]
    pub autotrace: bool,
    /// SVG coordinate precision: 1-6, or low/medium/high (1/2/6).
    #[arg(id = "img2svg-precision", long = "img2svg-precision", visible_alias = "img2svg-quality-preset", default_value_t = 2, value_parser = precision)]
    pub precision: u32,
    /// Bezier fitting tolerance in source pixels; lower preserves more detail.
    #[arg(id = "img2svg-curve-tolerance", long = "img2svg-curve-tolerance", default_value_t = 2.0, value_parser = nonnegative)]
    pub curve_tolerance: f32,
    /// Fit gradients inside traced regions instead of using only solid fills.
    #[arg(id = "img2svg-gradients", long = "img2svg-gradients")]
    pub gradients: bool,
    /// Group SVG paths by colour (does not change the rasterized appearance).
    #[arg(id = "img2svg-group-by-color", long = "img2svg-group-by-color")]
    pub group_by_color: bool,
    /// Use midpoint contours instead of subpixel contours.
    #[arg(
        id = "img2svg-no-subpixel-contours",
        long = "img2svg-no-subpixel-contours"
    )]
    pub no_subpixel_contours: bool,
    /// Representative region colour: mean, median or most common original colour.
    #[arg(
        id = "img2svg-recolor-mode",
        long = "img2svg-recolor-mode",
        value_enum,
        default_value = "mean"
    )]
    pub recolor_mode: Recolour,
    /// Majority-vote smoothing passes during Bezier-pipeline quantization.
    #[arg(id = "img2svg-smoothing-passes", long = "img2svg-smoothing-passes", default_value_t = 2, value_parser = clap::value_parser!(u32).range(0..=20))]
    pub smoothing_passes: u32,
    /// Window size for smoothing Bezier contour points.
    #[arg(id = "img2svg-smooth-window", long = "img2svg-smooth-window", default_value_t = 3, value_parser = clap::value_parser!(u32).range(1..=31))]
    pub smooth_window: u32,
    /// Bezier path simplification tolerance in source pixels.
    #[arg(id = "img2svg-simplification-tolerance", long = "img2svg-simplification-tolerance", default_value_t = 1.5, value_parser = nonnegative)]
    pub simplification_tolerance: f32,
    /// Corner detection threshold in degrees (0-180).
    #[arg(id = "img2svg-corner-threshold", long = "img2svg-corner-threshold", default_value_t = 60.0, value_parser = degrees)]
    pub corner_threshold: f32,
    /// Derive display colours from original pixels (true/false).
    #[arg(id = "img2svg-recolor", long = "img2svg-recolor", default_value_t = true, action = clap::ArgAction::Set)]
    pub recolor: bool,
    /// Maximum mean squared error accepted by gradient fitting.
    #[arg(id = "img2svg-gradient-error", long = "img2svg-gradient-error", default_value_t = 50.0, value_parser = nonnegative)]
    pub gradient_error: f32,
}

impl Default for Options {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl Options {
    pub const DEFAULT: Self = Self {
        max_size: 4096,
        colors: None,
        threshold: None,
        smooth: None,
        preset: Preset::None,
        preprocess: None,
        original: false,
        hierarchical: false,
        superpixel: false,
        centerline: false,
        optimize_corners: false,
        optimize_curves: false,
        optimize_tolerance: 1.5,
        logo_mode: None,
        min_region_area: 20,
        no_despeckle: false,
        autotrace: false,
        precision: 2,
        curve_tolerance: 2.0,
        gradients: false,
        group_by_color: false,
        no_subpixel_contours: false,
        recolor_mode: Recolour::Mean,
        smoothing_passes: 2,
        smooth_window: 3,
        simplification_tolerance: 1.5,
        corner_threshold: 60.0,
        recolor: true,
        gradient_error: 50.0,
    };

    pub fn colors(&self) -> u32 {
        self.colors.unwrap_or(match self.preset {
            Preset::None => 16,
            Preset::Logo => 8,
            Preset::Photo | Preset::Icon => 12,
        })
    }
    fn threshold(&self) -> f32 {
        self.threshold.unwrap_or(match self.preset {
            Preset::Logo | Preset::Photo => 0.15,
            _ => 0.1,
        })
    }
    fn smooth(&self) -> u8 {
        self.smooth.unwrap_or(match self.preset {
            Preset::None => 5,
            Preset::Photo => 3,
            _ => 2,
        })
    }
    fn preprocess(&self) -> bool {
        self.preprocess.unwrap_or(match self.preset {
            Preset::Logo | Preset::Icon => false,
            Preset::Photo => true,
            Preset::None => !self.original,
        })
    }
    fn logo_mode(&self) -> bool {
        self.logo_mode.unwrap_or(self.preset == Preset::Logo)
    }

    pub fn parameters(&self) -> Vec<EffectParameter> {
        let p = |label, value, min, max, step| EffectParameter {
            label,
            value,
            min,
            max,
            step,
        };
        vec![
            p("Colours (0=auto)", self.colors() as f32, 0.0, 256.0, 1.0),
            p("Preset", self.preset as u8 as f32, 0.0, 3.0, 1.0),
            p("Working max pixels", self.max_size as f32, 1.0, 8192.0, 1.0),
            p("Edge threshold", self.threshold(), 0.0, 1.0, 0.01),
            p(
                "Recolour method",
                self.recolor_mode as u8 as f32,
                0.0,
                2.0,
                1.0,
            ),
            p("Preprocess", self.preprocess() as u8 as f32, 0.0, 1.0, 1.0),
            p("Original tracer", self.original as u8 as f32, 0.0, 1.0, 1.0),
            p("Original smoothing", self.smooth() as f32, 0.0, 10.0, 1.0),
            p(
                "Original hierarchy",
                self.hierarchical as u8 as f32,
                0.0,
                1.0,
                1.0,
            ),
            p("Superpixels", self.superpixel as u8 as f32, 0.0, 1.0, 1.0),
            p("Centerlines", self.centerline as u8 as f32, 0.0, 1.0, 1.0),
            p(
                "Optimize corners",
                self.optimize_corners as u8 as f32,
                0.0,
                1.0,
                1.0,
            ),
            p(
                "Optimize curves",
                self.optimize_curves as u8 as f32,
                0.0,
                1.0,
                1.0,
            ),
            p(
                "Merge tolerance px",
                self.optimize_tolerance,
                0.0,
                100.0,
                0.1,
            ),
            p(
                "Logo fills/outlines",
                self.logo_mode() as u8 as f32,
                0.0,
                1.0,
                1.0,
            ),
            p(
                "Minimum region px",
                self.min_region_area as f32,
                0.0,
                1_000_000.0,
                1.0,
            ),
            p(
                "Disable despeckle",
                self.no_despeckle as u8 as f32,
                0.0,
                1.0,
                1.0,
            ),
            p("Auto tuning", self.autotrace as u8 as f32, 0.0, 1.0, 1.0),
            p("Coordinate precision", self.precision as f32, 1.0, 6.0, 1.0),
            p("Curve tolerance px", self.curve_tolerance, 0.0, 100.0, 0.1),
            p("Gradient fills", self.gradients as u8 as f32, 0.0, 1.0, 1.0),
            p(
                "Group paths by colour",
                self.group_by_color as u8 as f32,
                0.0,
                1.0,
                1.0,
            ),
            p(
                "Midpoint contours",
                self.no_subpixel_contours as u8 as f32,
                0.0,
                1.0,
                1.0,
            ),
            p(
                "Colour smooth passes",
                self.smoothing_passes as f32,
                0.0,
                20.0,
                1.0,
            ),
            p(
                "Contour smooth window",
                self.smooth_window as f32,
                1.0,
                31.0,
                1.0,
            ),
            p(
                "Simplify tolerance px",
                self.simplification_tolerance,
                0.0,
                100.0,
                0.1,
            ),
            p(
                "Corner threshold deg",
                self.corner_threshold,
                0.0,
                180.0,
                1.0,
            ),
            p(
                "Recolour from source",
                self.recolor as u8 as f32,
                0.0,
                1.0,
                1.0,
            ),
            p(
                "Gradient fit error",
                self.gradient_error,
                0.0,
                65_025.0,
                1.0,
            ),
        ]
    }

    pub fn choices(index: usize) -> Option<&'static [&'static str]> {
        match index {
            1 => Some(&["None", "Logo", "Photo", "Icon"]),
            4 => Some(&["Mean", "Median", "Mode"]),
            5 | 6 | 8..=12 | 14 | 16 | 17 | 20..=22 | 27 => Some(&["Off", "On"]),
            _ => None,
        }
    }

    pub fn set_parameter(&mut self, index: usize, value: f32) {
        match index {
            0 => self.colors = Some(value as u32),
            1 => {
                self.preset = match value as u8 {
                    1 => Preset::Logo,
                    2 => Preset::Photo,
                    3 => Preset::Icon,
                    _ => Preset::None,
                }
            }
            2 => self.max_size = value as u32,
            3 => self.threshold = Some(value),
            4 => {
                self.recolor_mode = match value as u8 {
                    1 => Recolour::Median,
                    2 => Recolour::Mode,
                    _ => Recolour::Mean,
                }
            }
            5 => self.preprocess = Some(value >= 0.5),
            6 => self.original = value >= 0.5,
            7 => self.smooth = Some(value as u8),
            8 => self.hierarchical = value >= 0.5,
            9 => self.superpixel = value >= 0.5,
            10 => self.centerline = value >= 0.5,
            11 => self.optimize_corners = value >= 0.5,
            12 => self.optimize_curves = value >= 0.5,
            13 => self.optimize_tolerance = value,
            14 => self.logo_mode = Some(value >= 0.5),
            15 => self.min_region_area = value as u32,
            16 => self.no_despeckle = value >= 0.5,
            17 => self.autotrace = value >= 0.5,
            18 => self.precision = value as u32,
            19 => self.curve_tolerance = value,
            20 => self.gradients = value >= 0.5,
            21 => self.group_by_color = value >= 0.5,
            22 => self.no_subpixel_contours = value >= 0.5,
            23 => self.smoothing_passes = value as u32,
            24 => self.smooth_window = value as u32,
            25 => self.simplification_tolerance = value,
            26 => self.corner_threshold = value,
            27 => self.recolor = value >= 0.5,
            28 => self.gradient_error = value,
            _ => {}
        }
    }
}

fn finite_range(value: &str, max: f32) -> Result<f32, String> {
    let value: f32 = value.parse().map_err(|_| "expected a number")?;
    if !value.is_finite() || !(0.0..=max).contains(&value) {
        return Err(format!("expected a finite number from 0 to {max}"));
    }
    Ok(value)
}
fn nonnegative(value: &str) -> Result<f32, String> {
    finite_range(value, f32::MAX)
}
fn unit_interval(value: &str) -> Result<f32, String> {
    finite_range(value, 1.0)
}
fn degrees(value: &str) -> Result<f32, String> {
    finite_range(value, 180.0)
}
fn precision(value: &str) -> Result<u32, String> {
    match value {
        "low" => Ok(1),
        "medium" => Ok(2),
        "high" => Ok(6),
        _ => value
            .parse::<u32>()
            .ok()
            .filter(|v| (1..=6).contains(v))
            .ok_or_else(|| "expected 1-6 or low/medium/high".into()),
    }
}

#[cfg(target_arch = "wasm32")]
pub fn apply(_image: &mut photon_rs::PhotonImage, _options: &Options) {
    log::error!("the img2svg effect is only available in native builds");
}

#[cfg(target_arch = "wasm32")]
pub fn apply_with_blending(
    image: &mut photon_rs::PhotonImage,
    options: &Options,
    _blend_colours: bool,
) {
    apply(image, options);
}

#[cfg(not(target_arch = "wasm32"))]
pub fn apply(image: &mut photon_rs::PhotonImage, options: &Options) {
    apply_with_blending(image, options, false);
}

#[cfg(not(target_arch = "wasm32"))]
pub fn apply_with_blending(
    image: &mut photon_rs::PhotonImage,
    options: &Options,
    blend_colours: bool,
) {
    match trace(image, options, blend_colours) {
        Ok(output) => *image = output,
        Err(error) => log::error!("img2svg effect failed: {error}"),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn trace(
    image: &photon_rs::PhotonImage,
    options: &Options,
    blend_colours: bool,
) -> Result<photon_rs::PhotonImage, Box<dyn std::error::Error>> {
    use img2svg::{EnhancedOptions, ImageData, RecolorMethod};
    let width = image.get_width();
    let height = image.get_height();
    if width == 0 || height == 0 {
        return Err("empty source image".into());
    }
    let source = if width.max(height) > options.max_size.max(1) {
        let scale = options.max_size.max(1) as f64 / width.max(height) as f64;
        photon_rs::transform::resize(
            image,
            (width as f64 * scale).round().max(1.0) as u32,
            (height as f64 * scale).round().max(1.0) as u32,
            if blend_colours {
                photon_rs::transform::SamplingFilter::Lanczos3
            } else {
                photon_rs::transform::SamplingFilter::Nearest
            },
        )
    } else {
        image.clone()
    };
    let pixels = source.get_raw_pixels();
    let mut data = ImageData {
        width: source.get_width(),
        height: source.get_height(),
        pixels: pixels
            .chunks_exact(4)
            .map(|p| [p[0], p[1], p[2], p[3]].into())
            .collect(),
    };
    let svg = if options.original {
        if options.preprocess() {
            data = img2svg::preprocessor::preprocess(
                &data,
                &img2svg::preprocessor::PreprocessOptions::photo(),
            )?;
        }
        img2svg::convert_to_svg_string(
            &data,
            &img2svg::ConversionOptions {
                num_colors: if options.colors() == 0 {
                    img2svg::enhanced_quantizer::adaptive_color_count(&data)
                } else {
                    options.colors() as usize
                },
                threshold: f64::from(options.threshold()),
                smooth_level: options.smooth(),
                hierarchical: options.hierarchical,
                advanced: false,
            },
        )?
    } else {
        let traced = img2svg::vectorize_enhanced(
            &data,
            &EnhancedOptions {
                num_colors: options.colors() as usize,
                curve_tolerance: f64::from(options.curve_tolerance),
                simplification_tolerance: f64::from(options.simplification_tolerance),
                corner_threshold: f64::from(options.corner_threshold),
                min_region_area: if options.no_despeckle {
                    0
                } else {
                    options.min_region_area as usize
                },
                edge_threshold: (options.threshold() * 255.0).round() as u8,
                smoothing_passes: options.smoothing_passes as usize,
                smooth_window: options.smooth_window as usize,
                preprocess: options.preprocess(),
                recolor: options.recolor,
                recolor_method: match options.recolor_mode {
                    Recolour::Mean => RecolorMethod::Mean,
                    Recolour::Median => RecolorMethod::Median,
                    Recolour::Mode => RecolorMethod::Mode,
                },
                use_superpixel: options.superpixel,
                centerline: options.centerline,
                optimize_corners: options.optimize_corners,
                optimize_curves: options.optimize_curves,
                optimize_tolerance: f64::from(options.optimize_tolerance),
                logo_mode: options.logo_mode(),
                progress: false,
                autotrace: options.autotrace,
                coordinate_precision: options.precision,
                detect_gradients: options.gradients,
                gradient_error_threshold: f64::from(options.gradient_error),
                group_by_color: options.group_by_color,
                subpixel_contours: !options.no_subpixel_contours,
            },
        )?;
        img2svg::generate_enhanced_svg_prec(&traced, options.precision)
    };
    // No system font discovery is needed: img2svg produces paths, not text.
    // Trace resolution is independent of output geometry: always restore the
    // exact input dimensions rather than rounding the aspect ratio twice.
    let tree = crate::parse_svg(
        svg.as_bytes(),
        blend_colours,
        &resvg::usvg::fontdb::Database::new(),
    )?;
    let mut pixmap =
        tiny_skia::Pixmap::new(width, height).ok_or("could not allocate SVG framebuffer")?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(
            width as f32 / tree.size().width(),
            height as f32 / tree.size().height(),
        ),
        &mut pixmap.as_mut(),
    );
    // tiny-skia uses premultiplied alpha; PhotonImage expects straight RGBA.
    for pixel in pixmap.data_mut().chunks_exact_mut(4) {
        if pixel[3] > 0 && pixel[3] < 255 {
            for channel in 0..3 {
                pixel[channel] =
                    (u32::from(pixel[channel]) * 255 / u32::from(pixel[3])).min(255) as u8;
            }
        }
    }
    Ok(photon_rs::PhotonImage::new(
        pixmap.data().to_vec(),
        width,
        height,
    ))
}
