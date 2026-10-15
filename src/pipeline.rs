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
    /// Margins removed together from the image at this pipeline position.
    Crop {
        top: u32,
        right: u32,
        bottom: u32,
        left: u32,
    },
    FlipH,
    FlipV,
    Rotate(f32), // degrees
    ScaleX(i32), // percent (100 = no scale)
    ScaleY(i32), // percent (100 = no scale)
    LumaContrast(f32),
    MedianBlur(i32),
    DarkLines(f32),
    LightLines(f32),
    ReplaceColour {
        from: [u8; 3],
        to: [u8; 3],
        tolerance: f32,
    },
    QuantizeColours(u32),
    /// Merge nearby shades before reducing the palette. An explicit palette
    /// overrides automatic selection; source chooses pixels versus finished art.
    QuantizePalette {
        colours: u32,
        #[serde(default)]
        threshold: f32,
        #[serde(default)]
        palette: Vec<[u8; 3]>,
        #[serde(default)]
        source: bool,
    },
}

#[derive(Clone, Debug)]
pub struct EffectParameter {
    pub label: &'static str,
    pub value: f32,
    pub min: f32,
    pub max: f32,
    pub step: f32,
}

impl ImageEffect {
    /// These operate on the finished glyph colours when rendering terminal art.
    pub fn is_output_colour(&self) -> bool {
        matches!(
            self,
            Self::ReplaceColour { .. }
                | Self::QuantizeColours(_)
                | Self::QuantizePalette { source: false, .. }
        )
    }

    pub fn name(&self) -> &'static str {
        match self {
            ImageEffect::LumaContrast(_) => "Luma Contrast",
            ImageEffect::MedianBlur(_) => "Median Blur",
            ImageEffect::DarkLines(_) | ImageEffect::LightLines(_) => "Line Thickness",
            ImageEffect::ReplaceColour { .. } => "Replace Colour",
            ImageEffect::QuantizeColours(_) | ImageEffect::QuantizePalette { .. } => {
                "Quantize Colours"
            }
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
            ImageEffect::Crop { .. } => "Crop",
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
                | ImageEffect::QuantizeColours(_)
                | ImageEffect::QuantizePalette { .. }
                | ImageEffect::Pixelize(_)
                | ImageEffect::Dither(_)
                | ImageEffect::Oil(_, _)
                | ImageEffect::CropTop(_)
                | ImageEffect::Crop { .. }
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
            Self::Crop { top, .. } => Some(*top as f32),
            ImageEffect::Brightness(v)
            | ImageEffect::LumaContrast(v)
            | ImageEffect::Contrast(v)
            | ImageEffect::Saturation(v)
            | ImageEffect::Gamma(v)
            | ImageEffect::Hue(v)
            | ImageEffect::DarkLines(v)
            | ImageEffect::Rotate(v) => Some(*v),
            ImageEffect::GaussBlur(v)
            | ImageEffect::LumaBlur(v)
            | ImageEffect::MedianBlur(v)
            | ImageEffect::Pixelize(v)
            | ImageEffect::CropTop(v)
            | ImageEffect::CropBottom(v)
            | ImageEffect::CropLeft(v)
            | ImageEffect::CropRight(v)
            | ImageEffect::ScaleX(v)
            | ImageEffect::ScaleY(v) => Some(*v as f32),
            // Legacy light-line effects use the same control with the sign
            // reversed, preserving saved pipelines without a second UI effect.
            ImageEffect::LightLines(v) => Some(-*v),
            ImageEffect::ReplaceColour { tolerance, .. } => Some(*tolerance),
            ImageEffect::Dither(v) => Some(*v as f32),
            ImageEffect::QuantizeColours(v) => Some(*v as f32),
            ImageEffect::QuantizePalette { colours, .. } => Some(*colours as f32),
            ImageEffect::Oil(r, _) => Some(*r as f32),
            _ => None,
        }
    }

    /// Get param range (min, max) for slider display
    pub fn param_range(&self) -> Option<(f32, f32)> {
        match self {
            Self::Crop { .. } => Some((0.0, 100_000.0)),
            ImageEffect::Brightness(_)
            | ImageEffect::LumaContrast(_)
            | ImageEffect::Contrast(_)
            | ImageEffect::Saturation(_)
            | ImageEffect::Gamma(_) => Some((-255.0, 255.0)),
            ImageEffect::ReplaceColour { .. } => Some((0.0, 100.0)),
            ImageEffect::QuantizeColours(_) => Some((1.0, 256.0)),
            ImageEffect::QuantizePalette { .. } => Some((0.0, 256.0)),
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
            Self::Crop { top, .. } => *top = val.clamp(0.0, 100_000.0).round() as u32,
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
            | ImageEffect::Pixelize(v)
            | ImageEffect::CropTop(v)
            | ImageEffect::CropBottom(v)
            | ImageEffect::CropLeft(v)
            | ImageEffect::CropRight(v)
            | ImageEffect::ScaleX(v)
            | ImageEffect::ScaleY(v) => *v = val as i32,
            ImageEffect::DarkLines(v) => *v = (val.clamp(-10.0, 10.0) * 10.0).round() / 10.0,
            ImageEffect::LightLines(v) => *v = -(val.clamp(-10.0, 10.0) * 10.0).round() / 10.0,
            ImageEffect::ReplaceColour { tolerance, .. } => *tolerance = val.clamp(0.0, 100.0),
            ImageEffect::Dither(v) => *v = val as u32,
            ImageEffect::QuantizeColours(v) => *v = val.clamp(1.0, 256.0) as u32,
            ImageEffect::QuantizePalette { colours, .. } => *colours = val.clamp(0.0, 256.0) as u32,
            ImageEffect::Oil(r, _) => *r = val as i32,
            _ => {}
        }
    }

    pub fn param_step(&self) -> f32 {
        match self {
            Self::DarkLines(_) | Self::LightLines(_) => 0.1,
            _ => 1.0,
        }
    }

    /// All numeric controls, independent of the compact list's primary slider.
    pub fn parameters(&self) -> Vec<EffectParameter> {
        let parameter = |label, value, min, max, step| EffectParameter {
            label,
            value,
            min,
            max,
            step,
        };
        match self {
            Self::Crop {
                top,
                right,
                bottom,
                left,
            } => vec![
                parameter("Top (pixels)", *top as f32, 0.0, 100_000.0, 1.0),
                parameter("Right (pixels)", *right as f32, 0.0, 100_000.0, 1.0),
                parameter("Bottom (pixels)", *bottom as f32, 0.0, 100_000.0, 1.0),
                parameter("Left (pixels)", *left as f32, 0.0, 100_000.0, 1.0),
            ],
            Self::QuantizePalette {
                colours, threshold, ..
            } => vec![
                parameter("Max colours (0=auto)", *colours as f32, 0.0, 256.0, 1.0),
                parameter("Merge threshold %", *threshold, 0.0, 100.0, 0.5),
            ],
            Self::Oil(radius, intensity) => vec![
                parameter("Radius", *radius as f32, 1.0, 16.0, 1.0),
                parameter("Intensity", *intensity as f32, 0.0, 255.0, 0.5),
            ],
            Self::ReplaceColour { tolerance, .. } => {
                vec![parameter("Tolerance %", *tolerance, 0.0, 100.0, 1.0)]
            }
            _ => self
                .param_f32()
                .zip(self.param_range())
                .map(|(value, (min, max))| {
                    vec![parameter("Amount", value, min, max, self.param_step())]
                })
                .unwrap_or_default(),
        }
    }

    pub fn set_parameter(&mut self, index: usize, value: f32) {
        let Some(parameter) = self.parameters().get(index).cloned() else {
            return;
        };
        if !value.is_finite() {
            return;
        }
        let value =
            ((value / parameter.step).round() * parameter.step).clamp(parameter.min, parameter.max);
        match (self, index) {
            (Self::Crop { right, .. }, 1) => *right = value as u32,
            (Self::Crop { bottom, .. }, 2) => *bottom = value as u32,
            (Self::Crop { left, .. }, 3) => *left = value as u32,
            (Self::QuantizePalette { threshold, .. }, 1) => *threshold = value,
            (Self::Oil(_, intensity), 1) => *intensity = f64::from(value),
            (effect, 0) => effect.set_param(value),
            _ => {}
        }
    }

    pub fn upgrade_quantizer(&mut self) {
        if let Self::QuantizeColours(colours) = self {
            *self = Self::QuantizePalette {
                colours: *colours,
                threshold: 0.0,
                palette: vec![],
                source: false,
            };
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
    fn controls_adjust_multiple_parameters_independently() {
        let mut quantize = ImageEffect::QuantizePalette {
            colours: 16,
            threshold: 3.0,
            palette: vec![],
            source: true,
        };
        quantize.set_parameter(1, 5.0);
        assert_eq!(quantize.parameters()[0].value, 16.0);
        assert_eq!(quantize.parameters()[1].value, 5.0);
        assert!(!quantize.is_output_colour());
        let mut oil = ImageEffect::Oil(3, 55.0);
        oil.set_parameter(1, 70.0);
        assert_eq!(oil, ImageEffect::Oil(3, 70.0));
        oil.set_parameter(0, 5.0);
        assert_eq!(oil, ImageEffect::Oil(5, 70.0));
    }

    #[test]
    fn saved_quantizer_upgrades_without_changing_its_behaviour() {
        let mut effect: ImageEffect = serde_json::from_str(r#"{"QuantizeColours":8}"#).unwrap();
        effect.upgrade_quantizer();
        assert_eq!(
            effect,
            ImageEffect::QuantizePalette {
                colours: 8,
                threshold: 0.0,
                palette: vec![],
                source: false
            }
        );
        assert!(effect.is_output_colour());
        assert_eq!(
            serde_json::from_str::<ImageEffect>(&serde_json::to_string(&effect).unwrap()).unwrap(),
            effect
        );
    }

    #[test]
    fn old_light_line_settings_use_the_same_signed_thickness_control() {
        let mut legacy: ImageEffect = serde_json::from_str(r#"{"LightLines":3}"#).unwrap();
        assert_eq!(legacy.name(), "Line Thickness");
        assert_eq!(legacy.param_f32(), Some(-3.0));
        legacy.set_param(2.0);
        assert_eq!(legacy, ImageEffect::LightLines(-2.0));
        assert_eq!(legacy.param_f32(), ImageEffect::DarkLines(2.0).param_f32());
    }
}
