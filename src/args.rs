use clap::Parser;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// image url or file path
    #[arg(index = 1)]
    pub image: String,

    /// render type (irc, ansi)
    #[arg(short, long)]
    pub render: Option<String>,

    /// image width to resize to
    #[arg(short, long, default_value_t = 50)]
    pub width: u32,

    /// brightness (-255 to 255)
    #[arg(short, long, require_equals = true, default_value_t = 0)]
    pub brightness: i16,

    /// hue (-180 to 180)
    #[arg(short = 'H', long, require_equals = true, default_value_t = 0)]
    pub hue: i16,

    /// contrast (-255 to 255)
    #[arg(short, long, require_equals = true, default_value_t = 0)]
    pub contrast: i16,

    /// saturation (-255 to 255)
    #[arg(short, long, require_equals = true, default_value_t = 0)]
    pub saturation: i16,

    /// opacity (-255 to 255)
    #[arg(short, long, require_equals = true, default_value_t = 0)]
    pub opacity: i16,

    /// gamma (-255 to 255)
    #[arg(short, long, require_equals = true, default_value_t = 0)]
    pub gamma: i16,

    /// dither (1 to 8)
    #[arg(long, default_value_t = 0)]
    pub dither: u32,

    /// pixelize size
    #[arg(long, default_value_t = 0)]
    pub pixelize: i32,

    /// gaussian blur radius
    #[arg(long, default_value_t = 0)]
    pub gaussian_blur: i32,

    /// oil ("<radius>,<intensity>")
    #[arg(long)]
    pub oil: Option<String>,

    /// grayscale
    #[arg(long, default_value_t = false)]
    pub grayscale: bool,

    /// halftone
    #[arg(long, default_value_t = false)]
    pub halftone: bool,

    /// sepia
    #[arg(long, default_value_t = false)]
    pub sepia: bool,

    /// normalize
    #[arg(long, default_value_t = false)]
    pub normalize: bool,

    /// noise
    #[arg(long, default_value_t = false)]
    pub noise: bool,

    /// emboss
    #[arg(long, default_value_t = false)]
    pub emboss: bool,

    /// box_blur
    #[arg(long, default_value_t = false)]
    pub box_blur: bool,

    /// identity
    #[arg(long, default_value_t = false)]
    pub identity: bool,

    /// laplace
    #[arg(long, default_value_t = false)]
    pub laplace: bool,

    /// noise reduction
    #[arg(long, default_value_t = false)]
    pub noise_reduction: bool,

    /// sharpen
    #[arg(long, default_value_t = false)]
    pub sharpen: bool,

    /// cali
    #[arg(long, default_value_t = false)]
    pub cali: bool,

    /// dramatic
    #[arg(long, default_value_t = false)]
    pub dramatic: bool,

    /// firenze
    #[arg(long, default_value_t = false)]
    pub firenze: bool,

    /// golden
    #[arg(long, default_value_t = false)]
    pub golden: bool,

    /// lix
    #[arg(long, default_value_t = false)]
    pub lix: bool,

    /// lofi
    #[arg(long, default_value_t = false)]
    pub lofi: bool,

    /// neue
    #[arg(long, default_value_t = false)]
    pub neue: bool,

    /// obsidian
    #[arg(long, default_value_t = false)]
    pub obsidian: bool,

    /// pastel_pink
    #[arg(long, default_value_t = false)]
    pub pastel_pink: bool,

    /// ryo
    #[arg(long, default_value_t = false)]
    pub ryo: bool,

    /// invert
    #[arg(long, default_value_t = false)]
    pub invert: bool,

    /// frosted glass
    #[arg(long, default_value_t = false)]
    pub frosted_glass: bool,

    /// solarize
    #[arg(long, default_value_t = false)]
    pub solarize: bool,

    /// edge detection
    #[arg(long, default_value_t = false)]
    pub edge_detection: bool,
}

pub fn parse_args() -> Args {
    Args::parse()
}