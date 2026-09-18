use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ImageEffect {
    Brightness(f32),
    Contrast(f32),
    Saturation(f32),
    Gamma(f32),
    Hue(f32),
    Eyedropper, // Added Eyedropper
    Invert,
    BoxBlur,
    GaussBlur(i32),
    LumaBlur(i32),
    Pixelize(i32),
    Halftone,
    Sepia,
    Solarize,
    Normalize,
    Noise,
    Sharpen,
    EdgeDetect,
    Emboss,
    FrostGlass,
    Grayscale,
    Identity,
    Laplace,
    Cali,
    Dramatic,
    Firenze,
    Golden,
    Lix,
    Lofi,
    Neue,
    Obsidian,
    PastelPink,
    Ryo,
    Oil(i32, f64),
    Dither(u32),
    // Geometry — these replace the old GENERAL_CONTROLS entries
    CropTop(i32),
    CropBottom(i32),
    CropLeft(i32),
    CropRight(i32),
    FlipH,
    FlipV,
    Rotate(f32), // degrees
    ScaleX(i32), // percent (100 = no scale)
    ScaleY(i32), // percent (100 = no scale)
    LumaContrast(f32),
    MedianBlur(i32),
    DarkLines(i32),
    LightLines(i32),
    ReplaceColour {
        from: [u8; 3],
        to: [u8; 3],
        tolerance: f32,
    },
}

impl ImageEffect {
    pub fn name(&self) -> &'static str {
        match self {
            ImageEffect::LumaContrast(_) => "Luma Contrast",
            ImageEffect::MedianBlur(_) => "Median Blur",
            ImageEffect::DarkLines(_) | ImageEffect::LightLines(_) => "Line Thickness",
            ImageEffect::ReplaceColour { .. } => "Replace Colour",
            ImageEffect::Brightness(_) => "Brightness",
            ImageEffect::Contrast(_) => "Contrast",
            ImageEffect::Saturation(_) => "Saturation",
            ImageEffect::Gamma(_) => "Gamma",
            ImageEffect::Hue(_) => "Hue",
            ImageEffect::Eyedropper => "Select Color",
            ImageEffect::Invert => "Invert",
            ImageEffect::BoxBlur => "Box Blur",
            ImageEffect::GaussBlur(_) => "Gaussian Blur",
            ImageEffect::LumaBlur(_) => "Luma Blur",
            ImageEffect::Pixelize(_) => "Pixelize",
            ImageEffect::Halftone => "Halftone",
            ImageEffect::Sepia => "Sepia",
            ImageEffect::Solarize => "Solarize",
            ImageEffect::Normalize => "Normalize",
            ImageEffect::Noise => "Noise",
            ImageEffect::Sharpen => "Sharpen",
            ImageEffect::EdgeDetect => "Edge Detect",
            ImageEffect::Emboss => "Emboss",
            ImageEffect::FrostGlass => "Frosted Glass",
            ImageEffect::Grayscale => "Grayscale",
            ImageEffect::Identity => "Identity",
            ImageEffect::Laplace => "Laplace",
            ImageEffect::Cali => "Cali",
            ImageEffect::Dramatic => "Dramatic",
            ImageEffect::Firenze => "Firenze",
            ImageEffect::Golden => "Golden",
            ImageEffect::Lix => "Lix",
            ImageEffect::Lofi => "Lofi",
            ImageEffect::Neue => "Neue",
            ImageEffect::Obsidian => "Obsidian",
            ImageEffect::PastelPink => "Pastel Pink",
            ImageEffect::Ryo => "Ryo",
            ImageEffect::Oil(_, _) => "Oil",
            ImageEffect::Dither(_) => "Dither",
            ImageEffect::CropTop(_) => "Crop Top",
            ImageEffect::CropBottom(_) => "Crop Bottom",
            ImageEffect::CropLeft(_) => "Crop Left",
            ImageEffect::CropRight(_) => "Crop Right",
            ImageEffect::FlipH => "Flip H",
            ImageEffect::FlipV => "Flip V",
            ImageEffect::Rotate(_) => "Rotate",
            ImageEffect::ScaleX(_) => "Scale X",
            ImageEffect::ScaleY(_) => "Scale Y",
        }
    }

    /// Returns true if this effect has a numeric parameter that can be scrolled
    pub fn has_param(&self) -> bool {
        matches!(
            self,
            ImageEffect::Brightness(_)
                | ImageEffect::LumaContrast(_)
                | ImageEffect::Contrast(_)
                | ImageEffect::Saturation(_)
                | ImageEffect::Gamma(_)
                | ImageEffect::Hue(_)
                | ImageEffect::Rotate(_)
                | ImageEffect::GaussBlur(_)
                | ImageEffect::LumaBlur(_)
                | ImageEffect::MedianBlur(_)
                | ImageEffect::DarkLines(_)
                | ImageEffect::LightLines(_)
                | ImageEffect::ReplaceColour { .. }
                | ImageEffect::Pixelize(_)
                | ImageEffect::Dither(_)
                | ImageEffect::Oil(_, _)
                | ImageEffect::CropTop(_)
                | ImageEffect::CropBottom(_)
                | ImageEffect::CropLeft(_)
                | ImageEffect::CropRight(_)
                | ImageEffect::ScaleX(_)
                | ImageEffect::ScaleY(_)
        )
    }

    /// Get param as f32 for display
    pub fn param_f32(&self) -> Option<f32> {
        match self {
            ImageEffect::Brightness(v)
            | ImageEffect::LumaContrast(v)
            | ImageEffect::Contrast(v)
            | ImageEffect::Saturation(v)
            | ImageEffect::Gamma(v)
            | ImageEffect::Hue(v)
            | ImageEffect::Rotate(v) => Some(*v),
            ImageEffect::GaussBlur(v)
            | ImageEffect::LumaBlur(v)
            | ImageEffect::MedianBlur(v)
            | ImageEffect::DarkLines(v)
            | ImageEffect::Pixelize(v)
            | ImageEffect::CropTop(v)
            | ImageEffect::CropBottom(v)
            | ImageEffect::CropLeft(v)
            | ImageEffect::CropRight(v)
            | ImageEffect::ScaleX(v)
            | ImageEffect::ScaleY(v) => Some(*v as f32),
            // Legacy light-line effects use the same control with the sign
            // reversed, preserving saved pipelines without a second UI effect.
            ImageEffect::LightLines(v) => Some(-(*v as f32)),
            ImageEffect::ReplaceColour { tolerance, .. } => Some(*tolerance),
            ImageEffect::Dither(v) => Some(*v as f32),
            ImageEffect::Oil(r, _) => Some(*r as f32),
            _ => None,
        }
    }

    /// Get param range (min, max) for slider display
    pub fn param_range(&self) -> Option<(f32, f32)> {
        match self {
            ImageEffect::Brightness(_)
            | ImageEffect::LumaContrast(_)
            | ImageEffect::Contrast(_)
            | ImageEffect::Saturation(_)
            | ImageEffect::Gamma(_) => Some((-255.0, 255.0)),
            ImageEffect::ReplaceColour { .. } => Some((0.0, 100.0)),
            ImageEffect::MedianBlur(_) => Some((0.0, 10.0)),
            ImageEffect::DarkLines(_) | ImageEffect::LightLines(_) => Some((-10.0, 10.0)),
            ImageEffect::Hue(_) => Some((0.0, 360.0)),
            ImageEffect::GaussBlur(_) | ImageEffect::LumaBlur(_) => Some((0.0, 20.0)),
            ImageEffect::Pixelize(_) => Some((1.0, 64.0)),
            ImageEffect::Dither(_) => Some((0.0, 8.0)),
            ImageEffect::Oil(_, _) => Some((1.0, 16.0)),
            ImageEffect::CropTop(_)
            | ImageEffect::CropBottom(_)
            | ImageEffect::CropLeft(_)
            | ImageEffect::CropRight(_) => Some((0.0, 5000.0)),
            ImageEffect::Rotate(_) => Some((-180.0, 180.0)),
            ImageEffect::ScaleX(_) | ImageEffect::ScaleY(_) => Some((50.0, 200.0)),
            _ => None,
        }
    }

    /// Set param from f32
    pub fn set_param(&mut self, val: f32) {
        match self {
            ImageEffect::Brightness(v)
            | ImageEffect::LumaContrast(v)
            | ImageEffect::Contrast(v)
            | ImageEffect::Saturation(v)
            | ImageEffect::Gamma(v)
            | ImageEffect::Hue(v)
            | ImageEffect::Rotate(v) => *v = val,
            ImageEffect::GaussBlur(v)
            | ImageEffect::LumaBlur(v)
            | ImageEffect::MedianBlur(v)
            | ImageEffect::DarkLines(v)
            | ImageEffect::Pixelize(v)
            | ImageEffect::CropTop(v)
            | ImageEffect::CropBottom(v)
            | ImageEffect::CropLeft(v)
            | ImageEffect::CropRight(v)
            | ImageEffect::ScaleX(v)
            | ImageEffect::ScaleY(v) => *v = val as i32,
            ImageEffect::LightLines(v) => *v = -(val as i32),
            ImageEffect::ReplaceColour { tolerance, .. } => *tolerance = val.clamp(0.0, 100.0),
            ImageEffect::Dither(v) => *v = val as u32,
            ImageEffect::Oil(r, _) => *r = val as i32,
            _ => {}
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageEffectNode {
    pub id: usize,
    pub effect: ImageEffect,
    pub disabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_light_line_settings_use_the_same_signed_thickness_control() {
        let mut legacy: ImageEffect = serde_json::from_str(r#"{"LightLines":3}"#).unwrap();
        assert_eq!(legacy.name(), "Line Thickness");
        assert_eq!(legacy.param_f32(), Some(-3.0));
        legacy.set_param(2.0);
        assert_eq!(legacy, ImageEffect::LightLines(-2));
        assert_eq!(legacy.param_f32(), ImageEffect::DarkLines(2).param_f32());
    }
}
