use clap::Parser;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(clap::ValueEnum, Clone, Debug, Serialize, Deserialize)]
pub enum SamplingFilter {
    Nearest,
    Triangle,
    CatmullRom,
    Gaussian,
    Lanczos3,
}

#[derive(clap::ValueEnum, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColourSpace {
    HSL,
    HSV,
    HSLUV,
    LCH,
}

#[derive(clap::ValueEnum, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Encoding {
    Utf8,
    Utf16,
    Utf16be,
    Utf16le,
    Cesu8,
}

#[derive(Copy, Clone, Debug, clap::ValueEnum, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Render {
    Irc,
    Ansi,
    Ansi24,
}

#[derive(Copy, Clone, Debug, clap::ValueEnum, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OcrModelTier {
    Tiny,
    Small,
    Medium,
}

fn default_discontinuity_threshold() -> f32 {
    0.5
}

fn default_ocr_max_text_height_ratio() -> f32 {
    2.5
}

#[derive(Clone, Debug)]
pub enum Param {
    Fixed(f32),
    Range { start: f32, end: f32, step: f32 },
}

impl FromStr for Param {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.contains(':') {
            let parts: Vec<&str> = s.split(':').collect();
            if parts.len() != 3 {
                return Err(format!(
                    "Invalid range format '{}'. Expected 'start:end:step' (e.g. '20:40:1')",
                    s
                ));
            }
            let start = parts[0]
                .parse()
                .map_err(|e| format!("Invalid start: {}", e))?;
            let end = parts[1]
                .parse()
                .map_err(|e| format!("Invalid end: {}", e))?;
            let step = parts[2]
                .parse()
                .map_err(|e| format!("Invalid step: {}", e))?;
            if step == 0.0 {
                return Err("Step cannot be zero".into());
            }
            Ok(Param::Range { start, end, step })
        } else {
            let val = s.parse().map_err(|e| format!("Invalid number: {}", e))?;
            Ok(Param::Fixed(val))
        }
    }
}

impl fmt::Display for Param {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Param::Fixed(v) => write!(f, "{}", v),
            Param::Range { start, end, step } => write!(f, "{}:{}:{}", start, end, step),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ColorSpec {
    Index(u8),
    Rgb([u8; 3]),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TextOverlay {
    pub text: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub fg: Option<ColorSpec>,
    pub bg: Option<ColorSpec>,
    #[serde(default)]
    pub wrap: bool,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    #[serde(default)]
    pub underline: bool,
}

// Struct used for actual rendering (all values resolved to f32)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RenderArgs {
    pub list_blocks: bool,
    pub list_ranges: bool,
    pub print_ranges: bool,
    pub show_block_bitmaps: bool,
    pub clear_cache: bool,

    pub auto_optimize_batch_size: u32,

    pub image: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub scale: Option<(f32, f32)>,
    pub crop: Option<(u32, u32, u32, u32)>,
    pub trim: Option<(u32, u32, u32, u32)>,
    pub filter: SamplingFilter,
    pub rotate: f32,
    pub fliph: bool,
    pub flipv: bool,
    pub render: Render,
    pub braille: bool,
    pub blocks: Vec<String>,
    pub include_range: Vec<String>,
    pub include: Option<String>,
    pub exclude_range: Vec<String>,
    pub exclude: Vec<char>,
    pub font: Vec<String>,
    pub font_size: f32,
    pub grayscale_tolerance: u8,

    // Numeric params
    pub brightness: f32,
    pub contrast: f32,
    pub gamma: f32,
    pub saturation: f32,
    pub hue: f32,
    pub invert: bool,
    pub dither: u32,
    pub as_preview: bool,
    pub luma_brightness: f32,
    pub luma_contrast: f32,
    pub luma_gamma: f32,
    pub luma_saturation: f32,
    pub luma_invert: bool,
    pub colorspace: ColourSpace,
    pub encoding: Encoding,
    pub grayscale: bool,
    pub nograyscale: bool,
    pub pixelize: i32,
    pub box_blur: bool,
    pub gaussian_blur: i32,
    pub luma_blur: i32,
    pub oil: Option<String>,
    pub halftone: bool,
    pub sepia: bool,
    pub normalize: bool,
    pub noise: bool,
    pub emboss: bool,
    pub identity: bool,
    pub laplace: bool,
    pub noise_reduction: bool,
    pub sharpen: bool,
    pub cali: bool,
    pub dramatic: bool,
    pub firenze: bool,
    pub golden: bool,
    pub lix: bool,
    pub lofi: bool,
    pub neue: bool,
    pub obsidian: bool,
    pub pastel_pink: bool,
    pub ryo: bool,
    pub frosted_glass: bool,
    pub solarize: bool,
    pub edge_detection: bool,
    pub show_discontinuities: bool,
    #[serde(default = "default_discontinuity_threshold")]
    pub discontinuity_threshold: f32,
    pub fix_disc: bool,
    pub fix_disc_passes: u32,
    pub fix_disc_shift_radius: u32,
    pub fix_disc_ref_tolerance_ratio: f32,
    pub fix_disc_ref_tolerance_pixels: u32,
    pub fix_disc_pool_size: u32,
    pub misc3: i32,
    pub misc4: i32,
    pub misc5: i32,
    pub misc6: i32,
    pub misc7: i32,
    pub misc8: i32,
    pub gpu: bool,
    pub score: bool,
    pub save: Option<String>,
    pub fft_debug_dir: Option<String>,
    pub ocr: bool,
    pub ocr_min_confidence: f32,
    pub ocr_min_ascii_ratio: f32,
    pub ocr_model_tier: OcrModelTier,
    pub ocr_model_cache: Option<String>,
    pub ocr_det_model: Option<String>,
    pub ocr_cls_model: Option<String>,
    pub ocr_rec_model: Option<String>,
    pub ocr_dict: Option<String>,
    #[serde(default)]
    pub ocr_textline_orientation: bool,
    #[serde(default)]
    pub ocr_most_angle: bool,
    #[serde(default)]
    pub ocr_doc_orientation: bool,
    pub ocr_doc_orientation_model: Option<String>,
    #[serde(default)]
    pub ocr_doc_unwarping: bool,
    pub ocr_threads: usize,
    pub ocr_max_side_len: u32,
    #[serde(default = "default_ocr_max_text_height_ratio")]
    pub ocr_max_text_height_ratio: f32,
    pub ocr_box_score_threshold: f32,
    pub ocr_box_threshold: f32,
    pub ocr_unclip_ratio: f32,
    #[serde(default)]
    pub min_width: Option<u32>,
    #[serde(default)]
    pub max_width: Option<u32>,
    pub pipeline: Vec<crate::pipeline::ImageEffect>,
    #[serde(default)]
    pub overlays: Vec<TextOverlay>,
}

#[derive(Parser, Clone, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// list available blocks
    #[arg(long, default_value_t = false)]
    pub list_blocks: bool,

    /// list available unicode ranges
    #[arg(long, default_value_t = false)]
    pub list_ranges: bool,

    /// print all characters in each range
    #[arg(long, default_value_t = false)]
    pub print_ranges: bool,

    /// show bitmaps for characters in blocks
    #[arg(long, default_value_t = false)]
    pub show_block_bitmaps: bool,

    /// clear the font cache
    #[arg(long, default_value_t = false)]
    pub clear_cache: bool,

    /// launch the TUI
    #[arg(long, default_value_t = false)]
    pub tui: bool,

    /// enable gpu acceleration (requires building with --features gpu)
    #[arg(long, default_value_t = false)]
    pub gpu: bool,

    /// Number of auto-optimize permutations processed at once
    #[arg(long, default_value_t = 10)]
    pub auto_optimize_batch_size: u32,

    #[arg(help = "input image path or URL")]
    pub image: Option<String>,

    /// output image width in columns
    #[arg(short = 'w', long)]
    pub width: Option<Param>,

    /// output image height in rows
    #[arg(short = 'H', long)]
    pub height: Option<Param>,

    /// scaling factors (x:y, e.g., "2:2")
    #[arg(long, value_parser = parse_xy_pair)]
    pub scale: Option<(f32, f32)>,

    /// scale x (or start:end:step)
    #[arg(long)]
    pub scalex: Option<Param>,

    /// scale y (or start:end:step)
    #[arg(long)]
    pub scaley: Option<Param>,

    /// crop image (x1,y1,x2,y2)
    #[arg(long, value_parser = parse_crop_coordinates)]
    pub crop: Option<(u32, u32, u32, u32)>,

    /// crop x1
    #[arg(long)]
    pub crop_x1: Option<Param>,
    /// crop y1
    #[arg(long)]
    pub crop_y1: Option<Param>,
    /// crop x2
    #[arg(long)]
    pub crop_x2: Option<Param>,
    /// crop y2
    #[arg(long)]
    pub crop_y2: Option<Param>,

    /// trim image (pixels)
    #[arg(long, value_parser = parse_trim)]
    pub trim: Option<(u32, u32, u32, u32)>,

    /// sampling filter
    #[arg(long, value_enum, default_value_t = SamplingFilter::Nearest)]
    pub filter: SamplingFilter,

    /// rotate degrees
    #[arg(long, default_value = "0.0")]
    pub rotate: Param,

    /// flip horizontal
    #[arg(long, default_value_t = false)]
    pub fliph: bool,

    /// flip vertical
    #[arg(long, default_value_t = false)]
    pub flipv: bool,

    /// colour mode to use
    #[arg(long, value_enum, default_value_t = Render::Ansi)]
    pub render: Render,

    /// use braille pixels
    #[arg(long, default_value_t = false, conflicts_with = "blocks")]
    pub braille: bool,

    /// use blocks
    #[arg(
        long,
        value_delimiter = ',',
        num_args = 0..,
        conflicts_with = "braille",
    )]
    pub blocks: Vec<String>,

    /// include unicode range (e.g. "2600-26FF")
    #[arg(long)]
    pub include_range: Vec<String>,

    /// include specific characters (e.g. "abcde")
    #[arg(long)]
    pub include: Option<String>,

    /// exclude unicode range (e.g. "2600-26FF")
    #[arg(short = 'E', long)]
    pub exclude_range: Vec<String>,

    /// exclude specific character
    #[arg(short = 'e', long)]
    pub exclude: Vec<char>,

    /// specify font (can be used multiple times)
    #[arg(long)]
    pub font: Vec<String>,

    /// font size (height in pixels)
    #[arg(long, default_value = "0")]
    pub font_size: Param,

    /// tolerance for considering a colour grayscale (0-255)
    #[arg(long, default_value = "32")]
    pub grayscale_tolerance: Param,

    // Iteratable params
    /// adjust brightness (0 = no change, or start:end:step)
    #[arg(short = 'b', long, default_value = "0.0", allow_hyphen_values = true)]
    pub brightness: Param,

    /// adjust contrast (0 = no change, or start:end:step)
    #[arg(short = 'c', long, default_value = "0.0", allow_hyphen_values = true)]
    pub contrast: Param,

    /// adjust gamma (0 to 255, or start:end:step)
    #[arg(short = 'g', long, default_value = "0.0", allow_hyphen_values = true)]
    pub gamma: Param,

    /// adjust saturation (0 = no change, or start:end:step)
    #[arg(short = 's', long, default_value = "0.0", allow_hyphen_values = true)]
    pub saturation: Param,

    /// rotate hue (0 to 360, or start:end:step)
    #[arg(short = 'u', long, default_value = "0.0")]
    pub hue: Param,

    /// colors are inverted, opposite on the color wheel
    #[arg(short = 'i', long, default_value_t = false)]
    pub invert: bool,

    /// dithering (1 to 8)
    #[arg(long, short = 'd', long, default_value_t = 0)]
    pub dither: u32,

    /// adjust luma brightness (braille only)
    #[arg(
        short = 'B',
        long,
        default_value = "0.0",
        allow_hyphen_values = true,
        requires = "braille"
    )]
    pub luma_brightness: Param,

    /// adjust luma contrast (braille only)
    #[arg(
        short = 'C',
        long,
        default_value = "0.0",
        allow_hyphen_values = true,
        requires = "braille"
    )]
    pub luma_contrast: Param,

    /// adjust luma gamma (braille only)
    #[arg(
        short = 'G',
        long,
        default_value = "0.0",
        allow_hyphen_values = true,
        requires = "braille"
    )]
    pub luma_gamma: Param,

    /// adjust luma saturation (braille only)
    #[arg(
        short = 'S',
        long,
        default_value = "0.0",
        allow_hyphen_values = true,
        requires = "braille"
    )]
    pub luma_saturation: Param,

    /// luminance is inverted
    #[arg(short = 'I', long, default_value_t = false, requires = "braille")]
    pub luma_invert: bool,

    /// colour space
    #[arg(long, value_enum, default_value_t = ColourSpace::HSV)]
    pub colorspace: ColourSpace,

    /// output encoding
    #[arg(long, value_enum, default_value_t = Encoding::Utf8)]
    pub encoding: Encoding,

    /// exclude grayscale when picking nearest colour in palette (unless already grayscale)
    #[arg(long, default_value_t = false, group = "grayscale_opts")]
    pub grayscale: bool,

    /// exclude grayscale colours from the palette
    #[arg(long, default_value_t = false, group = "grayscale_opts")]
    pub nograyscale: bool,

    /// pixelize pixel size
    #[arg(long, default_value = "0")]
    pub pixelize: Param,

    /// simple average of all the neighboring pixels surrounding a given pixel
    #[arg(long = "boxblur", default_value_t = false)]
    pub box_blur: bool,

    /// gaussian blur radius
    #[arg(long = "gaussianblur", default_value = "0")]
    pub gaussian_blur: Param,

    /// gaussian blur radius (only luminance)
    #[arg(long = "lumablur", default_value = "0")]
    pub luma_blur: Param,

    /// oil filter ("[radius],[intensity]")
    #[arg(long)]
    pub oil: Option<String>,

    /// made up of small dots creating a continuous-tone illusion
    #[arg(long, default_value_t = false)]
    pub halftone: bool,

    /// brownish, aged appearance like old photographs
    #[arg(long, default_value_t = false)]
    pub sepia: bool,

    /// adjusts brightness and contrast for better image quality
    #[arg(long, default_value_t = false)]
    pub normalize: bool,

    /// random variations in brightness and color like film grain
    #[arg(long, default_value_t = false)]
    pub noise: bool,

    /// gives a raised, 3d appearance
    #[arg(long, default_value_t = false)]
    pub emboss: bool,

    /// no modifications, unchanged image
    #[arg(long, default_value_t = false)]
    pub identity: bool,

    /// enhances edges and boundaries in an image
    #[arg(long, default_value_t = false)]
    pub laplace: bool,

    /// reduces noise for a cleaner, clearer image
    #[arg(long = "denoise", default_value_t = false)]
    pub noise_reduction: bool,

    /// increases clarity and definition, making edges and details more distinct
    #[arg(long, default_value_t = false)]
    pub sharpen: bool,

    /// cool blue tone with increased contrast
    #[arg(long, default_value_t = false)]
    pub cali: bool,

    /// high contrast and vivid colors for a dramatic effect
    #[arg(long, default_value_t = false)]
    pub dramatic: bool,

    /// warm, earthy tones reminiscent of tuscan landscapes
    #[arg(long, default_value_t = false)]
    pub firenze: bool,

    /// warm, golden glow like sunset light
    #[arg(long, default_value_t = false)]
    pub golden: bool,

    /// high-contrast black and white appearance with increased sharpness
    #[arg(long, default_value_t = false)]
    pub lix: bool,

    /// low-fidelity, retro appearance like old photographs or film
    #[arg(long, default_value_t = false)]
    pub lofi: bool,

    /// clean, modern appearance with neutral colors and simple design
    #[arg(long, default_value_t = false)]
    pub neue: bool,

    /// dark, monochromatic appearance with black and gray shades
    #[arg(long, default_value_t = false)]
    pub obsidian: bool,

    /// soft, delicate pink tint like pastel colors
    #[arg(long = "pastelpink", default_value_t = false)]
    pub pastel_pink: bool,

    /// bright, high-contrast appearance with vivid colors and sharp details
    #[arg(long, default_value_t = false)]
    pub ryo: bool,

    /// blurred, frosted appearance as if viewed through semi-transparent surface
    #[arg(long = "frostedglass", default_value_t = false)]
    pub frosted_glass: bool,

    /// strange, otherworldly appearance with inverted colors and surreal atmosphere
    #[arg(long, default_value_t = false)]
    pub solarize: bool,

    /// highlights edges and boundaries in an image
    #[arg(long = "edgedetection", default_value_t = false)]
    pub edge_detection: bool,

    /// highlight cells with discontinuities in red
    #[arg(
        long = "disc",
        visible_alias = "show-discontinuities",
        default_value_t = false
    )]
    pub show_discontinuities: bool,

    /// threshold above which a cell is treated as a discontinuity
    #[arg(
        long = "disc-threshold",
        visible_alias = "discontinuity-threshold",
        default_value = "0.5"
    )]
    pub discontinuity_threshold: f32,

    /// fix discontinuities by trying alternative glyphs
    #[arg(long = "fix", visible_alias = "fix-disc", default_value_t = false)]
    pub fix_disc: bool,

    /// number of fix-disc passes to run
    #[arg(
        long = "passes",
        visible_alias = "fix-disc-passes",
        default_value = "10"
    )]
    pub fix_disc_passes: u32,

    /// max pixel shift in x/y when matching the quantized reference
    #[arg(
        long = "radius",
        visible_alias = "fix-disc-shift-radius",
        default_value = "2"
    )]
    pub fix_disc_shift_radius: u32,

    /// allowed fractional increase in reference mismatch when continuity improves
    #[arg(
        long = "tol-ratio",
        visible_alias = "fix-disc-ref-tolerance-ratio",
        default_value = "0.20"
    )]
    pub fix_disc_ref_tolerance_ratio: f32,

    /// allowed absolute increase in reference mismatch pixels when continuity improves
    #[arg(
        long = "tol-pixels",
        visible_alias = "fix-disc-ref-tolerance-pixels",
        default_value = "2"
    )]
    pub fix_disc_ref_tolerance_pixels: u32,

    /// number of cheap glyph candidates to consider during fix-disc
    #[arg(
        long = "pool",
        visible_alias = "fix-disc-pool-size",
        default_value = "12"
    )]
    pub fix_disc_pool_size: u32,

    #[arg(long, default_value = "0", hide = true)]
    pub misc3: i32,

    #[arg(long, default_value = "0", hide = true)]
    pub misc4: i32,

    #[arg(long, default_value = "0", hide = true)]
    pub misc5: i32,

    #[arg(long, default_value = "0", hide = true)]
    pub misc6: i32,

    #[arg(long, default_value = "0", hide = true)]
    pub misc7: i32,

    #[arg(long, default_value = "0", hide = true)]
    pub misc8: i32,

    /// print smoothness score
    #[arg(long, default_value_t = false, hide = true)]
    pub score: bool,

    /// save output as PNG (path or directory)
    #[arg(long)]
    pub save: Option<String>,

    /// write exact FFT input windows and fix artifacts into this directory
    #[arg(long = "debug-dir", visible_alias = "fft-debug-dir")]
    pub fft_debug_dir: Option<String>,

    /// detect readable text with PaddleOCR and overlay it as normal text
    #[arg(long = "ocr", default_value_t = false)]
    pub ocr: bool,

    /// minimum OCR confidence for text overlays
    #[arg(long = "ocr-min-confidence", default_value_t = 0.0)]
    pub ocr_min_confidence: f32,

    /// minimum printable ASCII ratio for OCR text overlays
    #[arg(long = "ocr-min-ascii-ratio", default_value_t = 0.7)]
    pub ocr_min_ascii_ratio: f32,

    /// PP-OCRv6 model tier to use
    #[arg(long = "ocr-model-tier", value_enum, default_value_t = OcrModelTier::Tiny)]
    pub ocr_model_tier: OcrModelTier,

    /// directory for downloaded PP-OCR models
    #[arg(long = "ocr-model-cache")]
    pub ocr_model_cache: Option<String>,

    /// explicit PP-OCR detection ONNX path
    #[arg(long = "ocr-det-model")]
    pub ocr_det_model: Option<String>,

    /// explicit PP-OCR text-line orientation classifier ONNX path
    #[arg(long = "ocr-cls-model")]
    pub ocr_cls_model: Option<String>,

    /// explicit PP-OCR recognition ONNX path
    #[arg(long = "ocr-rec-model")]
    pub ocr_rec_model: Option<String>,

    /// explicit PP-OCR dictionary path
    #[arg(long = "ocr-dict")]
    pub ocr_dict: Option<String>,

    /// classify and correct per-line text orientation, matching PaddleOCR use_textline_orientation=True
    #[arg(long = "ocr-textline-orientation", default_value_t = false, action = clap::ArgAction::Set)]
    pub ocr_textline_orientation: bool,

    /// force all text lines to the majority 0/180-degree orientation
    #[arg(long = "ocr-most-angle", default_value_t = false)]
    pub ocr_most_angle: bool,

    /// classify whole-page orientation before OCR
    #[arg(long = "ocr-doc-orientation", default_value_t = false)]
    pub ocr_doc_orientation: bool,

    /// explicit PP-OCR document orientation classifier ONNX path
    #[arg(long = "ocr-doc-orientation-model")]
    pub ocr_doc_orientation_model: Option<String>,

    /// enable document unwarping stage if supported by the OCR backend
    #[arg(long = "ocr-doc-unwarping", default_value_t = false)]
    pub ocr_doc_unwarping: bool,

    /// OCR inference worker threads
    #[arg(long = "ocr-threads", default_value_t = 4)]
    pub ocr_threads: usize,

    /// maximum side length for OCR detection preprocessing
    #[arg(long = "ocr-max-side-len", default_value_t = 3000)]
    pub ocr_max_side_len: u32,

    /// skip OCR overlay/removal for text taller than this multiple of normal OCR line height; 0 disables
    #[arg(long = "ocr-max-text-height-ratio", default_value_t = 2.5)]
    pub ocr_max_text_height_ratio: f32,

    /// PP-OCR text-box score threshold
    #[arg(long = "ocr-box-score-threshold", default_value_t = 0.6)]
    pub ocr_box_score_threshold: f32,

    /// PP-OCR box binarization threshold
    #[arg(long = "ocr-box-threshold", default_value_t = 0.2)]
    pub ocr_box_threshold: f32,

    /// PP-OCR detected-box unclip ratio
    #[arg(long = "ocr-unclip-ratio", default_value_t = 1.6)]
    pub ocr_unclip_ratio: f32,

    /// minimum width in columns for auto-resized grids (OCR, etc.)
    #[arg(long = "min-width")]
    pub min_width: Option<u32>,

    /// maximum width in columns for auto-resized grids (OCR, etc.)
    #[arg(long = "max-width")]
    pub max_width: Option<u32>,
}

pub fn parse_args() -> Args {
    Args::parse()
}

fn parse_xy_pair(s: &str) -> Result<(f32, f32), String> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 2 {
        return Err(format!(
            "Invalid format. Expected 'value1:value2', got '{}'",
            s
        ));
    }

    let first = parts[0].parse::<f32>().map_err(|e| {
        format!(
            "Failed to parse the first value ('{}') as f32: {}",
            parts[0], e
        )
    })?;
    let second = parts[1].parse::<f32>().map_err(|e| {
        format!(
            "Failed to parse the second value ('{}') as f32: {}",
            parts[1], e
        )
    })?;

    if first <= 0.0 || second <= 0.0 {
        return Err("Both values must be positive numbers.".to_string());
    }

    Ok((first, second))
}

fn parse_crop_coordinates(s: &str) -> Result<(u32, u32, u32, u32), String> {
    let coords: Vec<&str> = s.split(',').collect();
    if coords.len() != 4 {
        return Err(format!(
            "Invalid crop format '{}'. Expected 'x1,y1,x2,y2' (e.g., '50,50,200,200').",
            s
        ));
    }

    let x1 = coords[0]
        .parse::<u32>()
        .map_err(|_| format!("Invalid x1 value '{}'.", coords[0]))?;
    let y1 = coords[1]
        .parse::<u32>()
        .map_err(|_| format!("Invalid y1 value '{}'.", coords[1]))?;
    let x2 = coords[2]
        .parse::<u32>()
        .map_err(|_| format!("Invalid x2 value '{}'.", coords[2]))?;
    let y2 = coords[3]
        .parse::<u32>()
        .map_err(|_| format!("Invalid y2 value '{}'.", coords[3]))?;

    Ok((x1, y1, x2, y2))
}

fn parse_trim(s: &str) -> Result<(u32, u32, u32, u32), String> {
    let parts: Vec<&str> = s.split(',').collect();
    let values_res: Result<Vec<u32>, _> = parts
        .iter()
        .map(|s| {
            if s.trim().is_empty() {
                Ok(0)
            } else {
                s.trim().parse::<u32>()
            }
        })
        .collect();

    let values =
        values_res.map_err(|_| format!("Invalid trim format '{}'. Expected numbers.", s))?;

    match values.len() {
        1 => {
            let v = values[0];
            Ok((v, v, v, v))
        }
        2 => {
            let v = values[0]; // Top/Bottom
            let h = values[1]; // Left/Right
            Ok((v, h, v, h))
        }
        3 => {
            let t = values[0];
            let h = values[1];
            let b = values[2];
            Ok((t, h, b, h))
        }
        4 => {
            let t = values[0];
            let r = values[1];
            let b = values[2];
            let l = values[3];
            Ok((t, r, b, l))
        }
        _ => Err(format!(
            "Invalid trim format '{}'. Expected 1, 2, 3, or 4 values.",
            s
        )),
    }
}
