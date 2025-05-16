use clap::Parser;

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum SamplingFilter {
    Nearest,
    Triangle, 
    CatmullRom,
    Gaussian,
    Lanczos3,
}

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum ColourSpace {
    HSL,
    HSV, 
    HSLUV,
    LCH,
}

#[derive(Copy, Clone, Debug, clap::ValueEnum, PartialEq, Eq, Hash)]
pub enum BlockKind {
    Full,       // U+2588
    Half,       // U+2580, U+2584, U+258C, U+2590
    Quarter,    // U+2596–U+259F
    Eighth,     // U+2581–U+2587, U+2589–U+258F, U+2594–U+2595
    Triangle,   // U+25B2, U+25B6, U+25BC, U+25C0
    Corner,     // U+25E2–U+25E5
    Geometric,  // U+25A0–U+25FF
    Box,        // U+2500–U+257F
    Legacy,     // U+1FB00–U+1FBFF
}

#[derive(Parser, Clone, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// image url or file path
    #[arg(index = 1)]
    pub image: String,

    /// output image width in columns
    #[arg(short = 'w', long)]
    pub width: Option<u32>,

    /// output image height in rows
    #[arg(short = 'H', long)]
    pub height: Option<u32>,

    /// scaling factors (x:y, e.g., "2:2")
    #[arg(long, value_parser = parse_xy_pair)]
    pub scale: Option<(f32, f32)>,

    /// crop image (x1,y1,x2,y2)
    #[arg(long, value_parser = parse_crop_coordinates)]
    pub crop: Option<(u32, u32, u32, u32)>,

    /// sampling filter
    #[arg(long, value_enum, default_value_t = SamplingFilter::Nearest)]
    pub filter: SamplingFilter,

    /// rotate degrees
    #[arg(long, default_value_t = 0)]
    pub rotate: i32,

    /// flip horizontal
    #[arg(long, default_value_t = false)]
    pub fliph: bool,

    /// flip vertical 
    #[arg(long, default_value_t = false)]
    pub flipv: bool,

    /// use IRC99 colours
    #[arg(long, default_value_t = false, group = "colour", required_unless_present_any = ["ansi", "ansi24"])]
    pub irc: bool,

    /// use 8-bit ANSI colours
    #[arg(long, default_value_t = false, group = "colour", required_unless_present_any = ["irc", "ansi24"])]
    pub ansi: bool,

    /// use 24-bit ANSI colours
    #[arg(long, default_value_t = false, group = "colour", required_unless_present_any = ["irc", "ansi"])]
    pub ansi24: bool,

    /// use braille pixels
    #[arg(
        long,
        default_value_t = false,
        conflicts_with = "block",
        required_unless_present = "blocks"
    )]
    pub braille: bool,
 
    /// use blocks
    #[arg(
        long,
        value_enum,
        value_delimiter = ',',
        num_args = 0..,
        default_missing_values = &[
            "full",
            "half",
            "quarter",
            "eighth",
            "triangle",
            "corner",
            "geometric",
            "box",
            "legacy",
        ],
        conflicts_with = "braille",
        required_unless_present = "braille"
    )]
    pub blocks: Vec<BlockKind>,

    /// adjust brightness (0 = no change)
    #[arg(short = 'b', long, default_value_t = 0.0, allow_hyphen_values = true)]    
    pub brightness: f32,

    /// adjust contrast (0 = no change)
    #[arg(short = 'c', long, default_value_t = 0.0, allow_hyphen_values = true)]
    pub contrast: f32,

    /// adjust gamma (0 to 255)
    #[arg(short = 'g', long, default_value_t = 0.0, allow_hyphen_values = true)]
    pub gamma: f32,

    /// adjust saturation (0 = no change)
    #[arg(short = 's', long, default_value_t = 0.0, allow_hyphen_values = true)]
    pub saturation: f32,

    /// rotate hue (0 to 360)
    #[arg(short = 'u', long, default_value_t = 0.0)]
    pub hue: f32,

    /// colors are inverted, opposite on the color wheel
    #[arg(short = 'i', long, default_value_t = false)]
    pub invert: bool,

    /// dithering (1 to 8)
    #[arg(long, short = 'd', long, default_value_t = 0)]
    pub dither: u32,

    /// adjust luma brightness (braille only)
    #[arg(short = 'B', long, default_value_t = 0.0, allow_hyphen_values = true, requires = "braille")]
    pub luma_brightness: f32,

    /// adjust luma contrast (braille only)
    #[arg(short = 'C', long, default_value_t = 0.0, allow_hyphen_values = true, requires = "braille")]
    pub luma_contrast: f32,

    /// adjust luma gamma (braille only)
    #[arg(short = 'G', long, default_value_t = 0.0, allow_hyphen_values = true, requires = "braille")]
    pub luma_gamma: f32,

    /// adjust luma saturation (braille only)
    #[arg(short = 'S', long, default_value_t = 0.0, allow_hyphen_values = true, requires = "braille")]
    pub luma_saturation: f32,

    /// luminance is inverted
    #[arg(short = 'I', long, default_value_t = false, requires = "braille")]
    pub luma_invert: bool,

    /// colour space
    #[arg(long, value_enum, default_value_t = ColourSpace::HSV)]
    pub colorspace: ColourSpace,

    /// exclude grayscale when picking nearest colour in palette (unless already grayscale)
    #[arg(long, default_value_t = false, group = "grayscale_opts")]
    pub grayscale: bool,

    /// exclude grayscale colours from the palette
    #[arg(long, default_value_t = false, group = "grayscale_opts")]
    pub nograyscale: bool,

    /// pixelize pixel size
    #[arg(long, default_value_t = 0)]
    pub pixelize: i32,

    /// simple average of all the neighboring pixels surrounding a given pixel
    #[arg(long="boxblur", default_value_t = false)]
    pub box_blur: bool,

    /// gaussian blur radius
    #[arg(long="gaussianblur", default_value_t = 0)]
    pub gaussian_blur: i32,

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
    #[arg(long="denoise", default_value_t = false)]
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
    #[arg(long="pastelpink", default_value_t = false)]
    pub pastel_pink: bool,

    /// bright, high-contrast appearance with vivid colors and sharp details
    #[arg(long, default_value_t = false)]
    pub ryo: bool,

    /// blurred, frosted appearance as if viewed through semi-transparent surface
    #[arg(long="frostedglass", default_value_t = false)]
    pub frosted_glass: bool,

    /// strange, otherworldly appearance with inverted colors and surreal atmosphere
    #[arg(long, default_value_t = false)]
    pub solarize: bool,

    /// highlights edges and boundaries in an image
    #[arg(long="edgedetection", default_value_t = false)]
    pub edge_detection: bool,
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

    let x1 = coords[0].parse::<u32>().map_err(|_| format!("Invalid x1 value '{}'.", coords[0]))?;
    let y1 = coords[1].parse::<u32>().map_err(|_| format!("Invalid y1 value '{}'.", coords[1]))?;
    let x2 = coords[2].parse::<u32>().map_err(|_| format!("Invalid x2 value '{}'.", coords[2]))?;
    let y2 = coords[3].parse::<u32>().map_err(|_| format!("Invalid y2 value '{}'.", coords[3]))?;

    Ok((x1, y1, x2, y2))
}
