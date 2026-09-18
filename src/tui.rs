pub mod explorer;
pub mod general;
pub mod glyphs;
pub mod pipeline;
pub mod text;
mod replace_colour;
use replace_colour::*;
mod figlet_picker;
use figlet_picker::*;
pub(crate) use general::*;
pub(crate) use glyphs::*;
pub(crate) use pipeline::*;
pub(crate) use text::*;

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{
    error::Error,
    io::{self, Read},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use ansi_to_tui::IntoText;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
        MouseButton, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use photon_rs::PhotonImage;
use regex::Regex;

use crate::{
    args::{ColourSpace, Encoding, OcrModelTier, Render, RenderArgs, SamplingFilter},
    draw, font,
};
use explorer::FileExplorer;
use ratatui::{
    backend::{Backend, CrosstermBackend},
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Tabs, Wrap},
    Terminal,
};
use ratatui_image::{
    picker::Picker, protocol::StatefulProtocol, Resize, ResizeEncodeRender, StatefulImage,
};
use ratatui_textarea::{CursorMove, TextArea};
use simplelog::*;
use std::fs::OpenOptions;
use std::process::Command;
use std::time::Instant;
use tokio::sync::{mpsc, watch};

// ── Theme ────────────────────────────────────────────────────────────────────
const BG: Color = Color::Rgb(22, 22, 30);
const SURFACE: Color = Color::Rgb(30, 30, 42);
const CONDITIONAL_BG: Color = Color::Rgb(42, 47, 72);
const NESTED_CONDITIONAL_BG: Color = Color::Rgb(49, 43, 76);
const BORDER: Color = Color::Rgb(60, 60, 80);
const TEXT: Color = Color::Rgb(200, 200, 220);
const TEXT_DIM: Color = Color::Rgb(110, 110, 140);
const ACCENT: Color = Color::Rgb(130, 140, 255);
const ACCENT_DIM: Color = Color::Rgb(80, 85, 160);
const BAR_FG: Color = Color::Rgb(100, 120, 255);
const TOGGLE_ON: Color = Color::Rgb(70, 190, 120);
const TOGGLE_OFF: Color = Color::Rgb(70, 70, 90);
const SELECTED_BG: Color = Color::Rgb(50, 50, 72);
const TAB_ACTIVE: Color = Color::Rgb(130, 140, 255);
const STATUS_BG: Color = Color::Rgb(35, 35, 50);
const OPTIMIZE_BG: Color = Color::Rgb(40, 60, 40);
const GENERAL_PANE_WIDTH: u16 = 40;
const PIPELINE_PANE_WIDTH: u16 = 60;
const GLYPHS_PANE_WIDTH: u16 = 45;
const TEXT_PANE_WIDTH: u16 = 44;
const ACTIVE_PANE_WIDTH: u16 = 60;
const SLIDER_BAR_CELLS: usize = 32;
const EIGHTH_BLOCKS: [&str; 8] = ["", "▏", "▎", "▍", "▌", "▋", "▊", "▉"];

#[derive(Debug)]
enum OptimizationUpdate {
    Progress(String),
    BestResult(RenderArgs, f64),
    Finished,
}

#[derive(Debug)]
enum ClipboardPasteResult {
    Loaded { path: String, bytes: usize },
    Empty,
    Failed(String),
}

#[derive(Debug, Clone)]
struct ClipboardImageUrl {
    url: String,
    referer: Option<String>,
}

// ── Control types ────────────────────────────────────────────────────────────
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ControlId {
    Width,
    RenderMode,
    GraphicsPreview,
    Ocr,
    OcrLock,
    OcrAutoWidth,
    OcrMegapixels,
    OcrFiglet,
    OcrFigletMinHeight,
    OcrFigletMinHeightRatio,
    OcrFigletFill,
    OcrFigletMaxWidthRatio,
    OcrFigletMaxHeightRatio,
    OcrMinConfidence,
    OcrModelTier,
    OcrAdvanced,
    OcrMaxTextHeight,
    OcrThreads,

    HighlightDisc,
    DiscThreshold,
    ScoreFix,
    ScoreFixCandidates,
    ScoreFixOrderings,
    ScoreFixGeometryFirst,
    ScoreFixNeighborhoodGuard,
    ContourBendingWeight,
    ContourEndpointWeight,
    ContourJunctionWeight,
    ContourFragmentWeight,
    ContourFidelityWeight,
    ContourBoundaryWeight,
    ContourPeakWeight,
    TrimTop,
    TrimRight,
    TrimBottom,
    TrimLeft,
    FlipH,
    FlipV,
    Rotate,
    ScaleX,
    ScaleY,
    FontSize,
    Font,
    Filter,
    Eyedropper,
    GrayTol,
    Brightness,
    Contrast,
    Saturation,
    Gamma,
    Hue,
    Colorspace,
    Encoding,

    Dither,
    Invert,
    Grayscale,
    Pixelize,
    GaussianBlur,
    BoxBlur,
    Oil,
    Halftone,
    Sepia,
    Normalize,
    Noise,
    Emboss,
    Identity,
    Laplace,
    NoiseReduction,
    Sharpen,
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
    FrostedGlass,
    Solarize,
    EdgeDetection,
    AutoOptimize,

    ShowAdvanced,
}

impl ControlId {
    fn layout_id(self) -> &'static str {
        match self {
            ControlId::Width => "width",
            ControlId::RenderMode => "mode",
            ControlId::GraphicsPreview => "graphics-preview",
            ControlId::Ocr => "ocr",
            ControlId::OcrLock => "ocr-lock",
            ControlId::OcrAutoWidth => "ocr-auto-width",
            ControlId::OcrMegapixels => "ocr-megapixels",
            ControlId::OcrFiglet => "ocr-figlet",
            ControlId::OcrFigletMinHeight => "ocr-figlet-min-height",
            ControlId::OcrFigletMinHeightRatio => "ocr-figlet-min-height-ratio",
            ControlId::OcrFigletFill => "ocr-figlet-fill",
            ControlId::OcrFigletMaxWidthRatio => "ocr-figlet-max-width-ratio",
            ControlId::OcrFigletMaxHeightRatio => "ocr-figlet-max-height-ratio",
            ControlId::OcrMinConfidence => "ocr-confidence",
            ControlId::OcrModelTier => "ocr-model",
            ControlId::OcrAdvanced => "ocr-advanced",
            ControlId::OcrMaxTextHeight => "ocr-max-height",
            ControlId::OcrThreads => "ocr-threads",
            ControlId::HighlightDisc => "show-discontinuities",
            ControlId::DiscThreshold => "discontinuity-threshold",
            ControlId::ScoreFix => "smooth",
            ControlId::ScoreFixCandidates => "smooth-candidates",
            ControlId::ScoreFixOrderings => "smooth-orders",
            ControlId::ScoreFixGeometryFirst => "smooth-shapes",
            ControlId::ScoreFixNeighborhoodGuard => "smooth-neighbors",
            ControlId::ContourBendingWeight => "contour-bending-weight",
            ControlId::ContourEndpointWeight => "contour-endpoint-weight",
            ControlId::ContourJunctionWeight => "contour-junction-weight",
            ControlId::ContourFragmentWeight => "contour-fragment-weight",
            ControlId::ContourFidelityWeight => "contour-fidelity-weight",
            ControlId::ContourBoundaryWeight => "contour-boundary-weight",
            ControlId::ContourPeakWeight => "contour-peak-weight",
            ControlId::TrimTop => "crop-top",
            ControlId::TrimRight => "crop-right",
            ControlId::TrimBottom => "crop-bottom",
            ControlId::TrimLeft => "crop-left",
            ControlId::FlipH => "flip-horizontal",
            ControlId::FlipV => "flip-vertical",
            ControlId::Rotate => "rotate",
            ControlId::ScaleX => "scale-x",
            ControlId::ScaleY => "scale-y",
            ControlId::FontSize => "font-size",
            ControlId::Font => "font",
            ControlId::Filter => "sampling-filter",
            ControlId::Eyedropper => "eyedropper",
            ControlId::GrayTol => "grayscale-tolerance",
            ControlId::Brightness => "brightness",
            ControlId::Contrast => "contrast",
            ControlId::Saturation => "saturation",
            ControlId::Gamma => "gamma",
            ControlId::Hue => "hue",
            ControlId::Colorspace => "colorspace",
            ControlId::Encoding => "encoding",
            ControlId::Dither => "dither",
            ControlId::Invert => "invert",
            ControlId::Grayscale => "grayscale",
            ControlId::Pixelize => "pixelize",
            ControlId::GaussianBlur => "gaussian-blur",
            ControlId::BoxBlur => "box-blur",
            ControlId::Oil => "oil",
            ControlId::Halftone => "halftone",
            ControlId::Sepia => "sepia",
            ControlId::Normalize => "normalize",
            ControlId::Noise => "noise",
            ControlId::Emboss => "emboss",
            ControlId::Identity => "identity",
            ControlId::Laplace => "laplace",
            ControlId::NoiseReduction => "noise-reduction",
            ControlId::Sharpen => "sharpen",
            ControlId::Cali => "cali",
            ControlId::Dramatic => "dramatic",
            ControlId::Firenze => "firenze",
            ControlId::Golden => "golden",
            ControlId::Lix => "lix",
            ControlId::Lofi => "lofi",
            ControlId::Neue => "neue",
            ControlId::Obsidian => "obsidian",
            ControlId::PastelPink => "pastel-pink",
            ControlId::Ryo => "ryo",
            ControlId::FrostedGlass => "frosted-glass",
            ControlId::Solarize => "solarize",
            ControlId::EdgeDetection => "edge-detection",
            ControlId::AutoOptimize => "auto-optimize",
            ControlId::ShowAdvanced => "advanced",
        }
    }

    fn requires_ocr(self) -> bool {
        matches!(
            self,
            ControlId::OcrLock
                | ControlId::OcrMegapixels
                | ControlId::OcrAutoWidth
                | ControlId::OcrFiglet
                | ControlId::OcrFigletMinHeight
                | ControlId::OcrFigletMinHeightRatio
                | ControlId::OcrFigletFill
                | ControlId::OcrFigletMaxWidthRatio
                | ControlId::OcrFigletMaxHeightRatio
                | ControlId::OcrMinConfidence
                | ControlId::OcrModelTier
                | ControlId::OcrAdvanced
                | ControlId::OcrMaxTextHeight
                | ControlId::OcrThreads
        )
    }

    fn available_in_build(self) -> bool {
        cfg!(feature = "ocr") || (self != ControlId::Ocr && !self.requires_ocr())
    }

    fn disabled_reason(self, ocr: bool, auto_width: bool, locked: bool) -> Option<&'static str> {
        if !ocr {
            return None;
        }
        if locked
            && ((self.requires_ocr() && !matches!(self, Self::OcrLock | Self::OcrAdvanced))
                || matches!(self, Self::Width | Self::AutoOptimize))
        {
            return Some(
                "Unlock OCR to change this setting; detections, overlays and grid size are frozen.",
            );
        }
        if auto_width {
            match self {
                Self::Width => return Some("Auto Output Width calculates the output width from detected text. Turn it off to set a manual width."),
                Self::AutoOptimize => return Some("Turn off Auto OCR Width before optimizing manual render dimensions."),
                _ => {}
            }
        }
        None
    }

    fn help(self) -> &'static str {
        match self {
            Self::Ocr => "Recognize text in the image, remove its original pixels and preserve it as editable text overlays.",
            Self::OcrAutoWidth => "Fit the output grid to recognized text. OCR input resolution is controlled separately by OCR Megapixels and remains editable.",
            Self::OcrLock => "Freeze current detections, text overlays and grid size while you adjust the image or edit text. Unlock to apply OCR settings again.",
            Self::OcrMegapixels => "Recognition detail in millions of pixels. Higher values retain small text but take longer. Keeps aspect ratio; never enlarges the source.",
            Self::OcrMinConfidence => "Keep text detections scoring at least this percentage. Raise it to reject uncertain text; lower it to recover faint or difficult text.",
            Self::OcrFigletFill => "Choose the largest font that fits the permitted extra width and height, constrained by nearby text and the canvas. Does not increase output width.",
            Self::OcrFiglet => "Render large detected text with decorative multi-row FIGlet fonts when they fit. Smaller text stays plain.",
            Self::OcrAdvanced => "Show or hide model, performance, text filtering and large-text styling settings. Hiding them preserves their values.",
            Self::OcrModelTier => "Recognition model size: Tiny is fastest; larger models use more memory and may improve difficult text recognition.",
            Self::OcrThreads => "Number of CPU workers used for OCR inference. More workers can speed recognition but use more CPU.",
            Self::OcrMaxTextHeight => "Ignore text taller than this percentage of the typical detected line. 0 keeps all text sizes; 200 allows lines up to twice as tall.",
            Self::OcrFigletMinHeight => "Minimum detected height in output rows before decorative text styling is allowed.",
            Self::OcrFigletMinHeightRatio => "Minimum text height relative to typical lines before styling. 125 means 25% taller; 0 disables this check.",
            Self::OcrFigletMaxWidthRatio => "Extra horizontal space decorative text may use beyond its box. 50 allows 50% extra, constrained by neighboring text.",
            Self::OcrFigletMaxHeightRatio => "Extra vertical space decorative text may use beyond its box. 50 allows 50% extra, constrained by neighboring text.",
            Self::Width => "Output width in character columns. 0 uses automatic image sizing. Auto OCR Width manages this when OCR is enabled.",
            Self::FontSize => "Size of the font used to match image regions to glyphs. 0 chooses a size automatically.",
            _ => "Arrow keys or mouse wheel adjust this option. Enter toggles or edits it; numeric values can also be typed directly.",
        }
    }

    fn requires_figlet(self) -> bool {
        matches!(
            self,
            ControlId::OcrFigletMinHeight
                | ControlId::OcrFigletMinHeightRatio
                | ControlId::OcrFigletFill
                | ControlId::OcrFigletMaxWidthRatio
                | ControlId::OcrFigletMaxHeightRatio
        )
    }

    fn requires_ocr_advanced(self) -> bool {
        self.requires_figlet()
            || matches!(
                self,
                Self::OcrModelTier | Self::OcrMaxTextHeight | Self::OcrThreads
            )
    }

    fn requires_advanced(self) -> bool {
        matches!(
            self,
            ControlId::ContourBendingWeight
                | ControlId::ContourEndpointWeight
                | ControlId::ContourJunctionWeight
                | ControlId::ContourFragmentWeight
                | ControlId::ContourFidelityWeight
                | ControlId::ContourBoundaryWeight
                | ControlId::ContourPeakWeight
        )
    }

    fn requires_score_fix(self) -> bool {
        matches!(
            self,
            ControlId::ScoreFixCandidates
                | ControlId::ScoreFixOrderings
                | ControlId::ScoreFixGeometryFirst
                | ControlId::ScoreFixNeighborhoodGuard
                | ControlId::ShowAdvanced
        ) || self.requires_advanced()
    }

    fn conditional_depth(self) -> u16 {
        if self.requires_ocr_advanced() || self.requires_advanced() {
            2
        } else if self.requires_ocr() || self.requires_score_fix() {
            1
        } else {
            0
        }
    }
}

#[derive(Clone, Copy)]
enum ControlKind {
    Slider {
        min: i32,
        max: i32,
        default: i32,
    },
    Toggle {
        default: bool,
    },
    Enum {
        options: &'static [&'static str],
        default: usize,
    },
    FontSelector,
    EyedropperWidget,
    Action,
}

#[derive(Clone, Copy)]
struct ControlDef {
    id: ControlId,
    label: &'static str,
    kind: ControlKind,
}

const GENERAL_CONTROLS: &[ControlDef] = &[
    ControlDef {
        id: ControlId::RenderMode,
        label: "Mode",
        kind: ControlKind::Enum {
            options: &["IRC", "ANSI", "ANSI24"],
            default: 1,
        },
    },
    ControlDef {
        id: ControlId::Width,
        label: "Width",
        kind: ControlKind::Slider {
            min: 0,
            max: 300,
            default: 80,
        },
    },
    ControlDef {
        id: ControlId::FontSize,
        label: "Font Size",
        kind: ControlKind::Slider {
            min: 0,
            max: 10240,
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::GraphicsPreview,
        label: "Kitty/Sixel Preview",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::Ocr,
        label: "OCR",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::OcrAutoWidth,
        label: "Auto Output Width",
        kind: ControlKind::Toggle { default: true },
    },
    ControlDef {
        id: ControlId::OcrLock,
        label: "Lock OCR",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::OcrMegapixels,
        label: "OCR Megapixels",
        kind: ControlKind::Slider {
            min: 1,
            max: 160,
            default: 15,
        },
    },
    ControlDef {
        id: ControlId::OcrMinConfidence,
        label: "Min Confidence %",
        kind: ControlKind::Slider {
            min: 0,
            max: 100,
            default: 70,
        },
    },
    ControlDef {
        id: ControlId::OcrFiglet,
        label: "Style Large Text",
        kind: ControlKind::Toggle { default: true },
    },
    ControlDef {
        id: ControlId::OcrFigletFill,
        label: "Fill Style Space",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::OcrFigletMinHeight,
        label: "Min Style Rows",
        kind: ControlKind::Slider {
            min: 0,
            max: 12,
            default: 2,
        },
    },
    ControlDef {
        id: ControlId::OcrFigletMinHeightRatio,
        label: "Min Relative Size %",
        kind: ControlKind::Slider {
            min: 0,
            max: 500,
            default: 125,
        },
    },
    ControlDef {
        id: ControlId::OcrFigletMaxWidthRatio,
        label: "Extra Style Width %",
        kind: ControlKind::Slider {
            min: 0,
            max: 200,
            default: 50,
        },
    },
    ControlDef {
        id: ControlId::OcrFigletMaxHeightRatio,
        label: "Extra Style Height %",
        kind: ControlKind::Slider {
            min: 0,
            max: 200,
            default: 50,
        },
    },
    ControlDef {
        id: ControlId::OcrAdvanced,
        label: "Advanced OCR",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::OcrModelTier,
        label: "Recognition Model",
        kind: ControlKind::Enum {
            options: &["Tiny", "Small", "Medium"],
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::OcrMaxTextHeight,
        label: "Max Text Size %",
        kind: ControlKind::Slider {
            min: 0,
            max: 1000,
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::OcrThreads,
        label: "Worker Threads",
        kind: ControlKind::Slider {
            min: 1,
            max: 32,
            default: 4,
        },
    },
    ControlDef {
        id: ControlId::HighlightDisc,
        label: "Show Disc",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::DiscThreshold,
        label: "Disc Thresh",
        kind: ControlKind::Slider {
            min: 0,
            max: 1000,
            default: 5,
        },
    },
    ControlDef {
        id: ControlId::ScoreFix,
        label: "Smoothing",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::ScoreFixCandidates,
        label: "Candidates",
        kind: ControlKind::Slider {
            min: 1,
            max: 256,
            default: 12,
        },
    },
    ControlDef {
        id: ControlId::ScoreFixOrderings,
        label: "Scan Patterns",
        kind: ControlKind::Slider {
            min: 1,
            max: 5,
            default: 1,
        },
    },
    ControlDef {
        id: ControlId::ScoreFixGeometryFirst,
        label: "Refine Shapes",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::ScoreFixNeighborhoodGuard,
        label: "Protect Neighbors",
        kind: ControlKind::Toggle { default: true },
    },
    ControlDef {
        id: ControlId::ShowAdvanced,
        label: "Advanced",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::ContourBendingWeight,
        label: "Bend W",
        kind: ControlKind::Slider {
            min: 0,
            max: 100,
            default: 100,
        },
    },
    ControlDef {
        id: ControlId::ContourEndpointWeight,
        label: "Endpoint W",
        kind: ControlKind::Slider {
            min: 0,
            max: 100,
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::ContourJunctionWeight,
        label: "Junction W",
        kind: ControlKind::Slider {
            min: 0,
            max: 100,
            default: 100,
        },
    },
    ControlDef {
        id: ControlId::ContourFragmentWeight,
        label: "Fragment W",
        kind: ControlKind::Slider {
            min: 0,
            max: 100,
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::ContourFidelityWeight,
        label: "Fidelity W",
        kind: ControlKind::Slider {
            min: 0,
            max: 100,
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::ContourBoundaryWeight,
        label: "Boundary W",
        kind: ControlKind::Slider {
            min: 0,
            max: 100,
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::ContourPeakWeight,
        label: "Peak Mix",
        kind: ControlKind::Slider {
            min: 0,
            max: 10,
            default: 0,
        },
    },
    // ControlDef {
    //     id: ControlId::Misc3,
    //     label: "Misc3",
    //     kind: ControlKind::Slider {
    //         min: 0,
    //         max: 100000,
    //         default: 0,
    //     },
    // },
    // ControlDef {
    //     id: ControlId::Misc4,
    //     label: "Relax Iter",
    //     kind: ControlKind::Slider {
    //         min: 0,
    //         max: 100,
    //         default: 0,
    //     },
    // },
    // ControlDef {
    //     id: ControlId::Misc5,
    //     label: "Misc5",
    //     kind: ControlKind::Slider {
    //         min: 0,
    //         max: 100000,
    //         default: 0,
    //     },
    // },
    // ControlDef {
    //     id: ControlId::Misc6,
    //     label: "Misc6",
    //     kind: ControlKind::Slider {
    //         min: 0,
    //         max: 100000,
    //         default: 0,
    //     },
    // },
    // ControlDef {
    //     id: ControlId::Misc7,
    //     label: "Misc7",
    //     kind: ControlKind::Slider {
    //         min: 0,
    //         max: 100000,
    //         default: 0,
    //     },
    // },
    // ControlDef {
    //     id: ControlId::Misc8,
    //     label: "Misc8",
    //     kind: ControlKind::Slider {
    //         min: 0,
    //         max: 100000,
    //         default: 0,
    //     },
    // },
    ControlDef {
        id: ControlId::Font,
        label: "Font",
        kind: ControlKind::FontSelector,
    },
    ControlDef {
        id: ControlId::Filter,
        label: "Filter",
        kind: ControlKind::Enum {
            options: &["Nearest", "Triangle", "CatmullRom", "Gaussian", "Lanczos3"],
            default: 4,
        },
    },
    ControlDef {
        id: ControlId::GrayTol,
        label: "Gray Tol",
        kind: ControlKind::Slider {
            min: 0,
            max: 255,
            default: 32,
        },
    },
    ControlDef {
        id: ControlId::Colorspace,
        label: "Colorspace",
        kind: ControlKind::Enum {
            options: &["HSL", "HSV", "HSLUV", "LCH"],
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::Encoding,
        label: "Encoding",
        kind: ControlKind::Enum {
            options: &["UTF-8", "UTF-16", "UTF-16BE", "UTF-16LE", "CESU-8"],
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::AutoOptimize,
        label: "Auto-Optimize",
        kind: ControlKind::Action,
    },
];

fn render_width_from_tui(width: i32) -> Option<u32> {
    (width > 0).then_some(width as u32)
}

const EFFECT_CONTROLS: &[ControlDef] = &[
    ControlDef {
        id: ControlId::Dither,
        label: "Dither",
        kind: ControlKind::Slider {
            min: 0,
            max: 8,
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::Invert,
        label: "Invert",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::Grayscale,
        label: "Grayscale",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::Pixelize,
        label: "Pixelize",
        kind: ControlKind::Slider {
            min: 0,
            max: 100,
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::GaussianBlur,
        label: "Gauss Blur",
        kind: ControlKind::Slider {
            min: 0,
            max: 100,
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::BoxBlur,
        label: "Box Blur",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::Oil,
        label: "Oil",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::Halftone,
        label: "Halftone",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::Sepia,
        label: "Sepia",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::Normalize,
        label: "Normalize",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::Noise,
        label: "Noise",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::Emboss,
        label: "Emboss",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::Identity,
        label: "Identity",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::Laplace,
        label: "Laplace",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::NoiseReduction,
        label: "Denoise",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::Sharpen,
        label: "Sharpen",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::Cali,
        label: "Cali",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::Dramatic,
        label: "Dramatic",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::Firenze,
        label: "Firenze",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::Golden,
        label: "Golden",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::Lix,
        label: "Lix",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::Lofi,
        label: "Lofi",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::Neue,
        label: "Neue",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::Obsidian,
        label: "Obsidian",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::PastelPink,
        label: "PastelPink",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::Ryo,
        label: "Ryo",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::FrostedGlass,
        label: "FrostGlass",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::Solarize,
        label: "Solarize",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::EdgeDetection,
        label: "EdgeDetect",
        kind: ControlKind::Toggle { default: false },
    },
];

fn get_control_def(id: ControlId) -> ControlDef {
    GENERAL_CONTROLS
        .iter()
        .find(|c| c.id == id)
        .copied()
        .expect("Control ID not found in GENERAL_CONTROLS")
}

#[derive(Clone)]
struct CatalogItem {
    label: &'static str,
    effect: Option<crate::pipeline::ImageEffect>,
}

impl CatalogItem {
    fn layout_id(&self) -> &'static str {
        use crate::pipeline::ImageEffect;

        match &self.effect {
            None => match self.label {
                "── Adjustments ──" => "adjustments",
                "── Geometry ──" => "geometry",
                "── Effects ──" => "effects",
                "── Stylize ──" => "stylize",
                "── Filters ──" => "filters",
                _ => "section",
            },
            Some(ImageEffect::Eyedropper) => "select-color",
            Some(ImageEffect::Brightness(_)) => "brightness",
            Some(ImageEffect::Contrast(_)) => "contrast",
            Some(ImageEffect::LumaContrast(_)) => "luma-contrast",
            Some(ImageEffect::MedianBlur(_)) => "median-blur",
            Some(ImageEffect::DarkLines(_) | ImageEffect::LightLines(_)) => "line-thickness",
            Some(ImageEffect::ReplaceColour { .. }) => "replace-colour",
            Some(ImageEffect::Saturation(_)) => "saturation",
            Some(ImageEffect::Gamma(_)) => "gamma",
            Some(ImageEffect::Hue(_)) => "hue",
            Some(ImageEffect::CropTop(_)) => "crop-top",
            Some(ImageEffect::CropBottom(_)) => "crop-bottom",
            Some(ImageEffect::CropLeft(_)) => "crop-left",
            Some(ImageEffect::CropRight(_)) => "crop-right",
            Some(ImageEffect::FlipH) => "flip-horizontal",
            Some(ImageEffect::FlipV) => "flip-vertical",
            Some(ImageEffect::Rotate(_)) => "rotate",
            Some(ImageEffect::ScaleX(_)) => "scale-x",
            Some(ImageEffect::ScaleY(_)) => "scale-y",
            Some(ImageEffect::GaussBlur(_)) => "gaussian-blur",
            Some(ImageEffect::BoxBlur) => "box-blur",
            Some(ImageEffect::Pixelize(_)) => "pixelize",
            Some(ImageEffect::Dither(_)) => "dither",
            Some(ImageEffect::Invert) => "invert",
            Some(ImageEffect::Grayscale) => "grayscale",
            Some(ImageEffect::Halftone) => "halftone",
            Some(ImageEffect::Sepia) => "sepia",
            Some(ImageEffect::Solarize) => "solarize",
            Some(ImageEffect::Normalize) => "normalize",
            Some(ImageEffect::Noise) => "noise",
            Some(ImageEffect::Sharpen) => "sharpen",
            Some(ImageEffect::EdgeDetect) => "edge-detection",
            Some(ImageEffect::Emboss) => "emboss",
            Some(ImageEffect::FrostGlass) => "frosted-glass",
            Some(ImageEffect::Laplace) => "laplace",
            Some(ImageEffect::Identity) => "identity",
            Some(ImageEffect::Cali) => "cali",
            Some(ImageEffect::Dramatic) => "dramatic",
            Some(ImageEffect::Firenze) => "firenze",
            Some(ImageEffect::Golden) => "golden",
            Some(ImageEffect::Lix) => "lix",
            Some(ImageEffect::Lofi) => "lofi",
            Some(ImageEffect::Neue) => "neue",
            Some(ImageEffect::Obsidian) => "obsidian",
            Some(ImageEffect::PastelPink) => "pastel-pink",
            Some(ImageEffect::Ryo) => "ryo",
            Some(ImageEffect::LumaBlur(_)) => "luma-blur",
            Some(ImageEffect::Oil(_, _)) => "oil",
        }
    }
}

const PIPELINE_CATALOG: &[CatalogItem] = &[
    CatalogItem {
        label: "── Adjustments ──",
        effect: None,
    },
    CatalogItem {
        label: "Select Color",
        effect: Some(crate::pipeline::ImageEffect::Eyedropper),
    },
    CatalogItem {
        label: "Brightness",
        effect: Some(crate::pipeline::ImageEffect::Brightness(0.0)),
    },
    CatalogItem {
        label: "Contrast",
        effect: Some(crate::pipeline::ImageEffect::Contrast(0.0)),
    },
    CatalogItem {
        label: "Replace Colour",
        effect: Some(crate::pipeline::ImageEffect::ReplaceColour { from: [255; 3], to: [0; 3], tolerance: 5.0 }),
    },
    CatalogItem {
        label: "Luma Contrast",
        effect: Some(crate::pipeline::ImageEffect::LumaContrast(0.0)),
    },
    CatalogItem {
        label: "Saturation",
        effect: Some(crate::pipeline::ImageEffect::Saturation(0.0)),
    },
    CatalogItem {
        label: "Gamma",
        effect: Some(crate::pipeline::ImageEffect::Gamma(0.0)),
    },
    CatalogItem {
        label: "Hue",
        effect: Some(crate::pipeline::ImageEffect::Hue(0.0)),
    },
    CatalogItem {
        label: "── Geometry ──",
        effect: None,
    },
    CatalogItem {
        label: "Crop Top",
        effect: Some(crate::pipeline::ImageEffect::CropTop(0)),
    },
    CatalogItem {
        label: "Crop Right",
        effect: Some(crate::pipeline::ImageEffect::CropRight(0)),
    },
    CatalogItem {
        label: "Crop Bottom",
        effect: Some(crate::pipeline::ImageEffect::CropBottom(0)),
    },
    CatalogItem {
        label: "Crop Left",
        effect: Some(crate::pipeline::ImageEffect::CropLeft(0)),
    },
    CatalogItem {
        label: "Flip Horizontal",
        effect: Some(crate::pipeline::ImageEffect::FlipH),
    },
    CatalogItem {
        label: "Flip Vertical",
        effect: Some(crate::pipeline::ImageEffect::FlipV),
    },
    CatalogItem {
        label: "Rotate",
        effect: Some(crate::pipeline::ImageEffect::Rotate(0.0)),
    },
    CatalogItem {
        label: "Scale X",
        effect: Some(crate::pipeline::ImageEffect::ScaleX(100)),
    },
    CatalogItem {
        label: "Scale Y",
        effect: Some(crate::pipeline::ImageEffect::ScaleY(100)),
    },
    CatalogItem {
        label: "── Effects ──",
        effect: None,
    },
    CatalogItem {
        label: "Median Blur",
        effect: Some(crate::pipeline::ImageEffect::MedianBlur(1)),
    },
    CatalogItem {
        label: "Line Thickness (+dark / -light)",
        effect: Some(crate::pipeline::ImageEffect::DarkLines(1)),
    },
    CatalogItem {
        label: "Gauss Blur",
        effect: Some(crate::pipeline::ImageEffect::GaussBlur(1)),
    },
    CatalogItem {
        label: "Box Blur",
        effect: Some(crate::pipeline::ImageEffect::BoxBlur),
    },
    CatalogItem {
        label: "Pixelize",
        effect: Some(crate::pipeline::ImageEffect::Pixelize(2)),
    },
    CatalogItem {
        label: "Dither",
        effect: Some(crate::pipeline::ImageEffect::Dither(1)),
    },
    CatalogItem {
        label: "── Stylize ──",
        effect: None,
    },
    CatalogItem {
        label: "Invert",
        effect: Some(crate::pipeline::ImageEffect::Invert),
    },
    CatalogItem {
        label: "Grayscale",
        effect: Some(crate::pipeline::ImageEffect::Grayscale),
    },
    CatalogItem {
        label: "Halftone",
        effect: Some(crate::pipeline::ImageEffect::Halftone),
    },
    CatalogItem {
        label: "Sepia",
        effect: Some(crate::pipeline::ImageEffect::Sepia),
    },
    CatalogItem {
        label: "Solarize",
        effect: Some(crate::pipeline::ImageEffect::Solarize),
    },
    CatalogItem {
        label: "Normalize",
        effect: Some(crate::pipeline::ImageEffect::Normalize),
    },
    CatalogItem {
        label: "Noise",
        effect: Some(crate::pipeline::ImageEffect::Noise),
    },
    CatalogItem {
        label: "Sharpen",
        effect: Some(crate::pipeline::ImageEffect::Sharpen),
    },
    CatalogItem {
        label: "Edge Detect",
        effect: Some(crate::pipeline::ImageEffect::EdgeDetect),
    },
    CatalogItem {
        label: "Emboss",
        effect: Some(crate::pipeline::ImageEffect::Emboss),
    },
    CatalogItem {
        label: "Frost Glass",
        effect: Some(crate::pipeline::ImageEffect::FrostGlass),
    },
    CatalogItem {
        label: "Laplace",
        effect: Some(crate::pipeline::ImageEffect::Laplace),
    },
    CatalogItem {
        label: "Identity",
        effect: Some(crate::pipeline::ImageEffect::Identity),
    },
    CatalogItem {
        label: "── Filters ──",
        effect: None,
    },
    CatalogItem {
        label: "Cali",
        effect: Some(crate::pipeline::ImageEffect::Cali),
    },
    CatalogItem {
        label: "Dramatic",
        effect: Some(crate::pipeline::ImageEffect::Dramatic),
    },
    CatalogItem {
        label: "Firenze",
        effect: Some(crate::pipeline::ImageEffect::Firenze),
    },
    CatalogItem {
        label: "Golden",
        effect: Some(crate::pipeline::ImageEffect::Golden),
    },
    CatalogItem {
        label: "Lix",
        effect: Some(crate::pipeline::ImageEffect::Lix),
    },
    CatalogItem {
        label: "Lofi",
        effect: Some(crate::pipeline::ImageEffect::Lofi),
    },
    CatalogItem {
        label: "Neue",
        effect: Some(crate::pipeline::ImageEffect::Neue),
    },
    CatalogItem {
        label: "Obsidian",
        effect: Some(crate::pipeline::ImageEffect::Obsidian),
    },
    CatalogItem {
        label: "Pastel Pink",
        effect: Some(crate::pipeline::ImageEffect::PastelPink),
    },
    CatalogItem {
        label: "Ryo",
        effect: Some(crate::pipeline::ImageEffect::Ryo),
    },
];

fn normalized_layout_id(id: &str) -> String {
    let normalized = id.trim().to_ascii_lowercase().replace('_', "-");
    match normalized.as_str() {
        "ocr-width" | "ocr-max-side" => "ocr-megapixels".to_string(),
        "score-fix" => "smooth".to_string(),
        "score-fix-candidates" => "smooth-candidates".to_string(),
        "score-fix-orderings" => "smooth-orders".to_string(),
        "score-fix-geometry" => "smooth-shapes".to_string(),
        "score-fix-overlap" => "smooth-neighbors".to_string(),
        _ => normalized,
    }
}

fn arrange_general_controls(layout: &crate::config::ItemLayout) -> Vec<ControlDef> {
    let hidden = layout
        .hidden
        .iter()
        .map(|id| normalized_layout_id(id))
        .collect::<std::collections::HashSet<_>>();
    let mut arranged = Vec::with_capacity(GENERAL_CONTROLS.len());
    let mut used = std::collections::HashSet::new();

    for requested in &layout.order {
        let requested = normalized_layout_id(requested);
        if hidden.contains(&requested) || !used.insert(requested.clone()) {
            continue;
        }
        if let Some(control) = GENERAL_CONTROLS
            .iter()
            .find(|control| control.id.layout_id() == requested)
        {
            if control.id.available_in_build() {
                arranged.push(*control);
            }
        } else {
            log::warn!("Unknown General layout item '{requested}', ignoring it");
        }
    }

    for control in GENERAL_CONTROLS {
        let id = control.id.layout_id();
        if control.id.available_in_build()
            && !hidden.contains(id)
            && used.insert(id.to_string())
        {
            arranged.push(*control);
        }
    }

    // Treat OCR as a group, including new controls absent from older layouts.
    // Preserve the parent's position and explicit hidden entries.
    if let Some(parent) = arranged.iter().position(|control| control.id == ControlId::Ocr) {
        let insertion = arranged[..=parent].iter().filter(|control| !control.id.requires_ocr()).count();
        let children = GENERAL_CONTROLS.iter().filter(|control| control.id.requires_ocr()
            && arranged.iter().any(|existing| existing.id == control.id)).copied().collect::<Vec<_>>();
        arranged.retain(|control| !control.id.requires_ocr());
        let basic = children.iter().filter(|control| !control.id.requires_ocr_advanced()
            && control.id != ControlId::OcrAdvanced).copied();
        let advanced_toggle = children.iter().filter(|control| control.id == ControlId::OcrAdvanced).copied();
        let advanced = children.iter().filter(|control| control.id.requires_ocr_advanced()).copied();
        arranged.splice(insertion..insertion, basic.chain(advanced_toggle).chain(advanced));
    } else {
        arranged.retain(|control| !control.id.requires_ocr());
    }

    // Several input handlers expect the General tab to retain a focusable
    // control. Keep Mode as a safe anchor if a layout hides every item.
    if arranged.is_empty() {
        log::warn!("General layout hid every control; keeping 'mode' visible");
        arranged.push(
            *GENERAL_CONTROLS
                .iter()
                .find(|control| control.id == ControlId::RenderMode)
                .expect("General Mode control must exist"),
        );
    }

    arranged
}

fn visible_general_controls(
    controls: &[ControlDef],
    ocr_enabled: bool,
    figlet_enabled: bool,
    score_fix_enabled: bool,
    show_advanced: bool,
    show_ocr_advanced: bool,
) -> Vec<ControlDef> {
    controls
        .iter()
        .filter(|control| {
            !matches!(control.id, ControlId::HighlightDisc | ControlId::DiscThreshold)
        })
        .filter(|control| ocr_enabled || !control.id.requires_ocr())
        .filter(|control| figlet_enabled || !control.id.requires_figlet())
        .filter(|control| score_fix_enabled || !control.id.requires_score_fix())
        .filter(|control| show_advanced || !control.id.requires_advanced())
        .filter(|control| show_ocr_advanced || !control.id.requires_ocr_advanced())
        .copied()
        .collect()
}

fn arrange_pipeline_catalog(layout: &crate::config::ItemLayout) -> Vec<CatalogItem> {
    let hidden = layout
        .hidden
        .iter()
        .map(|id| normalized_layout_id(id))
        .collect::<std::collections::HashSet<_>>();
    let mut arranged = Vec::with_capacity(PIPELINE_CATALOG.len());
    let mut used = std::collections::HashSet::new();

    for requested in &layout.order {
        let requested = normalized_layout_id(requested);
        if hidden.contains(&requested) || !used.insert(requested.clone()) {
            continue;
        }
        if let Some(item) = PIPELINE_CATALOG
            .iter()
            .find(|item| item.layout_id() == requested)
        {
            arranged.push(item.clone());
        } else {
            log::warn!("Unknown Pipeline layout item '{requested}', ignoring it");
        }
    }

    for item in PIPELINE_CATALOG {
        let id = item.layout_id();
        if !hidden.contains(id) && used.insert(id.to_string()) {
            arranged.push(item.clone());
        }
    }

    if !arranged.iter().any(|item| item.effect.is_some()) {
        log::warn!("Pipeline layout hid every effect; keeping 'select-color' visible");
        arranged.push(
            PIPELINE_CATALOG
                .iter()
                .find(|item| item.layout_id() == "select-color")
                .expect("Select Color catalog item must exist")
                .clone(),
        );
    }

    arranged
}

fn cancel_pipeline_editor(
    expanded: &mut Option<usize>,
    dragging: &mut Option<(usize, i16, i16)>,
) -> bool {
    if expanded.take().is_none() {
        return false;
    }
    *dragging = None;
    true
}

fn centered_rect(container: Rect, content: Rect) -> Rect {
    let width = content.width.min(container.width);
    let height = content.height.min(container.height);
    Rect::new(
        container
            .x
            .saturating_add(container.width.saturating_sub(width) / 2),
        container
            .y
            .saturating_add(container.height.saturating_sub(height) / 2),
        width,
        height,
    )
}

fn clamp_output_scroll(scroll: u16, line_count: u16, viewport_height: u16) -> u16 {
    scroll.min(line_count.saturating_sub(viewport_height))
}

#[cfg(test)]
mod ui_layout_tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn older_layouts_keep_all_ocr_options_beside_the_parent() {
        if !cfg!(feature = "ocr") {
            return;
        }
        let layout = crate::config::ItemLayout {
            order: vec![
                "ocr".into(),
                "ocr-width".into(),
                "ocr-max-side".into(),
                "font".into(),
                "width".into(),
            ],
            hidden: vec!["ocr-threads".into()],
        };
        let controls = arrange_general_controls(&layout);
        assert_eq!(controls[0].id, ControlId::Ocr);
        let end = controls
            .iter()
            .skip(1)
            .take_while(|c| c.id.requires_ocr())
            .count()
            + 1;
        assert!(controls[1..end]
            .iter()
            .any(|c| c.id == ControlId::OcrAutoWidth));
        assert!(controls[1..end].iter().any(|c| c.id == ControlId::OcrLock));
        assert_eq!(
            controls[1..end]
                .iter()
                .filter(|c| c.id == ControlId::OcrMegapixels)
                .count(),
            1
        );
        assert!(!controls[end..].iter().any(|c| c.id.requires_ocr()));
        assert!(!controls.iter().any(|c| c.id == ControlId::OcrThreads));
        let toggle = controls
            .iter()
            .position(|c| c.id == ControlId::OcrAdvanced)
            .unwrap();
        assert!(controls[1..toggle]
            .iter()
            .all(|c| !c.id.requires_ocr_advanced()));
        assert!(controls[toggle + 1..end]
            .iter()
            .all(|c| c.id.requires_ocr_advanced()));
    }

    #[test]
    fn ocr_advanced_is_independent_of_smoothing_advanced() {
        if !cfg!(feature = "ocr") {
            return;
        }
        let controls = arrange_general_controls(&crate::config::ItemLayout::default());
        let collapsed = visible_general_controls(&controls, true, true, true, true, false);
        assert!(collapsed.iter().any(|c| c.id == ControlId::OcrAdvanced));
        assert!(!collapsed.iter().any(|c| c.id.requires_ocr_advanced()));
        let expanded = visible_general_controls(&controls, true, true, true, false, true);
        assert!(expanded.iter().any(|c| c.id.requires_ocr_advanced()));
        assert!(!expanded.iter().any(|c| c.id.requires_advanced()));
    }

    #[test]
    fn ocr_input_budget_cache_ignores_inactive_manual_settings() {
        let parsed = crate::args::Args::parse_from(["img2irc"]);
        let mut args = crate::args_to_tui_render_args(&parsed);
        let automatic = OcrInferenceKey::from(&args);
        args.ocr_megapixels = Some(8.0);
        args.ocr_width = Some(400);
        args.ocr_max_side_len = 500;
        assert_eq!(OcrInferenceKey::from(&args), automatic);
        args.ocr_auto_width = false;
        let manual = OcrInferenceKey::from(&args);
        assert_ne!(manual, automatic);
        args.ocr_width = Some(800);
        args.ocr_max_side_len = 900;
        assert_eq!(OcrInferenceKey::from(&args), manual);
        args.ocr_megapixels = Some(2.0);
        assert_ne!(OcrInferenceKey::from(&args), manual);
    }

    #[test]
    fn disabled_ocr_controls_reject_edits_and_explain_why_on_screen() {
        if !cfg!(feature = "ocr") {
            return;
        }
        let parsed = crate::args::Args::parse_from(["img2irc", "--ocr"]);
        let args = crate::args_to_render_args(&parsed);
        let flag = Arc::new(AtomicBool::new(false));
        let (tx, _rx) = watch::channel((args.clone(), flag.clone(), Instant::now()));
        let (_result_tx, result_rx) = mpsc::channel(1);
        let mut app = App::new(args, tx, result_rx, flag);
        assert!(!app.show_ocr_advanced);
        app.selected_control = app
            .tab_controls(0)
            .iter()
            .position(|c| c.id == ControlId::OcrMegapixels)
            .unwrap();
        let original = app.get_value(ControlId::OcrMegapixels);
        let original_width = app.width;
        app.set_value(ControlId::Width, 999);
        assert_eq!(app.width, original_width);
        adjust_control(&mut app, 10);
        reset_control(&mut app);
        app.set_value(ControlId::OcrMegapixels, 90);
        assert_eq!(app.get_value(ControlId::OcrMegapixels), original);
        assert!(!app.args_dirty);

        let backend = ratatui::backend::TestBackend::new(64, 30);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| general::render_controls(frame, &mut app, frame.area()))
            .unwrap();
        let screen = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(screen.contains("OCR Megapixels"));
        assert!(screen.contains("Disabled"));
        assert!(screen.contains("manages detection detail"), "{screen}");
        assert!(!screen.contains("Recognition Model"));
        let row = app.control_areas[app.selected_control];
        let label_row = (row.x..row.right()).map(|x| terminal.backend().buffer()[(x, row.y)].symbol()).collect::<String>();
        assert!(label_row.contains("OCR Megapixels"));
        for (width, height) in [(40, 24), (24, 12), (8, 3)] {
            let mut small = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            small.draw(|frame| general::render_controls(frame, &mut app, frame.area())).unwrap();
        }

        app.set_value(ControlId::OcrAutoWidth, 0);
        app.set_value(ControlId::OcrMegapixels, 90);
        assert_eq!(app.get_value(ControlId::OcrMegapixels), 90);
        app.set_value(ControlId::OcrLock, 1);
        app.set_value(ControlId::OcrMegapixels, 40);
        assert_eq!(app.get_value(ControlId::OcrMegapixels), 90);
        app.set_value(ControlId::OcrAdvanced, 1);
        assert!(app.show_ocr_advanced);
        app.set_value(ControlId::OcrLock, 0);
        assert!(!app.ocr_lock);
    }

    #[test]
    fn layout_changes_do_not_invalidate_ocr_inference() {
        let parsed = crate::args::Args::parse_from(["img2irc"]);
        let mut args = crate::args_to_render_args(&parsed);
        let original = OcrInferenceKey::from(&args);

        args.width = Some(120);
        args.ocr_figlet = false;
        args.ocr_figlet_min_height = 4.0;
        args.ocr_figlet_min_height_ratio = 2.0;
        assert_eq!(OcrInferenceKey::from(&args), original);

        args.ocr_model_tier = OcrModelTier::Small;
        assert_ne!(OcrInferenceKey::from(&args), original);

        args.brightness = 20.0;
        assert_ne!(OcrInferenceKey::from(&args), original);
    }

    #[test]
    fn zero_width_is_available_and_means_auto() {
        let ControlKind::Slider { min, .. } = get_control_def(ControlId::Width).kind else {
            panic!("width control should be a slider");
        };

        assert_eq!(min, 0);
        assert_eq!(render_width_from_tui(0), None);
        assert_eq!(render_width_from_tui(80), Some(80));
    }

    #[test]
    fn geometry_is_only_in_the_pipeline_library() {
        let general = arrange_general_controls(&crate::config::ItemLayout::default());
        let pipeline = arrange_pipeline_catalog(&crate::config::ItemLayout::default());

        for id in [
            "crop-top",
            "crop-right",
            "crop-bottom",
            "crop-left",
            "flip-horizontal",
            "flip-vertical",
            "rotate",
            "scale-x",
            "scale-y",
        ] {
            assert!(!general.iter().any(|control| control.id.layout_id() == id));
            assert!(pipeline.iter().any(|item| item.layout_id() == id));
        }
    }

    #[test]
    fn hides_ocr_children_until_ocr_is_enabled() {
        let controls = arrange_general_controls(&crate::config::ItemLayout::default());

        if !cfg!(feature = "ocr") {
            assert!(!controls.iter().any(|control| {
                control.id == ControlId::Ocr || control.id.requires_ocr()
            }));
            return;
        }

        let disabled = visible_general_controls(&controls, false, true, true, true, true);
        let enabled = visible_general_controls(&controls, true, true, true, true, true);

        assert!(disabled.iter().any(|control| control.id == ControlId::Ocr));
        assert!(!disabled.iter().any(|control| control.id.requires_ocr()));
        assert!(enabled.iter().any(|control| control.id.requires_ocr()));
    }

    #[test]
    fn hides_figlet_thresholds_until_figlet_is_enabled() {
        let controls = arrange_general_controls(&crate::config::ItemLayout::default());

        if !cfg!(feature = "ocr") {
            assert!(!controls.iter().any(|control| {
                control.id == ControlId::OcrFiglet || control.id.requires_figlet()
            }));
            return;
        }

        let disabled = visible_general_controls(&controls, true, false, true, true, true);
        let enabled = visible_general_controls(&controls, true, true, true, true, true);

        assert!(disabled.iter().any(|control| control.id == ControlId::OcrFiglet));
        assert!(!disabled.iter().any(|control| control.id.requires_figlet()));
        assert!(enabled.iter().any(|control| control.id.requires_figlet()));
    }

    #[test]
    fn hides_score_fix_children_until_score_fix_is_enabled() {
        let controls = arrange_general_controls(&crate::config::ItemLayout::default());
        let disabled = visible_general_controls(&controls, true, true, false, true, true);
        let enabled = visible_general_controls(&controls, true, true, true, true, true);

        assert!(disabled.iter().any(|control| control.id == ControlId::ScoreFix));
        assert!(!disabled.iter().any(|control| control.id.requires_score_fix()));
        assert!(enabled.iter().any(|control| control.id.requires_score_fix()));
    }

    #[test]
    fn general_starts_with_mode_width_and_font_size() {
        let controls = arrange_general_controls(&crate::config::ItemLayout::default());
        let leading_ids = controls
            .iter()
            .take(3)
            .map(|control| control.id)
            .collect::<Vec<_>>();

        assert_eq!(
            leading_ids,
            [ControlId::RenderMode, ControlId::Width, ControlId::FontSize]
        );
    }

    #[test]
    fn discontinuity_debug_controls_are_not_visible() {
        let controls = arrange_general_controls(&crate::config::ItemLayout::default());
        let visible = visible_general_controls(&controls, true, true, true, true, true);

        assert!(!visible.iter().any(|control| matches!(
            control.id,
            ControlId::HighlightDisc | ControlId::DiscThreshold
        )));
    }

    #[test]
    fn conditional_controls_have_nested_hierarchy_depths() {
        assert_eq!(ControlId::OcrFiglet.conditional_depth(), 1);
        assert_eq!(ControlId::OcrFigletMinHeight.conditional_depth(), 2);
        assert_eq!(ControlId::ScoreFixCandidates.conditional_depth(), 1);
        assert_eq!(ControlId::ContourBendingWeight.conditional_depth(), 2);
        assert_ne!(SURFACE, CONDITIONAL_BG);
        assert_ne!(CONDITIONAL_BG, NESTED_CONDITIONAL_BG);
    }

    #[test]
    fn layout_reorders_and_hides_inputs() {
        let layout = crate::config::ItemLayout {
            order: vec!["font".into(), "width".into()],
            hidden: vec!["encoding".into()],
        };
        let controls = arrange_general_controls(&layout);

        assert_eq!(controls[0].id, ControlId::Font);
        assert_eq!(controls[1].id, ControlId::Width);
        assert!(!controls
            .iter()
            .any(|control| control.id == ControlId::Encoding));
    }

    #[test]
    fn bundled_layout_uses_known_stable_ids() {
        let layout: crate::config::UiLayout =
            toml::from_str(include_str!("../layout.toml")).unwrap();

        assert!(layout.general.order.iter().all(|requested| {
            let requested = normalized_layout_id(requested);
            GENERAL_CONTROLS
                .iter()
                .any(|control| control.id.layout_id() == requested)
        }));
        assert!(layout.pipeline.order.iter().all(|requested| {
            let requested = normalized_layout_id(requested);
            PIPELINE_CATALOG
                .iter()
                .any(|item| item.layout_id() == requested)
        }));
    }

    #[test]
    fn escape_cancels_an_open_pipeline_editor() {
        let mut expanded = Some(3);
        let mut dragging = Some((12, 2, 3));

        assert!(cancel_pipeline_editor(&mut expanded, &mut dragging));
        assert_eq!(expanded, None);
        assert_eq!(dragging, None);
        assert!(!cancel_pipeline_editor(&mut expanded, &mut dragging));
    }

    #[test]
    fn fitted_preview_area_is_centered_and_clamped() {
        let container = Rect::new(10, 20, 20, 10);
        assert_eq!(
            centered_rect(container, Rect::new(0, 0, 8, 4)),
            Rect::new(16, 23, 8, 4)
        );
        assert_eq!(
            centered_rect(container, Rect::new(0, 0, 30, 16)),
            container
        );
    }

    #[test]
    fn output_scroll_resets_when_new_content_fits_the_viewport() {
        assert_eq!(clamp_output_scroll(40, 12, 20), 0);
        assert_eq!(clamp_output_scroll(40, 100, 20), 40);
        assert_eq!(clamp_output_scroll(90, 100, 20), 80);
    }

    #[test]
    fn fit_to_text_tracks_explicit_lines_in_both_directions() {
        let mut overlay = new_text_overlay(2, 4, 5, true);
        overlay.text = "hello\nworld!".to_string();
        sync_text_overlay_auto_grow(&mut overlay);
        assert_eq!((overlay.w, overlay.h), (6, 2));

        overlay.text = "x".to_string();
        sync_text_overlay_auto_grow(&mut overlay);
        assert_eq!((overlay.w, overlay.h), (1, 1));
    }

    #[test]
    fn lower_right_corner_can_resize_from_both_sides() {
        let mut overlay = new_text_overlay(2, 10, 20, false);
        overlay.w = 8;
        overlay.h = 3;

        assert!(text_overlay_resize_hotspot(&overlay, 17, 22));
        assert!(text_overlay_resize_hotspot(&overlay, 18, 22));
        assert!(text_overlay_resize_hotspot(&overlay, 17, 23));
        assert!(!text_overlay_resize_hotspot(&overlay, 16, 22));
        assert!(!text_overlay_resize_hotspot(&overlay, 17, 21));
        assert!(!text_overlay_resize_hotspot(&overlay, 18, 23));
    }

    #[test]
    fn new_text_has_a_visible_foreground_for_every_render_mode() {
        assert_eq!(
            new_text_overlay(0, 0, 0, true).fg,
            Some(crate::args::ColorSpec::Index(0))
        );
        assert_eq!(
            new_text_overlay(1, 0, 0, true).fg,
            Some(crate::args::ColorSpec::Index(15))
        );
        assert_eq!(
            new_text_overlay(2, 0, 0, true).fg,
            Some(crate::args::ColorSpec::Rgb([255, 255, 255]))
        );
    }

    #[test]
    fn glyph_tree_supports_arbitrary_depth_and_clamped_navigation() {
        let blocks = vec![
            "blocks.eighth.vertical".to_string(),
            "blocks.eighth.horizontal".to_string(),
            "fidelity.legacy.one".to_string(),
        ];
        let mut tree = TreeState::new(&blocks);

        let blocks_node = tree.root.iter().find(|node| node.name == "blocks").unwrap();
        assert_eq!(blocks_node.full_path, "blocks");
        let eighth = blocks_node
            .children
            .iter()
            .find(|node| node.name == "eighth")
            .unwrap();
        assert_eq!(eighth.full_path, "blocks.eighth");
        assert_eq!(eighth.children.len(), 2);
        assert!(eighth.children.iter().all(|node| node.level == 2));

        let count = tree.flatten().len();
        for _ in 0..count + 5 {
            tree.select_next();
        }
        assert_eq!(tree.selected_idx, count - 1);
        for _ in 0..count + 5 {
            tree.select_prev();
        }
        assert_eq!(tree.selected_idx, 0);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextFieldFocus {
    List,
    TextInput,
    FgButton,
    ClearFgButton,
    BgButton,
    ClearBgButton,
    WrapButton,
    FitButton,
    FigletButton,
    FigletFontButton,
    BoldButton,
    ItalicButton,
    UnderlineButton,
    DeleteButton,
}

const TEXT_FOCUS_ORDER: &[TextFieldFocus] = &[
    TextFieldFocus::List,
    TextFieldFocus::TextInput,
    TextFieldFocus::FgButton,
    TextFieldFocus::ClearFgButton,
    TextFieldFocus::BgButton,
    TextFieldFocus::ClearBgButton,
    TextFieldFocus::WrapButton,
    TextFieldFocus::FitButton,
    TextFieldFocus::FigletButton,
    TextFieldFocus::FigletFontButton,
    TextFieldFocus::BoldButton,
    TextFieldFocus::ItalicButton,
    TextFieldFocus::UnderlineButton,
    TextFieldFocus::DeleteButton,
];

// ── App State ────────────────────────────────────────────────────────────────
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputMode {
    Normal,
    Explorer,
    ValueEdit,
    TextEdit,
    MetadataPrompt,
    AutoOptimize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FocusTarget {
    General,
    Pipeline,
    GlyphTree,
    GlyphList,
    GlyphSelectAll,
    GlyphSelectNone,
    GlyphAdd,
    GlyphDelete,
    Text,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SplitterKind {
    General,
    Pipeline,
    PipelineActive,
    Glyphs,
    GlyphTreeRows,
    GlyphListRows,
    Text,
}

#[derive(Clone, Copy, Debug)]
struct SplitterArea {
    kind: SplitterKind,
    area: Rect,
}

struct AutoOptimizeState {
    inputs: Vec<TextArea<'static>>,
    enabled_rows: [bool; 8],
    focused_idx: usize,
    optimize_shortest_longest_line: bool,
    batch_size: u32,
    randomize: bool,
    input_areas: [Rect; 24],
    row_toggle_areas: [Rect; 8],
    shortest_line_area: Rect,
    randomize_area: Rect,
    batch_size_area: Rect,
    start_area: Rect,
    cancel_area: Rect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AutoOptimizeMouseAction {
    None,
    Start,
    Cancel,
}

fn adjust_auto_optimize_input(
    state: &mut AutoOptimizeState,
    input_idx: usize,
    delta: i32,
    current_values: &[i32; 8],
) {
    let row = input_idx / 3;
    let column = input_idx % 3;
    let fallback = if column == 2 { 1 } else { current_values[row] };
    let current = state.inputs[input_idx]
        .lines()
        .first()
        .and_then(|line| line.trim().parse::<i32>().ok())
        .unwrap_or(fallback);
    let minimum = if column == 2 {
        1
    } else {
        match row {
            0 | 5 => 1,
            1..=4 => 0,
            _ => 1,
        }
    };
    let value = current.saturating_add(delta).max(minimum);
    let mut input = TextArea::new(vec![value.to_string()]);
    input.move_cursor(CursorMove::End);
    state.inputs[input_idx] = input;
    state.focused_idx = input_idx;
}

fn handle_auto_optimize_mouse(
    state: &mut AutoOptimizeState,
    mouse: crossterm::event::MouseEvent,
    current_values: &[i32; 8],
) -> AutoOptimizeMouseAction {
    let pos = ratatui::layout::Position::new(mouse.column, mouse.row);
    let hovered_input = state.input_areas.iter().position(|area| area.contains(pos));
    let hovered_toggle = state
        .row_toggle_areas
        .iter()
        .position(|area| area.contains(pos));

    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if let Some(row) = hovered_toggle {
                state.enabled_rows[row] = !state.enabled_rows[row];
            } else if let Some(input_idx) = hovered_input {
                state.focused_idx = input_idx;
            } else if state.shortest_line_area.contains(pos) {
                state.optimize_shortest_longest_line = !state.optimize_shortest_longest_line;
                state.focused_idx = 24;
            } else if state.randomize_area.contains(pos) {
                state.randomize = !state.randomize;
                state.focused_idx = 25;
            } else if state.batch_size_area.contains(pos) {
                state.focused_idx = 26;
            } else if state.start_area.contains(pos) {
                return AutoOptimizeMouseAction::Start;
            } else if state.cancel_area.contains(pos) {
                return AutoOptimizeMouseAction::Cancel;
            }
        }
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
            let magnitude = if mouse.modifiers.contains(KeyModifiers::SHIFT) {
                10
            } else {
                1
            };
            let delta = if mouse.kind == MouseEventKind::ScrollUp {
                magnitude
            } else {
                -magnitude
            };
            if let Some(input_idx) = hovered_input {
                adjust_auto_optimize_input(state, input_idx, delta, current_values);
            } else if state.batch_size_area.contains(pos) {
                state.focused_idx = 26;
                state.batch_size = state
                    .batch_size
                    .saturating_add_signed(delta)
                    .clamp(1, 100);
            }
        }
        _ => {}
    }
    AutoOptimizeMouseAction::None
}
struct ExplorerDialog {
    explorer: FileExplorer,
    mode: ExplorerMode,
    name_input: TextArea<'static>,
    focus_on_input: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExplorerMode {
    Open,
    SaveText,
    SavePng,
}

#[derive(Clone, Debug)]
struct TreeNode {
    name: String,
    full_path: String,
    children: Vec<TreeNode>,
    expanded: bool,
    level: usize,
    is_leaf: bool,
}

impl TreeNode {
    fn new(name: String, full_path: String, level: usize, is_leaf: bool) -> Self {
        Self {
            name,
            full_path,
            children: Vec::new(),
            expanded: false,
            level,
            is_leaf,
        }
    }
}

struct TreeState {
    root: Vec<TreeNode>,
    selected_idx: usize,
    scroll: u16,
}

impl TreeState {
    fn new(blocks: &[String]) -> Self {
        let mut root = Vec::new();
        for path in blocks {
            Self::insert_path(&mut root, path, path, "", 0);
        }
        Self::sort_tree(&mut root);
        TreeState {
            root,
            selected_idx: 0,
            scroll: 0,
        }
    }

    fn insert_path(
        nodes: &mut Vec<TreeNode>,
        path: &str,
        leaf_path: &str,
        parent_path: &str,
        level: usize,
    ) {
        let (head, tail) = match path.split_once('.') {
            Some((h, t)) => (h, Some(t)),
            None => (path, None),
        };
        let node_path = if parent_path.is_empty() {
            head.to_string()
        } else {
            format!("{parent_path}.{head}")
        };

        let mut idx = None;
        for (i, node) in nodes.iter().enumerate() {
            if node.name == head {
                idx = Some(i);
                break;
            }
        }

        if idx.is_none() {
            let node = TreeNode {
                name: head.to_string(),
                full_path: node_path.clone(),
                children: Vec::new(),
                expanded: true, // Default expanded? Or collapsed? User prefers expanded usually.
                level,
                is_leaf: tail.is_none(),
            };
            nodes.push(node);
            idx = Some(nodes.len() - 1);
        }

        let node_idx = idx.unwrap();
        if let Some(t) = tail {
            Self::insert_path(
                &mut nodes[node_idx].children,
                t,
                leaf_path,
                &node_path,
                level + 1,
            );
        } else {
            // It's a leaf content. Mark the existing node as leaf.
            nodes[node_idx].is_leaf = true;
            nodes[node_idx].full_path = leaf_path.to_string();
        }
    }

    fn sort_tree(nodes: &mut Vec<TreeNode>) {
        nodes.sort_by(|a, b| {
            if a.is_leaf != b.is_leaf {
                // folders before files
                return a.is_leaf.cmp(&b.is_leaf);
            }
            a.name.cmp(&b.name)
        });
        for node in nodes {
            Self::sort_tree(&mut node.children);
        }
    }

    fn flatten(&self) -> Vec<&TreeNode> {
        let mut result = Vec::new();
        for node in &self.root {
            Self::flatten_recursive(node, &mut result);
        }
        result
    }

    fn flatten_recursive<'a>(node: &'a TreeNode, result: &mut Vec<&'a TreeNode>) {
        result.push(node);
        if node.expanded {
            for child in &node.children {
                Self::flatten_recursive(child, result);
            }
        }
    }

    fn select_next(&mut self) {
        let count = self.flatten().len();
        if self.selected_idx + 1 < count {
            self.selected_idx += 1;
        }
    }

    fn select_prev(&mut self) {
        if self.selected_idx > 0 {
            self.selected_idx -= 1;
        }
    }

    #[allow(dead_code)]
    fn expand_selected(&mut self) {
        if let Some(node) = self.get_selected_node_mut() {
            if !node.is_leaf {
                node.expanded = true;
            }
        }
    }

    fn collapse_selected(&mut self) {
        if let Some(node) = self.get_selected_node_mut() {
            if !node.is_leaf {
                node.expanded = false;
            }
        }
    }

    fn toggle_selected_expansion(&mut self) {
        if let Some(node) = self.get_selected_node_mut() {
            if !node.children.is_empty() {
                node.expanded = !node.expanded;
            }
        }
        let count = self.flatten().len();
        self.selected_idx = self.selected_idx.min(count.saturating_sub(1));
    }

    fn get_selected_node_mut(&mut self) -> Option<&mut TreeNode> {
        let target = self.selected_idx;
        let mut count = 0;
        for node in &mut self.root {
            if let Some(n) = Self::get_node_recursive(node, &mut count, target) {
                return Some(n);
            }
        }
        None
    }

    fn get_node_recursive<'a>(
        node: &'a mut TreeNode,
        count: &mut usize,
        target: usize,
    ) -> Option<&'a mut TreeNode> {
        if *count == target {
            return Some(node);
        }
        *count += 1;
        if node.expanded {
            for child in &mut node.children {
                if let Some(n) = Self::get_node_recursive(child, count, target) {
                    return Some(n);
                }
            }
        }
        None
    }

    fn get_selected_node(&self) -> Option<&TreeNode> {
        self.flatten().into_iter().nth(self.selected_idx)
    }
}

impl App {
    fn has_selected_text_overlay(&self) -> bool {
        self.text_selected
            .is_some_and(|selected| selected > 0 && selected <= self.render_args.overlays.len())
    }

    fn cycle_text_field_focus(&mut self, direction: i32) {
        let usable_len = if self.has_selected_text_overlay() {
            TEXT_FOCUS_ORDER.len()
        } else {
            1
        };
        let current = TEXT_FOCUS_ORDER
            .iter()
            .take(usable_len)
            .position(|focus| *focus == self.text_field_focus)
            .unwrap_or(0) as i32;
        let next = (current + direction).rem_euclid(usable_len as i32) as usize;
        self.text_field_focus = TEXT_FOCUS_ORDER[next];
    }

    fn delete_selected_text_overlay(&mut self) {
        let Some(selected) = self.text_selected else {
            return;
        };
        if selected == 0 || selected > self.render_args.overlays.len() {
            return;
        }

        self.render_args.overlays.remove(selected - 1);
        let new_len = self.render_args.overlays.len();
        self.text_selected = Some(if new_len == 0 { 0 } else { selected.min(new_len) });
        self.text_list_state.select(self.text_selected);
        self.text_field_focus = TextFieldFocus::List;
        self.input_mode = InputMode::Normal;
        self.value_edit_buffer.clear();
        self.text_cursor_anchor = None;
        self.text_hovered = None;
        self.text_resize_hover = None;
        self.reapply_overlays();
    }

    fn apply_text_picker_color(&mut self) {
        if !self.has_selected_text_overlay() {
            return;
        }
        let Some(picker) = self.color_picker_state.as_ref() else {
            return;
        };
        let editing_fg = picker.editing_fg;
        let color = if editing_fg {
            picker.fg_idx.map_or_else(
                || {
                    let (r, g, b) = picker.fg_rgb;
                    crate::args::ColorSpec::Rgb([r, g, b])
                },
                |idx| crate::args::ColorSpec::Index(idx as u8),
            )
        } else {
            picker.bg_idx.map_or_else(
                || {
                    let (r, g, b) = picker.bg_rgb.unwrap_or(picker.rgb);
                    crate::args::ColorSpec::Rgb([r, g, b])
                },
                |idx| crate::args::ColorSpec::Index(idx as u8),
            )
        };

        let overlay = &mut self.render_args.overlays[self.text_selected.unwrap() - 1];
        if editing_fg {
            overlay.fg = Some(color);
        } else {
            overlay.bg = Some(color);
        }
        self.reapply_overlays();
    }

    fn move_text_picker(&mut self, dx: i32, dy: i32, adjust_hue: bool) {
        let editing_fg = match self.text_field_focus {
            TextFieldFocus::FgButton | TextFieldFocus::ClearFgButton => true,
            TextFieldFocus::BgButton | TextFieldFocus::ClearBgButton => false,
            _ => return,
        };
        let Some(picker) = self.color_picker_state.as_mut() else {
            return;
        };
        picker.editing_fg = editing_fg;
        if adjust_hue && picker.mode == crate::colorpicker::PaletteMode::Ansi24 {
            picker.update_from_hsv(
                (picker.hue + dx as f64 * 5.0).rem_euclid(360.0),
                picker.saturation,
                picker.value,
            );
        } else {
            picker.move_selection(dx, dy);
        }
        self.apply_text_picker_color();
    }

    fn activate_text_field_focus(&mut self) {
        if self.text_field_focus == TextFieldFocus::List {
            if self.text_selected == Some(0) {
                self.render_args.overlays.push(new_text_overlay(
                    self.render_mode_idx,
                    0,
                    0,
                    true,
                ));
                self.text_selected = Some(self.render_args.overlays.len());
                self.text_list_state.select(self.text_selected);
                self.text_field_focus = TextFieldFocus::TextInput;
                self.value_edit_buffer.clear();
                self.input_mode = InputMode::TextEdit;
                self.reapply_overlays();
            } else if self.has_selected_text_overlay() {
                self.text_field_focus = TextFieldFocus::TextInput;
                self.value_edit_buffer =
                    self.render_args.overlays[self.text_selected.unwrap() - 1]
                        .editable_text()
                        .to_string();
                self.input_mode = InputMode::TextEdit;
            }
            return;
        }
        if !self.has_selected_text_overlay() {
            self.text_field_focus = TextFieldFocus::List;
            return;
        }

        let selected = self.text_selected.unwrap() - 1;
        match self.text_field_focus {
            TextFieldFocus::TextInput => {
                self.value_edit_buffer = self.render_args.overlays[selected]
                    .editable_text()
                    .to_string();
                self.input_mode = InputMode::TextEdit;
            }
            TextFieldFocus::FgButton => {
                if let Some(picker) = self.color_picker_state.as_mut() {
                    picker.editing_fg = true;
                }
            }
            TextFieldFocus::ClearFgButton => {
                self.render_args.overlays[selected].fg = None;
                if let Some(picker) = self.color_picker_state.as_mut() {
                    picker.fg_idx = None;
                }
                self.reapply_overlays();
            }
            TextFieldFocus::BgButton => {
                if let Some(picker) = self.color_picker_state.as_mut() {
                    picker.editing_fg = false;
                }
            }
            TextFieldFocus::ClearBgButton => {
                self.render_args.overlays[selected].bg = None;
                if let Some(picker) = self.color_picker_state.as_mut() {
                    picker.bg_idx = None;
                    picker.bg_rgb = None;
                }
                self.reapply_overlays();
            }
            TextFieldFocus::WrapButton => {
                let overlay = &mut self.render_args.overlays[selected];
                if !overlay.figlet_enabled() {
                    overlay.wrap = !overlay.wrap;
                }
                self.reapply_overlays();
            }
            TextFieldFocus::FitButton => {
                let overlay = &mut self.render_args.overlays[selected];
                if !overlay.figlet_enabled() {
                    overlay.auto_grow = !overlay.auto_grow;
                    sync_text_overlay_auto_grow(overlay);
                }
                self.reapply_overlays();
            }
            TextFieldFocus::FigletButton => toggle_text_overlay_figlet(self, selected),
            TextFieldFocus::FigletFontButton => open_figlet_picker(self, selected),
            TextFieldFocus::BoldButton => {
                self.render_args.overlays[selected].bold =
                    !self.render_args.overlays[selected].bold;
                self.reapply_overlays();
            }
            TextFieldFocus::ItalicButton => {
                self.render_args.overlays[selected].italic =
                    !self.render_args.overlays[selected].italic;
                self.reapply_overlays();
            }
            TextFieldFocus::UnderlineButton => {
                self.render_args.overlays[selected].underline =
                    !self.render_args.overlays[selected].underline;
                self.reapply_overlays();
            }
            TextFieldFocus::DeleteButton => self.delete_selected_text_overlay(),
            TextFieldFocus::List => {}
        }
    }

    fn toggle_tree_selection(&mut self) {
        if let Some(node) = self.tree_state.get_selected_node() {
            let target = if !node.full_path.is_empty() {
                node.full_path.clone()
            } else {
                node.name.clone()
            };
            let is_leaf = node.is_leaf;

            // If leaf, toggle it
            if is_leaf {
                if let Some(pos) = self.render_args.blocks.iter().position(|x| *x == target) {
                    self.render_args.blocks.remove(pos);
                } else {
                    self.render_args.blocks.push(target);
                }
            } else {
                // Group selection: toggle all children
                let mut leaves = Vec::new();
                Self::collect_leaves(node, &mut leaves);

                // if all leaves are present, remove all. else add all missing.
                let all_present = leaves.iter().all(|l| self.render_args.blocks.contains(l));

                if all_present {
                    // Remove all
                    self.render_args.blocks.retain(|b| !leaves.contains(b));
                } else {
                    // Add missing
                    for leaf in leaves {
                        if !self.render_args.blocks.contains(&leaf) {
                            self.render_args.blocks.push(leaf);
                        }
                    }
                }
            }
            crate::font::save_enabled_glyph_sets(&self.render_args.blocks);
            self.mark_args_dirty();
        }
    }

    fn toggle_glyph_selection(&mut self) {
        let (config_groups, _) = crate::font::load_blocks_config_unfiltered();
        let (enabled_groups, _) = crate::font::load_blocks_config();
        let Some(node) = self.tree_state.get_selected_node() else {
            return;
        };
        if !node.is_leaf {
            return;
        }
        let target_key = node.full_path.clone();
        let Some(codes) = config_groups.get(&target_key) else {
            return;
        };
        let Some(code) = self
            .glyph_list_state
            .selected()
            .and_then(|idx| codes.get(idx))
            .copied()
        else {
            return;
        };

        let enabled = enabled_groups.get(&target_key);
        let mut exclusions = codes
            .iter()
            .copied()
            .filter(|candidate| enabled.is_none_or(|enabled| !enabled.contains(candidate)))
            .collect::<std::collections::HashSet<_>>();
        if !exclusions.insert(code) {
            exclusions.remove(&code);
        }
        let mut exclusions = exclusions
            .into_iter()
            .filter_map(char::from_u32)
            .collect::<Vec<_>>();
        exclusions.sort_unstable();
        crate::font::save_blocks_config(&target_key, &exclusions);
        self.mark_args_dirty();
    }

    fn set_all_glyphs_selected(&mut self, selected: bool) {
        let (config_groups, _) = crate::font::load_blocks_config_unfiltered();
        let Some((is_leaf, target_key, leaves)) =
            self.tree_state.get_selected_node().map(|node| {
                let mut leaves = Vec::new();
                Self::collect_leaves(node, &mut leaves);
                (node.is_leaf, node.full_path.clone(), leaves)
            })
        else {
            return;
        };

        if !is_leaf {
            if selected {
                for leaf in leaves {
                    if !self.render_args.blocks.contains(&leaf) {
                        self.render_args.blocks.push(leaf);
                    }
                }
            } else {
                self.render_args
                    .blocks
                    .retain(|block| !leaves.contains(block));
            }
            crate::font::save_enabled_glyph_sets(&self.render_args.blocks);
            self.mark_args_dirty();
            return;
        }
        let Some(codes) = config_groups.get(&target_key) else {
            return;
        };

        if selected {
            crate::font::save_blocks_config(&target_key, &[]);
        } else {
            let exclusions = codes
                .iter()
                .filter_map(|&code| char::from_u32(code))
                .collect::<Vec<_>>();
            crate::font::save_blocks_config(&target_key, &exclusions);
        }
        self.mark_args_dirty();
    }

    fn selected_glyph_count(&self) -> usize {
        let Some(node) = self.tree_state.get_selected_node() else {
            return 0;
        };
        if !node.is_leaf {
            return 0;
        }
        crate::font::load_blocks_config_unfiltered()
            .0
            .get(&node.full_path)
            .map_or(0, Vec::len)
    }

    fn move_selected_glyph(&mut self, delta: i32) {
        let count = self.selected_glyph_count();
        if count == 0 {
            self.glyph_list_state.select(None);
            return;
        }
        let current = self.glyph_list_state.selected().unwrap_or(0) as i32;
        self.glyph_list_state
            .select(Some((current + delta).clamp(0, count as i32 - 1) as usize));
    }

    fn activate_selected_glyph_control(&mut self) {
        match self.selected_control {
            0 => self.toggle_tree_selection(),
            1 => self.toggle_glyph_selection(),
            2 => self.set_all_glyphs_selected(true),
            3 => self.set_all_glyphs_selected(false),
            4 => self.open_unicode_explorer(),
            5 => self.delete_selected_glyph_range(),
            _ => {}
        }
    }

    fn open_unicode_explorer(&mut self) {
        let selected = self
            .unicode_list_state
            .selected()
            .unwrap_or(0)
            .min(UNICODE_BLOCKS.len().saturating_sub(1));
        self.unicode_list_state.select(Some(selected));
        self.unicode_range_path = UNICODE_BLOCKS
            .get(selected)
            .map(|(name, _, _)| self.suggested_unicode_range_path(name))
            .unwrap_or_default();
        self.unicode_path_focused = false;
        self.show_unicode_explorer = true;
    }

    fn suggested_unicode_range_path(&self, range_name: &str) -> String {
        let parent = self
            .tree_state
            .get_selected_node()
            .and_then(|node| {
                if !node.children.is_empty() && !node.is_leaf {
                    Some(node.full_path.as_str())
                } else {
                    node.full_path.rsplit_once('.').map(|(parent, _)| parent)
                }
            })
            .filter(|parent| !parent.is_empty());
        parent.map_or_else(
            || range_name.to_string(),
            |parent| format!("{parent}.{range_name}"),
        )
    }

    fn sync_unicode_range_path_to_selection(&mut self) {
        if let Some((name, _, _)) = self
            .unicode_list_state
            .selected()
            .and_then(|idx| UNICODE_BLOCKS.get(idx))
        {
            self.unicode_range_path = self.suggested_unicode_range_path(name);
        }
    }

    fn add_selected_unicode_range(&mut self) {
        let Some((default_name, start, end)) = self
            .unicode_list_state
            .selected()
            .and_then(|idx| UNICODE_BLOCKS.get(idx))
            .copied()
        else {
            return;
        };
        let path = self
            .unicode_range_path
            .replace('/', ".")
            .split('.')
            .map(str::trim)
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>()
            .join(".");
        let path = if path.is_empty() {
            default_name.to_string()
        } else {
            path
        };

        crate::font::add_range_to_blocks_config(&path, start, end);
        if !self.available_blocks.contains(&path) {
            self.available_blocks.push(path.clone());
            self.available_blocks.sort();
        }
        self.tree_state = TreeState::new(&self.available_blocks);
        self.last_glyph_list_key = None;
        self.show_unicode_explorer = false;
        log::info!(
            "Added Unicode range {} at {} (U+{:04X}-U+{:04X})",
            default_name,
            path,
            start,
            end
        );
    }

    fn delete_selected_glyph_range(&mut self) {
        let Some(node) = self.tree_state.get_selected_node() else {
            return;
        };
        if !node.is_leaf || node.full_path.is_empty() {
            return;
        }
        let name = node.full_path.clone();
        crate::font::remove_range_from_blocks_config(&name);
        self.available_blocks.retain(|block| block != &name);
        self.render_args.blocks.retain(|block| block != &name);
        self.tree_state = TreeState::new(&self.available_blocks);
        self.glyph_list_state = ListState::default();
        self.last_glyph_list_key = None;
        crate::font::save_enabled_glyph_sets(&self.render_args.blocks);
        self.mark_args_dirty();
        log::info!("Removed Unicode range: {}", name);
    }

    fn collect_leaves(node: &TreeNode, leaves: &mut Vec<String>) {
        if node.is_leaf && !node.full_path.is_empty() {
            leaves.push(node.full_path.clone());
        }
        for child in &node.children {
            Self::collect_leaves(child, leaves);
        }
    }
}

pub(crate) struct App {
    render_args: RenderArgs,
    input_mode: InputMode,
    selected_control: usize,
    current_tab: usize, // Focused pane group: 0=general, 1=pipeline, 2=glyphs
    general_controls: Vec<ControlDef>,
    show_general: bool,
    show_pipeline: bool,
    show_glyphs: bool,
    scroll: u16,
    general_scroll_offset: usize,

    // Cancellation & Debouncing
    tx_args: watch::Sender<(RenderArgs, Arc<AtomicBool>, Instant)>,
    current_cancel_flag: Arc<AtomicBool>,
    args_dirty: bool,
    last_args_change: Option<Instant>,

    rx_result: mpsc::Receiver<(
        draw::RenderResult,
        Arc<font::GlyphStore>,
        Vec<crate::args::TextOverlay>,
        Instant,
    )>,
    cached_render: String,
    cached_result: Option<draw::RenderResult>,
    /// Raw result from render thread (no overlays applied)
    base_render_result: Option<draw::RenderResult>,
    last_render_latency: Option<Duration>,
    is_rendering: Arc<AtomicBool>,
    render_tick: usize,
    dragged_control: Option<usize>,
    value_edit_buffer: String,

    pipeline_nodes: Vec<crate::pipeline::ImageEffectNode>,
    pipeline_catalog: Vec<CatalogItem>,
    pub pipeline_list_state: ratatui::widgets::ListState,
    pipeline_next_id: usize,
    pipeline_dragging: Option<(usize, i16, i16)>,
    pipeline_scroll: u16,
    pipeline_selected: Option<usize>, // ID of selected node for param editing
    pipeline_expanded: Option<usize>, // Index of the expanding node (nodes.len() is 'new' node)
    pipeline_library_selected: usize,
    pipeline_preview_effect: Option<crate::pipeline::ImageEffect>,

    // Shared state for PNG saving
    arc_glyphs: Option<Arc<font::GlyphStore>>,

    // Font Picker State
    show_font_picker: bool,
    font_search: String,
    font_list_state: ListState,

    // Unicode Explorer State
    show_unicode_explorer: bool,
    unicode_list_state: ListState,
    unicode_range_path: String,
    unicode_path_focused: bool,
    unicode_list_area: Rect,
    unicode_path_area: Rect,
    unicode_add_area: Rect,
    unicode_cancel_area: Rect,

    // Metadata prompt state
    metadata_prompt_settings: Option<RenderArgs>,
    metadata_prompt_path: String,
    metadata_prompt_png_path: String,
    clipboard_paste_rx: Option<std::sync::mpsc::Receiver<ClipboardPasteResult>>,
    clipboard_paste_in_progress: bool,
    clipboard_paste_status: Option<String>,

    // Image path
    image_path: String,
    ocr_overlay_count: usize,

    // Control values
    width: i32,
    render_mode_idx: usize,
    ocr: bool,
    ocr_lock: bool,
    ocr_auto_width: bool,
    ocr_figlet: bool,
    ocr_figlet_min_height: i32,
    ocr_figlet_min_height_ratio: i32,
    ocr_figlet_fill: bool,
    ocr_figlet_max_width_ratio: i32,
    ocr_figlet_max_height_ratio: i32,
    ocr_min_confidence: i32,
    ocr_megapixels: i32,
    show_ocr_advanced: bool,
    ocr_model_tier_idx: usize,
    ocr_max_text_height_ratio: i32,
    ocr_threads: i32,
    dither: i32,
    show_discontinuities: bool,
    discontinuity_threshold: i32,
    graphics_preview: bool,
    show_advanced: bool,
    score_fix: bool,
    score_fix_candidates: i32,
    score_fix_orderings: i32,
    score_fix_geometry_first: bool,
    score_fix_neighborhood_guard: bool,
    contour_bending_weight: i32,
    contour_endpoint_weight: i32,
    contour_junction_weight: i32,
    contour_fragment_weight: i32,
    contour_fidelity_weight: i32,
    contour_boundary_weight: i32,
    contour_peak_weight: i32,

    misc3: i32,
    misc4: i32,
    misc5: i32,
    misc6: i32,
    misc7: i32,
    misc8: i32,

    invert: bool,
    grayscale: bool,
    brightness: i32,
    contrast: i32,
    saturation: i32,
    gamma: i32,
    hue: i32,
    trim_top: i32,
    trim_right: i32,
    trim_bottom: i32,
    trim_left: i32,
    fliph: bool,
    flipv: bool,
    rotate: i32,
    scalex: i32, // percentage, 100 = 1.0
    scaley: i32,
    font_size: f32,
    fonts: Vec<String>,
    font_idx: usize,
    filter_idx: usize,
    grayscale_tolerance: i32,
    colorspace_idx: usize,
    encoding_idx: usize,
    rx_optimization: Option<mpsc::Receiver<OptimizationUpdate>>,

    // Slider state
    slider_last_press: Option<std::time::Instant>,
    slider_repeat_count: u32,

    // Effects
    pixelize: i32,
    gaussian_blur: i32,
    box_blur: bool,
    oil: bool,
    halftone: bool,
    sepia: bool,
    normalize: bool,
    noise: bool,
    emboss: bool,
    identity: bool,
    laplace: bool,
    noise_reduction: bool,
    sharpen: bool,
    cali: bool,
    dramatic: bool,
    firenze: bool,
    golden: bool,
    lix: bool,
    lofi: bool,
    neue: bool,
    obsidian: bool,
    pastel_pink: bool,
    ryo: bool,
    frosted_glass: bool,
    solarize: bool,
    edge_detection: bool,

    // Blocks
    available_blocks: Vec<String>,
    tree_state: TreeState,
    glyph_list_state: ListState,
    last_glyph_list_key: Option<String>,

    // Explorer dialog state
    explorer_dialog: Option<ExplorerDialog>,

    // Track rendered control areas for mouse
    control_areas: Vec<Rect>,
    tab_areas: Vec<Rect>,
    path_area: Rect,
    general_area: Rect,
    pipeline_column_area: Rect,
    pipeline_active_area: Rect,
    glyphs_area: Rect,
    tree_area: Rect,
    glyph_list_area: Rect,
    glyph_select_all_area: Rect,
    glyph_select_none_area: Rect,
    glyph_add_area: Rect,
    glyph_delete_area: Rect,
    explorer_area: Rect,
    preview_area: Rect,
    output_area: Rect,
    options_pane_area: Rect,
    options_areas: Vec<Rect>,

    // Image preview for xplr
    image_preview_protocol: Option<StatefulProtocol>,
    output_preview_protocol: Option<StatefulProtocol>,
    output_preview_dirty: bool,
    preview_picker: Picker,
    last_preview_path: Option<String>,

    // Eyedropper state
    eyedropper_color: Option<(u8, u8, u8)>,
    replace_colour_dialog: Option<ReplaceColourDialog>,
    replace_colour_button_area: Rect,
    figlet_picker: Option<FigletPicker>,
    is_optimizing: bool,
    optimization_progress: String,
    optimization_best_score: Option<f64>,
    auto_optimize_state: Option<AutoOptimizeState>,
    saved_auto_optimize_inputs: Vec<String>,
    saved_auto_optimize_enabled_rows: [bool; 8],
    saved_auto_optimize_shortest_longest_line: bool,
    saved_auto_optimize_randomize: bool,
    auto_optimize_cancel: Option<Arc<AtomicBool>>,
    general_pane_width: u16,
    pipeline_pane_width: u16,
    glyphs_pane_width: u16,
    glyph_tree_height: u16,
    glyph_list_height: u16,
    splitter_areas: Vec<SplitterArea>,
    dragging_splitter: Option<SplitterKind>,
    terminal_area: Rect,

    text_pane_width: u16,
    text_area: Rect,
    text_list_state: ListState,
    text_list_area: Rect,
    color_picker_area: Rect,
    text_selected: Option<usize>,
    text_drag_mode: TextDragMode,
    text_drag_start: Option<(i32, i32)>,
    last_text_click: Option<(std::time::Instant, i32, i32)>,
    text_cursor_anchor: Option<(i32, i32)>,
    text_hovered: Option<usize>,
    text_resize_hover: Option<usize>,
    base_grid: Option<Vec<Vec<crate::draw::RenderedCell>>>,
    text_field_focus: TextFieldFocus,
    text_field_areas: Vec<Rect>, // areas for each field row in the editor
    text_btn_areas: Vec<Rect>,   // wrap, fit, FIGlet, styles, delete, and colour actions
    color_picker_state: Option<crate::colorpicker::ColorPickerState>,
}

const UNICODE_BLOCKS: &[(&str, u32, u32)] = &[
    ("Basic Latin", 0x0020, 0x007F),
    ("Latin-1 Supplement", 0x0080, 0x00FF),
    ("Latin Extended-A", 0x0100, 0x017F),
    ("Latin Extended-B", 0x0180, 0x024F),
    ("IPA Extensions", 0x0250, 0x02AF),
    ("Spacing Modifier Letters", 0x02B0, 0x02FF),
    ("Combining Diacritical Marks", 0x0300, 0x036F),
    ("Greek and Coptic", 0x0370, 0x03FF),
    ("Cyrillic", 0x0400, 0x04FF),
    ("Cyrillic Supplement", 0x0500, 0x052F),
    ("Armenian", 0x0530, 0x058F),
    ("Hebrew", 0x0590, 0x05FF),
    ("Arabic", 0x0600, 0x06FF),
    ("Syriac", 0x0700, 0x074F),
    ("Arabic Supplement", 0x0750, 0x077F),
    ("Thaana", 0x0780, 0x07BF),
    ("NKo", 0x07C0, 0x07FF),
    ("Samaritan", 0x0800, 0x083F),
    ("Mandaic", 0x0840, 0x085F),
    ("Arabic Extended-A", 0x08A0, 0x08FF),
    ("Devanagari", 0x0900, 0x097F),
    ("Bengali", 0x0980, 0x09FF),
    ("Gurmukhi", 0x0A00, 0x0A7F),
    ("Gujarati", 0x0A80, 0x0AFF),
    ("Oriya", 0x0B00, 0x0B7F),
    ("Tamil", 0x0B80, 0x0BFF),
    ("Telugu", 0x0C00, 0x0C7F),
    ("Kannada", 0x0C80, 0x0CFF),
    ("Malayalam", 0x0D00, 0x0D7F),
    ("Sinhala", 0x0D80, 0x0DFF),
    ("Thai", 0x0E00, 0x0E7F),
    ("Lao", 0x0E80, 0x0EFF),
    ("Tibetan", 0x0F00, 0x0FFF),
    ("Myanmar", 0x1000, 0x109F),
    ("Georgian", 0x10A0, 0x10FF),
    ("Hangul Jamo", 0x1100, 0x11FF),
    ("Ethiopic", 0x1200, 0x137F),
    ("Ethiopic Supplement", 0x1380, 0x139F),
    ("Cherokee", 0x13A0, 0x13FF),
    ("Unified Canadian Aboriginal Syllabics", 0x1400, 0x167F),
    ("Ogham", 0x1680, 0x169F),
    ("Runic", 0x16A0, 0x16FF),
    ("Tagalog", 0x1700, 0x171F),
    ("Hanunoo", 0x1720, 0x173F),
    ("Buhid", 0x1740, 0x175F),
    ("Tagbanwa", 0x1760, 0x177F),
    ("Khmer", 0x1780, 0x17FF),
    ("Mongolian", 0x1800, 0x18AF),
    (
        "Unified Canadian Aboriginal Syllabics Extended",
        0x18B0,
        0x18FF,
    ),
    ("Limbu", 0x1900, 0x194F),
    ("Tai Le", 0x1950, 0x197F),
    ("New Tai Lue", 0x1980, 0x19DF),
    ("Khmer Symbols", 0x19E0, 0x19FF),
    ("Buginese", 0x1A00, 0x1A1F),
    ("Tai Tham", 0x1A20, 0x1AAF),
    ("Combining Diacritical Marks Extended", 0x1AB0, 0x1AFF),
    ("Balinese", 0x1B00, 0x1B7F),
    ("Sundanese", 0x1B80, 0x1BBF),
    ("Batak", 0x1BC0, 0x1BFF),
    ("Lepcha", 0x1C00, 0x1C4F),
    ("Ol Chiki", 0x1C50, 0x1C7F),
    ("Sundanese Supplement", 0x1CC0, 0x1CCF),
    ("Vedic Extensions", 0x1CD0, 0x1CFF),
    ("Phonetic Extensions", 0x1D00, 0x1D7F),
    ("Phonetic Extensions Supplement", 0x1D80, 0x1DBF),
    ("Combining Diacritical Marks Supplement", 0x1DC0, 0x1DFF),
    ("Latin Extended Additional", 0x1E00, 0x1EFF),
    ("Greek Extended", 0x1F00, 0x1FFF),
    ("General Punctuation", 0x2000, 0x206F),
    ("Superscripts and Subscripts", 0x2070, 0x209F),
    ("Currency Symbols", 0x20A0, 0x20CF),
    ("Combining Diacritical Marks for Symbols", 0x20D0, 0x20FF),
    ("Letterlike Symbols", 0x2100, 0x214F),
    ("Number Forms", 0x2150, 0x218F),
    ("Arrows", 0x2190, 0x21FF),
    ("Mathematical Operators", 0x2200, 0x22FF),
    ("Miscellaneous Technical", 0x2300, 0x23FF),
    ("Control Pictures", 0x2400, 0x243F),
    ("Optical Character Recognition", 0x2440, 0x245F),
    ("Enclosed Alphanumerics", 0x2460, 0x24FF),
    ("Box Drawing", 0x2500, 0x257F),
    ("Block Elements", 0x2580, 0x259F),
    ("Geometric Shapes", 0x25A0, 0x25FF),
    ("Miscellaneous Symbols", 0x2600, 0x26FF),
    ("Dingbats", 0x2700, 0x27BF),
    ("Miscellaneous Mathematical Symbols-A", 0x27C0, 0x27EF),
    ("Supplemental Arrows-A", 0x27F0, 0x27FF),
    ("Braille Patterns", 0x2800, 0x28FF),
    ("Supplemental Arrows-B", 0x2900, 0x297F),
    ("Miscellaneous Mathematical Symbols-B", 0x2980, 0x29FF),
    ("Supplemental Mathematical Operators", 0x2A00, 0x2AFF),
    ("Miscellaneous Symbols and Arrows", 0x2B00, 0x2BFF),
    ("Glagolitic", 0x2C00, 0x2C5F),
    ("Latin Extended-C", 0x2C60, 0x2C7F),
    ("Coptic", 0x2C80, 0x2CFF),
    ("Georgian Supplement", 0x2D00, 0x2D2F),
    ("Tifinagh", 0x2D30, 0x2D7F),
    ("Ethiopic Extended", 0x2D80, 0x2DDF),
    ("Cyrillic Extended-A", 0x2DE0, 0x2DFF),
    ("Supplemental Punctuation", 0x2E00, 0x2E7F),
    ("CJK Radicals Supplement", 0x2E80, 0x2EFF),
    ("Kangxi Radicals", 0x2F00, 0x2FDF),
    ("Ideographic Description Characters", 0x2FF0, 0x2FFF),
    ("CJK Symbols and Punctuation", 0x3000, 0x303F),
    ("Hiragana", 0x3040, 0x309F),
    ("Katakana", 0x30A0, 0x30FF),
    ("Bopomofo", 0x3100, 0x312F),
    ("Hangul Compatibility Jamo", 0x3130, 0x318F),
    ("Kanbun", 0x3190, 0x319F),
    ("Bopomofo Extended", 0x31A0, 0x31BF),
    ("CJK Strokes", 0x31C0, 0x31EF),
    ("Katakana Phonetic Extensions", 0x31F0, 0x31FF),
    ("Enclosed CJK Letters and Months", 0x3200, 0x32FF),
    ("CJK Compatibility", 0x3300, 0x33FF),
    ("CJK Unified Ideographs Extension A", 0x3400, 0x4DBF),
    ("Yijing Hexagram Symbols", 0x4DC0, 0x4DFF),
    ("CJK Unified Ideographs", 0x4E00, 0x9FFF),
    ("Yi Syllables", 0xA000, 0xA48F),
    ("Yi Radicals", 0xA490, 0xA4CF),
    ("Lisu", 0xA4D0, 0xA4FF),
    ("Vai", 0xA500, 0xA63F),
    ("Cyrillic Extended-B", 0xA640, 0xA69F),
    ("Bamum", 0xA6A0, 0xA6FF),
    ("Modifier Tone Letters", 0xA700, 0xA71F),
    ("Latin Extended-D", 0xA720, 0xA7FF),
    ("Syloti Nagri", 0xA800, 0xA82F),
    ("Common Indic Number Forms", 0xA830, 0xA83F),
    ("Phags-pa", 0xA840, 0xA87F),
    ("Saurashtra", 0xA880, 0xA8DF),
    ("Devanagari Extended", 0xA8E0, 0xA8FF),
    ("Kayah Li", 0xA900, 0xA92F),
    ("Rejang", 0xA930, 0xA95F),
    ("Hangul Jamo Extended-A", 0xA960, 0xA97F),
    ("Javanese", 0xA980, 0xA9DF),
    ("Myanmar Extended-B", 0xA9E0, 0xA9FF),
    ("Cham", 0xAA00, 0xAA5F),
    ("Myanmar Extended-A", 0xAA60, 0xAA7F),
    ("Tai Viet", 0xAA80, 0xAADF),
    ("Meetei Mayek Extensions", 0xAAE0, 0xAAFF),
    ("Ethiopic Extended-A", 0xAB00, 0xAB2F),
    ("Latin Extended-E", 0xAB30, 0xAB6F),
    ("Cherokee Supplement", 0xAB70, 0xABBF),
    ("Meetei Mayek", 0xABC0, 0xABFF),
    ("Hangul Syllables", 0xAC00, 0xD7AF),
    ("Hangul Jamo Extended-B", 0xD7B0, 0xD7FF),
    ("High Surrogates", 0xD800, 0xDB7F),
    ("High Private Use Surrogates", 0xDB80, 0xDBFF),
    ("Low Surrogates", 0xDC00, 0xDFFF),
    ("Private Use Area", 0xE000, 0xF8FF),
    ("CJK Compatibility Ideographs", 0xF900, 0xFAFF),
    ("Alphabetic Presentation Forms", 0xFB00, 0xFB4F),
    ("Arabic Presentation Forms-A", 0xFB50, 0xFDFF),
    ("Variation Selectors", 0xFE00, 0xFE0F),
    ("Vertical Forms", 0xFE10, 0xFE1F),
    ("Combining Half Marks", 0xFE20, 0xFE2F),
    ("CJK Compatibility Forms", 0xFE30, 0xFE4F),
    ("Small Form Variants", 0xFE50, 0xFE6F),
    ("Arabic Presentation Forms-B", 0xFE70, 0xFEFF),
    ("Halfwidth and Fullwidth Forms", 0xFF00, 0xFFEF),
    ("Specials", 0xFFF0, 0xFFFF),
    // Supplementary Planes (UTF-16 Surrogate-accessible ranges)
    ("Linear B Syllabary", 0x10000, 0x1007F),
    ("Linear B Ideograms", 0x10080, 0x100FF),
    ("Aegean Numbers", 0x10100, 0x1013F),
    ("Ancient Greek Numbers", 0x10140, 0x1018F),
    ("Ancient Symbols", 0x10190, 0x101CF),
    ("Phaistos Disc", 0x101D0, 0x101FF),
    ("Lycian", 0x10280, 0x1029F),
    ("Carian", 0x102A0, 0x102DF),
    ("Coptic Epact Numbers", 0x102E0, 0x102FF),
    ("Old Italic", 0x10300, 0x1032F),
    ("Gothic", 0x10330, 0x1034F),
    ("Old Permic", 0x10350, 0x1037F),
    ("Ugaritic", 0x10380, 0x1039F),
    ("Old Persian", 0x103A0, 0x103DF),
    ("Deseret", 0x10400, 0x1044F),
    ("Shavian", 0x10450, 0x1047F),
    ("Osmanya", 0x10480, 0x104AF),
    ("Osage", 0x104B0, 0x104FF),
    ("Elbasan", 0x10500, 0x1052F),
    ("Caucasian Albanian", 0x10530, 0x1056F),
    ("Linear A", 0x10600, 0x1077F),
    ("Cypriot Syllabary", 0x10800, 0x1083F),
    ("Imperial Aramaic", 0x10840, 0x1085F),
    ("Palmyrene", 0x10860, 0x1087F),
    ("Nabataean", 0x10880, 0x108AF),
    ("Hatran", 0x108E0, 0x108FF),
    ("Phoenician", 0x10900, 0x1091F),
    ("Lydian", 0x10920, 0x1093F),
    ("Meroitic Hieroglyphs", 0x10980, 0x1099F),
    ("Meroitic Cursive", 0x109A0, 0x109FF),
    ("Old South Arabian", 0x10A00, 0x10A5F),
    ("Old North Arabian", 0x10A60, 0x10A7F),
    ("Manichaean", 0x10AC0, 0x10AFF),
    ("Avestan", 0x10B00, 0x10B3F),
    ("Inscriptional Parthian", 0x10B40, 0x10B5F),
    ("Inscriptional Pahlavi", 0x10B60, 0x10B7F),
    ("Psalter Pahlavi", 0x10B80, 0x10BAF),
    ("Old Turkic", 0x10C00, 0x10C4F),
    ("Old Hungarian", 0x10C80, 0x10CFF),
    ("Hanifi Rohingya", 0x10D00, 0x10D3F),
    ("Rumi Numeral Symbols", 0x10E60, 0x10E7F),
    ("Yezidi", 0x10E80, 0x10EAF),
    ("Old Sogdian", 0x10F00, 0x10F2F),
    ("Sogdian", 0x10F30, 0x10F6F),
    ("Chorasmian", 0x10FB0, 0x10FDF),
    ("Elymaic", 0x10FE0, 0x10FFF),
    ("Brahmi", 0x11000, 0x1107F),
    ("Kaithi", 0x11080, 0x110CF),
    ("Sora Sompeng", 0x110D0, 0x110FF),
    ("Chakma", 0x11100, 0x1114F),
    ("Mahajani", 0x11150, 0x1117F),
    ("Sharada", 0x11180, 0x111DF),
    ("Sinhala Archaic Numbers", 0x111E0, 0x111FF),
    ("Khojki", 0x11200, 0x1124F),
    ("Multani", 0x11280, 0x112AF),
    ("Khudawadi", 0x112B0, 0x112FF),
    ("Grantha", 0x11300, 0x1137F),
    ("Newa", 0x11400, 0x1147F),
    ("Tirhuta", 0x11480, 0x114DF),
    ("Siddham", 0x11580, 0x115FF),
    ("Modi", 0x11600, 0x1165F),
    ("Mongolian Supplement", 0x11660, 0x1167F),
    ("Takri", 0x11680, 0x116CF),
    ("Ahom", 0x11700, 0x1173F),
    ("Dogra", 0x11800, 0x1184F),
    ("Warang Citi", 0x118A0, 0x118FF),
    ("Dives Akuru", 0x11900, 0x1195F),
    ("Nandinagari", 0x119A0, 0x119FF),
    ("Zanabazar Square", 0x11A00, 0x11A4F),
    ("Soyombo", 0x11A50, 0x11AAF),
    ("Pau Cin Hau", 0x11AC0, 0x11AFF),
    ("Bhaiksuki", 0x11C00, 0x11C6F),
    ("Marchen", 0x11C70, 0x11CBF),
    ("Masaram Gondi", 0x11D00, 0x11D5F),
    ("Gunjala Gondi", 0x11D60, 0x11DAF),
    ("Makasar", 0x11EE0, 0x11EFF),
    ("Lisu Supplement", 0x11FB0, 0x11FBF),
    ("Tamil Supplement", 0x11FC0, 0x11FFF),
    ("Cuneiform", 0x12000, 0x123FF),
    ("Cuneiform Numbers and Punctuation", 0x12400, 0x1247F),
    ("Early Dynastic Cuneiform", 0x12480, 0x1254F),
    ("Egyptian Hieroglyphs", 0x13000, 0x1342F),
    ("Anatolian Hieroglyphs", 0x14400, 0x1467F),
    ("Bamum Supplement", 0x16800, 0x16A3F),
    ("Mro", 0x16A40, 0x16A6F),
    ("Tangsa", 0x16AD0, 0x16AFF),
    ("Bassa Vah", 0x16AD0, 0x16AF0),
    ("Pahawh Hmong", 0x16B00, 0x16B8F),
    ("Medefaidrin", 0x16E40, 0x16E9F),
    ("Miao", 0x16F00, 0x16F9F),
    ("Ideographic Symbols and Punctuation", 0x16FE0, 0x16FFF),
    ("Tangut", 0x17000, 0x187FF),
    ("Tangut Components", 0x18800, 0x18AFF),
    ("Khitan Small Script", 0x18B00, 0x18CFF),
    ("Tangut Supplement", 0x18D00, 0x18D8F),
    ("Kana Supplement", 0x1B000, 0x1B0FF),
    ("Kana Extended-A", 0x1B100, 0x1B12F),
    ("Small Kana Extension", 0x1B130, 0x1B16F),
    ("Nushu", 0x1B170, 0x1B2FF),
    ("Duployan", 0x1BC00, 0x1BC9F),
    ("Shorthand Format Controls", 0x1BCA0, 0x1BCAF),
    ("Znamenny Musical Notation", 0x1CF00, 0x1CFCF),
    ("Byzantine Musical Symbols", 0x1D000, 0x1D0FF),
    ("Musical Symbols", 0x1D100, 0x1D1FF),
    ("Ancient Greek Musical Notation", 0x1D200, 0x1D24F),
    ("Mayan Numerals", 0x1D2E0, 0x1D2FF),
    ("Tai Xuan Jing Symbols", 0x1D300, 0x1D35F),
    ("Counting Rod Numerals", 0x1D360, 0x1D37F),
    ("Mathematical Alphanumeric Symbols", 0x1D400, 0x1D7FF),
    ("Sutton SignWriting", 0x1D800, 0x1DAAF),
    ("Latin Extended-G", 0x1DF00, 0x1DFFF),
    ("Glagolitic Supplement", 0x1E000, 0x1E02F),
    ("Nyiakeng Puachue Hmong", 0x1E100, 0x1E14F),
    ("Wancho", 0x1E2C0, 0x1E2FF),
    ("Adlam", 0x1E900, 0x1E95F),
    ("Indic Siyaq Numbers", 0x1EC70, 0x1ECBF),
    ("Ottoman Siyaq Numbers", 0x1ED00, 0x1ED4F),
    ("Arabic Mathematical Alphabetic Symbols", 0x1EE00, 0x1EEFF),
    ("Mahjong Tiles", 0x1F000, 0x1F02F),
    ("Domino Tiles", 0x1F030, 0x1F09F),
    ("Playing Cards", 0x1F0A0, 0x1F0FF),
    ("Enclosed Alphanumeric Supplement", 0x1F100, 0x1F1FF),
    ("Enclosed Ideographic Supplement", 0x1F200, 0x1F2FF),
    ("Miscellaneous Symbols and Pictographs", 0x1F300, 0x1F5FF),
    ("Emoticons", 0x1F600, 0x1F64F),
    ("Ornamental Dingbats", 0x1F650, 0x1F67F),
    ("Transport and Map Symbols", 0x1F680, 0x1F6FF),
    ("Alchemical Symbols", 0x1F700, 0x1F77F),
    ("Geometric Shapes Extended", 0x1F780, 0x1F7FF),
    ("Supplemental Arrows-C", 0x1F800, 0x1F8FF),
    ("Miscellaneous Symbols and Arrows", 0x1F900, 0x1F9FF),
    ("Supplemental Symbols and Pictographs", 0x1FA00, 0x1FA6F),
    ("Chess Symbols", 0x1FA70, 0x1FAFF),
    ("Symbols and Pictographs Extended-A", 0x1FB00, 0x1FBFF),
    ("Symbols for Legacy Computing", 0x1FB00, 0x1FBFF),
    ("Symbols for Legacy Computing Supplement", 0x1CC00, 0x1CEBF),
];

fn render_unicode_explorer(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(ratatui::symbols::border::ROUNDED)
        .title(" Unicode Range Explorer ")
        .style(Style::default().bg(SURFACE));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let main_chunks = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(3), // Arbitrarily nested set path
        Constraint::Length(3), // Add/cancel buttons
    ])
    .split(inner);

    let explorer_chunks =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(main_chunks[0]);

    // 1. List of ranges (Left)
    let items: Vec<ListItem> = UNICODE_BLOCKS
        .iter()
        .map(|(name, start, end)| {
            ListItem::new(format!("{:<40} U+{:04X} - U+{:04X}", name, start, end))
        })
        .collect();

    app.unicode_list_area = explorer_chunks[0];
    app.unicode_path_area = main_chunks[1];

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::RIGHT)
                .border_style(Style::default().fg(if app.unicode_path_focused {
                    BORDER
                } else {
                    ACCENT
                })),
        )
        .highlight_style(Style::default().bg(ACCENT).fg(Color::White))
        .highlight_symbol("▸ ");

    f.render_stateful_widget(list, explorer_chunks[0], &mut app.unicode_list_state);

    // 2. Character Grid (Right)
    if let Some(idx) = app.unicode_list_state.selected() {
        let (_name, start, end) = UNICODE_BLOCKS[idx];
        let mut chars_text = Text::default();
        let width = explorer_chunks[1].width as usize / 2; // 2 chars per glyph spot roughly
        if width > 0 {
            let mut current_line = Line::from(vec![]);
            for code in start..=end {
                if let Some(ch) = char::from_u32(code) {
                    current_line.spans.push(Span::raw(format!("{} ", ch)));
                    if current_line.spans.len() >= width {
                        chars_text.lines.push(current_line);
                        current_line = Line::from(vec![]);
                    }
                }
            }
            if !current_line.spans.is_empty() {
                chars_text.lines.push(current_line);
            }
        }

        let char_grid = Paragraph::new(chars_text)
            .block(Block::default().title(format!(" Characters (U+{:04X}-U+{:04X}) ", start, end)))
            .wrap(Wrap { trim: false });
        f.render_widget(char_grid, explorer_chunks[1]);
    }

    let path_text = if app.unicode_path_focused {
        format!("{}▌", app.unicode_range_path)
    } else {
        app.unicode_range_path.clone()
    };
    let path = Paragraph::new(path_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(if app.unicode_path_focused {
                    ACCENT
                } else {
                    BORDER
                }))
                .title(" Set path (dot-separated; any depth) "),
        )
        .style(Style::default().fg(TEXT));
    f.render_widget(path, main_chunks[1]);

    let action_areas =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(main_chunks[2]);
    app.unicode_add_area = action_areas[0];
    app.unicode_cancel_area = action_areas[1];
    let add = Paragraph::new(" Add Range · Enter ")
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(TOGGLE_ON)),
        )
        .alignment(Alignment::Center);
    let cancel = Paragraph::new(" Cancel · Esc ")
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(BORDER)),
        )
        .alignment(Alignment::Center);
    f.render_widget(add, action_areas[0]);
    f.render_widget(cancel, action_areas[1]);
}

impl App {
    fn new(
        render_args: RenderArgs,
        tx_args: watch::Sender<(RenderArgs, Arc<AtomicBool>, Instant)>,
        rx_result: mpsc::Receiver<(
            draw::RenderResult,
            Arc<font::GlyphStore>,
            Vec<crate::args::TextOverlay>,
            Instant,
        )>,
        initial_flag: Arc<AtomicBool>,
    ) -> App {
        let (blocks_map, _) = crate::font::load_blocks_config();
        let mut available_blocks: Vec<String> = blocks_map.keys().cloned().collect();
        available_blocks.sort();
        let tree_state = TreeState::new(&available_blocks);
        let ui_layout = crate::config::UiLayout::load().unwrap_or_default();
        let general_controls = arrange_general_controls(&ui_layout.general);
        let pipeline_catalog = arrange_pipeline_catalog(&ui_layout.pipeline);
        let pipeline_library_selected = pipeline_catalog
            .iter()
            .position(|item| item.effect.is_some())
            .unwrap_or(0);
        let pipeline_preview_effect = pipeline_catalog
            .get(pipeline_library_selected)
            .and_then(|item| item.effect.clone());

        let mut ra_final = render_args.clone();
        if ra_final.blocks.is_empty() {
            ra_final.blocks = available_blocks.clone();
        }
        let ui_font_size = if render_args.font_size == 0.0 {
            0.0
        } else {
            render_args.font_size.round()
        };
        ra_final.font_size = ui_font_size;

        App {
            render_args: ra_final,
            input_mode: InputMode::Normal,
            selected_control: 0,
            current_tab: 0,
            general_controls,
            show_general: true,
            show_pipeline: true,
            show_glyphs: true,
            scroll: 0,
            general_scroll_offset: 0,
            tx_args,
            current_cancel_flag: initial_flag,
            args_dirty: false,
            last_args_change: None,
            rx_result,
            cached_render: String::new(),
            cached_result: None,
            base_render_result: None,
            last_render_latency: None,
            is_rendering: Arc::new(AtomicBool::new(true)),
            render_tick: 0,
            dragged_control: None,
            value_edit_buffer: String::new(),
            arc_glyphs: None,

            image_preview_protocol: None,
            output_preview_protocol: None,
            output_preview_dirty: true,
            preview_picker: Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks()),
            last_preview_path: None,

            show_font_picker: false,
            font_search: String::new(),
            font_list_state: ListState::default(),
            show_unicode_explorer: false,
            unicode_list_state: ListState::default(),
            unicode_range_path: String::new(),
            unicode_path_focused: false,
            unicode_list_area: Rect::default(),
            unicode_path_area: Rect::default(),
            unicode_add_area: Rect::default(),
            unicode_cancel_area: Rect::default(),
            metadata_prompt_settings: None,

            metadata_prompt_path: String::new(),
            clipboard_paste_rx: None,
            clipboard_paste_in_progress: false,
            clipboard_paste_status: None,
            render_mode_idx: match render_args.render {
                Render::Irc => 0,
                Render::Ansi => 1,
                Render::Ansi24 => 2,
            },
            metadata_prompt_png_path: String::new(),

            tree_state,
            glyph_list_state: ListState::default(),
            last_glyph_list_key: None,

            image_path: render_args.image.clone().unwrap_or_default(),
            ocr_overlay_count: 0,
            width: render_args.width.unwrap_or(0) as i32,
            ocr: render_args.ocr,
            ocr_lock: render_args.ocr_lock,
            ocr_auto_width: render_args.ocr_auto_width,
            ocr_figlet: render_args.ocr_figlet,
            ocr_figlet_min_height: render_args.ocr_figlet_min_height.ceil() as i32,
            ocr_figlet_min_height_ratio:
                (render_args.ocr_figlet_min_height_ratio * 100.0).round() as i32,
            ocr_figlet_fill: render_args.ocr_figlet_fill,
            ocr_figlet_max_width_ratio:
                (render_args.ocr_figlet_max_width_ratio * 100.0).round() as i32,
            ocr_figlet_max_height_ratio:
                (render_args.ocr_figlet_max_height_ratio * 100.0).round() as i32,
            ocr_min_confidence: (render_args.ocr_min_confidence * 100.0).round() as i32,
            ocr_megapixels: (render_args.ocr_megapixels.unwrap_or(1.5) * 10.0).round() as i32,
            show_ocr_advanced: false,
            ocr_model_tier_idx: match render_args.ocr_model_tier {
                OcrModelTier::Tiny => 0,
                OcrModelTier::Small => 1,
                OcrModelTier::Medium => 2,
            },
            ocr_max_text_height_ratio: (render_args.ocr_max_text_height_ratio * 100.0).round()
                as i32,
            ocr_threads: render_args.ocr_threads as i32,
            dither: render_args.dither as i32,
            show_discontinuities: render_args.show_discontinuities,
            discontinuity_threshold: (render_args.discontinuity_threshold * 10.0).round() as i32,
            graphics_preview: false,
            show_advanced: false,
            score_fix: render_args.score_fix,
            score_fix_candidates: render_args.score_fix_candidates as i32,
            score_fix_orderings: render_args.score_fix_orderings as i32,
            score_fix_geometry_first: render_args.score_fix_geometry_first,
            score_fix_neighborhood_guard: render_args.score_fix_neighborhood_guard,
            contour_bending_weight: (render_args.contour_bending_weight * 10.0).round() as i32,
            contour_endpoint_weight: (render_args.contour_endpoint_weight * 10.0).round() as i32,
            contour_junction_weight: (render_args.contour_junction_weight * 10.0).round() as i32,
            contour_fragment_weight: (render_args.contour_fragment_weight * 10.0).round() as i32,
            contour_fidelity_weight: (render_args.contour_fidelity_weight * 10.0).round() as i32,
            contour_boundary_weight: (render_args.contour_boundary_weight * 10.0).round() as i32,
            contour_peak_weight: (render_args.contour_peak_weight * 10.0).round() as i32,

            misc3: render_args.misc3,
            misc4: render_args.misc4,
            misc5: render_args.misc5,
            misc6: render_args.misc6,
            misc7: render_args.misc7,
            misc8: render_args.misc8,
            invert: render_args.invert,
            grayscale: render_args.grayscale,
            brightness: 0,
            contrast: 0,
            saturation: 0,
            gamma: render_args.gamma as i32,
            hue: render_args.hue as i32,

            slider_last_press: None,
            slider_repeat_count: 0,

            trim_top: render_args.trim.map(|t| t.0 as i32).unwrap_or(0),
            trim_right: render_args.trim.map(|t| t.1 as i32).unwrap_or(0),
            trim_bottom: render_args.trim.map(|t| t.2 as i32).unwrap_or(0),
            trim_left: render_args.trim.map(|t| t.3 as i32).unwrap_or(0),
            fliph: render_args.fliph,
            flipv: render_args.flipv,
            rotate: render_args.rotate as i32,
            scalex: render_args
                .scale
                .map(|s| (s.0 * 100.0) as i32)
                .unwrap_or(100),
            scaley: render_args
                .scale
                .map(|s| (s.1 * 100.0) as i32)
                .unwrap_or(100),
            font_size: ui_font_size,
            fonts: list_system_fonts(),
            font_idx: {
                // Match config font in the system fonts list
                let sys_fonts = list_system_fonts();
                let config_font = render_args.font.first().map(|s| s.as_str()).unwrap_or("");
                if config_font.is_empty() {
                    0 // "Default"
                } else {
                    sys_fonts
                        .iter()
                        .position(|f| f.eq_ignore_ascii_case(config_font))
                        .unwrap_or(0)
                }
            },
            filter_idx: match render_args.filter {
                SamplingFilter::Nearest => 0,
                SamplingFilter::Triangle => 1,
                SamplingFilter::CatmullRom => 2,
                SamplingFilter::Gaussian => 3,
                SamplingFilter::Lanczos3 => 4,
            },
            grayscale_tolerance: render_args.grayscale_tolerance as i32,
            colorspace_idx: match render_args.colorspace {
                ColourSpace::HSL => 0,
                ColourSpace::HSV => 1,
                ColourSpace::HSLUV => 2,
                ColourSpace::LCH => 3,
            },
            encoding_idx: match render_args.encoding {
                Encoding::Utf8 => 0,
                Encoding::Utf16 => 1,
                Encoding::Utf16be => 2,
                Encoding::Utf16le => 3,
                Encoding::Cesu8 => 4,
            },

            pixelize: render_args.pixelize,
            gaussian_blur: render_args.gaussian_blur,
            box_blur: render_args.box_blur,
            oil: render_args.oil.is_some(),
            halftone: render_args.halftone,
            sepia: render_args.sepia,
            normalize: render_args.normalize,
            noise: render_args.noise,
            emboss: render_args.emboss,
            identity: render_args.identity,
            laplace: render_args.laplace,
            noise_reduction: render_args.noise_reduction,
            sharpen: render_args.sharpen,
            cali: render_args.cali,
            dramatic: render_args.dramatic,
            firenze: render_args.firenze,
            golden: render_args.golden,
            lix: render_args.lix,
            lofi: render_args.lofi,
            neue: render_args.neue,
            obsidian: render_args.obsidian,
            pastel_pink: render_args.pastel_pink,
            ryo: render_args.ryo,
            frosted_glass: render_args.frosted_glass,
            solarize: render_args.solarize,
            edge_detection: render_args.edge_detection,

            pipeline_nodes: Vec::new(),
            pipeline_catalog,
            pipeline_list_state: ratatui::widgets::ListState::default(),
            pipeline_next_id: 1,
            pipeline_dragging: None,
            pipeline_scroll: 0,
            pipeline_selected: None,
            pipeline_expanded: None,
            pipeline_library_selected,
            pipeline_preview_effect,

            available_blocks,
            explorer_dialog: None,
            control_areas: Vec::new(),
            tab_areas: Vec::new(),
            path_area: Rect::default(),
            general_area: Rect::default(),
            pipeline_column_area: Rect::default(),
            pipeline_active_area: Rect::default(),
            glyphs_area: Rect::default(),
            tree_area: Rect::default(),
            glyph_list_area: Rect::default(),
            glyph_select_all_area: Rect::default(),
            glyph_select_none_area: Rect::default(),
            glyph_add_area: Rect::default(),
            glyph_delete_area: Rect::default(),
            explorer_area: Rect::default(),
            preview_area: Rect::default(),
            output_area: Rect::default(),
            options_pane_area: Rect::default(),
            options_areas: Vec::new(),
            eyedropper_color: None,
            replace_colour_dialog: None,
            replace_colour_button_area: Rect::default(),
            figlet_picker: None,
            is_optimizing: false,
            optimization_progress: String::new(),
            optimization_best_score: None,
            rx_optimization: None,
            auto_optimize_state: None,
            saved_auto_optimize_inputs: Vec::new(),
            saved_auto_optimize_enabled_rows: [true; 8],
            saved_auto_optimize_shortest_longest_line: false,
            saved_auto_optimize_randomize: false,
            auto_optimize_cancel: None,
            general_pane_width: ui_layout
                .panes
                .general_width
                .unwrap_or(GENERAL_PANE_WIDTH)
                .clamp(18, 240),
            pipeline_pane_width: ui_layout
                .panes
                .pipeline_width
                .unwrap_or(PIPELINE_PANE_WIDTH)
                .clamp(24, 240),
            glyphs_pane_width: ui_layout
                .panes
                .glyphs_width
                .unwrap_or(GLYPHS_PANE_WIDTH)
                .clamp(22, 240),
            glyph_tree_height: ui_layout
                .panes
                .glyph_tree_height
                .unwrap_or(10)
                .clamp(5, 240),
            glyph_list_height: ui_layout
                .panes
                .glyph_list_height
                .unwrap_or(30)
                .clamp(5, 240),
            splitter_areas: Vec::new(),
            dragging_splitter: None,
            terminal_area: Rect::default(),

            text_pane_width: ui_layout
                .panes
                .text_width
                .unwrap_or(TEXT_PANE_WIDTH)
                .clamp(41, 240),
            text_area: Rect::default(),
            text_list_state: ratatui::widgets::ListState::default(),
            text_list_area: Rect::default(),
            color_picker_area: Rect::default(),
            text_selected: None,
            text_drag_mode: TextDragMode::None,
            text_drag_start: None,
            last_text_click: None,
            text_cursor_anchor: None,
            text_hovered: None,
            text_resize_hover: None,
            base_grid: None,
            text_field_focus: TextFieldFocus::List,
            text_field_areas: Vec::new(),
            text_btn_areas: Vec::new(),
            color_picker_state: None,
        }
    }

    fn apply_settings(&mut self, render_args: &RenderArgs) {
        self.render_args = render_args.clone();
        let ui_font_size = if render_args.font_size == 0.0 {
            0.0
        } else {
            render_args.font_size.round()
        };
        self.render_args.font_size = ui_font_size;
        self.width = render_args.width.unwrap_or(0) as i32;
        self.ocr = render_args.ocr;
        self.ocr_lock = render_args.ocr_lock;
        self.ocr_auto_width = render_args.ocr_auto_width;
        self.ocr_figlet = render_args.ocr_figlet;
        self.ocr_figlet_min_height = render_args.ocr_figlet_min_height.ceil() as i32;
        self.ocr_figlet_min_height_ratio =
            (render_args.ocr_figlet_min_height_ratio * 100.0).round() as i32;
        self.ocr_figlet_fill = render_args.ocr_figlet_fill;
        self.ocr_figlet_max_width_ratio =
            (render_args.ocr_figlet_max_width_ratio * 100.0).round() as i32;
        self.ocr_figlet_max_height_ratio =
            (render_args.ocr_figlet_max_height_ratio * 100.0).round() as i32;
        self.ocr_min_confidence = (render_args.ocr_min_confidence * 100.0).round() as i32;
        self.ocr_megapixels = (render_args.ocr_megapixels.unwrap_or(1.5) * 10.0).round() as i32;
        self.ocr_model_tier_idx = match render_args.ocr_model_tier {
            OcrModelTier::Tiny => 0,
            OcrModelTier::Small => 1,
            OcrModelTier::Medium => 2,
        };
        self.ocr_max_text_height_ratio =
            (render_args.ocr_max_text_height_ratio * 100.0).round() as i32;
        self.ocr_threads = render_args.ocr_threads as i32;
        self.render_mode_idx = match render_args.render {
            Render::Irc => 0,
            Render::Ansi => 1,
            Render::Ansi24 => 2,
        };
        self.dither = render_args.dither as i32;
        self.show_discontinuities = render_args.show_discontinuities;
        self.discontinuity_threshold = (render_args.discontinuity_threshold * 10.0).round() as i32;
        self.score_fix = render_args.score_fix;
        self.score_fix_candidates = render_args.score_fix_candidates as i32;
        self.score_fix_orderings = render_args.score_fix_orderings as i32;
        self.score_fix_geometry_first = render_args.score_fix_geometry_first;
        self.score_fix_neighborhood_guard = render_args.score_fix_neighborhood_guard;
        self.contour_bending_weight = (render_args.contour_bending_weight * 10.0).round() as i32;
        self.contour_endpoint_weight =
            (render_args.contour_endpoint_weight * 10.0).round() as i32;
        self.contour_junction_weight =
            (render_args.contour_junction_weight * 10.0).round() as i32;
        self.contour_fragment_weight =
            (render_args.contour_fragment_weight * 10.0).round() as i32;
        self.contour_fidelity_weight =
            (render_args.contour_fidelity_weight * 10.0).round() as i32;
        self.contour_boundary_weight =
            (render_args.contour_boundary_weight * 10.0).round() as i32;
        self.contour_peak_weight = (render_args.contour_peak_weight * 10.0).round() as i32;
        self.misc3 = render_args.misc3;
        self.misc4 = render_args.misc4;
        self.misc5 = render_args.misc5;
        self.misc6 = render_args.misc6;
        self.misc7 = render_args.misc7;
        self.misc8 = render_args.misc8;
        self.invert = render_args.invert;
        self.grayscale = render_args.grayscale;
        self.brightness = render_args.brightness as i32;
        self.contrast = render_args.contrast as i32;
        self.saturation = render_args.saturation as i32;
        self.gamma = render_args.gamma as i32;
        self.hue = render_args.hue as i32;
        self.trim_top = render_args.trim.map(|t| t.0 as i32).unwrap_or(0);
        self.trim_right = render_args.trim.map(|t| t.1 as i32).unwrap_or(0);
        self.trim_bottom = render_args.trim.map(|t| t.2 as i32).unwrap_or(0);
        self.trim_left = render_args.trim.map(|t| t.3 as i32).unwrap_or(0);
        self.fliph = render_args.fliph;
        self.flipv = render_args.flipv;
        self.rotate = render_args.rotate as i32;
        self.scalex = render_args
            .scale
            .map(|s| (s.0 * 100.0) as i32)
            .unwrap_or(100);
        self.scaley = render_args
            .scale
            .map(|s| (s.1 * 100.0) as i32)
            .unwrap_or(100);
        self.font_size = ui_font_size;

        let sys_fonts = list_system_fonts();
        let config_font = render_args.font.first().map(|s| s.as_str()).unwrap_or("");
        self.font_idx = if config_font.is_empty() {
            0
        } else {
            sys_fonts
                .iter()
                .position(|f| f.eq_ignore_ascii_case(config_font))
                .unwrap_or(0)
        };

        self.filter_idx = match render_args.filter {
            SamplingFilter::Nearest => 0,
            SamplingFilter::Triangle => 1,
            SamplingFilter::CatmullRom => 2,
            SamplingFilter::Gaussian => 3,
            SamplingFilter::Lanczos3 => 4,
        };
        self.grayscale_tolerance = render_args.grayscale_tolerance as i32;
        self.colorspace_idx = match render_args.colorspace {
            ColourSpace::HSL => 0,
            ColourSpace::HSV => 1,
            ColourSpace::HSLUV => 2,
            ColourSpace::LCH => 3,
        };
        self.encoding_idx = match render_args.encoding {
            Encoding::Utf8 => 0,
            Encoding::Utf16 => 1,
            Encoding::Utf16be => 2,
            Encoding::Utf16le => 3,
            Encoding::Cesu8 => 4,
        };

        self.pixelize = render_args.pixelize;
        self.gaussian_blur = render_args.gaussian_blur;
        self.box_blur = render_args.box_blur;
        self.oil = render_args.oil.is_some();
        self.halftone = render_args.halftone;
        self.sepia = render_args.sepia;
        self.normalize = render_args.normalize;
        self.noise = render_args.noise;
        self.emboss = render_args.emboss;
        self.identity = render_args.identity;
        self.laplace = render_args.laplace;
        self.noise_reduction = render_args.noise_reduction;
        self.sharpen = render_args.sharpen;
        self.cali = render_args.cali;
        self.dramatic = render_args.dramatic;
        self.firenze = render_args.firenze;
        self.golden = render_args.golden;
        self.lix = render_args.lix;
        self.lofi = render_args.lofi;
        self.neue = render_args.neue;
        self.obsidian = render_args.obsidian;
        self.pastel_pink = render_args.pastel_pink;
        self.ryo = render_args.ryo;
        self.frosted_glass = render_args.frosted_glass;
        self.solarize = render_args.solarize;
        self.edge_detection = render_args.edge_detection;
    }

    fn get_current_control_id(&self) -> ControlId {
        if self.current_tab == 0 {
            let ctrls = self.tab_controls(0);
            ctrls[self.selected_control.min(ctrls.len() - 1)].id
        } else {
            ControlId::Width // Fallback
        }
    }

    fn is_tab_visible(&self, tab: usize) -> bool {
        match tab {
            0 => self.show_general,
            1 => self.show_pipeline,
            2 => self.show_glyphs,
            _ => false,
        }
    }

    fn visible_tabs(&self) -> Vec<usize> {
        vec![0, 1, 2, 3]
    }

    fn visible_focus_targets(&self) -> Vec<FocusTarget> {
        match self.current_tab {
            0 => vec![FocusTarget::General],
            1 => vec![FocusTarget::Pipeline],
            2 => vec![
                FocusTarget::GlyphTree,
                FocusTarget::GlyphList,
                FocusTarget::GlyphSelectAll,
                FocusTarget::GlyphSelectNone,
                FocusTarget::GlyphAdd,
                FocusTarget::GlyphDelete,
            ],
            3 => vec![FocusTarget::Text],
            _ => vec![FocusTarget::General],
        }
    }

    fn focus_target(&self) -> Option<FocusTarget> {
        match self.current_tab {
            0 => Some(FocusTarget::General),
            1 => Some(FocusTarget::Pipeline),
            2 => Some(match self.selected_control {
                0 => FocusTarget::GlyphTree,
                1 => FocusTarget::GlyphList,
                2 => FocusTarget::GlyphSelectAll,
                3 => FocusTarget::GlyphSelectNone,
                4 => FocusTarget::GlyphAdd,
                5 => FocusTarget::GlyphDelete,
                _ => FocusTarget::GlyphTree,
            }),
            3 => Some(FocusTarget::Text),
            _ => None,
        }
    }

    fn set_focus_target(&mut self, target: FocusTarget) {
        match target {
            FocusTarget::General => {
                self.current_tab = 0;
            }
            FocusTarget::Pipeline => {
                self.current_tab = 1;
                self.ensure_pipeline_selection();
            }
            FocusTarget::GlyphTree => {
                self.current_tab = 2;
                self.selected_control = 0;
            }
            FocusTarget::GlyphList => {
                self.current_tab = 2;
                self.selected_control = 1;
            }
            FocusTarget::GlyphSelectAll => {
                self.current_tab = 2;
                self.selected_control = 2;
            }
            FocusTarget::GlyphSelectNone => {
                self.current_tab = 2;
                self.selected_control = 3;
            }
            FocusTarget::GlyphAdd => {
                self.current_tab = 2;
                self.selected_control = 4;
            }
            FocusTarget::GlyphDelete => {
                self.current_tab = 2;
                self.selected_control = 5;
            }
            FocusTarget::Text => {
                self.current_tab = 3;
            }
        }
    }

    fn cycle_focus_target(&mut self, direction: i32) {
        let targets = self.visible_focus_targets();
        if targets.is_empty() {
            return;
        }

        let current_idx = self
            .focus_target()
            .and_then(|target| targets.iter().position(|candidate| *candidate == target))
            .unwrap_or(0) as i32;
        let next_idx = (current_idx + direction).rem_euclid(targets.len() as i32) as usize;
        self.set_focus_target(targets[next_idx]);
    }

    fn ensure_visible_focus(&mut self) {
        if self.focus_target().is_some() {
            return;
        }

        if let Some(target) = self.visible_focus_targets().into_iter().next() {
            self.set_focus_target(target);
        } else {
            self.current_tab = 0;
            self.selected_control = 0;
        }
    }

    fn toggle_tab(&mut self, tab: usize) {
        self.current_tab = tab.min(3);
        self.selected_control = 0;
    }

    fn focus_next_visible_tab(&mut self, direction: i32) {
        let visible = self.visible_tabs();
        if visible.is_empty() {
            self.current_tab = 0;
            self.selected_control = 0;
            return;
        }

        let current_idx = visible
            .iter()
            .position(|tab| *tab == self.current_tab)
            .unwrap_or(0) as i32;
        let next_idx = (current_idx + direction).rem_euclid(visible.len() as i32) as usize;
        self.current_tab = visible[next_idx];
        self.selected_control = 0;
    }

    fn pipeline_selected_index(&self) -> Option<usize> {
        self.pipeline_selected
            .and_then(|id| self.pipeline_nodes.iter().position(|node| node.id == id))
    }

    pub fn get_pipeline_clicked_layer(&self, inner_y: usize) -> (Option<usize>, Option<usize>) {
        let visual_y = inner_y + self.pipeline_list_state.offset();
        let mut current_y = 0;
        let total_nodes = self.pipeline_nodes.len();
        for i in 0..=total_nodes {
            if visual_y == current_y {
                return (Some(i), None);
            }
            current_y += 1;
            if self.pipeline_expanded == Some(i) {
                let catalog_len = self.pipeline_catalog.len();
                if visual_y >= current_y && visual_y < current_y + catalog_len {
                    return (Some(i), Some(visual_y - current_y));
                }
                current_y += catalog_len;
            }
        }
        (None, None)
    }

    fn ensure_pipeline_selection(&mut self) {
        if let Some(idx) = self.pipeline_selected_index() {
            self.selected_control = idx;
        } else {
            self.selected_control = self.selected_control.min(self.pipeline_nodes.len());
            if self.selected_control < self.pipeline_nodes.len() {
                self.pipeline_selected = Some(self.pipeline_nodes[self.selected_control].id);
            } else {
                self.pipeline_selected = None;
            }
        }

        if self.pipeline_library_selected >= self.pipeline_catalog.len() {
            self.pipeline_library_selected = self.pipeline_catalog.len().saturating_sub(1);
        }
    }

    fn get_value(&self, id: ControlId) -> i32 {
        match id {
            ControlId::Width => self.width,
            ControlId::RenderMode => self.render_mode_idx as i32,
            ControlId::GraphicsPreview => self.graphics_preview as i32,
            ControlId::Ocr => self.ocr as i32,
            ControlId::OcrLock => self.ocr_lock as i32,
            ControlId::OcrAutoWidth => self.ocr_auto_width as i32,
            ControlId::OcrFiglet => self.ocr_figlet as i32,
            ControlId::OcrFigletMinHeight => self.ocr_figlet_min_height,
            ControlId::OcrFigletMinHeightRatio => self.ocr_figlet_min_height_ratio,
            ControlId::OcrFigletFill => self.ocr_figlet_fill as i32,
            ControlId::OcrFigletMaxWidthRatio => self.ocr_figlet_max_width_ratio,
            ControlId::OcrFigletMaxHeightRatio => self.ocr_figlet_max_height_ratio,
            ControlId::OcrMinConfidence => self.ocr_min_confidence,
            ControlId::OcrMegapixels => self.ocr_megapixels,
            ControlId::OcrAdvanced => self.show_ocr_advanced as i32,
            ControlId::OcrModelTier => self.ocr_model_tier_idx as i32,
            ControlId::OcrMaxTextHeight => self.ocr_max_text_height_ratio,
            ControlId::OcrThreads => self.ocr_threads,
            ControlId::TrimTop => self.trim_top,
            ControlId::TrimRight => self.trim_right,
            ControlId::TrimBottom => self.trim_bottom,
            ControlId::TrimLeft => self.trim_left,
            ControlId::FlipH => self.fliph as i32,
            ControlId::FlipV => self.flipv as i32,
            ControlId::Rotate => self.rotate,
            ControlId::ScaleX => self.scalex,
            ControlId::ScaleY => self.scaley,
            ControlId::FontSize => (self.font_size * 10.0).round() as i32,
            ControlId::Font => self.font_idx as i32,
            ControlId::Filter => self.filter_idx as i32,
            ControlId::GrayTol => self.grayscale_tolerance,
            ControlId::Brightness => self.brightness,
            ControlId::Contrast => self.contrast,
            ControlId::Saturation => self.saturation,
            ControlId::Gamma => self.gamma,
            ControlId::Hue => self.hue,
            ControlId::Colorspace => self.colorspace_idx as i32,
            ControlId::Encoding => self.encoding_idx as i32,

            ControlId::Dither => self.dither,
            ControlId::HighlightDisc => self.show_discontinuities as i32,
            ControlId::DiscThreshold => self.discontinuity_threshold,
            ControlId::ScoreFix => self.score_fix as i32,
            ControlId::ScoreFixCandidates => self.score_fix_candidates,
            ControlId::ScoreFixOrderings => self.score_fix_orderings,
            ControlId::ScoreFixGeometryFirst => self.score_fix_geometry_first as i32,
            ControlId::ScoreFixNeighborhoodGuard => self.score_fix_neighborhood_guard as i32,
            ControlId::ContourBendingWeight => self.contour_bending_weight,
            ControlId::ContourEndpointWeight => self.contour_endpoint_weight,
            ControlId::ContourJunctionWeight => self.contour_junction_weight,
            ControlId::ContourFragmentWeight => self.contour_fragment_weight,
            ControlId::ContourFidelityWeight => self.contour_fidelity_weight,
            ControlId::ContourBoundaryWeight => self.contour_boundary_weight,
            ControlId::ContourPeakWeight => self.contour_peak_weight,
            ControlId::ShowAdvanced => self.show_advanced as i32,

            ControlId::Invert => self.invert as i32,
            ControlId::Grayscale => self.grayscale as i32,
            ControlId::Pixelize => self.pixelize,
            ControlId::GaussianBlur => self.gaussian_blur,
            ControlId::BoxBlur => self.box_blur as i32,
            ControlId::Oil => self.oil as i32,
            ControlId::Halftone => self.halftone as i32,
            ControlId::Sepia => self.sepia as i32,
            ControlId::Normalize => self.normalize as i32,
            ControlId::Noise => self.noise as i32,
            ControlId::Emboss => self.emboss as i32,
            ControlId::Identity => self.identity as i32,
            ControlId::Laplace => self.laplace as i32,
            ControlId::NoiseReduction => self.noise_reduction as i32,
            ControlId::Sharpen => self.sharpen as i32,
            ControlId::Cali => self.cali as i32,
            ControlId::Dramatic => self.dramatic as i32,
            ControlId::Firenze => self.firenze as i32,
            ControlId::Golden => self.golden as i32,
            ControlId::Lix => self.lix as i32,
            ControlId::Lofi => self.lofi as i32,
            ControlId::Neue => self.neue as i32,
            ControlId::Obsidian => self.obsidian as i32,
            ControlId::PastelPink => self.pastel_pink as i32,
            ControlId::Ryo => self.ryo as i32,
            ControlId::FrostedGlass => self.frosted_glass as i32,
            ControlId::Solarize => self.solarize as i32,
            ControlId::EdgeDetection => self.edge_detection as i32,
            ControlId::Eyedropper => 0,
            ControlId::AutoOptimize => 0,
        }
    }

    fn set_value(&mut self, id: ControlId, val: i32) {
        if self.control_disabled_reason(id).is_some() { return; }
        match id {
            ControlId::Width => {
                let old_w = self.width;
                self.width = val;
                if old_w > 0 && val > 0 && old_w != val && !self.render_args.overlays.is_empty() {
                    let scale = val as f32 / old_w as f32;
                    for ov in &mut self.render_args.overlays {
                        ov.x = (ov.x as f32 * scale).round() as i32;
                        ov.y = (ov.y as f32 * scale).round() as i32;
                    }
                }
            }
            ControlId::RenderMode => self.render_mode_idx = val as usize,
            ControlId::GraphicsPreview => {
                self.graphics_preview = val != 0;
                self.output_preview_dirty = true;
                if !self.graphics_preview {
                    self.output_preview_protocol = None;
                }
            }
            ControlId::Ocr => self.ocr = val != 0,
            ControlId::OcrLock => self.ocr_lock = val != 0,
            ControlId::OcrAutoWidth => self.ocr_auto_width = val != 0,
            ControlId::OcrFiglet => self.ocr_figlet = val != 0,
            ControlId::OcrFigletMinHeight => self.ocr_figlet_min_height = val.clamp(0, 12),
            ControlId::OcrFigletMinHeightRatio => {
                self.ocr_figlet_min_height_ratio = val.clamp(0, 500)
            }
            ControlId::OcrFigletFill => self.ocr_figlet_fill = val != 0,
            ControlId::OcrFigletMaxWidthRatio => {
                self.ocr_figlet_max_width_ratio = val.clamp(0, 200)
            }
            ControlId::OcrFigletMaxHeightRatio => {
                self.ocr_figlet_max_height_ratio = val.clamp(0, 200)
            }
            ControlId::OcrMinConfidence => self.ocr_min_confidence = val.clamp(0, 100),
            ControlId::OcrMegapixels => self.ocr_megapixels = val.clamp(1, 160),
            ControlId::OcrAdvanced => self.show_ocr_advanced = val != 0,
            ControlId::OcrModelTier => self.ocr_model_tier_idx = val.clamp(0, 2) as usize,
            ControlId::OcrMaxTextHeight => self.ocr_max_text_height_ratio = val.clamp(0, 1000),
            ControlId::OcrThreads => self.ocr_threads = val.max(1),
            ControlId::TrimTop => self.trim_top = val,
            ControlId::TrimRight => self.trim_right = val,
            ControlId::TrimBottom => self.trim_bottom = val,
            ControlId::TrimLeft => self.trim_left = val,
            ControlId::FlipH => self.fliph = val != 0,
            ControlId::FlipV => self.flipv = val != 0,
            ControlId::Rotate => self.rotate = val,
            ControlId::ScaleX => self.scalex = val,
            ControlId::ScaleY => self.scaley = val,
            // The slider retains its historical tenths-based range so its bar
            // and persisted values remain compatible, but every applied value
            // is quantized to a whole point.
            ControlId::FontSize => self.font_size = (val as f32 / 10.0).round(),
            ControlId::Font => self.font_idx = val as usize,
            ControlId::Filter => self.filter_idx = val as usize,
            ControlId::GrayTol => self.grayscale_tolerance = val,
            ControlId::Brightness => self.brightness = val,
            ControlId::Contrast => self.contrast = val,
            ControlId::Saturation => self.saturation = val,
            ControlId::Gamma => self.gamma = val,
            ControlId::Hue => self.hue = val,
            ControlId::Colorspace => self.colorspace_idx = val as usize,
            ControlId::Encoding => self.encoding_idx = val as usize,

            ControlId::Dither => self.dither = val,
            ControlId::HighlightDisc => self.show_discontinuities = val != 0,
            ControlId::DiscThreshold => self.discontinuity_threshold = val.max(0),
            ControlId::ScoreFix => self.score_fix = val != 0,
            ControlId::ScoreFixCandidates => self.score_fix_candidates = val.max(1),
            ControlId::ScoreFixOrderings => self.score_fix_orderings = val.clamp(1, 5),
            ControlId::ScoreFixGeometryFirst => self.score_fix_geometry_first = val != 0,
            ControlId::ScoreFixNeighborhoodGuard => self.score_fix_neighborhood_guard = val != 0,
            ControlId::ContourBendingWeight => self.contour_bending_weight = val.max(0),
            ControlId::ContourEndpointWeight => self.contour_endpoint_weight = val.max(0),
            ControlId::ContourJunctionWeight => self.contour_junction_weight = val.max(0),
            ControlId::ContourFragmentWeight => self.contour_fragment_weight = val.max(0),
            ControlId::ContourFidelityWeight => self.contour_fidelity_weight = val.max(0),
            ControlId::ContourBoundaryWeight => self.contour_boundary_weight = val.max(0),
            ControlId::ContourPeakWeight => self.contour_peak_weight = val.clamp(0, 10),
            ControlId::ShowAdvanced => {
                self.show_advanced = val != 0;
                let count = self.tab_controls(0).len();
                if self.selected_control >= count {
                    self.selected_control = count.saturating_sub(1);
                }
            }

            ControlId::Invert => self.invert = val != 0,
            ControlId::Grayscale => self.grayscale = val != 0,
            ControlId::Pixelize => self.pixelize = val,
            ControlId::GaussianBlur => self.gaussian_blur = val,
            ControlId::BoxBlur => self.box_blur = val != 0,
            ControlId::Oil => self.oil = val != 0,
            ControlId::Halftone => self.halftone = val != 0,
            ControlId::Sepia => self.sepia = val != 0,
            ControlId::Normalize => self.normalize = val != 0,
            ControlId::Noise => self.noise = val != 0,
            ControlId::Emboss => self.emboss = val != 0,
            ControlId::Identity => self.identity = val != 0,
            ControlId::Laplace => self.laplace = val != 0,
            ControlId::NoiseReduction => self.noise_reduction = val != 0,
            ControlId::Sharpen => self.sharpen = val != 0,
            ControlId::Cali => self.cali = val != 0,
            ControlId::Dramatic => self.dramatic = val != 0,
            ControlId::Firenze => self.firenze = val != 0,
            ControlId::Golden => self.golden = val != 0,
            ControlId::Lix => self.lix = val != 0,
            ControlId::Lofi => self.lofi = val != 0,
            ControlId::Neue => self.neue = val != 0,
            ControlId::Obsidian => self.obsidian = val != 0,
            ControlId::PastelPink => self.pastel_pink = val != 0,
            ControlId::Ryo => self.ryo = val != 0,
            ControlId::FrostedGlass => self.frosted_glass = val != 0,
            ControlId::Solarize => self.solarize = val != 0,
            ControlId::EdgeDetection => self.edge_detection = val != 0,
            ControlId::Eyedropper | ControlId::AutoOptimize => {}
        }
    }

    fn control_disabled_reason(&self, id: ControlId) -> Option<&'static str> {
        id.disabled_reason(self.ocr, self.ocr_auto_width, self.ocr_lock)
    }

    fn tab_controls(&self, tab: usize) -> Vec<ControlDef> {
        match tab {
            0 => visible_general_controls(
                &self.general_controls,
                self.ocr,
                self.ocr_figlet,
                self.score_fix,
                self.show_advanced,
                self.show_ocr_advanced,
            ),
            1 => vec![], // Pipeline Canvas is handled totally separately
            2 => self
                .available_blocks
                .iter()
                .map(|_name| ControlDef {
                    id: ControlId::Width, // Placeholder
                    label: "block",
                    kind: ControlKind::Toggle { default: false },
                })
                .collect(),
            _ => vec![],
        }
    }

    fn control_count(&self) -> usize {
        self.tab_controls(self.current_tab).len()
    }

    fn get_control_value(&self, _tab: usize, idx: usize) -> i32 {
        if self.current_tab == 3 {
            if idx < self.available_blocks.len() {
                let name = &self.available_blocks[idx];
                return if self.render_args.blocks.is_empty()
                    || self.render_args.blocks.contains(name)
                {
                    1
                } else {
                    0
                };
            }
            return 0;
        }
        let id = if self.current_tab == 0 {
            let ctrls = self.tab_controls(0);
            ctrls[idx.min(ctrls.len() - 1)].id
        } else {
            ControlId::Width
        };
        self.get_value(id)
    }

    fn mark_args_dirty(&mut self) {
        self.args_dirty = true;
        self.last_args_change = Some(Instant::now());
    }

    fn matching_font_indices(&self) -> Vec<usize> {
        let query = self.font_search.trim().to_lowercase();
        self.fonts
            .iter()
            .enumerate()
            .filter_map(|(idx, font)| {
                (query.is_empty() || font.to_lowercase().contains(&query)).then_some(idx)
            })
            .collect()
    }

    fn sync_font_picker_selection(&mut self) {
        let matches = self.matching_font_indices();
        let selected = if matches.is_empty() {
            None
        } else if self.font_search.trim().is_empty() {
            Some(
                matches
                    .iter()
                    .position(|&idx| idx == self.font_idx)
                    .unwrap_or(0),
            )
        } else {
            let query = self.font_search.trim();
            Some(
                matches
                    .iter()
                    .position(|&idx| self.fonts[idx].eq_ignore_ascii_case(query))
                    .unwrap_or(0),
            )
        };
        self.font_list_state.select(selected);
    }

    fn open_font_picker(&mut self) {
        if self.show_font_picker {
            return;
        }
        self.show_font_picker = true;
        self.font_search.clear();
        self.sync_font_picker_selection();
    }

    fn move_font_picker_selection(&mut self, delta: i32) {
        let count = self.matching_font_indices().len();
        if count == 0 {
            self.font_list_state.select(None);
            return;
        }
        let current = self.font_list_state.selected().unwrap_or(0).min(count - 1);
        let next = (current as i32 + delta).clamp(0, count as i32 - 1) as usize;
        self.font_list_state.select(Some(next));
    }

    fn commit_font_picker(&mut self) {
        let matches = self.matching_font_indices();
        let Some(selected) = self.font_list_state.selected() else {
            return;
        };
        let Some(&font_idx) = matches.get(selected) else {
            return;
        };

        let changed = self.font_idx != font_idx;
        self.font_idx = font_idx;
        self.show_font_picker = false;
        self.font_search.clear();
        if changed {
            self.mark_args_dirty();
        }
    }

    fn cancel_font_picker(&mut self) {
        self.show_font_picker = false;
        self.font_search.clear();
        self.font_list_state.select(None);
    }

    /// Re-apply text overlays to the cached base render result.
    /// This is O(grid_cells) — safe to call on every keypress without triggering a re-render.
    fn reapply_overlays(&mut self) {
        if let Some(base) = &self.base_render_result {
            let render = match self.render_mode_idx {
                0 => Render::Irc,
                1 => Render::Ansi,
                _ => Render::Ansi24,
            };
            let overlayed = draw::apply_overlays_to_result(
                base,
                &self.render_args.overlays,
                render,
                true,
                self.grayscale_tolerance as u8,
            );
            self.cached_render = overlayed.content.clone();
            self.cached_result = Some(overlayed);
            self.output_preview_dirty = true;
        }
    }

    fn perform_auto_optimize(&mut self) {
        if self.is_optimizing {
            self.is_optimizing = false;
            self.optimization_progress = "Cancelling...".to_string();
            if let Some(flag) = &self.auto_optimize_cancel {
                flag.store(true, Ordering::Relaxed);
            }
            return;
        }

        let mut inputs = Vec::new();
        for i in 0..24 {
            let ta = if self.saved_auto_optimize_inputs.len() == 24 {
                TextArea::new(vec![self.saved_auto_optimize_inputs[i].clone()])
            } else {
                if i % 3 == 2 {
                    TextArea::new(vec!["1".to_string()])
                } else {
                    TextArea::default()
                }
            };
            inputs.push(ta);
        }

        self.auto_optimize_state = Some(AutoOptimizeState {
            inputs,
            enabled_rows: self.saved_auto_optimize_enabled_rows,
            focused_idx: 0,
            optimize_shortest_longest_line: self.saved_auto_optimize_shortest_longest_line,
            batch_size: self.render_args.auto_optimize_batch_size,
            randomize: self.saved_auto_optimize_randomize,
            input_areas: [Rect::default(); 24],
            row_toggle_areas: [Rect::default(); 8],
            shortest_line_area: Rect::default(),
            randomize_area: Rect::default(),
            batch_size_area: Rect::default(),
            start_area: Rect::default(),
            cancel_area: Rect::default(),
        });
        self.input_mode = InputMode::AutoOptimize;
    }

    fn start_auto_optimization(&mut self) {
        if self.is_optimizing {
            return;
        }

        if let Some(state) = &self.auto_optimize_state {
            self.render_args.auto_optimize_batch_size = state.batch_size;
        }
        self.flush_render_args();

        let state = self.auto_optimize_state.take().unwrap();

        let optimize_shortest_longest_line = state.optimize_shortest_longest_line;
        let randomize = state.randomize;

        fn parse_range_i32(s_min: &str, s_max: &str, s_step: &str, current: i32) -> Vec<i32> {
            let p0 = s_min.trim().parse().unwrap_or(current);
            let p1 = if s_max.trim().is_empty() {
                p0
            } else {
                s_max.trim().parse().unwrap_or(p0)
            };
            let step = s_step.trim().parse::<i32>().unwrap_or(1).max(1);
            let min = p0.min(p1);
            let max = p0.max(p1);
            let mut result = Vec::new();
            let mut curr = min;
            while curr <= max {
                result.push(curr);
                curr += step;
            }
            result
        }

        fn parse_enabled_range_i32(
            enabled: bool,
            inputs: &[TextArea<'static>],
            offset: usize,
            current: i32,
        ) -> Vec<i32> {
            if !enabled {
                return vec![current];
            }
            parse_range_i32(
                inputs[offset].lines()[0].as_str(),
                inputs[offset + 1].lines()[0].as_str(),
                inputs[offset + 2].lines()[0].as_str(),
                current,
            )
        }

        let ws = parse_enabled_range_i32(state.enabled_rows[0], &state.inputs, 0, self.width);
        let ct = parse_enabled_range_i32(state.enabled_rows[1], &state.inputs, 3, self.trim_top);
        let cr = parse_enabled_range_i32(state.enabled_rows[2], &state.inputs, 6, self.trim_right);
        let cb = parse_enabled_range_i32(state.enabled_rows[3], &state.inputs, 9, self.trim_bottom);
        let cl = parse_enabled_range_i32(state.enabled_rows[4], &state.inputs, 12, self.trim_left);
        let fs: Vec<f32> = parse_enabled_range_i32(
            state.enabled_rows[5],
            &state.inputs,
            15,
            self.font_size.round() as i32,
        )
        .into_iter()
        .map(|x| x as f32)
        .collect();
        let sx = parse_enabled_range_i32(state.enabled_rows[6], &state.inputs, 18, self.scalex);
        let sy = parse_enabled_range_i32(state.enabled_rows[7], &state.inputs, 21, self.scaley);

        let total_perms =
            ws.len() * ct.len() * cr.len() * cb.len() * cl.len() * fs.len() * sx.len() * sy.len();

        if total_perms == 0 {
            self.is_optimizing = false;
            return;
        }

        self.saved_auto_optimize_inputs =
            state.inputs.iter().map(|i| i.lines()[0].clone()).collect();
        self.saved_auto_optimize_enabled_rows = state.enabled_rows;
        self.saved_auto_optimize_shortest_longest_line = optimize_shortest_longest_line;
        self.saved_auto_optimize_randomize = randomize;

        self.is_optimizing = true;
        self.optimization_best_score = None;
        self.optimization_progress = format!("Starting... ({} perms)", total_perms);

        let cancel_flag = Arc::new(AtomicBool::new(false));
        self.auto_optimize_cancel = Some(cancel_flag.clone());

        let (tx, rx) = mpsc::channel(100);
        self.rx_optimization = Some(rx);

        let mut args = self.render_args.clone();
        let original_score = args.score;
        args.score = true;

        let glyph_store = self.arc_glyphs.clone().unwrap();
        let image_path = self.image_path.clone();

        tokio::spawn(async move {
            let image = if image_path.is_empty() {
                PhotonImage::new(vec![0; 4], 1, 1)
            } else {
                match crate::load_image_from_url_or_path(&image_path, &args, &glyph_store).await {
                    Ok(img) => img,
                    Err(_) => PhotonImage::new(vec![0; 4], 1, 1),
                }
            };

            let true_initial_score = tokio::task::spawn_blocking({
                let img = image.clone();
                let gs = glyph_store.clone();
                let a = args.clone();
                move || {
                    let fg = crate::draw::prepare_glyphs(&gs, &a);
                    let result = crate::render_output(a, &gs, img, &fg, None);
                    if optimize_shortest_longest_line {
                        result.longest_line_bytes as f64
                    } else {
                        result.score
                    }
                }
            })
            .await
            .unwrap_or(f64::MAX);

            let mut best_score = true_initial_score;
            let mut best_args = args.clone();
            let mut curr = 0;

            let concurrency = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1);
            let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(concurrency));
            let (res_tx, mut res_rx) = tokio::sync::mpsc::channel(concurrency * 2);

            let args_len = total_perms;
            let tx_clone = tx.clone();
            let img_clone = image.clone();
            let gs_clone = glyph_store.clone();
            let cancel_clone = cancel_flag.clone();
            let batch_size = args.auto_optimize_batch_size.max(1);

            tokio::spawn(async move {
                let mut aborted = false;
                let mut fg_map = std::collections::HashMap::new();
                for &f in &fs {
                    if tx_clone.is_closed()
                        || cancel_clone.load(std::sync::atomic::Ordering::Relaxed)
                    {
                        aborted = true;
                        break;
                    }
                    let gs = if (f - args.font_size).abs() < std::f32::EPSILON {
                        gs_clone.clone()
                    } else {
                        let new_store = crate::font::glyph_store(
                            &args.blocks,
                            &args.exclude_range,
                            &args.exclude,
                            &args.include_range,
                            args.include.as_ref(),
                            &args.font,
                            f,
                            false,
                            Some(cancel_clone.clone()),
                        );
                        std::sync::Arc::new(new_store)
                    };
                    let fg = crate::draw::prepare_glyphs(&gs, &args);
                    fg_map.insert(f.to_bits(), (gs, std::sync::Arc::new(fg)));
                }

                if aborted {
                    return;
                }

                struct Perm {
                    w: i32,
                    t: i32,
                    r: i32,
                    b: i32,
                    l: i32,
                    f: f32,
                    x: i32,
                    y: i32,
                }

                let mut perms = Vec::with_capacity(total_perms);
                for &w in &ws {
                    for &t in &ct {
                        for &r in &cr {
                            for &b in &cb {
                                for &l in &cl {
                                    for &f in &fs {
                                        for &x in &sx {
                                            for &y in &sy {
                                                perms.push(Perm {
                                                    w,
                                                    t,
                                                    r,
                                                    b,
                                                    l,
                                                    f,
                                                    x,
                                                    y,
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if randomize {
                    let mut seed = 123456789u64;
                    let mut xorshift = || {
                        seed ^= seed << 13;
                        seed ^= seed >> 7;
                        seed ^= seed << 17;
                        seed
                    };

                    let n = perms.len();
                    for i in (1..n).rev() {
                        let j = (xorshift() as usize) % (i + 1);
                        perms.swap(i, j);
                    }
                }

                for p in perms {
                    if tx_clone.is_closed()
                        || cancel_clone.load(std::sync::atomic::Ordering::Relaxed)
                    {
                        break;
                    }
                    let mut a = args.clone();
                    a.width = Some(p.w as u32);
                    a.trim = Some((p.t as u32, p.r as u32, p.b as u32, p.l as u32));
                    a.font_size = p.f;
                    a.scale = Some((p.x as f32 / 100.0, p.y as f32 / 100.0));

                    let permit = semaphore.clone().acquire_owned().await.unwrap();
                    let img = img_clone.clone();
                    let (gs, fg) = fg_map.get(&p.f.to_bits()).unwrap().clone();
                    let rtx = res_tx.clone();
                    let cancel_inner = cancel_clone.clone();
                    tokio::task::spawn_blocking(move || {
                        let result =
                            crate::render_output(a.clone(), &gs, img, &fg, Some(cancel_inner));
                        if result.content != "Cancelled" {
                            let score = if optimize_shortest_longest_line {
                                result.longest_line_bytes as f64
                            } else {
                                result.score
                            };
                            let _ = rtx.blocking_send((a, score));
                        }
                        drop(permit);
                    });
                }
            });

            let optimize_start_time = std::time::Instant::now();

            while let Some((a, score)) = res_rx.recv().await {
                curr += 1;
                if score < best_score {
                    best_score = score;
                    best_args = a;
                    let _ = tx
                        .send(OptimizationUpdate::BestResult(
                            best_args.clone(),
                            best_score,
                        ))
                        .await;
                }

                if curr % batch_size as usize == 0 || curr == args_len {
                    let elapsed = optimize_start_time.elapsed().as_secs_f64();
                    let rate = if elapsed > 0.0 {
                        curr as f64 / elapsed
                    } else {
                        0.0
                    };
                    let progress = (curr as f64 / args_len as f64 * 100.0) as u32;
                    let _ = tx
                        .send(OptimizationUpdate::Progress(format!(
                            "{}% -> {}/{} (Best: {:.2}, {:.1} it/s)",
                            progress, curr, args_len, best_score, rate
                        )))
                        .await;
                }
            }

            best_args.score = original_score;
            let _ = tx
                .send(OptimizationUpdate::BestResult(best_args, best_score))
                .await;
            let _ = tx.send(OptimizationUpdate::Finished).await;
        });
    }

    fn cancel_render(&mut self) {
        self.current_cancel_flag.store(true, Ordering::Relaxed);
        self.is_rendering.store(false, Ordering::Relaxed);
    }

    fn flush_render_args(&mut self) {
        if !self.args_dirty {
            return;
        }
        // Don't render if no image is loaded
        if self
            .render_args
            .image
            .as_deref()
            .map(|p| p.is_empty())
            .unwrap_or(true)
        {
            self.args_dirty = false;
            return;
        }

        self.render_args.brightness = self.brightness as f32;
        self.render_args.contrast = self.contrast as f32;
        self.render_args.saturation = self.saturation as f32;
        self.render_args.gamma = self.gamma as f32;
        self.render_args.hue = self.hue as f32;
        self.render_args.width = render_width_from_tui(self.width);
        self.render_args.ocr = self.ocr;
        self.render_args.ocr_lock = self.ocr_lock;
        self.render_args.ocr_auto_width = self.ocr_auto_width;
        self.render_args.ocr_figlet = self.ocr_figlet;
        self.render_args.ocr_figlet_min_height = self.ocr_figlet_min_height.max(0) as f32;
        self.render_args.ocr_figlet_min_height_ratio =
            self.ocr_figlet_min_height_ratio.max(0) as f32 / 100.0;
        self.render_args.ocr_figlet_fill = self.ocr_figlet_fill;
        self.render_args.ocr_figlet_max_width_ratio =
            self.ocr_figlet_max_width_ratio.max(0) as f32 / 100.0;
        self.render_args.ocr_figlet_max_height_ratio =
            self.ocr_figlet_max_height_ratio.max(0) as f32 / 100.0;
        self.render_args.ocr_min_confidence = self.ocr_min_confidence as f32 / 100.0;
        self.render_args.ocr_megapixels = Some(self.ocr_megapixels as f32 / 10.0);
        self.render_args.ocr_model_tier = match self.ocr_model_tier_idx {
            0 => OcrModelTier::Tiny,
            1 => OcrModelTier::Small,
            _ => OcrModelTier::Medium,
        };
        self.render_args.ocr_max_text_height_ratio =
            (self.ocr_max_text_height_ratio.max(0) as f32) / 100.0;
        self.render_args.ocr_threads = self.ocr_threads.max(1) as usize;
        self.render_args.dither = self.dither as u32;
        self.render_args.invert = self.invert;
        self.render_args.grayscale = self.grayscale;
        self.render_args.show_discontinuities = self.show_discontinuities;
        self.render_args.discontinuity_threshold = self.discontinuity_threshold as f32 / 10.0;
        self.render_args.score_fix = self.score_fix;
        self.render_args.score_fix_candidates = self.score_fix_candidates.max(1) as u32;
        self.render_args.score_fix_orderings = self.score_fix_orderings.clamp(1, 5) as u32;
        self.render_args.score_fix_geometry_first = self.score_fix_geometry_first;
        self.render_args.score_fix_neighborhood_guard = self.score_fix_neighborhood_guard;
        self.render_args.contour_bending_weight = self.contour_bending_weight as f32 / 10.0;
        self.render_args.contour_endpoint_weight = self.contour_endpoint_weight as f32 / 10.0;
        self.render_args.contour_junction_weight = self.contour_junction_weight as f32 / 10.0;
        self.render_args.contour_fragment_weight = self.contour_fragment_weight as f32 / 10.0;
        self.render_args.contour_fidelity_weight = self.contour_fidelity_weight as f32 / 10.0;
        self.render_args.contour_boundary_weight = self.contour_boundary_weight as f32 / 10.0;
        self.render_args.contour_peak_weight = self.contour_peak_weight as f32 / 10.0;

        self.render_args.misc3 = self.misc3;
        self.render_args.misc4 = self.misc4;
        self.render_args.misc5 = self.misc5;
        self.render_args.misc6 = self.misc6;
        self.render_args.misc7 = self.misc7;
        self.render_args.misc8 = self.misc8;
        self.render_args.pixelize = self.pixelize;
        self.render_args.gaussian_blur = self.gaussian_blur;
        self.render_args.box_blur = self.box_blur;
        self.render_args.oil = if self.oil {
            Some("2,20.0".to_string())
        } else {
            None
        };
        self.render_args.halftone = self.halftone;
        self.render_args.sepia = self.sepia;
        self.render_args.normalize = self.normalize;
        self.render_args.noise = self.noise;
        self.render_args.emboss = self.emboss;
        self.render_args.identity = self.identity;
        self.render_args.laplace = self.laplace;
        self.render_args.noise_reduction = self.noise_reduction;
        self.render_args.sharpen = self.sharpen;
        self.render_args.cali = self.cali;
        self.render_args.dramatic = self.dramatic;
        self.render_args.firenze = self.firenze;
        self.render_args.golden = self.golden;
        self.render_args.lix = self.lix;
        self.render_args.lofi = self.lofi;
        self.render_args.neue = self.neue;
        self.render_args.obsidian = self.obsidian;
        self.render_args.pastel_pink = self.pastel_pink;
        self.render_args.ryo = self.ryo;
        self.render_args.frosted_glass = self.frosted_glass;
        self.render_args.solarize = self.solarize;
        self.render_args.edge_detection = self.edge_detection;
        self.render_args.render = match self.render_mode_idx {
            0 => Render::Irc,
            1 => Render::Ansi,
            _ => Render::Ansi24,
        };
        self.render_args.as_preview = true;
        self.render_args.fliph = self.fliph;
        self.render_args.flipv = self.flipv;
        self.render_args.rotate = self.rotate as f32;
        self.render_args.scale = Some((self.scalex as f32 / 100.0, self.scaley as f32 / 100.0));
        self.render_args.font_size = self.font_size;
        if !self.fonts.is_empty() && self.font_idx < self.fonts.len() {
            let selected = &self.fonts[self.font_idx];
            if selected == "Default" {
                self.render_args.font = Vec::new();
            } else {
                self.render_args.font = vec![selected.clone()];
            }
        }
        self.render_args.filter = match self.filter_idx {
            0 => SamplingFilter::Nearest,
            1 => SamplingFilter::Triangle,
            2 => SamplingFilter::CatmullRom,
            3 => SamplingFilter::Gaussian,
            _ => SamplingFilter::Lanczos3,
        };
        self.render_args.grayscale_tolerance = self.grayscale_tolerance as u8;
        self.render_args.colorspace = match self.colorspace_idx {
            0 => ColourSpace::HSL,
            1 => ColourSpace::HSV,
            2 => ColourSpace::HSLUV,
            _ => ColourSpace::LCH,
        };
        self.render_args.encoding = match self.encoding_idx {
            0 => Encoding::Utf8,
            1 => Encoding::Utf16,
            2 => Encoding::Utf16be,
            3 => Encoding::Utf16le,
            _ => Encoding::Cesu8,
        };
        if self.trim_top > 0 || self.trim_right > 0 || self.trim_bottom > 0 || self.trim_left > 0 {
            self.render_args.trim = Some((
                self.trim_top as u32,
                self.trim_right as u32,
                self.trim_bottom as u32,
                self.trim_left as u32,
            ));
        } else {
            self.render_args.trim = None;
        }
        let mut pipeline = Vec::new();
        for (i, node) in self.pipeline_nodes.iter().enumerate() {
            if self.current_tab == 1 && Some(i) == self.pipeline_expanded {
                if let Some(preview) = &self.pipeline_preview_effect {
                    pipeline.push(preview.clone());
                    continue; // Skip the original effect if we are previewing a replacement
                }
            }
            if !node.disabled {
                pipeline.push(node.effect.clone());
            }
        }

        if self.current_tab == 1 && self.pipeline_expanded == Some(self.pipeline_nodes.len()) {
            if let Some(preview) = &self.pipeline_preview_effect {
                pipeline.push(preview.clone());
            }
        }

        self.render_args.pipeline = pipeline;

        // Cancel previous render
        self.current_cancel_flag.store(true, Ordering::Relaxed);
        // Create new flag
        let new_flag = Arc::new(AtomicBool::new(false));
        self.current_cancel_flag = new_flag.clone();

        // Strip overlays before sending to render thread — overlays are applied as a
        // post-process in the display layer, so typing text never triggers a full re-render.
        let mut args_for_render = self.render_args.clone();
        args_for_render.overlays = Vec::new();
        let _ = self
            .tx_args
            .send((args_for_render, new_flag, Instant::now()));
        log::info!(
            "Flushing render args: width={}, scale={:?}, font={:?}",
            self.width,
            self.render_args.scale,
            self.render_args.font
        );
        self.args_dirty = false;
        self.last_args_change = None;
    }

    fn open_explorer(&mut self, mode: ExplorerMode) {
        if let Ok(mut explorer) = FileExplorer::new() {
            if matches!(mode, ExplorerMode::Open) {
                let filter = Arc::new(|file: &explorer::File| {
                    if file.is_dir() {
                        return true;
                    }
                    let ext = file
                        .path()
                        .extension()
                        .and_then(|s| s.to_str())
                        .unwrap_or_default()
                        .to_lowercase();
                    matches!(
                        ext.as_str(),
                        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "tiff"
                    )
                });
                explorer = explorer.with_filter(filter).unwrap();
            }

            let mut name_input = TextArea::default();
            name_input.set_block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Filename (Enter to Save) "),
            );
            name_input.set_cursor_line_style(Style::default());

            // If we have a current image path, maybe suggest a default name for saving
            if !matches!(mode, ExplorerMode::Open) {
                let default_name = Path::new(&self.image_path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("output")
                    .to_string();
                name_input.insert_str(default_name);
            }

            self.explorer_dialog = Some(ExplorerDialog {
                explorer,
                mode,
                name_input,
                focus_on_input: matches!(mode, ExplorerMode::SaveText | ExplorerMode::SavePng),
            });
            self.input_mode = InputMode::Explorer;
        }
    }

    fn handle_explorer_result(&mut self, mode: ExplorerMode, path: String) {
        if path.is_empty() {
            return;
        }

        match mode {
            ExplorerMode::Open => {
                self.load_image(path);
            }
            ExplorerMode::SaveText => {
                let save_data = self
                    .cached_result
                    .as_ref()
                    .and_then(|r| r.save_content.as_ref())
                    .unwrap_or(&self.cached_render);
                if let Err(e) = std::fs::write(&path, save_data) {
                    log::info!("Failed to save text: {}", e);
                }
            }
            ExplorerMode::SavePng => {
                if let (Some(result), Some(glyphs)) = (&self.cached_result, &self.arc_glyphs) {
                    let png_path = if path.ends_with(".png") {
                        path
                    } else {
                        format!("{}.png", path)
                    };
                    // Serialize current settings as JSON metadata
                    let metadata = serde_json::to_string(&self.render_args).unwrap_or_default();
                    if let Err(e) = draw::save_as_png(result, glyphs, &png_path, &metadata) {
                        log::info!("Failed to save PNG: {}", e);
                    }
                }
            }
        }
    }

    fn load_image(&mut self, path: String) {
        if path.to_lowercase().ends_with(".png") {
            if let Ok(file) = std::fs::File::open(&path) {
                let reader = std::io::BufReader::new(file);
                let decoder = png::Decoder::new(reader);
                if let Ok(png_reader) = decoder.read_info() {
                    let info: &png::Info = png_reader.info();
                    for chunk in &info.uncompressed_latin1_text {
                        if chunk.keyword == "img2irc_settings" {
                            if let Ok(settings) = serde_json::from_str::<RenderArgs>(&chunk.text) {
                                if let Some(ref root_image) = settings.image {
                                    let png_dir = std::path::Path::new(&path)
                                        .parent()
                                        .unwrap_or(std::path::Path::new(""));
                                    let input_path = png_dir.join(root_image);
                                    let found_path = if input_path.exists() {
                                        Some(input_path.to_string_lossy().to_string())
                                    } else if std::path::Path::new(root_image).exists() {
                                        Some(root_image.clone())
                                    } else {
                                        None
                                    };

                                    if let Some(real_input_path) = found_path {
                                        self.metadata_prompt_settings = Some(settings);
                                        self.metadata_prompt_path = real_input_path;
                                        self.metadata_prompt_png_path = path;
                                        self.input_mode = InputMode::MetadataPrompt;
                                        return;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        self.image_path = path.clone();
        self.render_args.image = Some(path);
        self.scroll = 0;
        self.mark_args_dirty();
    }

    /// Try to paste an image from the system clipboard.
    /// Attempts xclip (X11) first, then wl-paste (Wayland).
    /// Saves the image to a temp file and loads it.
    fn paste_from_clipboard(&mut self) {
        if self.clipboard_paste_in_progress {
            log::info!("Clipboard paste already in progress");
            self.clipboard_paste_status = Some("Paste already running".to_string());
            return;
        }

        let (tx, rx) = std::sync::mpsc::channel();
        self.clipboard_paste_rx = Some(rx);
        self.clipboard_paste_in_progress = true;
        self.clipboard_paste_status = Some("Pasting image...".to_string());

        std::thread::spawn(move || {
            let _ = tx.send(Self::clipboard_paste_worker());
        });
    }

    fn poll_clipboard_paste(&mut self) {
        let Some(rx) = self.clipboard_paste_rx.take() else {
            return;
        };

        match rx.try_recv() {
            Ok(ClipboardPasteResult::Loaded { path, bytes }) => {
                log::info!("Loaded image from clipboard: {} ({} bytes)", path, bytes);
                self.image_path = path.clone();
                self.render_args.image = Some(path);
                self.scroll = 0;
                self.clipboard_paste_in_progress = false;
                self.clipboard_paste_status = Some(format!("Pasted image ({} bytes)", bytes));
                self.mark_args_dirty();
            }
            Ok(ClipboardPasteResult::Empty) => {
                log::info!("No image found in clipboard");
                self.clipboard_paste_in_progress = false;
                self.clipboard_paste_status = Some("No clipboard image found".to_string());
            }
            Ok(ClipboardPasteResult::Failed(err)) => {
                log::info!("Failed to paste image from clipboard: {}", err);
                self.clipboard_paste_in_progress = false;
                self.clipboard_paste_status = Some(format!("Paste failed: {}", err));
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                self.clipboard_paste_rx = Some(rx);
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                log::info!("Clipboard paste worker disconnected");
                self.clipboard_paste_in_progress = false;
                self.clipboard_paste_status = Some("Paste worker stopped".to_string());
            }
        }
    }

    fn clipboard_paste_worker() -> ClipboardPasteResult {
        let advertised_image = Self::clipboard_advertises_image_target();

        if let Some(result) = Self::try_clipboard_native_image() {
            return result;
        }

        let clipboard_data = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            Self::try_clipboard_wayland().or_else(|| Self::try_clipboard_x11())
        } else {
            Self::try_clipboard_x11().or_else(|| Self::try_clipboard_wayland())
        };

        if let Some(data) = clipboard_data {
            return Self::save_clipboard_image_bytes(data);
        }

        if advertised_image {
            return ClipboardPasteResult::Failed(
                "clipboard advertises image data, but img2irc could not read it".to_string(),
            );
        }

        if let Some(image_url) = Self::try_clipboard_url() {
            if let Some(path) =
                image_url
                    .url
                    .strip_prefix("file://")
                    .or(Some(image_url.url.as_str()).filter(|_| {
                        !image_url.url.starts_with("http://")
                            && !image_url.url.starts_with("https://")
                    }))
            {
                let path = path.to_string();
                return ClipboardPasteResult::Loaded {
                    bytes: std::fs::metadata(&path)
                        .map(|m| m.len() as usize)
                        .unwrap_or(0),
                    path,
                };
            }
            return Self::download_clipboard_url(&image_url);
        }

        ClipboardPasteResult::Empty
    }

    fn try_clipboard_native_image() -> Option<ClipboardPasteResult> {
        let mut clipboard = match arboard::Clipboard::new() {
            Ok(clipboard) => clipboard,
            Err(e) => {
                log::info!("Native clipboard unavailable: {}", e);
                return None;
            }
        };

        let image = match clipboard.get_image() {
            Ok(image) => image,
            Err(e) => {
                log::info!("Native clipboard image unavailable: {}", e);
                return None;
            }
        };

        let tmp_path = Self::clipboard_tmp_path("png");
        let bytes = image.bytes.into_owned();
        let width = image.width as u32;
        let height = image.height as u32;
        match image::save_buffer_with_format(
            &tmp_path,
            &bytes,
            width,
            height,
            image::ColorType::Rgba8,
            image::ImageFormat::Png,
        ) {
            Ok(()) => {
                let size = std::fs::metadata(&tmp_path)
                    .map(|m| m.len() as usize)
                    .unwrap_or(bytes.len());
                log::info!(
                    "Read native clipboard image {}x{} ({} bytes)",
                    width,
                    height,
                    size
                );
                Some(ClipboardPasteResult::Loaded {
                    path: tmp_path,
                    bytes: size,
                })
            }
            Err(e) => Some(ClipboardPasteResult::Failed(format!(
                "failed to encode native clipboard image: {}",
                e
            ))),
        }
    }

    fn save_clipboard_image_bytes(data: Vec<u8>) -> ClipboardPasteResult {
        use std::io::Write;

        if data.is_empty() {
            return ClipboardPasteResult::Empty;
        }

        let Some(ext) = Self::image_extension_for_bytes(&data) else {
            return ClipboardPasteResult::Failed(
                "clipboard image data has an unknown format".to_string(),
            );
        };

        let tmp_path = Self::clipboard_tmp_path(ext);
        match std::fs::File::create(&tmp_path) {
            Ok(mut f) => {
                if let Err(e) = f.write_all(&data) {
                    return ClipboardPasteResult::Failed(format!(
                        "failed to write clipboard image: {}",
                        e
                    ));
                }
            }
            Err(e) => {
                return ClipboardPasteResult::Failed(format!("failed to create temp file: {}", e));
            }
        }

        ClipboardPasteResult::Loaded {
            path: tmp_path,
            bytes: data.len(),
        }
    }

    fn download_clipboard_url(image_url: &ClipboardImageUrl) -> ClipboardPasteResult {
        let client = match reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
        {
            Ok(client) => client,
            Err(e) => {
                return ClipboardPasteResult::Failed(format!("failed to build HTTP client: {}", e))
            }
        };

        let mut request = client
            .get(&image_url.url)
            .header(
                reqwest::header::USER_AGENT,
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126 Safari/537.36",
            )
            .header(
                reqwest::header::ACCEPT,
                "image/avif,image/webp,image/apng,image/svg+xml,image/*,*/*;q=0.8",
            );

        let fallback_referer = Self::url_origin(&image_url.url);
        let referer = image_url.referer.as_deref().or(fallback_referer.as_deref());
        if let Some(referer) = referer {
            request = request.header(reqwest::header::REFERER, referer);
        }

        let response = match request.send() {
            Ok(response) => response,
            Err(e) => return ClipboardPasteResult::Failed(format!("failed to fetch URL: {}", e)),
        };

        if !response.status().is_success() {
            return ClipboardPasteResult::Failed(format!(
                "URL returned HTTP {}",
                response.status()
            ));
        }

        let bytes = match response.bytes() {
            Ok(bytes) => bytes.to_vec(),
            Err(e) => return ClipboardPasteResult::Failed(format!("failed to read URL: {}", e)),
        };

        Self::save_clipboard_image_bytes(bytes)
    }

    fn url_origin(url: &str) -> Option<String> {
        let parsed = url::Url::parse(url).ok()?;
        let scheme = parsed.scheme();
        let host = parsed.host_str()?;
        let port = parsed.port().map(|p| format!(":{}", p)).unwrap_or_default();
        Some(format!("{}://{}{}/", scheme, host, port))
    }

    fn clipboard_tmp_path(ext: &str) -> String {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        format!("/tmp/img2irc_clipboard_{}.{}", ts, ext)
    }

    fn image_extension_for_bytes(data: &[u8]) -> Option<&'static str> {
        if data.starts_with(&[0x89, b'P', b'N', b'G']) {
            Some("png")
        } else if data.starts_with(&[0xFF, 0xD8]) {
            Some("jpg")
        } else if data.starts_with(b"GIF") {
            Some("gif")
        } else if data.starts_with(b"BM") {
            Some("bmp")
        } else if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
            Some("webp")
        } else if data.starts_with(b"II*\0") || data.starts_with(b"MM\0*") {
            Some("tiff")
        } else {
            None
        }
    }

    fn try_clipboard_x11() -> Option<Vec<u8>> {
        for target in &[
            "image/png",
            "image/jpeg",
            "image/webp",
            "image/tiff",
            "image/bmp",
        ] {
            if let Some(data) = Self::run_clipboard_command(
                "xclip",
                &["-selection", "clipboard", "-t", target, "-o"],
                Duration::from_secs(15),
            ) {
                log::info!(
                    "Read clipboard image target {} via xclip ({} bytes)",
                    target,
                    data.len()
                );
                return Some(data);
            }
        }
        None
    }

    fn try_clipboard_wayland() -> Option<Vec<u8>> {
        for mime in &[
            "image/png",
            "image/jpeg",
            "image/webp",
            "image/tiff",
            "image/bmp",
            "image",
        ] {
            if let Some(data) =
                Self::run_clipboard_command("wl-paste", &["--type", mime], Duration::from_secs(15))
            {
                log::info!(
                    "Read clipboard image target {} via wl-paste ({} bytes)",
                    mime,
                    data.len()
                );
                return Some(data);
            }
        }
        None
    }

    fn clipboard_advertises_image_target() -> bool {
        let has_wayland_image = || {
            Self::run_clipboard_command("wl-paste", &["--list-types"], Duration::from_secs(2))
                .and_then(|data| String::from_utf8(data).ok())
                .map_or(false, |targets| {
                    targets
                        .lines()
                        .any(|line| line.trim().starts_with("image/"))
                })
        };

        let has_x11_image = || {
            Self::run_clipboard_command(
                "xclip",
                &["-selection", "clipboard", "-t", "TARGETS", "-o"],
                Duration::from_secs(2),
            )
            .and_then(|data| String::from_utf8(data).ok())
            .map_or(false, |targets| {
                targets
                    .lines()
                    .any(|line| line.trim().starts_with("image/"))
            })
        };

        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            has_wayland_image() || has_x11_image()
        } else {
            has_x11_image() || has_wayland_image()
        }
    }

    fn try_clipboard_url() -> Option<ClipboardImageUrl> {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            Self::try_clipboard_url_wayland().or_else(|| Self::try_clipboard_url_x11())
        } else {
            Self::try_clipboard_url_x11().or_else(|| Self::try_clipboard_url_wayland())
        }
    }

    fn try_clipboard_url_x11() -> Option<ClipboardImageUrl> {
        for target in &[
            "text/uri-list",
            "text/html",
            "text/plain;charset=utf-8",
            "text/plain",
            "UTF8_STRING",
        ] {
            if let Some(data) = Self::run_clipboard_command(
                "xclip",
                &["-selection", "clipboard", "-t", target, "-o"],
                Duration::from_secs(2),
            ) {
                if let Ok(text) = String::from_utf8(data) {
                    if let Some(url) = Self::extract_url_from_clipboard_text(&text) {
                        return Some(url);
                    }
                }
            }
        }
        None
    }

    fn try_clipboard_url_wayland() -> Option<ClipboardImageUrl> {
        for mime in &[
            "text/uri-list",
            "text/html",
            "text/plain;charset=utf-8",
            "text/plain",
        ] {
            if let Some(data) =
                Self::run_clipboard_command("wl-paste", &["--type", mime], Duration::from_secs(2))
            {
                if let Ok(text) = String::from_utf8(data) {
                    if let Some(url) = Self::extract_url_from_clipboard_text(&text) {
                        return Some(url);
                    }
                }
            }
        }
        None
    }

    fn extract_url_from_clipboard_text(text: &str) -> Option<ClipboardImageUrl> {
        let source_url = Regex::new(r"(?im)^SourceURL:(\S+)\s*$")
            .ok()
            .and_then(|re| re.captures(text))
            .and_then(|caps| caps.get(1))
            .map(|m| m.as_str().to_string());

        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with("http://") || line.starts_with("https://") {
                return Some(ClipboardImageUrl {
                    url: line.to_string(),
                    referer: source_url.clone(),
                });
            }
            if line.starts_with("file://") {
                return Some(ClipboardImageUrl {
                    url: line.to_string(),
                    referer: None,
                });
            }
        }

        let html_src = Regex::new(r#"(?i)<img[^>]+src=["']([^"']+)["']"#).ok()?;
        html_src
            .captures(text)
            .and_then(|caps| caps.get(1))
            .map(|m| ClipboardImageUrl {
                url: m.as_str().replace("&amp;", "&"),
                referer: source_url,
            })
    }

    fn run_clipboard_command(program: &str, args: &[&str], timeout: Duration) -> Option<Vec<u8>> {
        let mut child = Command::new(program)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;

        let mut stdout = child.stdout.take()?;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut data = Vec::new();
            let result = stdout.read_to_end(&mut data).map(|_| data);
            let _ = tx.send(result);
        });

        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let data = rx
                        .recv_timeout(Duration::from_secs(10))
                        .ok()
                        .and_then(Result::ok)?;
                    if status.success() && !data.is_empty() {
                        return Some(data);
                    }
                    log::info!(
                        "Clipboard command returned no data: {} {:?}, status={}, bytes={}",
                        program,
                        args,
                        status,
                        data.len()
                    );
                    return None;
                }
                Ok(None) if started.elapsed() >= timeout => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = rx.recv_timeout(Duration::from_millis(250));
                    log::info!("Clipboard command timed out: {} {:?}", program, args);
                    return None;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(e) => {
                    log::info!("Clipboard command failed: {} {:?}: {}", program, args, e);
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
        }
    }
}

fn list_system_fonts() -> Vec<String> {
    // Try to use fc-list with spacing=100 (monospace)
    if let Ok(output) = Command::new("fc-list").output() {
        if output.status.success() {
            let s = String::from_utf8_lossy(&output.stdout);
            let mut fonts: Vec<String> = s
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| {
                    // Clean up font name: remove path, take first family name
                    // e.g. "/usr/share/fonts/TTF/CascadiaCode.ttf: Cascadia Code" -> "Cascadia Code"
                    // e.g. "Cascadia Code,Cascadia Mono:style=Regular" -> "Cascadia Code"
                    l.split(':')
                        .nth(1)
                        .unwrap_or(l) // everything after first : (path separator)
                        .split(',')
                        .next()
                        .unwrap_or(l) // everything before first ,
                        .trim()
                        .to_string()
                })
                .collect();
            fonts.extend(crate::font::bundled_font_names().map(str::to_string));
            fonts.sort();
            fonts.dedup();
            // Move Monospace to top if exists, but we want "Default" first
            if let Some(pos) = fonts
                .iter()
                .position(|x| x.eq_ignore_ascii_case("monospace"))
            {
                let f = fonts.remove(pos);
                fonts.insert(0, f);
            }
            fonts.insert(0, "Default".to_string());
            if !fonts.is_empty() {
                return fonts;
            }
        }
    }
    std::iter::once("Default".to_string())
        .chain(crate::font::bundled_font_names().map(str::to_string))
        .collect()
}

// ── Custom Widgets ───────────────────────────────────────────────────────────

fn quantized_slider_units(value: i32, min: i32, max: i32, cells: usize) -> usize {
    if cells == 0 || max <= min {
        return 0;
    }

    let clamped = value.clamp(min, max) - min;
    let range = (max - min) as i64;
    let total_units = (cells * 8) as i64;
    ((clamped as i64) * total_units / range).clamp(0, total_units) as usize
}

fn quantized_slider_units_f32(value: f32, min: f32, max: f32, cells: usize) -> usize {
    if cells == 0 || max <= min {
        return 0;
    }

    let clamped = value.clamp(min, max) - min;
    let range = (max - min).max(f32::EPSILON);
    let total_units = (cells * 8) as f32;
    ((clamped * total_units / range).floor() as i32).clamp(0, total_units as i32) as usize
}

fn render_eighth_block_bar(units: usize, cells: usize) -> String {
    if cells == 0 {
        return String::new();
    }

    let capped_units = units.min(cells * 8);
    let full_blocks = capped_units / 8;
    let remainder = capped_units % 8;
    let used_cells = full_blocks + usize::from(remainder > 0 && full_blocks < cells);

    let mut bar = String::with_capacity(cells);
    bar.push_str(&"█".repeat(full_blocks.min(cells)));
    if remainder > 0 && full_blocks < cells {
        bar.push_str(EIGHTH_BLOCKS[remainder]);
    }
    bar.push_str(&" ".repeat(cells.saturating_sub(used_cells)));
    bar
}

fn slider_bar_layout(
    area: Rect,
    label: &str,
    _display_value: Option<&str>,
) -> Option<(u16, usize)> {
    let label_text = format!("  {}", label);
    let label_width = label_text.chars().take(slider_label_limit(area)).count() as u16;
    let bar_cells = area
        .width
        .saturating_sub(label_width + 4)
        .min(SLIDER_BAR_CELLS as u16) as usize;
    if bar_cells == 0 {
        None
    } else {
        Some((area.x + label_width + 3, bar_cells))
    }
}

fn slider_label_limit(area: Rect) -> usize {
    // Keep room for a useful bar/value while allowing descriptive option names.
    area.width.saturating_sub(12).min(24) as usize
}

/// A quantized slider bar using left eighth-blocks.
fn render_bar(
    buf: &mut Buffer,
    area: Rect,
    label: &str,
    value: i32,
    min: i32,
    max: i32,
    selected: bool,
    display_value: Option<&str>,
    unselected_bg: Color,
) {
    if area.height < 1 || area.width < 4 {
        return;
    }

    let bg_style = if selected {
        Style::new().bg(SELECTED_BG)
    } else {
        Style::new().bg(unselected_bg)
    };
    buf.set_style(area, bg_style);

    // Draw label on the left
    let label_text = if selected {
        format!("> {}", label)
    } else {
        format!("  {}", label)
    };
    let lx = area.x + 1;
    let ly = area.y + area.height.saturating_sub(1) / 2;
    for (i, ch) in label_text.chars().take(slider_label_limit(area)).enumerate() {
        let x = lx + i as u16;
        if x >= area.x + area.width {
            break;
        }
        let cell = &mut buf[(x, ly)];
        cell.set_char(ch);
        let style = if value != 0 {
            Style::new().fg(Color::White).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(TEXT)
        };
        cell.set_style(style);
    }

    if let Some((bar_x, bar_cells)) = slider_bar_layout(area, label, display_value) {
        let units = quantized_slider_units(value, min, max, bar_cells);
        let bar_str = render_eighth_block_bar(units, bar_cells);
        let bg_color = Color::Rgb(15, 15, 20);
        let fg_color = if selected { BAR_FG } else { ACCENT };

        for (i, ch) in bar_str.chars().enumerate() {
            let x = bar_x + i as u16;
            if x >= area.x + area.width {
                continue;
            }
            let cell = &mut buf[(x, ly)];
            cell.set_char(ch);
            cell.set_style(Style::new().fg(fg_color).bg(bg_color));
        }

        let display_str = if let Some(dv) = display_value {
            format!("{}", dv)
        } else {
            format!("{}", value)
        };
        let text_len = display_str.chars().count();
        let center_idx = bar_cells.saturating_sub(text_len) / 2;
        let is_filled = (units / 8) >= center_idx + text_len / 2;

        for (i, ch) in display_str.chars().enumerate() {
            let x = bar_x + center_idx as u16 + i as u16;
            if x >= area.x + area.width {
                continue;
            }
            let cell = &mut buf[(x, ly)];
            cell.set_char(ch);
            if is_filled {
                cell.set_style(
                    Style::new()
                        .fg(bg_color)
                        .bg(fg_color)
                        .add_modifier(Modifier::BOLD),
                );
            } else {
                cell.set_style(
                    Style::new()
                        .fg(fg_color)
                        .bg(bg_color)
                        .add_modifier(Modifier::BOLD),
                );
            }
        }
    }

    // Border highlight for selected
    if selected {
        // Draw left accent line
        let accent_style = Style::new().bg(ACCENT).fg(ACCENT);
        for y in area.y..area.y + area.height {
            buf.set_string(area.x, y, "▎", accent_style);
        }
    }
}

/// An enum selector (< Option >)
fn render_enum(
    buf: &mut Buffer,
    area: Rect,
    label: &str,
    options: &[&str],
    value: usize,
    selected: bool,
    unselected_bg: Color,
) {
    if area.height < 1 || area.width < 4 {
        return;
    }

    let bg = if selected {
        SELECTED_BG
    } else {
        unselected_bg
    };
    buf.set_style(area, Style::new().bg(bg));

    let ly = area.y + area.height.saturating_sub(1) / 2;
    let label_text = if selected {
        format!("> {}", label)
    } else {
        format!("  {}", label)
    };
    for (i, ch) in label_text.chars().enumerate() {
        let x = area.x + 1 + i as u16;
        if x >= area.x + area.width {
            break;
        }
        let cell = &mut buf[(x, ly)];
        cell.set_char(ch);
        let style = if value > 0 {
            Style::new()
                .fg(if selected { Color::White } else { TEXT })
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(if selected { Color::White } else { TEXT })
        };
        cell.set_style(style);
    }

    let opt = options.get(value).unwrap_or(&"?");
    let val_text = format!("◂ {} ▸ ", opt);
    let vx = area.x + area.width - val_text.len() as u16;
    for (i, ch) in val_text.chars().enumerate() {
        let x = vx + i as u16;
        if x < area.x || x >= area.x + area.width {
            continue;
        }
        let cell = &mut buf[(x, ly)];
        cell.set_char(ch);
        cell.set_style(Style::new().fg(ACCENT).add_modifier(Modifier::BOLD));
    }

    if selected {
        let accent_style = Style::new().bg(ACCENT).fg(ACCENT);
        for y in area.y..area.y + area.height {
            buf.set_string(area.x, y, "▎", accent_style);
        }
    }
}

/// A specialized widget for eyedropper color
fn render_eyedropper_widget(
    buf: &mut Buffer,
    area: Rect,
    label: &str,
    color: Option<(u8, u8, u8)>,
    selected: bool,
    unselected_bg: Color,
) {
    if area.height < 1 || area.width < 4 {
        return;
    }

    let bg = if selected {
        SELECTED_BG
    } else {
        unselected_bg
    };
    buf.set_style(area, Style::new().bg(bg));

    let ly = area.y + area.height.saturating_sub(1) / 2;
    let label_text = if selected {
        format!("> {}", label)
    } else {
        format!("  {}", label)
    };
    for (i, ch) in label_text.chars().enumerate() {
        let x = area.x + 1 + i as u16;
        if x >= area.x + area.width {
            break;
        }
        let cell = &mut buf[(x, ly)];
        cell.set_char(ch);
        let style = Style::new()
            .fg(if selected { Color::White } else { TEXT })
            .add_modifier(Modifier::BOLD);
        cell.set_style(style);
    }

    let val_text = "◖██◗ ";
    let vx = area.x + area.width - val_text.chars().count() as u16;

    let swatch_style = if let Some((r, g, b)) = color {
        Style::new().fg(Color::Rgb(r, g, b))
    } else {
        Style::new().fg(TEXT_DIM)
    };

    for (i, ch) in val_text.chars().enumerate() {
        let x = vx + i as u16;
        if x < area.x || x >= area.x + area.width {
            continue;
        }
        let cell = &mut buf[(x, ly)];
        cell.set_char(ch);
        cell.set_style(swatch_style);
    }

    if selected {
        let accent_style = Style::new().bg(ACCENT).fg(ACCENT);
        for y in area.y..area.y + area.height {
            buf.set_string(area.x, y, "▎", accent_style);
        }
    }
}

/// A toggle switch
fn render_toggle(
    buf: &mut Buffer,
    area: Rect,
    label: &str,
    on: bool,
    selected: bool,
    unselected_bg: Color,
) {
    if area.height < 1 || area.width < 4 {
        return;
    }

    let bg = if selected {
        SELECTED_BG
    } else {
        unselected_bg
    };
    buf.set_style(area, Style::new().bg(bg));

    let ly = area.y + area.height.saturating_sub(1) / 2;

    // Label
    let label_text = if selected {
        format!("> {}", label)
    } else {
        format!("  {}", label)
    };
    for (i, ch) in label_text.chars().enumerate() {
        let x = area.x + 1 + i as u16;
        if x >= area.x + area.width {
            break;
        }
        let cell = &mut buf[(x, ly)];
        cell.set_char(ch);
        cell.set_style(Style::new().fg(if selected { Color::White } else { TEXT }));
    }

    // Toggle indicator on the right
    let (indicator, color, bold) = if on {
        (" ● ON ", TOGGLE_ON, true)
    } else {
        (" ○ OFF ", TOGGLE_OFF, false)
    };
    let style = if bold {
        Style::new().fg(color).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(color)
    };
    let vx = area.x + area.width - indicator.len() as u16 - 1;
    for (i, ch) in indicator.chars().enumerate() {
        let x = vx + i as u16;
        if x < area.x || x >= area.x + area.width {
            continue;
        }
        let cell = &mut buf[(x, ly)];
        cell.set_char(ch);
        cell.set_style(style);
    }

    if selected {
        let accent_style = Style::new().bg(ACCENT).fg(ACCENT);
        for y in area.y..area.y + area.height {
            buf.set_string(area.x, y, "▎", accent_style);
        }
    }
}

fn render_button(
    buf: &mut Buffer,
    area: Rect,
    label: &str,
    selected: bool,
    status: &str,
    unselected_bg: Color,
) {
    let bg = if selected {
        SELECTED_BG
    } else if !status.is_empty() {
        OPTIMIZE_BG
    } else {
        unselected_bg
    };
    buf.set_style(area, Style::new().bg(bg));

    let ly = area.y + area.height.saturating_sub(1) / 2;
    let label_text = if selected {
        format!("> [ {} ]", label)
    } else {
        format!("  [ {} ]", label)
    };

    buf.set_string(
        area.x,
        ly,
        &label_text,
        Style::new()
            .fg(if selected { Color::White } else { TEXT })
            .add_modifier(Modifier::BOLD),
    );

    if !status.is_empty() {
        let status_text = format!("({}) ", status);
        let sx = area.x + area.width - status_text.len() as u16;
        if sx > area.x + label_text.len() as u16 {
            buf.set_string(
                sx,
                ly,
                &status_text,
                Style::new().fg(ACCENT).add_modifier(Modifier::ITALIC),
            );
        }
    }

    if selected {
        let accent_style = Style::new().bg(ACCENT).fg(ACCENT);
        for y in area.y..area.y + area.height {
            buf.set_string(area.x, y, "▎", accent_style);
        }
    }
}

// ── Main Entry Point ─────────────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq)]
struct OcrInferenceKey {
    model_tier: OcrModelTier,
    model_cache: Option<String>,
    det_model: Option<String>,
    cls_model: Option<String>,
    rec_model: Option<String>,
    dict: Option<String>,
    textline_orientation: bool,
    most_angle: bool,
    doc_orientation: bool,
    doc_orientation_model: Option<String>,
    doc_unwarping: bool,
    threads: usize,
    ocr_width: Option<u32>,
    megapixels: Option<u32>,
    max_side_len: u32,
    box_score_threshold: u32,
    box_threshold: u32,
    unclip_ratio: u32,
}

impl From<&RenderArgs> for OcrInferenceKey {
    fn from(args: &RenderArgs) -> Self {
        Self {
            model_tier: args.ocr_model_tier,
            model_cache: args.ocr_model_cache.clone(),
            det_model: args.ocr_det_model.clone(),
            cls_model: args.ocr_cls_model.clone(),
            rec_model: args.ocr_rec_model.clone(),
            dict: args.ocr_dict.clone(),
            textline_orientation: args.ocr_textline_orientation,
            most_angle: args.ocr_most_angle,
            doc_orientation: args.ocr_doc_orientation,
            doc_orientation_model: args.ocr_doc_orientation_model.clone(),
            doc_unwarping: args.ocr_doc_unwarping,
            threads: args.ocr_threads,
            ocr_width: if args.ocr_detection_budget().is_some() { None } else { args.ocr_width },
            megapixels: args.ocr_detection_budget().map(f32::to_bits),
            max_side_len: if args.ocr_detection_budget().is_some() { 0 } else { args.ocr_max_side_len },
            box_score_threshold: args.ocr_box_score_threshold.to_bits(),
            box_threshold: args.ocr_box_threshold.to_bits(),
            unclip_ratio: args.ocr_unclip_ratio.to_bits(),
        }
    }
}

pub async fn run(mut render_args: RenderArgs) -> Result<(), Box<dyn Error>> {
    if !cfg!(feature = "ocr") && render_args.ocr {
        return Err("OCR support is not compiled in; rebuild with `--features ocr`".into());
    }

    // Initialize logging to file
    let log_config = simplelog::ConfigBuilder::new()
        .set_time_format_rfc3339()
        .build();
    let _ = WriteLogger::init(
        LevelFilter::Info,
        log_config,
        OpenOptions::new()
            .create(true)
            .append(true)
            .open("img2irc.log")
            .unwrap(),
    );

    std::panic::set_hook(Box::new(|panic_info| {
        let message = if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
            *s
        } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
            &s[..]
        } else {
            "Unknown panic"
        };
        let location = panic_info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown location".to_string());
        log::error!("PANIC at {}: {}", location, message);
    }));

    log::info!("Starting img2irc TUI...");

    render_args.render = Render::Ansi;
    render_args.as_preview = true;

    let no_initial_image = render_args
        .image
        .as_deref()
        .map(|p| p.is_empty())
        .unwrap_or(true);
    let glyph_store = if no_initial_image {
        font::GlyphStore {
            glyphs: Vec::new(),
            groups: std::collections::HashMap::new(),
            selected: std::collections::HashSet::new(),
            metrics: (1, 1),
            float_metrics: (1.0, 1.0),
        }
    } else {
        font::glyph_store(
            &render_args.blocks,
            &render_args.exclude_range,
            &render_args.exclude,
            &render_args.include_range,
            render_args.include.as_ref(),
            &render_args.font,
            if (render_args.font_size - 0.0).abs() < f32::EPSILON {
                24.0
            } else {
                render_args.font_size
            },
            true,
            None,
        )
    };
    let initial_glyphs_loaded = !no_initial_image;

    let image = if let Some(path) = &render_args.image {
        match crate::load_image_from_url_or_path(path, &render_args, &glyph_store).await {
            Ok(img) => img,
            Err(e) => {
                log::info!("Failed to load image: {}", e);
                PhotonImage::new(vec![0; 4], 1, 1)
            }
        }
    } else {
        PhotonImage::new(vec![0; 4], 1, 1)
    };

    let arc_image = Arc::new(image);
    let arc_glyphs = Arc::new(glyph_store);

    let initial_flag = Arc::new(AtomicBool::new(false));
    let (tx_args, mut rx_args) = watch::channel((
        render_args.clone(),
        initial_flag.clone(),
        Instant::now(),
    ));
    let (tx_result, rx_result) = mpsc::channel(1);
    let rendering_flag = Arc::new(AtomicBool::new(true));
    let rendering_flag2 = rendering_flag.clone();
    let arc_glyphs_shared = arc_glyphs.clone();

    let initial_image_path = render_args.image.clone();
    let thread_render_args = render_args.clone();
    tokio::spawn(async move {
        let mut fast_glyphs_store: Option<Vec<draw::FastGlyph>> = None;
        let mut arc_image = arc_image;
        let mut current_image_path = initial_image_path;
        let mut image_revision = 0u64;
        let mut ocr_detection_cache: Option<(
            u64,
            OcrInferenceKey,
            Vec<crate::ocr::OcrDetection>,
        )> = None;

        // Use thread_render_args here instead of capturing render_args
        let mut arc_glyphs = arc_glyphs;
        let mut last_font = thread_render_args.font.clone();
        let mut last_font_size = thread_render_args.font_size;
        let mut last_blocks = thread_render_args.blocks.clone();
        let mut last_exclude_range = thread_render_args.exclude_range.clone();
        let mut last_exclude = thread_render_args.exclude.clone();
        let mut last_include_range = thread_render_args.include_range.clone();
        let mut last_include = thread_render_args.include.clone();
        let mut glyphs_loaded = initial_glyphs_loaded;
        let mut last_locked_ocr_detections: Option<Vec<crate::ocr::OcrDetection>> = None;
        let mut last_locked_base_image: Option<photon_rs::PhotonImage> = None;
        let mut last_ocr_grid: Option<(u32, u32)> = None;

        loop {
            // Signal rendering start
            rendering_flag2.store(true, Ordering::Relaxed);

            let (current_args, cancel_flag, render_requested_at) = {
                rx_args.borrow_and_update().clone()
            };

            if current_args
                .image
                .as_deref()
                .map(|p| p.is_empty())
                .unwrap_or(true)
            {
                if current_args.image != current_image_path {
                    current_image_path = current_args.image.clone();
                    image_revision = image_revision.wrapping_add(1);
                    ocr_detection_cache = None;
                    last_locked_ocr_detections = None;
                    last_locked_base_image = None;
                    last_ocr_grid = None;
                }
                rendering_flag2.store(false, Ordering::Relaxed);
                if rx_args.changed().await.is_err() {
                    break;
                }
                continue;
            }

            // Reload image if path changed
            if current_args.image != current_image_path {
                if let Some(path) = &current_args.image {
                    match crate::load_image_from_url_or_path(path, &current_args, &arc_glyphs).await
                    {
                        Ok(img) => {
                            arc_image = Arc::new(img);
                            current_image_path = Some(path.clone());
                            image_revision = image_revision.wrapping_add(1);
                            ocr_detection_cache = None;
                            last_locked_ocr_detections = None;
                            last_locked_base_image = None;
                            last_ocr_grid = None;
                        }
                        Err(e) => {
                            log::info!("Failed to reload image: {}", e);
                            // Keep old image
                        }
                    }
                }
            }

            // Reload font/glyphs if changed
            let mut effective_args = current_args.clone();
            let raw_image = (*arc_image).clone();
            let ocr_detections = if effective_args.ocr {
                if effective_args.ocr_lock && last_locked_ocr_detections.is_some() {
                    last_locked_ocr_detections.clone().unwrap()
                } else {
                    let inference_key = OcrInferenceKey::from(&effective_args);
                    let cached = ocr_detection_cache
                        .as_ref()
                        .filter(|(revision, key, _)| {
                            *revision == image_revision && *key == inference_key
                        })
                        .map(|(_, _, detections)| detections.clone());
                    let detection_result = if let Some(detections) = cached {
                        log::debug!("Reusing cached OCR detections");
                        Ok(detections)
                    } else {
                        let detection_image = Arc::new(raw_image.clone());
                        let detection_args = effective_args.clone();
                        match tokio::task::spawn_blocking(move || {
                            crate::ocr::detect_text(&detection_image, &detection_args)
                                .map_err(|error| error.to_string())
                        })
                        .await
                        {
                            Ok(result) => result,
                            Err(error) => Err(format!("OCR worker failed: {error}")),
                        }
                    };

                    match detection_result {
                        Ok(detections) => {
                            ocr_detection_cache = Some((
                                image_revision,
                                inference_key,
                                detections.clone(),
                            ));
                            crate::ocr::log_detections(&detections, &effective_args);
                            if !crate::ocr::has_renderable_detections(
                                &detections,
                                &effective_args,
                                raw_image.get_width(),
                                raw_image.get_height(),
                            ) {
                                log::info!("No renderable text detected via OCR. Falling back to standard render.");
                                effective_args.ocr = false;
                                Vec::new()
                            } else {
                                detections
                            }
                        }
                        Err(e) => {
                            log::error!("OCR text detection failed: {}", e);
                            Vec::new()
                        }
                    }
                }
            } else {
                last_locked_ocr_detections = None;
                last_locked_base_image = None;
                last_ocr_grid = None;
                Vec::new()
            };

            let base_image = if effective_args.ocr {
                if effective_args.ocr_lock && last_locked_base_image.is_some() {
                    last_locked_base_image.clone().unwrap()
                } else {
                    let cleaned = crate::ocr::remove_detected_text(&raw_image, &effective_args, &ocr_detections);
                    if effective_args.ocr_lock {
                        last_locked_ocr_detections = Some(ocr_detections.clone());
                        last_locked_base_image = Some(cleaned.clone());
                    }
                    cleaned
                }
            } else {
                last_locked_ocr_detections = None;
                last_locked_base_image = None;
                last_ocr_grid = None;
                raw_image.clone()
            };

            if effective_args.ocr && effective_args.ocr_auto_width && !ocr_detections.is_empty() {
                effective_args.width = None;
                effective_args.height = None;
            }
            if effective_args.font_size == 0.0 {
                crate::resolve_auto_font_size(&mut effective_args, &raw_image, &arc_glyphs);
            }

            if effective_args.font != last_font
                || (effective_args.font_size - last_font_size).abs() > ::std::f32::EPSILON
                || effective_args.blocks != last_blocks
                || effective_args.exclude_range != last_exclude_range
                || effective_args.exclude != last_exclude
                || effective_args.include_range != last_include_range
                || effective_args.include != last_include
                || !glyphs_loaded
            {
                let new_store = font::glyph_store(
                    &effective_args.blocks,
                    &effective_args.exclude_range,
                    &effective_args.exclude,
                    &effective_args.include_range,
                    effective_args.include.as_ref(),
                    &effective_args.font,
                    effective_args.font_size,
                    false,
                    Some(cancel_flag.clone()),
                );
                if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
                    continue;
                }
                arc_glyphs = Arc::new(new_store);
                fast_glyphs_store = None;
                last_font = effective_args.font.clone();
                last_font_size = effective_args.font_size;
                last_blocks = effective_args.blocks.clone();
                last_exclude_range = effective_args.exclude_range.clone();
                last_exclude = effective_args.exclude.clone();
                last_include_range = effective_args.include_range.clone();
                last_include = effective_args.include.clone();
                glyphs_loaded = true;
            }

            if fast_glyphs_store.is_none() {
                fast_glyphs_store = Some(draw::prepare_glyphs(&arc_glyphs, &effective_args));
            }

            let ocr_overlays = if effective_args.ocr {
                if let Some((width, height)) = last_ocr_grid.filter(|_| effective_args.ocr_lock) {
                    effective_args.width = Some(width);
                    effective_args.height = Some(height);
                    effective_args.scale = None;
                } else {
                    crate::ocr::choose_text_render_geometry(
                        &mut effective_args, &raw_image, &arc_glyphs, &ocr_detections,
                    );
                }
                let (pw, ph) = crate::effects::calculate_dimensions(
                    &effective_args, &arc_glyphs,
                    raw_image.get_width() as f32, raw_image.get_height() as f32,
                );
                let (gw, gh) = if effective_args.braille { (2, 4) } else { arc_glyphs.metrics };
                last_ocr_grid = Some((pw / gw.max(1) as u32, ph / gh.max(1) as u32));
                crate::ocr::detections_to_overlays(
                    &raw_image,
                    &base_image,
                    &effective_args,
                    &arc_glyphs,
                    &ocr_detections,
                )
            } else {
                Vec::new()
            };

            let render_image = crate::effects::apply_effects(&effective_args, &arc_glyphs, base_image);

            // Render!
            let glyphs = fast_glyphs_store.as_ref().unwrap();
            log::info!("Starting render optimization... ({} glyphs)", glyphs.len());
            let start = Instant::now();
            let mut output_args = effective_args.clone();
            output_args.pipeline.clear();
            output_args.rotate = 0.0;
            output_args.fliph = false;
            output_args.flipv = false;
            output_args.scale = None;
            output_args.brightness = 0.0;
            output_args.contrast = 0.0;
            output_args.gamma = 0.0;
            output_args.saturation = 0.0;
            output_args.hue = 0.0;
            output_args.invert = false;
            output_args.dither = 0;
            output_args.grayscale = false;
            output_args.nograyscale = false;
            output_args.pixelize = 0;
            output_args.box_blur = false;
            output_args.gaussian_blur = 0;

            let result = crate::render_output(
                output_args,
                &arc_glyphs,
                render_image,
                glyphs,
                Some(cancel_flag.clone()),
            );
            log::info!("Render finished in {:?}", start.elapsed());

            rendering_flag2.store(false, Ordering::Relaxed);

            // Only send if not cancelled (though render output might say cancelled)
            if !cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
                if tx_result
                    .send((
                        result,
                        arc_glyphs.clone(),
                        ocr_overlays,
                        render_requested_at,
                    ))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            if rx_args.changed().await.is_err() {
                break;
            }
        }
    });

    let mut app = App::new(render_args, tx_args, rx_result, initial_flag);
    app.is_rendering = rendering_flag;
    app.arc_glyphs = Some(arc_glyphs_shared);
    // Don't trigger update immediately here, we have initial args
    // However, we want to ensure any logic in update_render_args runs, so lets run flush
    // Actually just mark dirty and let loop handle it
    app.mark_args_dirty();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_app(&mut terminal, app).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        log::error!("TUI error: {:?}", err);
    }
    Ok(())
}

// ── App Loop ─────────────────────────────────────────────────────────────────
async fn run_app<B: Backend>(terminal: &mut Terminal<B>, mut app: App) -> io::Result<()> {
    loop {
        app.render_tick = app.render_tick.wrapping_add(1);
        app.poll_clipboard_paste();

        if let Some(mut rx) = app.rx_optimization.take() {
            while let Ok(update) = rx.try_recv() {
                match update {
                    OptimizationUpdate::Progress(p) => app.optimization_progress = p,
                    OptimizationUpdate::BestResult(args, score) => {
                        app.apply_settings(&args);
                        app.optimization_best_score = Some(score);
                        // Don't mark_args_dirty here — avoid triggering
                        // stale re-renders that overwrite the score display.
                        // A single final render is triggered on Finished.
                    }
                    OptimizationUpdate::Finished => {
                        app.is_optimizing = false;
                        app.optimization_progress = "Finished".to_string();
                        // Now trigger one clean render with the best settings
                        app.mark_args_dirty();
                    }
                }
            }
            app.rx_optimization = Some(rx);
        }

        while let Ok((result, glyphs, ocr_overlays, render_requested_at)) =
            app.rx_result.try_recv()
        {
            if result.content == "Cancelled" {
                continue;
            }
            if !app.render_args.ocr_lock {
                if app.ocr_overlay_count > 0 {
                    let keep = app
                        .render_args
                        .overlays
                        .len()
                        .saturating_sub(app.ocr_overlay_count);
                    app.render_args.overlays.truncate(keep);
                    app.ocr_overlay_count = 0;
                }
                if !ocr_overlays.is_empty() {
                    app.ocr_overlay_count = ocr_overlays.len();
                    app.render_args.overlays.extend(ocr_overlays);
                }
            }
            // Store the raw result (no overlays) and apply overlays as a fast post-process
            let render = match app.render_mode_idx {
                0 => Render::Irc,
                1 => Render::Ansi,
                _ => Render::Ansi24,
            };
            let overlayed = draw::apply_overlays_to_result(
                &result,
                &app.render_args.overlays,
                render,
                true, // as_preview
                app.grayscale_tolerance as u8,
            );
            app.cached_render = overlayed.content.clone();
            app.cached_result = Some(overlayed);
            app.base_render_result = Some(result);
            app.arc_glyphs = Some(glyphs);
            app.output_preview_dirty = true;
            // The next terminal draw immediately below presents this cached
            // output. Include debounce/queueing, font reload, effects, Score
            // Fix, geometry refinement, channel wait, and overlay application.
            app.last_render_latency = Some(render_requested_at.elapsed());
        }

        terminal
            .draw(|f| ui(f, &mut app))
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

        if crossterm::event::poll(Duration::from_millis(50))? {
            let event = event::read()?;
            if app.figlet_picker.is_some() {
                handle_figlet_picker(&mut app, event);
                continue;
            }
            if app.replace_colour_dialog.is_some() {
                handle_replace_colour(&mut app, event);
                continue;
            }

            if app.show_font_picker {
                match &event {
                    Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                        KeyCode::Esc => app.cancel_font_picker(),
                        KeyCode::Enter => app.commit_font_picker(),
                        KeyCode::Up => app.move_font_picker_selection(-1),
                        KeyCode::Down => app.move_font_picker_selection(1),
                        KeyCode::PageUp => app.move_font_picker_selection(-10),
                        KeyCode::PageDown => app.move_font_picker_selection(10),
                        KeyCode::Home => {
                            app.font_list_state.select(
                                (!app.matching_font_indices().is_empty()).then_some(0),
                            );
                        }
                        KeyCode::End => {
                            let count = app.matching_font_indices().len();
                            app.font_list_state.select(count.checked_sub(1));
                        }
                        KeyCode::Backspace => {
                            app.font_search.pop();
                            app.sync_font_picker_selection();
                        }
                        KeyCode::Char('u')
                            if key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            app.font_search.clear();
                            app.sync_font_picker_selection();
                        }
                        KeyCode::Char(c)
                            if !key.modifiers.contains(KeyModifiers::CONTROL)
                                && !key.modifiers.contains(KeyModifiers::ALT) =>
                        {
                            app.font_search.push(c);
                            app.sync_font_picker_selection();
                        }
                        _ => {}
                    },
                    Event::Mouse(mouse) => match mouse.kind {
                        MouseEventKind::ScrollUp => app.move_font_picker_selection(-1),
                        MouseEventKind::ScrollDown => app.move_font_picker_selection(1),
                        _ => {}
                    },
                    _ => {}
                }
                continue;
            }

            if let Event::Key(key) = &event {
                if key.kind == KeyEventKind::Press && key.code == KeyCode::Esc {
                    app.cancel_render();
                    app.args_dirty = false;
                }
            }

            if app.show_unicode_explorer {
                match event {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        match key.code {
                            KeyCode::Esc => app.show_unicode_explorer = false,
                            KeyCode::Tab | KeyCode::BackTab => {
                                app.unicode_path_focused = !app.unicode_path_focused;
                            }
                            KeyCode::Backspace if app.unicode_path_focused => {
                                app.unicode_range_path.pop();
                            }
                            KeyCode::Char(c)
                                if app.unicode_path_focused
                                    && !key
                                        .modifiers
                                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                            {
                                app.unicode_range_path.push(c);
                            }
                            KeyCode::Up if !app.unicode_path_focused => {
                                let curr = app.unicode_list_state.selected().unwrap_or(0);
                                app.unicode_list_state.select(Some(curr.saturating_sub(1)));
                                app.sync_unicode_range_path_to_selection();
                            }
                            KeyCode::Down if !app.unicode_path_focused => {
                                let curr = app.unicode_list_state.selected().unwrap_or(0);
                                if curr < UNICODE_BLOCKS.len() - 1 {
                                    app.unicode_list_state.select(Some(curr + 1));
                                }
                                app.sync_unicode_range_path_to_selection();
                            }
                            KeyCode::PageUp if !app.unicode_path_focused => {
                                let curr = app.unicode_list_state.selected().unwrap_or(0);
                                app.unicode_list_state.select(Some(curr.saturating_sub(10)));
                                app.sync_unicode_range_path_to_selection();
                            }
                            KeyCode::PageDown if !app.unicode_path_focused => {
                                let curr = app.unicode_list_state.selected().unwrap_or(0);
                                app.unicode_list_state
                                    .select(Some((curr + 10).min(UNICODE_BLOCKS.len() - 1)));
                                app.sync_unicode_range_path_to_selection();
                            }
                            KeyCode::Enter => app.add_selected_unicode_range(),
                            _ => {}
                        }
                    }
                    Event::Mouse(mouse) => {
                        let pos = ratatui::layout::Position::new(mouse.column, mouse.row);
                        match mouse.kind {
                            MouseEventKind::Down(MouseButton::Left)
                                if app.unicode_add_area.contains(pos) =>
                            {
                                app.add_selected_unicode_range();
                            }
                            MouseEventKind::Down(MouseButton::Left)
                                if app.unicode_cancel_area.contains(pos) =>
                            {
                                app.show_unicode_explorer = false;
                            }
                            MouseEventKind::Down(MouseButton::Left)
                                if app.unicode_path_area.contains(pos) =>
                            {
                                app.unicode_path_focused = true;
                            }
                            MouseEventKind::Down(MouseButton::Left)
                                if app.unicode_list_area.contains(pos) =>
                            {
                                app.unicode_path_focused = false;
                                let inner_row = mouse
                                    .row
                                    .saturating_sub(app.unicode_list_area.y)
                                    as usize;
                                let idx = app.unicode_list_state.offset() + inner_row;
                                if idx < UNICODE_BLOCKS.len() {
                                    app.unicode_list_state.select(Some(idx));
                                    app.sync_unicode_range_path_to_selection();
                                }
                            }
                            MouseEventKind::ScrollUp => {
                                let curr = app.unicode_list_state.selected().unwrap_or(0);
                                app.unicode_list_state.select(Some(curr.saturating_sub(1)));
                                app.sync_unicode_range_path_to_selection();
                            }
                            MouseEventKind::ScrollDown => {
                                let curr = app.unicode_list_state.selected().unwrap_or(0);
                                app.unicode_list_state.select(Some(
                                    (curr + 1).min(UNICODE_BLOCKS.len().saturating_sub(1)),
                                ));
                                app.sync_unicode_range_path_to_selection();
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
                continue;
            }

            if app.input_mode == InputMode::Explorer {
                if let Event::Key(key) = event {
                    if let Some(mut dialog) = app.explorer_dialog.take() {
                        if key.code == KeyCode::Tab {
                            if !matches!(dialog.mode, ExplorerMode::Open) {
                                dialog.focus_on_input = !dialog.focus_on_input;
                            }
                            app.explorer_dialog = Some(dialog);
                        } else if dialog.focus_on_input {
                            if key.code == KeyCode::Enter {
                                // Perform Save
                                let filename = dialog.name_input.lines()[0].clone();
                                let mut path = dialog.explorer.cwd().clone();
                                path.push(filename);
                                let path_str = path.to_string_lossy().to_string();
                                let mode = dialog.mode;
                                app.handle_explorer_result(mode, path_str);
                                if app.input_mode != InputMode::MetadataPrompt {
                                    app.input_mode = InputMode::Normal;
                                }
                                app.explorer_dialog = None;
                            } else if key.code == KeyCode::Esc {
                                app.input_mode = InputMode::Normal;
                                app.explorer_dialog = None;
                            } else {
                                dialog.name_input.input(key);
                                app.explorer_dialog = Some(dialog);
                            }
                        } else {
                            // Focus on Explorer
                            if (key.code == KeyCode::Enter || key.code == KeyCode::Char('l'))
                                && dialog.explorer.current().is_file()
                            {
                                if matches!(dialog.mode, ExplorerMode::Open) {
                                    let path = dialog
                                        .explorer
                                        .current()
                                        .path()
                                        .to_string_lossy()
                                        .to_string();
                                    let mode = dialog.mode;
                                    app.handle_explorer_result(mode, path);
                                    if app.input_mode != InputMode::MetadataPrompt {
                                        app.input_mode = InputMode::Normal;
                                    }
                                    app.explorer_dialog = None;
                                } else {
                                    // In Save mode, enter on file copies name to input
                                    let name = dialog.explorer.current().name().replace("/", "");
                                    dialog.name_input = TextArea::new(vec![name]);
                                    dialog.name_input.set_block(
                                        Block::default()
                                            .borders(Borders::ALL)
                                            .title(" Filename (Enter to Save) "),
                                    );
                                    dialog.name_input.set_cursor_line_style(Style::default());
                                    dialog.focus_on_input = true;
                                    app.explorer_dialog = Some(dialog);
                                }
                            } else if key.code == KeyCode::Char('q') || key.code == KeyCode::Esc {
                                app.input_mode = InputMode::Normal;
                                app.explorer_dialog = None;
                            } else {
                                // Let explorer handle the key
                                let _ = dialog.explorer.handle(&event);
                                app.explorer_dialog = Some(dialog);
                            }
                        }
                    } else {
                        app.input_mode = InputMode::Normal;
                    }
                } else if let Event::Mouse(mouse) = event {
                    if let Some(mut dialog) = app.explorer_dialog.take() {
                        let is_click = mouse.kind == MouseEventKind::Down(MouseButton::Left);
                        let is_scroll_up = mouse.kind == MouseEventKind::ScrollUp;
                        let is_scroll_down = mouse.kind == MouseEventKind::ScrollDown;

                        if mouse.column >= app.explorer_area.x
                            && mouse.column < app.explorer_area.x + app.explorer_area.width
                            && mouse.row >= app.explorer_area.y + 1
                            && mouse.row
                                < app.explorer_area.y + app.explorer_area.height.saturating_sub(1)
                        {
                            if is_scroll_up {
                                let _ = dialog.explorer.handle(explorer::Input::Up);
                            } else if is_scroll_down {
                                let _ = dialog.explorer.handle(explorer::Input::Down);
                            } else if is_click {
                                let offset = dialog.explorer.list_offset();
                                let clicked_idx =
                                    offset + (mouse.row - (app.explorer_area.y + 1)) as usize;
                                if clicked_idx < dialog.explorer.files().len() {
                                    if dialog.explorer.selected_idx() == clicked_idx {
                                        // Double click equivalent
                                        if dialog.explorer.current().is_file() {
                                            if matches!(dialog.mode, ExplorerMode::Open) {
                                                let path = dialog
                                                    .explorer
                                                    .current()
                                                    .path()
                                                    .to_string_lossy()
                                                    .to_string();
                                                app.handle_explorer_result(dialog.mode, path);
                                                if app.input_mode != InputMode::MetadataPrompt {
                                                    app.input_mode = InputMode::Normal;
                                                }
                                                app.explorer_dialog = None;
                                                continue;
                                            } else {
                                                let name = dialog
                                                    .explorer
                                                    .current()
                                                    .name()
                                                    .replace("/", "");
                                                dialog.name_input = TextArea::new(vec![name]);
                                                dialog.name_input.set_block(
                                                    ratatui::widgets::Block::default()
                                                        .borders(ratatui::widgets::Borders::ALL)
                                                        .title(" Filename (Enter to Save) "),
                                                );
                                                dialog.name_input.set_cursor_line_style(
                                                    ratatui::style::Style::default(),
                                                );
                                                dialog.focus_on_input = true;
                                            }
                                        } else if dialog.explorer.current().is_dir() {
                                            let _ = dialog.explorer.handle(explorer::Input::Right);
                                        }
                                    } else {
                                        dialog.explorer.set_selected_idx(clicked_idx);
                                    }
                                }
                            }
                            app.explorer_dialog = Some(dialog);
                        } else if is_click {
                            // Clicked outside the left pane, let's just restore dialog and do nothing, or maybe unfocus
                            app.explorer_dialog = Some(dialog);
                        } else {
                            app.explorer_dialog = Some(dialog);
                        }
                    } else {
                        app.input_mode = InputMode::Normal;
                    }
                }
                continue; // Consume event
            }

            // --- Value Edit Mode: direct numeric input for sliders ---
            if app.input_mode == InputMode::ValueEdit {
                if let Event::Key(key) = event {
                    if key.kind == KeyEventKind::Press {
                        match key.code {
                            KeyCode::Enter => {
                                // Confirm: parse and apply the value
                                if let Ok(val) = app.value_edit_buffer.parse::<f32>() {
                                    let controls = app.tab_controls(app.current_tab);
                                    if let Some(ctrl) = controls.get(app.selected_control) {
                                        if let ControlKind::Slider { min, max, .. } = &ctrl.kind {
                                            let raw_value = if matches!(ctrl.id, ControlId::FontSize | ControlId::OcrMegapixels) {
                                                (val * 10.0).round() as i32
                                            } else {
                                                val.round() as i32
                                            };
                                            let clamped = raw_value.clamp(*min, *max);
                                            let sc = app.selected_control;
                                            set_control_value(&mut app, sc, clamped);
                                        }
                                    }
                                }
                                app.input_mode = InputMode::Normal;
                                app.value_edit_buffer.clear();
                            }
                            KeyCode::Esc => {
                                // Cancel
                                app.input_mode = InputMode::Normal;
                                app.value_edit_buffer.clear();
                            }
                            KeyCode::Backspace => {
                                app.value_edit_buffer.pop();
                            }
                            KeyCode::Char(c) if c.is_ascii_digit() || c == '-' || c == '.' => {
                                app.value_edit_buffer.push(c);
                            }
                            _ => {}
                        }
                    }
                }
                continue;
            }

            // --- Text Edit Mode: for arbitrary text overlays ---
            if app.input_mode == InputMode::TextEdit {
                if let Event::Key(key) = event {
                    if key.kind == KeyEventKind::Press {
                        match key.code {
                            KeyCode::Tab => {
                                finish_text_edit(&mut app);
                                app.cycle_text_field_focus(1);
                            }
                            KeyCode::BackTab => {
                                finish_text_edit(&mut app);
                                app.cycle_text_field_focus(-1);
                            }
                            KeyCode::Enter => {
                                if key.modifiers.contains(KeyModifiers::CONTROL) {
                                    // Ctrl+Enter finishes editing; plain Enter is a newline.
                                    finish_text_edit(&mut app);
                                } else {
                                    app.value_edit_buffer.push('\n');
                                    apply_text_edit_buffer(&mut app);
                                }
                            }
                            KeyCode::Esc => {
                                // Keep the live edit and leave the text box.
                                finish_text_edit(&mut app);
                            }
                            KeyCode::Backspace => {
                                app.value_edit_buffer.pop();
                                // Update overlay text and reapply cheaply (no full re-render)
                                apply_text_edit_buffer(&mut app);
                            }
                            KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                if let Some(sel) = app.text_selected {
                                    if sel > 0 && sel <= app.render_args.overlays.len() {
                                        app.render_args.overlays[sel - 1].bold =
                                            !app.render_args.overlays[sel - 1].bold;
                                        app.reapply_overlays();
                                    }
                                }
                            }
                            KeyCode::Char('i') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                if let Some(sel) = app.text_selected {
                                    if sel > 0 && sel <= app.render_args.overlays.len() {
                                        app.render_args.overlays[sel - 1].italic =
                                            !app.render_args.overlays[sel - 1].italic;
                                        app.reapply_overlays();
                                    }
                                }
                            }
                            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                if let Some(sel) = app.text_selected {
                                    if sel > 0 && sel <= app.render_args.overlays.len() {
                                        app.render_args.overlays[sel - 1].underline =
                                            !app.render_args.overlays[sel - 1].underline;
                                        app.reapply_overlays();
                                    }
                                }
                            }
                            KeyCode::Char(c) => {
                                app.value_edit_buffer.push(c);
                                // Update overlay text and reapply cheaply (no full re-render)
                                apply_text_edit_buffer(&mut app);
                            }
                            _ => {}
                        }
                    }
                    continue;
                }
            }

            // --- Metadata Prompt Mode ---
            if app.input_mode == InputMode::MetadataPrompt {
                if let Event::Key(key) = event {
                    if key.kind == KeyEventKind::Press {
                        match key.code {
                            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                                // Load metadata
                                if let Some(settings) = app.metadata_prompt_settings.take() {
                                    app.apply_settings(&settings);
                                    app.image_path = app.metadata_prompt_path.clone();
                                    app.render_args.image = Some(app.metadata_prompt_path.clone());
                                    app.mark_args_dirty();
                                }
                                app.input_mode = InputMode::Normal;
                            }
                            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                                // Just load image
                                let png_path = app.metadata_prompt_png_path.clone();
                                app.image_path = png_path.clone();
                                app.render_args.image = Some(png_path);
                                app.mark_args_dirty();
                                app.input_mode = InputMode::Normal;
                                app.metadata_prompt_settings = None;
                            }
                            _ => {}
                        }
                    }
                }
                continue;
            }

            // --- Auto Optimize Mode ---
            if app.input_mode == InputMode::AutoOptimize {
                let mut action = AutoOptimizeMouseAction::None;
                match event {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        if let Some(state) = app.auto_optimize_state.as_mut() {
                            match key.code {
                                KeyCode::Esc => {
                                    action = AutoOptimizeMouseAction::Cancel;
                                }
                                KeyCode::Enter => {
                                    action = AutoOptimizeMouseAction::Start;
                                }
                                KeyCode::Up => {
                                    if state.focused_idx == 24 {
                                        state.focused_idx = 21;
                                    } else if state.focused_idx == 25 {
                                        state.focused_idx = 22;
                                    } else if state.focused_idx == 26 {
                                        state.focused_idx = 24;
                                    } else if state.focused_idx >= 3 {
                                        state.focused_idx -= 3;
                                    } else {
                                        state.focused_idx = 26;
                                    }
                                }
                                KeyCode::Down => {
                                    if state.focused_idx == 24 || state.focused_idx == 25 {
                                        state.focused_idx = 26;
                                    } else if state.focused_idx == 26 {
                                        state.focused_idx = 0;
                                    } else if state.focused_idx < 21 {
                                        state.focused_idx += 3;
                                    } else {
                                        let col = state.focused_idx % 3;
                                        if col == 0 {
                                            state.focused_idx = 24;
                                        } else if col == 1 {
                                            state.focused_idx = 25;
                                        } else {
                                            state.focused_idx = 26;
                                        }
                                    }
                                }
                                KeyCode::Left => {
                                    if state.focused_idx == 26 {
                                        state.batch_size =
                                            state.batch_size.saturating_sub(1).max(1);
                                    } else if state.focused_idx > 0 {
                                        state.focused_idx -= 1;
                                    } else {
                                        state.focused_idx = 26;
                                    }
                                }
                                KeyCode::Right => {
                                    if state.focused_idx == 26 {
                                        state.batch_size =
                                            state.batch_size.saturating_add(1).min(100);
                                    } else if state.focused_idx < 26 {
                                        state.focused_idx += 1;
                                    } else {
                                        state.focused_idx = 0;
                                    }
                                }
                                KeyCode::BackTab => {
                                    if state.focused_idx > 0 {
                                        state.focused_idx -= 1;
                                    } else {
                                        state.focused_idx = 26;
                                    }
                                }
                                KeyCode::Tab => {
                                    if state.focused_idx < 26 {
                                        state.focused_idx += 1;
                                    } else {
                                        state.focused_idx = 0;
                                    }
                                }
                                KeyCode::Char(' ') if state.focused_idx == 24 => {
                                    state.optimize_shortest_longest_line =
                                        !state.optimize_shortest_longest_line;
                                }
                                KeyCode::Char(' ') if state.focused_idx == 25 => {
                                    state.randomize = !state.randomize;
                                }
                                KeyCode::Char(' ')
                                    if state.focused_idx < 24
                                        && key.modifiers.contains(KeyModifiers::CONTROL) =>
                                {
                                    let row = state.focused_idx / 3;
                                    state.enabled_rows[row] = !state.enabled_rows[row];
                                }
                                KeyCode::Char('-') | KeyCode::Char('_')
                                    if state.focused_idx == 26 =>
                                {
                                    state.batch_size = state.batch_size.saturating_sub(1).max(1);
                                }
                                KeyCode::Char('+') | KeyCode::Char('=')
                                    if state.focused_idx == 26 =>
                                {
                                    state.batch_size = state.batch_size.saturating_add(1).min(100);
                                }
                                _ => {
                                    if state.focused_idx < 24 {
                                        state.inputs[state.focused_idx].input(key);
                                    }
                                }
                            }
                        }
                    }
                    Event::Mouse(mouse) => {
                        let current_values = [
                            app.width,
                            app.trim_top,
                            app.trim_right,
                            app.trim_bottom,
                            app.trim_left,
                            app.font_size.round() as i32,
                            app.scalex,
                            app.scaley,
                        ];
                        if let Some(state) = app.auto_optimize_state.as_mut() {
                            action = handle_auto_optimize_mouse(state, mouse, &current_values);
                        }
                    }
                    _ => {}
                }
                match action {
                    AutoOptimizeMouseAction::Start => {
                        app.input_mode = InputMode::Normal;
                        app.start_auto_optimization();
                    }
                    AutoOptimizeMouseAction::Cancel => {
                        app.input_mode = InputMode::Normal;
                    }
                    AutoOptimizeMouseAction::None => {}
                }
                continue;
            }

            match event {
                Event::Mouse(mouse) => {
                    if app.current_tab == 3
                        && app.input_mode == InputMode::TextEdit
                        && mouse.kind == MouseEventKind::Down(MouseButton::Left)
                    {
                        let mouse_pos = ratatui::layout::Position {
                            x: mouse.column,
                            y: mouse.row,
                        };
                        if app.output_area.contains(mouse_pos) {
                            let over_edited_box = output_grid_position(
                                &app,
                                mouse.column,
                                mouse.row,
                            )
                            .is_some_and(|(cell_x, cell_y)| {
                                app.text_selected
                                    .and_then(|sel| sel.checked_sub(1))
                                    .and_then(|idx| app.render_args.overlays.get(idx))
                                    .is_some_and(|overlay| {
                                        text_overlay_contains(overlay, cell_x, cell_y)
                                            || text_overlay_resize_hotspot(overlay, cell_x, cell_y)
                                    })
                            });
                            if !over_edited_box {
                                finish_text_edit(&mut app);
                                app.last_text_click = None;
                                app.text_drag_mode = TextDragMode::None;
                                app.text_drag_start = None;
                                continue;
                            }
                        } else {
                            let is_text_control = app.color_picker_area.contains(mouse_pos)
                                || app
                                    .text_field_areas
                                    .iter()
                                    .any(|area| area.contains(mouse_pos))
                                || app
                                    .text_btn_areas
                                    .iter()
                                    .any(|area| area.contains(mouse_pos));
                            if !is_text_control {
                                finish_text_edit(&mut app);
                            }
                        }
                    }

                    if app.current_tab == 3 {
                        let cp_rect = app.color_picker_area;
                        let mut handled = false;

                        if let Some(cp_state) = &mut app.color_picker_state {
                            if mouse.kind
                                == crossterm::event::MouseEventKind::Down(MouseButton::Left)
                            {
                                handled = cp_state.handle_mouse_down(
                                    mouse.column,
                                    mouse.row,
                                    cp_rect,
                                    None,
                                );
                            } else if mouse.kind
                                == crossterm::event::MouseEventKind::Down(MouseButton::Middle)
                            {
                                handled = cp_state.handle_mouse_down(
                                    mouse.column,
                                    mouse.row,
                                    cp_rect,
                                    Some(true),
                                );
                            } else if mouse.kind
                                == crossterm::event::MouseEventKind::Down(MouseButton::Right)
                            {
                                handled = cp_state.handle_mouse_down(
                                    mouse.column,
                                    mouse.row,
                                    cp_rect,
                                    Some(false),
                                );
                            } else if mouse.kind
                                == crossterm::event::MouseEventKind::Drag(MouseButton::Left)
                            {
                                handled =
                                    cp_state.handle_mouse_drag(mouse.column, mouse.row, cp_rect);
                            } else if mouse.kind
                                == crossterm::event::MouseEventKind::Up(MouseButton::Left)
                            {
                                handled = cp_state.is_dragging_main || cp_state.is_dragging_hue;
                                cp_state.is_dragging_main = false;
                                cp_state.is_dragging_hue = false;
                            }

                            if handled {
                                if let Some(sel) = app.text_selected {
                                    if sel > 0 && sel <= app.render_args.overlays.len() {
                                        let ov = &mut app.render_args.overlays[sel - 1];
                                        if cp_state.editing_fg {
                                            if let Some(fg_idx) = cp_state.fg_idx {
                                                ov.fg = Some(crate::args::ColorSpec::Index(
                                                    fg_idx as u8,
                                                ));
                                            } else {
                                                let rgb = cp_state.fg_rgb;
                                                ov.fg = Some(crate::args::ColorSpec::Rgb([
                                                    rgb.0, rgb.1, rgb.2,
                                                ]));
                                            }
                                        } else {
                                            if let Some(bg_idx) = cp_state.bg_idx {
                                                ov.bg = Some(crate::args::ColorSpec::Index(
                                                    bg_idx as u8,
                                                ));
                                            } else {
                                                if let Some(rgb) = cp_state.bg_rgb {
                                                    ov.bg = Some(crate::args::ColorSpec::Rgb([
                                                        rgb.0, rgb.1, rgb.2,
                                                    ]));
                                                }
                                            }
                                        }
                                        app.reapply_overlays();
                                    }
                                }
                                continue;
                            }
                        }
                    }

                    match mouse.kind {
                        MouseEventKind::ScrollUp => {
                            // Check if over output area
                            if mouse.column >= app.output_area.x
                                && mouse.column < app.output_area.x + app.output_area.width
                                && mouse.row >= app.output_area.y
                                && mouse.row <= app.output_area.y + app.output_area.height
                            {
                                let line_count = app.cached_render.lines().count() as u16;
                                if line_count > app.output_area.height {
                                    app.scroll = app.scroll.saturating_sub(3);
                                }
                                continue;
                            }

                            // Replaced with fallback scrolling

                            if mouse.column >= app.tree_area.x
                                && mouse.column < app.tree_area.x + app.tree_area.width
                                && mouse.row >= app.tree_area.y
                                && mouse.row < app.tree_area.y + app.tree_area.height
                            {
                                app.current_tab = 2;
                                app.selected_control = 0;
                                app.tree_state.select_prev();
                                continue;
                            }
                            if mouse.column >= app.glyph_list_area.x
                                && mouse.column < app.glyph_list_area.x + app.glyph_list_area.width
                                && mouse.row >= app.glyph_list_area.y
                                && mouse.row < app.glyph_list_area.y + app.glyph_list_area.height
                            {
                                app.current_tab = 2;
                                app.selected_control = 1;
                                app.move_selected_glyph(-1);
                                continue;
                            }

                            for i in 0..app.control_areas.len() {
                                let area = app.control_areas[i];
                                if area.width == 0 {
                                    continue;
                                }
                                if mouse.column >= area.x
                                    && mouse.column < area.x + area.width
                                    && mouse.row >= area.y
                                    && mouse.row < area.y + area.height
                                {
                                    app.current_tab = 0;
                                    app.selected_control = i;
                                    if let Some(ctrl) = app.tab_controls(0).get(i) {
                                        if let ControlKind::Slider { .. } = ctrl.kind {
                                            let delta =
                                                if mouse.modifiers.contains(KeyModifiers::SHIFT) {
                                                    10
                                                } else {
                                                    1
                                                };
                                            adjust_control(&mut app, delta);
                                            continue;
                                        }
                                    }
                                }
                            }

                            if app.current_tab == 2 {
                                match app.selected_control {
                                    0 => app.tree_state.select_prev(),
                                    1 => app.move_selected_glyph(-1),
                                    _ => {}
                                }
                            } else if app.current_tab == 0 && app.selected_control > 0 {
                                app.selected_control -= 1;
                            } else if app.current_tab == 1 && app.pipeline_expanded.is_some() {
                                move_pipeline_library_selection(&mut app, -1);
                            } else if app.current_tab == 1 && app.selected_control > 0 {
                                app.selected_control -= 1;
                                if let Some(node) = app.pipeline_nodes.get(app.selected_control) {
                                    app.pipeline_selected = Some(node.id);
                                } else {
                                    app.pipeline_selected = None;
                                }
                            }
                        }
                        MouseEventKind::ScrollDown => {
                            // Check if over output area
                            if mouse.column >= app.output_area.x
                                && mouse.column < app.output_area.x + app.output_area.width
                                && mouse.row >= app.output_area.y
                                && mouse.row <= app.output_area.y + app.output_area.height
                            {
                                let line_count = app.cached_render.lines().count() as u16;
                                if line_count > app.output_area.height {
                                    app.scroll = app.scroll.saturating_add(3);
                                }
                                continue;
                            }

                            // Replaced with fallback scrolling

                            if mouse.column >= app.tree_area.x
                                && mouse.column < app.tree_area.x + app.tree_area.width
                                && mouse.row >= app.tree_area.y
                                && mouse.row < app.tree_area.y + app.tree_area.height
                            {
                                app.current_tab = 2;
                                app.selected_control = 0;
                                app.tree_state.select_next();
                                continue;
                            }
                            if mouse.column >= app.glyph_list_area.x
                                && mouse.column < app.glyph_list_area.x + app.glyph_list_area.width
                                && mouse.row >= app.glyph_list_area.y
                                && mouse.row < app.glyph_list_area.y + app.glyph_list_area.height
                            {
                                app.current_tab = 2;
                                app.selected_control = 1;
                                app.move_selected_glyph(1);
                                continue;
                            }

                            for i in 0..app.control_areas.len() {
                                let area = app.control_areas[i];
                                if area.width == 0 {
                                    continue;
                                }
                                if mouse.column >= area.x
                                    && mouse.column < area.x + area.width
                                    && mouse.row >= area.y
                                    && mouse.row < area.y + area.height
                                {
                                    app.current_tab = 0;
                                    app.selected_control = i;
                                    if let Some(ctrl) = app.tab_controls(0).get(i) {
                                        if let ControlKind::Slider { .. } = ctrl.kind {
                                            let delta =
                                                if mouse.modifiers.contains(KeyModifiers::SHIFT) {
                                                    -10
                                                } else {
                                                    -1
                                                };
                                            adjust_control(&mut app, delta);
                                            continue;
                                        }
                                    }
                                }
                            }

                            if app.current_tab == 2 {
                                match app.selected_control {
                                    0 => app.tree_state.select_next(),
                                    1 => app.move_selected_glyph(1),
                                    _ => {}
                                }
                            } else if app.current_tab == 0 {
                                app.selected_control = (app.selected_control + 1)
                                    .min(app.control_count().saturating_sub(1));
                            } else if app.current_tab == 1 && app.pipeline_expanded.is_some() {
                                move_pipeline_library_selection(&mut app, 1);
                            } else if app.current_tab == 1 {
                                let max_idx = app.pipeline_nodes.len();
                                app.selected_control = (app.selected_control + 1).min(max_idx);
                                if let Some(node) = app.pipeline_nodes.get(app.selected_control) {
                                    app.pipeline_selected = Some(node.id);
                                } else {
                                    app.pipeline_selected = None;
                                }
                            }
                        }
                        MouseEventKind::Moved => {
                            let hovered_cell = (app.current_tab == 3)
                                .then(|| output_grid_position(&app, mouse.column, mouse.row))
                                .flatten();
                            app.text_hovered = hovered_cell.and_then(|(cell_x, cell_y)| {
                                app.render_args
                                    .overlays
                                    .iter()
                                    .enumerate()
                                    .rev()
                                    .find(|(_, overlay)| {
                                        text_overlay_hover_contains(overlay, cell_x, cell_y)
                                    })
                                    .map(|(idx, _)| idx)
                            });
                            app.text_resize_hover = hovered_cell.and_then(|(cell_x, cell_y)| {
                                app.render_args
                                    .overlays
                                    .iter()
                                    .enumerate()
                                    .rev()
                                    .find(|(_, overlay)| {
                                        text_overlay_resize_hotspot(overlay, cell_x, cell_y)
                                    })
                                    .map(|(idx, _)| idx)
                            });
                        }
                        MouseEventKind::Down(MouseButton::Left) => {
                            app.dragged_control = None;
                            app.dragging_splitter = None;

                            for splitter in app.splitter_areas.iter().copied() {
                                if mouse.column >= splitter.area.x
                                    && mouse.column < splitter.area.x + splitter.area.width
                                    && mouse.row >= splitter.area.y
                                    && mouse.row < splitter.area.y + splitter.area.height
                                {
                                    app.dragging_splitter = Some(splitter.kind);
                                    continue;
                                }
                            }
                            if app.dragging_splitter.is_some() {
                                continue;
                            }

                            // Check if over output area for eyedropper
                            if mouse.column >= app.output_area.x
                                && mouse.column < app.output_area.x + app.output_area.width
                                && mouse.row >= app.output_area.y
                                && mouse.row < app.output_area.y + app.output_area.height
                            {
                                if app.current_tab == 3 {
                                    let inner_height = app.output_area.height.saturating_sub(2);
                                    let line_count = app.cached_render.lines().count() as u16;
                                    let pad_y = if line_count < inner_height {
                                        inner_height.saturating_sub(line_count).saturating_div(2)
                                    } else {
                                        0
                                    };
                                    if mouse.row >= app.output_area.y + 1 + pad_y {
                                        let content_y =
                                            (mouse.row - (app.output_area.y + 1 + pad_y)) as usize
                                                + app.scroll as usize;
                                        if let Some(res) = &app.cached_result {
                                            if content_y < res.grid.len() {
                                                let row = &res.grid[content_y];
                                                let inner_width =
                                                    app.output_area.width.saturating_sub(2);
                                                let row_len = row.len() as u16;
                                                let pad_x = if inner_width > row_len {
                                                    (inner_width - row_len) / 2
                                                } else {
                                                    0
                                                };
                                                let start_x = app.output_area.x + 1 + pad_x;
                                                if mouse.column >= start_x
                                                    && mouse.column < start_x + row_len
                                                {
                                                    let content_x =
                                                        (mouse.column - start_x) as usize;
                                                    let cell_x = content_x as i32;
                                                    let cell_y = content_y as i32;

                                                    let resize_found = app
                                                        .render_args
                                                        .overlays
                                                        .iter()
                                                        .enumerate()
                                                        .rev()
                                                        .find(|(_, overlay)| {
                                                            text_overlay_resize_hotspot(
                                                                overlay, cell_x, cell_y,
                                                            )
                                                        })
                                                        .map(|(idx, _)| idx);
                                                    let found = resize_found.or_else(|| {
                                                        app.render_args
                                                            .overlays
                                                            .iter()
                                                            .enumerate()
                                                            .rev()
                                                            .find(|(_, overlay)| {
                                                                text_overlay_contains(
                                                                    overlay, cell_x, cell_y,
                                                                )
                                                            })
                                                            .map(|(idx, _)| idx)
                                                    });

                                                    let now = std::time::Instant::now();
                                                    let is_double = app.last_text_click.map_or(
                                                        false,
                                                        |(t, lx, ly)| {
                                                            now.duration_since(t).as_millis() < 500
                                                                && lx == cell_x
                                                                && ly == cell_y
                                                        },
                                                    );

                                                    if let Some(idx) = found {
                                                        let (overlay_x, overlay_y) = {
                                                            let overlay =
                                                                &app.render_args.overlays[idx];
                                                            (overlay.x, overlay.y)
                                                        };
                                                        let is_resize_area =
                                                            resize_found == Some(idx);
                                                        app.text_cursor_anchor = None;
                                                        app.text_hovered = Some(idx);

                                                        if is_resize_area {
                                                            if app.input_mode == InputMode::TextEdit {
                                                                finish_text_edit(&mut app);
                                                            }
                                                            app.text_selected = Some(idx + 1);
                                                            app.text_drag_mode =
                                                                TextDragMode::Sizing;
                                                            app.text_drag_start =
                                                                Some((overlay_x, overlay_y));
                                                            app.render_args.overlays[idx]
                                                                .auto_grow = false;
                                                            app.text_resize_hover = Some(idx);
                                                            app.last_text_click =
                                                                Some((now, cell_x, cell_y));
                                                        } else if is_double {
                                                            app.text_selected = Some(idx + 1);
                                                            app.value_edit_buffer =
                                                                app.render_args.overlays[idx]
                                                                    .text
                                                                    .clone();
                                                            app.input_mode = InputMode::TextEdit;
                                                            app.text_drag_mode = TextDragMode::None;
                                                            app.last_text_click = None;
                                                        } else {
                                                            if app.input_mode == InputMode::TextEdit {
                                                                finish_text_edit(&mut app);
                                                            }
                                                            app.text_selected = Some(idx + 1);
                                                            app.text_drag_mode =
                                                                TextDragMode::Moving;
                                                            app.text_drag_start = Some((
                                                                cell_x - overlay_x,
                                                                cell_y - overlay_y,
                                                            ));
                                                            app.last_text_click =
                                                                Some((now, cell_x, cell_y));
                                                        }
                                                    } else {
                                                        // Require double-click to create new overlay
                                                        if is_double {
                                                            app.render_args.overlays.push(
                                                                new_text_overlay(
                                                                    app.render_mode_idx,
                                                                    cell_x,
                                                                    cell_y,
                                                                    false,
                                                                ),
                                                            );
                                                            app.text_selected = Some(
                                                                app.render_args.overlays.len(),
                                                            );
                                                            app.reapply_overlays();
                                                            app.text_drag_mode =
                                                                TextDragMode::Sizing;
                                                            app.text_drag_start =
                                                                Some((cell_x, cell_y));
                                                            app.text_cursor_anchor = None;
                                                            app.last_text_click = None;
                                                        } else {
                                                            app.last_text_click =
                                                                Some((now, cell_x, cell_y));
                                                            app.text_selected = None;
                                                            app.text_cursor_anchor =
                                                                Some((cell_x, cell_y));
                                                            app.text_field_focus =
                                                                TextFieldFocus::TextInput;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    continue;
                                } else if app.get_current_control_id() == ControlId::Eyedropper {
                                    if let Some(res) = &app.cached_result {
                                        let inner_height = app.output_area.height.saturating_sub(2);
                                        let line_count = app.cached_render.lines().count() as u16;
                                        let pad_y = if line_count < inner_height {
                                            inner_height
                                                .saturating_sub(line_count)
                                                .saturating_div(2)
                                        } else {
                                            0
                                        };
                                        if mouse.row >= app.output_area.y + 1 + pad_y {
                                            let content_y = (mouse.row
                                                - (app.output_area.y + 1 + pad_y))
                                                as usize
                                                + app.scroll as usize;
                                            if content_y < res.grid.len() {
                                                let row = &res.grid[content_y];
                                                let inner_width =
                                                    app.output_area.width.saturating_sub(2);
                                                let row_len = row.len() as u16;
                                                let pad_x = if inner_width > row_len {
                                                    (inner_width - row_len) / 2
                                                } else {
                                                    0
                                                };
                                                let start_x = app.output_area.x + 1 + pad_x;
                                                if mouse.column >= start_x
                                                    && mouse.column < start_x + row_len
                                                {
                                                    let content_x =
                                                        (mouse.column - start_x) as usize;
                                                    let cell = &row[content_x];
                                                    let mut picked_color = cell.fg;
                                                    if let Some(bg) = cell.bg {
                                                        if let Some(g) = &app.arc_glyphs {
                                                            if let Some((_, bitmap, _)) = g
                                                                .glyphs
                                                                .iter()
                                                                .find(|(c, _, _)| *c == cell.char)
                                                            {
                                                                let total_pixels =
                                                                    g.metrics.0 * g.metrics.1;
                                                                let set_bits: usize = bitmap
                                                                    .iter()
                                                                    .map(|row: &Vec<u8>| {
                                                                        row.iter()
                                                                            .filter(|&&b| b > 0)
                                                                            .count()
                                                                    })
                                                                    .sum();
                                                                let is_mostly_bg = if cell.inverted
                                                                {
                                                                    set_bits > total_pixels / 2
                                                                } else {
                                                                    set_bits < total_pixels / 2
                                                                };
                                                                if is_mostly_bg {
                                                                    picked_color = bg;
                                                                }
                                                            }
                                                        }
                                                    }
                                                    app.eyedropper_color = Some((
                                                        picked_color[0],
                                                        picked_color[1],
                                                        picked_color[2],
                                                    ));
                                                    // Trigger a re-render
                                                }
                                            }
                                        }
                                    }
                                }
                                // It could also be intended to just return Focus to output
                            }

                            // Check path click
                            if mouse.column >= app.path_area.x
                                && mouse.column < app.path_area.x + app.path_area.width
                                && mouse.row == app.path_area.y
                            {
                                app.input_mode = InputMode::Explorer;
                                app.explorer_dialog = Some(ExplorerDialog {
                                    explorer: FileExplorer::new().unwrap(),
                                    mode: ExplorerMode::Open,
                                    name_input: TextArea::default(),
                                    focus_on_input: false,
                                });
                                continue;
                            }

                            if mouse.column >= app.tree_area.x
                                && mouse.column < app.tree_area.x + app.tree_area.width
                                && mouse.row >= app.tree_area.y
                                && mouse.row < app.tree_area.y + app.tree_area.height
                            {
                                app.current_tab = 2;
                                app.selected_control = 0;
                                let row = (mouse.row - app.tree_area.y) as usize;
                                let scroll = app.tree_state.scroll as usize;
                                let clicked_idx = scroll + row;
                                if clicked_idx < app.tree_state.flatten().len() {
                                    app.tree_state.selected_idx = clicked_idx;
                                    let (is_leaf, level, has_children) = app
                                        .tree_state
                                        .get_selected_node()
                                        .map(|node| {
                                            (node.is_leaf, node.level, !node.children.is_empty())
                                        })
                                        .unwrap_or((false, 0, false));
                                    let marker_x = app.tree_area.x
                                        + (level as u16)
                                            .saturating_mul(2)
                                            .min(app.tree_area.width.saturating_sub(4));
                                    let expansion_end = marker_x.saturating_add(2);
                                    let selection_end = marker_x.saturating_add(4);
                                    if has_children && mouse.column < expansion_end {
                                        app.tree_state.toggle_selected_expansion();
                                    } else if mouse.column < selection_end
                                        && (is_leaf || has_children)
                                    {
                                        app.toggle_tree_selection();
                                    }
                                    app.last_glyph_list_key = None;
                                }
                                continue;
                            } else if mouse.column >= app.glyph_list_area.x
                                && mouse.column < app.glyph_list_area.x + app.glyph_list_area.width
                                && mouse.row >= app.glyph_list_area.y
                                && mouse.row < app.glyph_list_area.y + app.glyph_list_area.height
                            {
                                app.current_tab = 2;
                                app.selected_control = 1;
                                let row = (mouse.row - app.glyph_list_area.y) as usize;
                                let idx = app.glyph_list_state.offset() + row;
                                let count = app
                                    .tree_state
                                    .get_selected_node()
                                    .and_then(|node| {
                                        crate::font::load_blocks_config_unfiltered()
                                            .0
                                            .get(&node.full_path)
                                            .map(Vec::len)
                                    })
                                    .unwrap_or(0);
                                if idx < count {
                                    app.glyph_list_state.select(Some(idx));
                                    if mouse.column < app.glyph_list_area.x.saturating_add(3) {
                                        app.toggle_glyph_selection();
                                    }
                                }
                                continue;
                            } else if mouse.column >= app.glyph_select_all_area.x
                                && mouse.column
                                    < app.glyph_select_all_area.x + app.glyph_select_all_area.width
                                && mouse.row >= app.glyph_select_all_area.y
                                && mouse.row
                                    < app.glyph_select_all_area.y + app.glyph_select_all_area.height
                            {
                                app.current_tab = 2;
                                app.selected_control = 2;
                                app.set_all_glyphs_selected(true);
                                continue;
                            } else if mouse.column >= app.glyph_select_none_area.x
                                && mouse.column
                                    < app.glyph_select_none_area.x + app.glyph_select_none_area.width
                                && mouse.row >= app.glyph_select_none_area.y
                                && mouse.row
                                    < app.glyph_select_none_area.y
                                        + app.glyph_select_none_area.height
                            {
                                app.current_tab = 2;
                                app.selected_control = 3;
                                app.set_all_glyphs_selected(false);
                                continue;
                            } else if mouse.column >= app.glyph_add_area.x
                                && mouse.column < app.glyph_add_area.x + app.glyph_add_area.width
                                && mouse.row >= app.glyph_add_area.y
                                && mouse.row < app.glyph_add_area.y + app.glyph_add_area.height
                            {
                                app.current_tab = 2;
                                app.selected_control = 4;
                                app.open_unicode_explorer();
                                continue;
                            } else if mouse.column >= app.glyph_delete_area.x
                                && mouse.column
                                    < app.glyph_delete_area.x + app.glyph_delete_area.width
                                && mouse.row >= app.glyph_delete_area.y
                                && mouse.row
                                    < app.glyph_delete_area.y + app.glyph_delete_area.height
                            {
                                app.current_tab = 2;
                                app.selected_control = 5;
                                app.delete_selected_glyph_range();
                                continue;
                            }

                            // Check options pane click
                            if mouse.column >= app.options_pane_area.x
                                && mouse.column
                                    < app.options_pane_area.x + app.options_pane_area.width
                                && mouse.row >= app.options_pane_area.y
                                && mouse.row
                                    < app.options_pane_area.y + app.options_pane_area.height
                            {
                                for i in 0..app.options_areas.len() {
                                    let area = app.options_areas[i];
                                    if mouse.column >= area.x
                                        && mouse.column < area.x + area.width
                                        && mouse.row == area.y
                                    {
                                        let ctrl_id = app.get_current_control_id();
                                        if ctrl_id == ControlId::Font {
                                            app.open_font_picker();
                                            app.font_list_state.select(Some(i));
                                        } else {
                                            app.mark_args_dirty();
                                        }
                                        continue;
                                    }
                                }
                            }

                            // Check tab clicks
                            for i in 0..app.tab_areas.len() {
                                let area = app.tab_areas[i];
                                if mouse.column >= area.x
                                    && mouse.column < area.x + area.width
                                    && mouse.row >= area.y
                                    && mouse.row < area.y + area.height
                                {
                                    app.toggle_tab(i);
                                    break;
                                }
                            }

                            if app.replace_colour_button_area.contains(ratatui::layout::Position::new(mouse.column, mouse.row)) {
                                open_replace_colour(&mut app).await;
                                continue;
                            }

                            if mouse.column >= app.pipeline_active_area.x
                                && mouse.column
                                    < app.pipeline_active_area.x + app.pipeline_active_area.width
                                && mouse.row >= app.pipeline_active_area.y
                                && mouse.row
                                    < app.pipeline_active_area.y + app.pipeline_active_area.height
                            {
                                app.current_tab = 1;
                                let inner_y =
                                    mouse.row.saturating_sub(app.pipeline_active_area.y + 1)
                                        as usize;

                                let total_nodes = app.pipeline_nodes.len();
                                match app.get_pipeline_clicked_layer(inner_y) {
                                    (Some(i), None) => {
                                        if i < total_nodes {
                                            app.pipeline_selected = Some(app.pipeline_nodes[i].id);
                                        } else {
                                            app.pipeline_selected = None;
                                        }
                                        if app.selected_control == i
                                            && app.pipeline_expanded.is_none()
                                        {
                                            app.pipeline_expanded = Some(i);
                                        } else if app.selected_control != i {
                                            app.selected_control = i;
                                            app.pipeline_expanded = None;
                                        }

                                        let inner_x =
                                            mouse.column.saturating_sub(app.pipeline_active_area.x);
                                        if inner_x >= 22 && inner_x <= 54 {
                                            let is_expanded = app.pipeline_expanded == Some(i);
                                            let mut used_range: Option<(f32, f32)> = None;
                                            let mut active_param: Option<f32> = None;

                                            if is_expanded {
                                                if let Some(preview) = &app.pipeline_preview_effect
                                                {
                                                    used_range = preview.param_range();
                                                    active_param = preview.param_f32();
                                                }
                                            } else if i < total_nodes {
                                                used_range =
                                                    app.pipeline_nodes[i].effect.param_range();
                                                active_param =
                                                    app.pipeline_nodes[i].effect.param_f32();
                                            }

                                            if let (Some(_), Some((min, max))) =
                                                (active_param, used_range)
                                            {
                                                let ratio = ((inner_x as f32 - 22.0) / 32.0)
                                                    .clamp(0.0, 1.0);
                                                let new_val = min + ratio * (max - min);
                                                if is_expanded {
                                                    if let Some(preview) =
                                                        &mut app.pipeline_preview_effect
                                                    {
                                                        preview.set_param(new_val);
                                                    }
                                                    app.pipeline_dragging = Some((0, 2, i as i16));
                                                } else {
                                                    let node = &mut app.pipeline_nodes[i];
                                                    node.effect.set_param(new_val);
                                                    app.pipeline_dragging = Some((node.id, 1, 0));
                                                }
                                                app.mark_args_dirty();
                                            } else if i < total_nodes {
                                                app.pipeline_dragging =
                                                    Some((app.pipeline_nodes[i].id, 0, 0));
                                            }
                                        } else if i < total_nodes {
                                            app.pipeline_dragging =
                                                Some((app.pipeline_nodes[i].id, 0, 0));
                                        }
                                    }
                                    (Some(_), Some(lib_idx)) => {
                                        if app
                                            .pipeline_catalog
                                            .get(lib_idx)
                                            .is_some_and(|item| item.effect.is_some())
                                        {
                                            select_pipeline_library(&mut app, lib_idx);
                                        }
                                    }
                                    _ => {}
                                }
                                continue;
                            }

                            // Check general control clicks
                            for i in 0..app.control_areas.len() {
                                let area = app.control_areas[i];
                                if area.width == 0 {
                                    continue;
                                }
                                if mouse.column >= area.x
                                    && mouse.column < area.x + area.width
                                    && mouse.row >= area.y
                                    && mouse.row < area.y + area.height
                                {
                                    app.current_tab = 0;
                                    app.dragged_control = Some(i);
                                    let controls = app.tab_controls(0);
                                    let was_selected = app.selected_control == i;
                                    app.selected_control = i;
                                    if let Some(ctrl) = controls.get(i) {
                                        match &ctrl.kind {
                                            ControlKind::Slider { min, max, .. } => {
                                                if was_selected {
                                                    let current_value = app.get_control_value(0, i);
                                                    let display_value =
                                                        if ctrl.id == ControlId::FontSize {
                                                            Some((current_value / 10).to_string())
                                                        } else if matches!(
                                                            ctrl.id,
                                                            ControlId::DiscThreshold
                                                                | ControlId::ContourBendingWeight
                                                                | ControlId::ContourEndpointWeight
                                                                | ControlId::ContourJunctionWeight
                                                                | ControlId::ContourFragmentWeight
                                                                | ControlId::ContourFidelityWeight
                                                                | ControlId::ContourBoundaryWeight
                                                                | ControlId::ContourPeakWeight
                                                        ) {
                                                            Some(format!(
                                                                "{:.1}",
                                                                current_value as f32 / 10.0
                                                            ))
                                                        } else {
                                                            None
                                                        };
                                                    if let Some((bar_x, bar_cells)) =
                                                        slider_bar_layout(
                                                            area,
                                                            ctrl.label,
                                                            display_value.as_deref(),
                                                        )
                                                    {
                                                        let rel_cell = mouse
                                                            .column
                                                            .saturating_sub(bar_x)
                                                            .min(bar_cells.saturating_sub(1) as u16)
                                                            as f64;
                                                        let ratio = ((rel_cell + 0.5)
                                                            / bar_cells as f64)
                                                            .clamp(0.0, 1.0);
                                                        let new_val = *min
                                                            + (((*max - *min) as f64) * ratio)
                                                                as i32;
                                                        set_control_value(
                                                            &mut app,
                                                            i,
                                                            new_val.clamp(*min, *max),
                                                        );
                                                    }
                                                }
                                            }
                                            ControlKind::Toggle { .. }
                                            | ControlKind::Enum { .. } => {
                                                if was_selected {
                                                    toggle_control(&mut app)
                                                }
                                            }
                                            ControlKind::EyedropperWidget => {}
                                            ControlKind::FontSelector => {
                                                app.selected_control = i;
                                                if was_selected {
                                                    app.open_font_picker();
                                                }
                                            }
                                            ControlKind::Action => toggle_control(&mut app),
                                        }
                                    }
                                    break;
                                }
                            }

                            let mouse_pos = ratatui::layout::Position {
                                x: mouse.column,
                                y: mouse.row,
                            };
                            if app.current_tab == 3
                                && app
                                    .text_field_areas
                                    .first()
                                    .is_some_and(|area| area.contains(mouse_pos))
                            {
                                if let Some(sel) = app.text_selected {
                                    if sel > 0 && sel <= app.render_args.overlays.len() {
                                        app.value_edit_buffer =
                                            app.render_args.overlays[sel - 1]
                                                .editable_text()
                                                .to_string();
                                        app.text_field_focus = TextFieldFocus::TextInput;
                                        app.input_mode = InputMode::TextEdit;
                                    }
                                }
                                continue;
                            }

                            if app.current_tab == 3
                                && mouse.column >= app.text_list_area.x
                                && mouse.column < app.text_list_area.x + app.text_list_area.width
                                && mouse.row > app.text_list_area.y
                                && mouse.row
                                    < app
                                        .text_list_area
                                        .y
                                        .saturating_add(app.text_list_area.height)
                                        .saturating_sub(1)
                            {
                                app.current_tab = 3;
                                if app.input_mode == InputMode::TextEdit {
                                    finish_text_edit(&mut app);
                                }
                                let row =
                                    mouse.row.saturating_sub(app.text_list_area.y + 1) as usize;
                                let text_offset = app.text_list_state.offset();
                                let selected_idx = text_offset + row;

                                let total_len = app.render_args.overlays.len();
                                if selected_idx <= total_len {
                                    if selected_idx == 0 {
                                        app.render_args.overlays.push(new_text_overlay(
                                            app.render_mode_idx,
                                            0,
                                            0,
                                            true,
                                        ));
                                        app.text_selected = Some(app.render_args.overlays.len());
                                        app.text_list_state.select(app.text_selected);
                                        app.text_field_focus = TextFieldFocus::TextInput;
                                        app.value_edit_buffer.clear();
                                        app.input_mode = InputMode::TextEdit;
                                        app.reapply_overlays();
                                    } else {
                                        app.text_selected = Some(selected_idx);
                                        app.text_list_state.select(Some(selected_idx));
                                        app.text_field_focus = TextFieldFocus::List;
                                    }
                                }
                            }

                            let pos = ratatui::layout::Position {
                                x: mouse.column,
                                y: mouse.row,
                            };

                            if app.current_tab == 3 && app.text_btn_areas.len() >= 11 {
                                if app
                                    .text_btn_areas
                                    .get(11)
                                    .is_some_and(|area| area.contains(pos))
                                {
                                    app.text_field_focus = TextFieldFocus::FigletFontButton;
                                    if let Some(sel) = app.text_selected {
                                        if sel > 0 && sel <= app.render_args.overlays.len() {
                                            open_figlet_picker(&mut app, sel - 1);
                                        }
                                    }
                                } else if app.text_btn_areas[7].contains(pos) {
                                    if let Some(cp) = app.color_picker_state.as_mut() {
                                        cp.editing_fg = true;
                                    }
                                    app.text_field_focus = TextFieldFocus::FgButton;
                                } else if app.text_btn_areas[8].contains(pos) {
                                    if let Some(cp) = app.color_picker_state.as_mut() {
                                        cp.editing_fg = false;
                                    }
                                    app.text_field_focus = TextFieldFocus::BgButton;
                                } else if app.text_btn_areas[9].contains(pos) {
                                    app.text_field_focus = TextFieldFocus::ClearFgButton;
                                    if let Some(sel) = app.text_selected {
                                        if sel > 0 && sel <= app.render_args.overlays.len() {
                                            app.render_args.overlays[sel - 1].fg = None;
                                            if let Some(cp) = app.color_picker_state.as_mut() {
                                                cp.fg_idx = None;
                                            }
                                            app.reapply_overlays();
                                        }
                                    }
                                } else if app.text_btn_areas[10].contains(pos) {
                                    app.text_field_focus = TextFieldFocus::ClearBgButton;
                                    if let Some(sel) = app.text_selected {
                                        if sel > 0 && sel <= app.render_args.overlays.len() {
                                            app.render_args.overlays[sel - 1].bg = None;
                                            if let Some(cp) = app.color_picker_state.as_mut() {
                                                cp.bg_idx = None;
                                                cp.bg_rgb = None;
                                            }
                                            app.reapply_overlays();
                                        }
                                    }
                                } else if app.text_btn_areas[6].contains(pos) {
                                    // Delete button (index 6)
                                    app.current_tab = 3;
                                    app.text_field_focus = TextFieldFocus::DeleteButton;
                                    app.delete_selected_text_overlay();
                                } else if app.text_btn_areas[0].contains(pos) {
                                    // Wrap button (index 0)
                                    app.current_tab = 3;
                                    app.text_field_focus = TextFieldFocus::WrapButton;
                                    if let Some(sel) = app.text_selected {
                                        if sel > 0 && sel <= app.render_args.overlays.len() {
                                            let overlay = &mut app.render_args.overlays[sel - 1];
                                            if !overlay.figlet_enabled() {
                                                overlay.wrap = !overlay.wrap;
                                            }
                                            app.reapply_overlays();
                                        }
                                    }
                                } else if app.text_btn_areas[1].contains(pos) {
                                    // Fit-to-text (index 1)
                                    app.text_field_focus = TextFieldFocus::FitButton;
                                    if let Some(sel) = app.text_selected {
                                        if sel > 0 && sel <= app.render_args.overlays.len() {
                                            let overlay = &mut app.render_args.overlays[sel - 1];
                                            if !overlay.figlet_enabled() {
                                                overlay.auto_grow = !overlay.auto_grow;
                                                sync_text_overlay_auto_grow(overlay);
                                            }
                                            app.reapply_overlays();
                                        }
                                    }
                                } else if app.text_btn_areas[2].contains(pos) {
                                    // FIGlet (index 2)
                                    app.text_field_focus = TextFieldFocus::FigletButton;
                                    if let Some(sel) = app.text_selected {
                                        if sel > 0 && sel <= app.render_args.overlays.len() {
                                            toggle_text_overlay_figlet(&mut app, sel - 1);
                                        }
                                    }
                                } else if app.text_btn_areas[3].contains(pos) {
                                    // Bold (index 3)
                                    app.text_field_focus = TextFieldFocus::BoldButton;
                                    if let Some(sel) = app.text_selected {
                                        if sel > 0 && sel <= app.render_args.overlays.len() {
                                            app.render_args.overlays[sel - 1].bold =
                                                !app.render_args.overlays[sel - 1].bold;
                                            app.reapply_overlays();
                                        }
                                    }
                                } else if app.text_btn_areas[4].contains(pos) {
                                    // Italic (index 4)
                                    app.text_field_focus = TextFieldFocus::ItalicButton;
                                    if let Some(sel) = app.text_selected {
                                        if sel > 0 && sel <= app.render_args.overlays.len() {
                                            app.render_args.overlays[sel - 1].italic =
                                                !app.render_args.overlays[sel - 1].italic;
                                            app.reapply_overlays();
                                        }
                                    }
                                } else if app.text_btn_areas[5].contains(pos) {
                                    // Underline (index 5)
                                    app.text_field_focus = TextFieldFocus::UnderlineButton;
                                    if let Some(sel) = app.text_selected {
                                        if sel > 0 && sel <= app.render_args.overlays.len() {
                                            app.render_args.overlays[sel - 1].underline =
                                                !app.render_args.overlays[sel - 1].underline;
                                            app.reapply_overlays();
                                        }
                                    }
                                }
                            }
                        }
                        MouseEventKind::Drag(MouseButton::Left) => {
                            if app.current_tab == 3 && app.text_drag_mode != TextDragMode::None {
                                if let Some(res) = &app.cached_result {
                                    let inner_height = app.output_area.height.saturating_sub(2);
                                    let line_count = app.cached_render.lines().count() as u16;
                                    let pad_y = if line_count < inner_height {
                                        inner_height.saturating_sub(line_count) / 2
                                    } else {
                                        0
                                    };
                                    if mouse.row >= app.output_area.y + 1 + pad_y {
                                        let content_y =
                                            (mouse.row - (app.output_area.y + 1 + pad_y)) as usize
                                                + app.scroll as usize;
                                        if content_y < res.grid.len() {
                                            let row = &res.grid[content_y];
                                            let inner_width =
                                                app.output_area.width.saturating_sub(2);
                                            let row_len = row.len() as u16;
                                            let pad_x = if inner_width > row_len {
                                                (inner_width - row_len) / 2
                                            } else {
                                                0
                                            };
                                            let start_x = app.output_area.x + 1 + pad_x;

                                            if mouse.column >= start_x {
                                                let c_x = (mouse.column - start_x) as i32;
                                                let c_y = content_y as i32;

                                                if let Some(sel) = app.text_selected {
                                                    if sel > 0
                                                        && sel <= app.render_args.overlays.len()
                                                    {
                                                        let index = sel - 1;
                                                        let cell_x = c_x;
                                                        let cell_y = c_y;
                                                        if app.text_drag_mode
                                                            == TextDragMode::Moving
                                                        {
                                                            // Move
                                                            if let Some((sx, sy)) =
                                                                app.text_drag_start
                                                            {
                                                                let ov = &mut app.render_args
                                                                    .overlays[index];
                                                                ov.x = cell_x - sx;
                                                                ov.y = cell_y - sy;
                                                                app.reapply_overlays();
                                                            }
                                                        } else if app.text_drag_mode
                                                            == TextDragMode::Sizing
                                                        {
                                                            // Resize
                                                            if let Some((ox, oy)) =
                                                                app.text_drag_start
                                                            {
                                                                let figlet = {
                                                                    let ov = &mut app.render_args
                                                                        .overlays[index];
                                                                    ov.auto_grow = false;
                                                                    ov.w =
                                                                        (cell_x - ox + 1).max(1);
                                                                    ov.h =
                                                                        (cell_y - oy + 1).max(1);
                                                                    ov.figlet_enabled()
                                                                };
                                                                if figlet {
                                                                    refresh_text_overlay_figlet(
                                                                        &mut app, index,
                                                                    );
                                                                }
                                                                app.reapply_overlays();
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            } else if let Some(splitter) = app.dragging_splitter {
                                let root = Layout::vertical([
                                    Constraint::Length(1), // title/tabs
                                    Constraint::Min(0),    // content
                                    Constraint::Length(1), // status bar
                                ])
                                .split(app.terminal_area); // We need terminal area here

                                let left_width = match app.current_tab {
                                    0 => app.general_pane_width,
                                    1 => app.pipeline_pane_width,
                                    2 => app.glyphs_pane_width,
                                    3 => app.text_pane_width,
                                    _ => 30,
                                };

                                let panes = Layout::horizontal([
                                    Constraint::Length(left_width),
                                    Constraint::Length(1),
                                    Constraint::Min(0),
                                ])
                                .split(root[1]);

                                match splitter {
                                    SplitterKind::Text => {
                                        let left_edge = panes[0].x;
                                        let right_boundary =
                                            panes[2].x.saturating_add(panes[2].width);
                                        let max_width = right_boundary
                                            .saturating_sub(left_edge)
                                            .saturating_sub(16)
                                            .max(41);
                                        app.text_pane_width = mouse
                                            .column
                                            .saturating_sub(left_edge)
                                            .clamp(41, max_width);
                                    }
                                    SplitterKind::General => {
                                        let left_edge = panes[0].x;
                                        let right_boundary =
                                            panes[2].x.saturating_add(panes[2].width);
                                        let max_width = right_boundary
                                            .saturating_sub(left_edge)
                                            .saturating_sub(16)
                                            .max(18);
                                        app.general_pane_width = mouse
                                            .column
                                            .saturating_sub(left_edge)
                                            .clamp(18, max_width);
                                    }
                                    SplitterKind::Pipeline => {
                                        let left_edge = panes[0].x;
                                        let right_boundary =
                                            panes[2].x.saturating_add(panes[2].width);
                                        let max_width = right_boundary
                                            .saturating_sub(left_edge)
                                            .saturating_sub(16)
                                            .max(24);
                                        app.pipeline_pane_width = mouse
                                            .column
                                            .saturating_sub(left_edge)
                                            .clamp(24, max_width);
                                    }
                                    SplitterKind::PipelineActive => {}
                                    SplitterKind::Glyphs => {
                                        let left_edge = panes[0].x;
                                        let right_boundary =
                                            panes[2].x.saturating_add(panes[2].width);
                                        let max_width = right_boundary
                                            .saturating_sub(left_edge)
                                            .saturating_sub(16)
                                            .max(22);
                                        app.glyphs_pane_width = mouse
                                            .column
                                            .saturating_sub(left_edge)
                                            .clamp(22, max_width);
                                    }
                                    SplitterKind::GlyphTreeRows => {
                                        let max_height =
                                            app.glyphs_area.height.saturating_sub(12).max(5);
                                        app.glyph_tree_height = mouse
                                            .row
                                            .saturating_sub(app.glyphs_area.y)
                                            .clamp(5, max_height);
                                    }
                                    SplitterKind::GlyphListRows => {
                                        let start =
                                            app.glyph_list_area.y.saturating_sub(app.glyphs_area.y);
                                        let max_height =
                                            app.glyphs_area.height.saturating_sub(start + 6).max(5);
                                        app.glyph_list_height = mouse
                                            .row
                                            .saturating_sub(app.glyph_list_area.y)
                                            .clamp(5, max_height);
                                    }
                                }
                                continue;
                            } else if app.current_tab == 1 {
                                if let Some((id, drag_mode, _)) = app.pipeline_dragging {
                                    let active_list_area = app.pipeline_active_area;
                                    if mouse.column >= active_list_area.x
                                        && mouse.column
                                            < active_list_area.x + active_list_area.width
                                        && mouse.row >= active_list_area.y
                                        && mouse.row < active_list_area.y + active_list_area.height
                                    {
                                        let inner_y =
                                            mouse.row.saturating_sub(active_list_area.y + 1)
                                                as usize;

                                        if drag_mode == 1 {
                                            let inner_x =
                                                mouse.column.saturating_sub(active_list_area.x);
                                            if let Some(node) =
                                                app.pipeline_nodes.iter_mut().find(|n| n.id == id)
                                            {
                                                if let (Some(_), Some((min, max))) = (
                                                    node.effect.param_f32(),
                                                    node.effect.param_range(),
                                                ) {
                                                    let ratio = ((inner_x.saturating_sub(21))
                                                        as f32
                                                        / 32.0)
                                                        .clamp(0.0, 1.0);
                                                    node.effect
                                                        .set_param(min + ratio * (max - min));
                                                    app.mark_args_dirty();
                                                }
                                            }
                                        } else if drag_mode == 0
                                            && inner_y < app.pipeline_nodes.len()
                                        {
                                            let current_idx = app
                                                .pipeline_nodes
                                                .iter()
                                                .position(|n| n.id == id)
                                                .unwrap();
                                            if current_idx != inner_y {
                                                let node = app.pipeline_nodes.remove(current_idx);
                                                app.pipeline_nodes.insert(inner_y, node);
                                                app.selected_control = inner_y;
                                                app.mark_args_dirty();
                                            }
                                        }
                                    }
                                }
                            } else if let Some(visual_idx) = app.dragged_control {
                                if let Some(area) = app.control_areas.get(visual_idx) {
                                    if area.width > 0 {
                                        let controls = app.tab_controls(0);
                                        if let Some(ctrl) = controls.get(visual_idx) {
                                            if let ControlKind::Slider { min, max, .. } = &ctrl.kind
                                            {
                                                let current_value =
                                                    app.get_control_value(0, visual_idx);
                                                let display_value =
                                                    if ctrl.id == ControlId::FontSize {
                                                        Some((current_value / 10).to_string())
                                                    } else if matches!(
                                                        ctrl.id,
                                                        ControlId::DiscThreshold
                                                            | ControlId::ContourBendingWeight
                                                            | ControlId::ContourEndpointWeight
                                                            | ControlId::ContourJunctionWeight
                                                            | ControlId::ContourFragmentWeight
                                                            | ControlId::ContourFidelityWeight
                                                            | ControlId::ContourBoundaryWeight
                                                            | ControlId::ContourPeakWeight
                                                    ) {
                                                        Some(format!(
                                                            "{:.1}",
                                                            current_value as f32 / 10.0
                                                        ))
                                                    } else {
                                                        None
                                                    };
                                                if let Some((bar_x, bar_cells)) = slider_bar_layout(
                                                    *area,
                                                    ctrl.label,
                                                    display_value.as_deref(),
                                                ) {
                                                    let rel_cell = mouse
                                                        .column
                                                        .saturating_sub(bar_x)
                                                        .min(bar_cells.saturating_sub(1) as u16)
                                                        as f64;
                                                    let ratio = ((rel_cell + 0.5)
                                                        / bar_cells as f64)
                                                        .clamp(0.0, 1.0);
                                                    let new_val = *min
                                                        + (((*max - *min) as f64) * ratio) as i32;
                                                    set_control_value(
                                                        &mut app,
                                                        visual_idx,
                                                        new_val.clamp(*min, *max),
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        MouseEventKind::Up(MouseButton::Left) => {
                            app.pipeline_dragging = None;
                            app.dragged_control = None;
                            app.dragging_splitter = None;
                            if app.text_drag_mode != TextDragMode::None {
                                app.text_drag_mode = TextDragMode::None;
                                app.text_drag_start = None;
                                app.reapply_overlays();
                            }
                        }
                        MouseEventKind::Down(MouseButton::Right) => {
                            if mouse.column >= app.pipeline_active_area.x
                                && mouse.column
                                    < app.pipeline_active_area.x + app.pipeline_active_area.width
                                && mouse.row >= app.pipeline_active_area.y
                                && mouse.row
                                    < app.pipeline_active_area.y + app.pipeline_active_area.height
                            {
                                let inner_y =
                                    mouse.row.saturating_sub(app.pipeline_active_area.y + 1)
                                        as usize;
                                if let (Some(idx), None) = app.get_pipeline_clicked_layer(inner_y) {
                                    if idx < app.pipeline_nodes.len() {
                                        app.current_tab = 1;
                                        app.pipeline_selected = Some(app.pipeline_nodes[idx].id);
                                        app.selected_control = idx;
                                        let node = &mut app.pipeline_nodes[idx];
                                        node.disabled = !node.disabled;
                                        app.mark_args_dirty();
                                    }
                                }
                            }
                        }
                        MouseEventKind::Down(MouseButton::Middle) => {
                            if mouse.column >= app.pipeline_active_area.x
                                && mouse.column
                                    < app.pipeline_active_area.x + app.pipeline_active_area.width
                                && mouse.row >= app.pipeline_active_area.y
                                && mouse.row
                                    < app.pipeline_active_area.y + app.pipeline_active_area.height
                            {
                                let inner_y =
                                    mouse.row.saturating_sub(app.pipeline_active_area.y + 1)
                                        as usize;
                                if let (Some(idx), None) = app.get_pipeline_clicked_layer(inner_y) {
                                    if idx < app.pipeline_nodes.len() {
                                        let id = app.pipeline_nodes[idx].id;
                                        app.pipeline_nodes.retain(|n| n.id != id);
                                        if app.pipeline_selected == Some(id) {
                                            app.pipeline_selected = None;
                                        }
                                        app.ensure_pipeline_selection();
                                        app.mark_args_dirty();
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Event::Key(key) => {
                    if key.kind == KeyEventKind::Press {
                        if app.current_tab == 0
                            && app.get_current_control_id() == ControlId::Font
                        {
                            if let KeyCode::Char(c) = key.code {
                                if c != ' '
                                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                                    && !key.modifiers.contains(KeyModifiers::ALT)
                                {
                                    app.open_font_picker();
                                    app.font_search.push(c);
                                    app.sync_font_picker_selection();
                                    continue;
                                }
                            }
                        }
                        if app.current_tab == 3 && app.text_cursor_anchor.is_some() {
                            match key.code {
                                KeyCode::Char(c)
                                    if !key
                                        .modifiers
                                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                                {
                                    begin_auto_growing_text_at_anchor(&mut app, c.to_string());
                                    continue;
                                }
                                KeyCode::Enter => {
                                    begin_auto_growing_text_at_anchor(&mut app, "\n".to_string());
                                    continue;
                                }
                                KeyCode::Esc => {
                                    app.text_cursor_anchor = None;
                                    continue;
                                }
                                _ => {}
                            }
                        }
                        if app.current_tab == 3 {
                            if let Some(sel) = app.text_selected {
                                if sel > 0 && sel <= app.render_args.overlays.len() {
                                    let index = sel - 1;
                                    let figlet = app.render_args.overlays[index].figlet_enabled();
                                    let mut handled = true;
                                    {
                                        let ov = &mut app.render_args.overlays[index];
                                        let text = ov.source_text.as_mut().unwrap_or(&mut ov.text);
                                        match key.code {
                                            KeyCode::Backspace
                                                if app.text_field_focus
                                                    == TextFieldFocus::TextInput =>
                                            {
                                                text.pop();
                                            }
                                            KeyCode::Enter
                                                if app.text_field_focus
                                                    == TextFieldFocus::TextInput =>
                                            {
                                                text.push('\n');
                                            }
                                            KeyCode::Char(c)
                                                if (app.text_field_focus
                                                    == TextFieldFocus::TextInput
                                                    || (app.text_field_focus
                                                        == TextFieldFocus::List
                                                        && c != ' '))
                                                    && !key.modifiers.intersects(
                                                        KeyModifiers::CONTROL | KeyModifiers::ALT,
                                                    ) =>
                                            {
                                                app.text_field_focus = TextFieldFocus::TextInput;
                                                text.push(c);
                                            }
                                            KeyCode::Esc => {
                                                app.text_selected = None;
                                            }
                                            _ => {
                                                handled = false;
                                            }
                                        }
                                    }
                                    if handled {
                                        if figlet {
                                            refresh_text_overlay_figlet(&mut app, index);
                                        } else {
                                            sync_text_overlay_auto_grow(
                                                &mut app.render_args.overlays[index],
                                            );
                                        }
                                        app.reapply_overlays();
                                        continue;
                                    }
                                }
                            }
                        }
                        match key.code {
                            KeyCode::Tab => {
                                if app.current_tab == 3 {
                                    app.cycle_text_field_focus(1);
                                } else {
                                    app.cycle_focus_target(1);
                                }
                            }
                            KeyCode::BackTab => {
                                if app.current_tab == 3 {
                                    app.cycle_text_field_focus(-1);
                                } else {
                                    app.cycle_focus_target(-1);
                                }
                            }
                            KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                return Ok(());
                            }
                            KeyCode::Char('r') | KeyCode::Delete | KeyCode::Backspace
                                if app.current_tab == 1 =>
                            {
                                if let Some((id, _, _)) = app.pipeline_dragging {
                                    app.pipeline_nodes.retain(|n| n.id != id);
                                    app.pipeline_dragging = None;
                                    app.ensure_pipeline_selection();
                                    app.mark_args_dirty();
                                } else if app.pipeline_expanded.is_none() {
                                    if let Some(id) = app.pipeline_selected {
                                        app.pipeline_nodes.retain(|n| n.id != id);
                                        app.ensure_pipeline_selection();
                                        app.mark_args_dirty();
                                    }
                                }
                            }
                            KeyCode::Char('r') | KeyCode::Delete | KeyCode::Backspace
                                if app.current_tab == 0 =>
                            {
                                reset_control(&mut app);
                            }
                            KeyCode::Delete | KeyCode::Backspace
                                if app.current_tab == 3 && app.text_selected.unwrap_or(0) > 0 =>
                            {
                                let sel = app.text_selected.unwrap();
                                app.render_args.overlays.remove(sel - 1);
                                let new_len = app.render_args.overlays.len();
                                if new_len == 0 {
                                    app.text_selected = Some(0);
                                } else {
                                    app.text_selected = Some(sel.min(new_len));
                                }
                                app.text_list_state.select(app.text_selected);
                                app.text_field_focus = TextFieldFocus::List;
                                app.text_cursor_anchor = None;
                                app.text_hovered = None;
                                app.text_resize_hover = None;
                                app.reapply_overlays();
                            }
                            KeyCode::Char(c)
                                if app.current_tab == 3
                                    && app.text_selected.unwrap_or(0) > 0
                                    && (c == 'c' || c == 'C') =>
                            {
                                // Start Color Picker
                                let mode = match app.render_mode_idx {
                                    0 => crate::colorpicker::PaletteMode::Irc,
                                    1 => crate::colorpicker::PaletteMode::Ansi,
                                    2 => crate::colorpicker::PaletteMode::Ansi24,
                                    _ => crate::colorpicker::PaletteMode::Irc,
                                };
                                let editing_bg = app.text_field_focus == TextFieldFocus::BgButton;
                                let mut cp = crate::colorpicker::ColorPickerState::new(mode, true);
                                cp.editing_fg = !editing_bg;
                                app.color_picker_state = Some(cp);
                            }
                            KeyCode::Char('c') if app.current_tab == 1 && app.pipeline_expanded.is_none() => {
                                open_replace_colour(&mut app).await;
                            }
                            KeyCode::Char(c)
                                if app.current_tab == 0 && (c.is_ascii_digit() || c == '-') =>
                            {
                                let ctrl_id = app.get_current_control_id();
                                let def = get_control_def(ctrl_id);
                                if app.control_disabled_reason(ctrl_id).is_some() { continue; }
                                if let ControlKind::Slider { .. } = def.kind {
                                    app.value_edit_buffer = c.to_string();
                                    app.input_mode = InputMode::ValueEdit;
                                    continue;
                                }
                            }
                            KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                app.open_explorer(ExplorerMode::Open);
                            }
                            KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                app.paste_from_clipboard();
                            }
                            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                app.open_explorer(ExplorerMode::SaveText);
                            }
                            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                app.open_explorer(ExplorerMode::SavePng);
                            }
                            KeyCode::Char('g') => app.toggle_tab(0),
                            KeyCode::Char('p') => app.toggle_tab(1),
                            KeyCode::Char('y') => app.toggle_tab(2),
                            KeyCode::Delete | KeyCode::Backspace
                                if app.current_tab == 2 && app.selected_control == 0 =>
                            {
                                app.delete_selected_glyph_range();
                            }
                            KeyCode::Left if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                app.focus_next_visible_tab(-1);
                            }
                            KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                app.focus_next_visible_tab(1);
                            }
                            KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                if app.current_tab == 3 {
                                    if let Some(sel) = app.text_selected {
                                        if sel > 0 && sel <= app.render_args.overlays.len() {
                                            app.render_args.overlays[sel - 1].bold =
                                                !app.render_args.overlays[sel - 1].bold;
                                            app.reapply_overlays();
                                        }
                                    }
                                }
                            }
                            KeyCode::Char('i') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                if app.current_tab == 3 {
                                    if let Some(sel) = app.text_selected {
                                        if sel > 0 && sel <= app.render_args.overlays.len() {
                                            app.render_args.overlays[sel - 1].italic =
                                                !app.render_args.overlays[sel - 1].italic;
                                            app.reapply_overlays();
                                        }
                                    }
                                }
                            }
                            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                if app.current_tab == 3 {
                                    if let Some(sel) = app.text_selected {
                                        if sel > 0 && sel <= app.render_args.overlays.len() {
                                            app.render_args.overlays[sel - 1].underline =
                                                !app.render_args.overlays[sel - 1].underline;
                                            app.reapply_overlays();
                                        }
                                    }
                                }
                            }
                            KeyCode::Esc => {
                                app.cancel_render();
                                app.args_dirty = false;

                                if app.is_optimizing {
                                    app.perform_auto_optimize();
                                    continue;
                                }

                                if app.current_tab == 1
                                    && cancel_pipeline_editor(
                                        &mut app.pipeline_expanded,
                                        &mut app.pipeline_dragging,
                                    )
                                {
                                    // The expanded library is a live preview. Closing it must
                                    // trigger a render so an uncommitted add/replacement vanishes.
                                    app.mark_args_dirty();
                                } else if app.current_tab == 3
                                    && app.text_field_focus != TextFieldFocus::List
                                {
                                    app.text_field_focus = TextFieldFocus::List;
                                } else if app.current_tab == 2 {
                                    if app.selected_control > 0 {
                                        app.selected_control = 0;
                                    } else {
                                        app.tree_state.collapse_selected();
                                    }
                                }
                            }
                            KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('J') => {
                                if app.current_tab == 1
                                    && app.pipeline_expanded.is_none()
                                    && (key.modifiers.contains(KeyModifiers::SHIFT)
                                        || key.code == KeyCode::Char('J'))
                                {
                                    let i = app.selected_control;
                                    if i + 1 < app.pipeline_nodes.len() {
                                        app.pipeline_nodes.swap(i, i + 1);
                                        app.selected_control += 1;
                                        app.pipeline_selected =
                                            Some(app.pipeline_nodes[app.selected_control].id);
                                        app.mark_args_dirty();
                                    }
                                } else if app.current_tab == 1 && app.pipeline_expanded.is_some() {
                                    move_pipeline_library_selection(&mut app, 1);
                                } else if app.current_tab == 1 {
                                    if app.selected_control < app.pipeline_nodes.len() {
                                        app.selected_control += 1;
                                        if let Some(node) =
                                            app.pipeline_nodes.get(app.selected_control)
                                        {
                                            app.pipeline_selected = Some(node.id);
                                        } else {
                                            app.pipeline_selected = None;
                                        }
                                    }
                                } else if app.current_tab == 2 {
                                    match app.selected_control {
                                        0 => app.tree_state.select_next(),
                                        1 => app.move_selected_glyph(1),
                                        _ => {}
                                    }
                                } else if app.current_tab == 3 {
                                    let has_sel = app.text_selected.map_or(false, |s| {
                                        s > 0 && s <= app.render_args.overlays.len()
                                    });
                                    if app.text_field_focus == TextFieldFocus::List || !has_sel {
                                        // Navigate list
                                        let items_len = app.render_args.overlays.len() + 1;
                                        let curr = app.text_selected.unwrap_or(0);
                                        let next = (curr + 1).min(items_len - 1);
                                        app.text_selected = Some(next);
                                        app.text_list_state.select(Some(next));
                                    } else if matches!(
                                        app.text_field_focus,
                                        TextFieldFocus::FgButton
                                            | TextFieldFocus::ClearFgButton
                                            | TextFieldFocus::BgButton
                                            | TextFieldFocus::ClearBgButton
                                    ) {
                                        app.move_text_picker(0, 1, false);
                                    } else {
                                        app.cycle_text_field_focus(1);
                                    }
                                } else {
                                    let count = app.control_count();
                                    if count > 0 {
                                        app.selected_control =
                                            (app.selected_control + 1).min(count - 1);
                                    }
                                }
                            }
                            KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('K') => {
                                if app.current_tab == 1
                                    && app.pipeline_expanded.is_none()
                                    && (key.modifiers.contains(KeyModifiers::SHIFT)
                                        || key.code == KeyCode::Char('K'))
                                {
                                    let i = app.selected_control;
                                    if i > 0 {
                                        app.pipeline_nodes.swap(i, i - 1);
                                        app.selected_control -= 1;
                                        app.pipeline_selected =
                                            Some(app.pipeline_nodes[app.selected_control].id);
                                        app.mark_args_dirty();
                                    }
                                } else if app.current_tab == 1 && app.pipeline_expanded.is_some() {
                                    move_pipeline_library_selection(&mut app, -1);
                                } else if app.current_tab == 1 {
                                    if app.selected_control > 0 {
                                        app.selected_control -= 1;
                                        if let Some(node) =
                                            app.pipeline_nodes.get(app.selected_control)
                                        {
                                            app.pipeline_selected = Some(node.id);
                                        } else {
                                            app.pipeline_selected = None;
                                        }
                                    }
                                } else if app.current_tab == 2 {
                                    match app.selected_control {
                                        0 => app.tree_state.select_prev(),
                                        1 => app.move_selected_glyph(-1),
                                        _ => {}
                                    }
                                } else if app.current_tab == 3 {
                                    let has_sel = app.text_selected.map_or(false, |s| {
                                        s > 0 && s <= app.render_args.overlays.len()
                                    });
                                    if app.text_field_focus == TextFieldFocus::List || !has_sel {
                                        let curr = app.text_selected.unwrap_or(0);
                                        let next = curr.saturating_sub(1);
                                        app.text_selected = Some(next);
                                        app.text_list_state.select(Some(next));
                                    } else if matches!(
                                        app.text_field_focus,
                                        TextFieldFocus::FgButton
                                            | TextFieldFocus::ClearFgButton
                                            | TextFieldFocus::BgButton
                                            | TextFieldFocus::ClearBgButton
                                    ) {
                                        app.move_text_picker(0, -1, false);
                                    } else {
                                        app.cycle_text_field_focus(-1);
                                    }
                                } else {
                                    app.selected_control = app.selected_control.saturating_sub(1);
                                }
                            }
                            KeyCode::Left | KeyCode::Char('h') => {
                                if app.current_tab == 3 {
                                    if app.text_field_focus == TextFieldFocus::FigletFontButton {
                                        if let Some(sel) = app.text_selected {
                                            if sel > 0 && sel <= app.render_args.overlays.len() {
                                                cycle_text_overlay_figlet_font(
                                                    &mut app,
                                                    sel - 1,
                                                    -1,
                                                );
                                            }
                                        }
                                    } else if matches!(
                                        app.text_field_focus,
                                        TextFieldFocus::FgButton
                                            | TextFieldFocus::ClearFgButton
                                            | TextFieldFocus::BgButton
                                            | TextFieldFocus::ClearBgButton
                                    ) {
                                        app.move_text_picker(
                                            -1,
                                            0,
                                            key.modifiers.contains(KeyModifiers::SHIFT),
                                        );
                                    } else if !matches!(
                                        app.text_field_focus,
                                        TextFieldFocus::List | TextFieldFocus::TextInput
                                    ) {
                                        app.cycle_text_field_focus(-1);
                                    }
                                } else if app.current_tab == 2 {
                                    if app.selected_control == 0 {
                                        app.tree_state.collapse_selected();
                                    } else {
                                        app.selected_control -= 1;
                                    }
                                } else if app.current_tab == 1 {
                                    let delta = if key.modifiers.contains(KeyModifiers::SHIFT) {
                                        -10.0
                                    } else {
                                        -1.0
                                    };
                                    if app.pipeline_expanded.is_some() {
                                        let has_param = app
                                            .pipeline_preview_effect
                                            .as_ref()
                                            .map(|e| e.param_f32().is_some())
                                            .unwrap_or(false);
                                        if has_param {
                                            adjust_pipeline_library_preview(&mut app, delta);
                                        } else {
                                        }
                                    } else {
                                        adjust_selected_pipeline_param(&mut app, delta);
                                    }
                                } else {
                                    let ctrl_id = app.get_current_control_id();
                                    let d = if ctrl_id == ControlId::FontSize {
                                        if key.modifiers.contains(KeyModifiers::SHIFT) {
                                            100
                                        } else {
                                            10
                                        }
                                    } else if key.modifiers.contains(KeyModifiers::SHIFT) {
                                        10
                                    } else {
                                        1
                                    };
                                    if let ControlKind::Slider { .. } =
                                        get_control_def(ctrl_id).kind
                                    {
                                        let now = std::time::Instant::now();
                                        if app.slider_last_press.map_or(false, |t| {
                                            now.duration_since(t).as_millis() < 300
                                        }) {
                                            app.slider_repeat_count += 1;
                                        } else {
                                            app.slider_repeat_count = 0;
                                        }
                                        let multiplier = if ctrl_id == ControlId::FontSize {
                                            1
                                        } else {
                                            2_i32.pow((app.slider_repeat_count / 8).min(6) as u32)
                                        };
                                        let applied_delta = -d * multiplier;
                                        app.slider_last_press = Some(now);
                                        adjust_control(&mut app, applied_delta);
                                    } else {
                                        adjust_control(&mut app, -d);
                                    }
                                }
                            }
                            KeyCode::Right | KeyCode::Char('l') => {
                                if app.current_tab == 3 {
                                    if app.text_field_focus == TextFieldFocus::FigletFontButton {
                                        if let Some(sel) = app.text_selected {
                                            if sel > 0 && sel <= app.render_args.overlays.len() {
                                                cycle_text_overlay_figlet_font(
                                                    &mut app,
                                                    sel - 1,
                                                    1,
                                                );
                                            }
                                        }
                                    } else if matches!(
                                        app.text_field_focus,
                                        TextFieldFocus::FgButton
                                            | TextFieldFocus::ClearFgButton
                                            | TextFieldFocus::BgButton
                                            | TextFieldFocus::ClearBgButton
                                    ) {
                                        app.move_text_picker(
                                            1,
                                            0,
                                            key.modifiers.contains(KeyModifiers::SHIFT),
                                        );
                                    } else if !matches!(
                                        app.text_field_focus,
                                        TextFieldFocus::List | TextFieldFocus::TextInput
                                    ) {
                                        app.cycle_text_field_focus(1);
                                    }
                                } else if app.current_tab == 2 {
                                    if app.selected_control == 0 {
                                        app.tree_state.expand_selected();
                                    } else if app.selected_control < 5 {
                                        app.selected_control += 1;
                                    }
                                } else if app.current_tab == 1 {
                                    let delta = if key.modifiers.contains(KeyModifiers::SHIFT) {
                                        10.0
                                    } else {
                                        1.0
                                    };
                                    if app.pipeline_expanded.is_some() {
                                        let has_param = app
                                            .pipeline_preview_effect
                                            .as_ref()
                                            .map(|e| e.param_f32().is_some())
                                            .unwrap_or(false);
                                        if has_param {
                                            adjust_pipeline_library_preview(&mut app, delta);
                                        } else {
                                        }
                                    } else {
                                        adjust_selected_pipeline_param(&mut app, delta);
                                    }
                                } else {
                                    let ctrl_id = app.get_current_control_id();
                                    let d = if ctrl_id == ControlId::FontSize {
                                        if key.modifiers.contains(KeyModifiers::SHIFT) {
                                            100
                                        } else {
                                            10
                                        }
                                    } else if key.modifiers.contains(KeyModifiers::SHIFT) {
                                        10
                                    } else {
                                        1
                                    };
                                    if let ControlKind::Slider { .. } =
                                        get_control_def(ctrl_id).kind
                                    {
                                        let now = std::time::Instant::now();
                                        if app.slider_last_press.map_or(false, |t| {
                                            now.duration_since(t).as_millis() < 300
                                        }) {
                                            app.slider_repeat_count += 1;
                                        } else {
                                            app.slider_repeat_count = 0;
                                        }
                                        let multiplier = if ctrl_id == ControlId::FontSize {
                                            1
                                        } else {
                                            2_i32.pow((app.slider_repeat_count / 8).min(6) as u32)
                                        };
                                        let applied_delta = d * multiplier;
                                        app.slider_last_press = Some(now);
                                        adjust_control(&mut app, applied_delta);
                                    } else {
                                        adjust_control(&mut app, d);
                                    }
                                }
                            }
                            KeyCode::Char(' ') => {
                                if app.current_tab == 2 {
                                    app.activate_selected_glyph_control();
                                } else if app.current_tab == 3 {
                                    app.activate_text_field_focus();
                                } else if app.current_tab == 1 {
                                    if app.pipeline_expanded.is_none() {
                                        if let Some(node) =
                                            app.pipeline_nodes.get_mut(app.selected_control)
                                        {
                                            node.disabled = !node.disabled;
                                            app.mark_args_dirty();
                                        }
                                    }
                                } else {
                                    toggle_control(&mut app);
                                }
                            }
                            KeyCode::Enter if app.current_tab == 3 => {
                                app.activate_text_field_focus();
                            }
                            KeyCode::Enter => {
                                if app.current_tab == 2 {
                                    app.activate_selected_glyph_control();
                                } else if app.current_tab == 1 {
                                    if let Some(expanded_idx) = app.pipeline_expanded {
                                        let selected_catalog_idx = app.pipeline_library_selected;
                                        if let Some(effect) =
                                            app.pipeline_preview_effect.take().or_else(|| {
                                                app.pipeline_catalog
                                                    .get(selected_catalog_idx)
                                                    .and_then(|c| c.effect.clone())
                                            })
                                        {
                                            if expanded_idx < app.pipeline_nodes.len() {
                                                app.pipeline_nodes[expanded_idx].effect = effect;
                                            } else {
                                                let new_id = app.pipeline_next_id;
                                                app.pipeline_next_id += 1;
                                                app.pipeline_nodes.push(
                                                    crate::pipeline::ImageEffectNode {
                                                        id: new_id,
                                                        effect,
                                                        disabled: false,
                                                    },
                                                );
                                                app.pipeline_selected = Some(new_id);
                                                app.selected_control = app.pipeline_nodes.len();
                                                // Select the new "Add Effect" node
                                            }
                                            app.mark_args_dirty();
                                        }
                                        app.pipeline_expanded = None;
                                        if app.pipeline_nodes.get(expanded_idx).is_some_and(|node| matches!(node.effect, crate::pipeline::ImageEffect::ReplaceColour { .. })) {
                                            app.selected_control = expanded_idx;
                                            app.pipeline_selected = Some(app.pipeline_nodes[expanded_idx].id);
                                            open_replace_colour(&mut app).await;
                                        }
                                    } else if app.pipeline_nodes.get(app.selected_control).is_some_and(|node| matches!(node.effect, crate::pipeline::ImageEffect::ReplaceColour { .. })) {
                                        open_replace_colour(&mut app).await;
                                    } else {
                                        app.pipeline_expanded = Some(app.selected_control);
                                    }
                                } else {
                                    let controls = app.tab_controls(0);
                                    if let Some(ctrl) = controls.get(app.selected_control) {
                                        if let ControlKind::Slider { .. } = &ctrl.kind {
                                            if app.control_disabled_reason(ctrl.id).is_some() { continue; }
                                            let current_val = app.get_value(ctrl.id);
                                            app.value_edit_buffer =
                                                if ctrl.id == ControlId::FontSize {
                                                    (current_val / 10).to_string()
                                                } else if ctrl.id == ControlId::OcrMegapixels {
                                                    format!("{:.1}", current_val as f32 / 10.0)
                                                } else {
                                                    current_val.to_string()
                                                };
                                            app.input_mode = InputMode::ValueEdit;
                                            continue;
                                        }
                                    }
                                    toggle_control(&mut app);
                                }
                            }
                            KeyCode::PageUp => {
                                let line_count = app.cached_render.lines().count() as u16;
                                if line_count > app.output_area.height {
                                    app.scroll = app.scroll.saturating_sub(10);
                                }
                            }
                            KeyCode::PageDown => {
                                let line_count = app.cached_render.lines().count() as u16;
                                if line_count > app.output_area.height {
                                    app.scroll = app.scroll.saturating_add(10);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }

        // Slider pending logic removed — changes apply immediately now

        if app.args_dirty {
            if let Some(last) = app.last_args_change {
                if last.elapsed() >= std::time::Duration::from_millis(50) {
                    app.flush_render_args();
                }
            }
        }
    }
}

// ── Control Logic ────────────────────────────────────────────────────────────
fn mark_control_changed(app: &mut App, id: ControlId) {
    if matches!(id, ControlId::OcrAdvanced | ControlId::ShowAdvanced) {
        return;
    }
    if id == ControlId::GraphicsPreview {
        app.output_preview_dirty = true;
    } else {
        app.mark_args_dirty();
    }
}

fn adjust_control(app: &mut App, delta: i32) {
    let id = app.get_current_control_id();
    if app.control_disabled_reason(id).is_some() { return; }
    let def = get_control_def(id);

    match def.kind {
        ControlKind::Slider { min, max, .. } => {
            let val = app.get_value(id);
            app.set_value(id, (val + delta).clamp(min, max));
        }
        ControlKind::Enum { options, .. } => {
            let val = app.get_value(id);
            let count = options.len() as i32;
            if delta > 0 {
                app.set_value(id, (val + 1) % count);
            } else {
                app.set_value(id, (val + count - 1) % count);
            }
        }
        ControlKind::FontSelector => {
            app.open_font_picker();
            app.move_font_picker_selection(delta.signum());
            return;
        }
        ControlKind::Toggle { .. } => {
            // Toggles don't adjust by delta normally, but let's toggle if delta != 0
            if delta != 0 {
                let val = app.get_value(id);
                app.set_value(id, if val == 0 { 1 } else { 0 });
            }
        }
        ControlKind::EyedropperWidget | ControlKind::Action => {}
    }

    log::debug!(
        "TUI Adjusted: tab={}, ctrl={}, id={:?}",
        app.current_tab,
        app.selected_control,
        id
    );
    mark_control_changed(app, id);
}

fn set_control_value(app: &mut App, visual_idx: usize, val: i32) {
    let id = if app.current_tab == 0 {
        let ctrls = app.tab_controls(0);
        ctrls[visual_idx.min(ctrls.len() - 1)].id
    } else {
        return;
    };

    if app.control_disabled_reason(id).is_some() { return; }
    app.set_value(id, val);

    log::debug!(
        "Value set: tab={}, visual={}, id={:?}, val={}",
        app.current_tab,
        visual_idx,
        id,
        val
    );
    mark_control_changed(app, id);
}

fn toggle_control(app: &mut App) {
    let id = app.get_current_control_id();
    if app.control_disabled_reason(id).is_some() { return; }
    let def = get_control_def(id);

    match def.kind {
        ControlKind::Toggle { .. } => {
            let val = app.get_value(id);
            app.set_value(id, if val == 0 { 1 } else { 0 });
            if id == ControlId::Ocr {
                if let Some(index) = app
                    .tab_controls(0)
                    .iter()
                    .position(|control| control.id == ControlId::Ocr)
                {
                    app.selected_control = index;
                }
            }
        }
        ControlKind::Enum { options, .. } => {
            let val = app.get_value(id);
            app.set_value(id, (val + 1) % options.len() as i32);
        }
        ControlKind::FontSelector => {
            app.open_font_picker();
            return;
        }
        ControlKind::Action => {
            if id == ControlId::AutoOptimize {
                app.perform_auto_optimize();
            }
        }
        _ => {}
    }

    log::info!(
        "TUI Toggled: tab={}, visual={}, id={:?}",
        app.current_tab,
        app.selected_control,
        id
    );
    mark_control_changed(app, id);
}

fn reset_control(app: &mut App) {
    let id = app.get_current_control_id();
    if app.control_disabled_reason(id).is_some() { return; }
    let def = get_control_def(id);

    match def.kind {
        ControlKind::Slider { default, .. } => app.set_value(id, default),
        ControlKind::Toggle { default } => app.set_value(id, if default { 1 } else { 0 }),
        ControlKind::Enum { default, .. } => app.set_value(id, default as i32),
        ControlKind::FontSelector => app.font_idx = 0,
        ControlKind::EyedropperWidget | ControlKind::Action => {}
    }

    log::info!("Control reset: tab={}, id={:?}", app.current_tab, id);
    mark_control_changed(app, id);
}

// ── UI Rendering ─────────────────────────────────────────────────────────────
fn ui(f: &mut ratatui::Frame, app: &mut App) {
    let area = f.area();
    app.terminal_area = area;
    app.ensure_visible_focus();

    // Background
    f.render_widget(Block::default().style(Style::new().bg(BG)), area);

    // Layout: [title_bar | main_content | status_bar]
    let root = Layout::vertical([
        Constraint::Length(1), // title/tabs
        Constraint::Min(0),    // content
        Constraint::Length(1), // status bar
    ])
    .split(area);

    // ── Title bar with tabs ──
    render_title_bar(f, app, root[0]);

    app.control_areas.clear();
    app.general_area = Rect::default();
    app.pipeline_column_area = Rect::default();
    app.pipeline_active_area = Rect::default();
    app.glyphs_area = Rect::default();
    app.tree_area = Rect::default();
    app.glyph_list_area = Rect::default();
    app.preview_area = Rect::default();
    app.splitter_areas.clear();

    let splitter_style = Style::new().fg(BORDER).bg(BG);

    let left_width = match app.current_tab {
        0 => app.general_pane_width,
        1 => app.pipeline_pane_width,
        2 => app.glyphs_pane_width,
        3 => app.text_pane_width,
        _ => 30,
    };

    let panes = Layout::horizontal([
        Constraint::Length(left_width),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(root[1]);

    app.output_area = panes[2];

    match app.current_tab {
        0 => {
            app.general_area = panes[0];
            render_controls(f, app, panes[0]);
            f.render_widget(Block::default().style(splitter_style), panes[1]);
            app.splitter_areas.push(SplitterArea {
                kind: SplitterKind::General,
                area: panes[1],
            });
        }
        1 => {
            app.pipeline_column_area = panes[0];
            render_active_pipeline_list(f, app, panes[0]);
            f.render_widget(Block::default().style(splitter_style), panes[1]);
            app.splitter_areas.push(SplitterArea {
                kind: SplitterKind::Pipeline,
                area: panes[1],
            });
        }
        2 => {
            app.glyphs_area = panes[0];
            render_glyphs_tab(f, app, panes[0]);
            f.render_widget(Block::default().style(splitter_style), panes[1]);
            app.splitter_areas.push(SplitterArea {
                kind: SplitterKind::Glyphs,
                area: panes[1],
            });
        }
        3 => {
            app.text_area = panes[0];
            render_text_tab(f, app, panes[0]);
            f.render_widget(Block::default().style(splitter_style), panes[1]);
            app.splitter_areas.push(SplitterArea {
                kind: SplitterKind::Text,
                area: panes[1],
            });
        }
        _ => {
            app.output_area = root[1];
        }
    }

    render_output(f, app, panes[2]);

    // ── Status bar ──
    render_status_bar(f, app, root[2]);

    // ── Font Picker Overlay ──
    if app.show_font_picker {
        let output_area = app.output_area;
        let height = (output_area.height / 2).max(5).min(output_area.height);
        let area = Rect::new(
            output_area.x,
            output_area.y + output_area.height.saturating_sub(height),
            output_area.width,
            height,
        );

        f.render_widget(Clear, area);

        let title = if app.font_search.is_empty() {
            " Font: type to filter, Enter to apply, Esc to cancel ".to_string()
        } else {
            format!(
                " Font: {} (Enter to apply, Esc to cancel) ",
                app.font_search
            )
        };

        let block = Block::default()
            .title(title)
            .borders(Borders::TOP | Borders::LEFT | Borders::RIGHT) // Maybe just TOP?
            .style(Style::new().bg(SURFACE));

        // List
        let matching_indices = app.matching_font_indices();
        let items: Vec<ListItem> = matching_indices
            .iter()
            .map(|&idx| {
                let suffix = if idx == app.font_idx { "  (current)" } else { "" };
                ListItem::new(format!("{}{}", app.fonts[idx], suffix))
                    .style(Style::new().fg(TEXT))
            })
            .collect();

        if matching_indices.is_empty() {
            app.font_list_state.select(None);
        }

        let list = List::new(items)
            .block(block)
            .highlight_style(Style::new().add_modifier(Modifier::REVERSED));

        f.render_stateful_widget(list, area, &mut app.font_list_state);
    }

    if app.show_unicode_explorer {
        let area = Rect::new(
            (f.area().width.saturating_sub(80)) / 2,
            (f.area().height.saturating_sub(30)) / 2,
            80.min(f.area().width),
            30.min(f.area().height),
        );
        f.render_widget(Clear, area);
        render_unicode_explorer(f, app, area);
    }

    if app.input_mode == InputMode::AutoOptimize {
        let dialog_width = 84.min(area.width.saturating_sub(2));
        let dialog_height = 37.min(area.height.saturating_sub(2));
        let overlay_area = Rect::new(
            area.x + area.width.saturating_sub(dialog_width) / 2,
            area.y + area.height.saturating_sub(dialog_height) / 2,
            dialog_width,
            dialog_height,
        );
        f.render_widget(Clear, overlay_area);
        render_auto_optimize_dialog(f, app, overlay_area);
    } else if app.is_optimizing {
        let th = 5;
        let p_area = Rect::new(
            app.output_area.x.saturating_add(4),
            app.output_area
                .y
                .saturating_add(app.output_area.height)
                .saturating_sub(th + 2),
            app.output_area.width.saturating_sub(8),
            th,
        );

        f.render_widget(Clear, p_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .title(" ⭐ Auto-Optimization Running (Press Esc to Stop) ")
            .style(Style::default().fg(ratatui::style::Color::Cyan).bg(BG));

        let progress_text = app.optimization_progress.clone();
        let content = Paragraph::new(progress_text)
            .block(block)
            .alignment(Alignment::Center);

        f.render_widget(content, p_area);
    }

    // Render text-box guides and cursors after the output so they remain visible.
    if app.current_tab == 3 {
        if let Some(sel) = app.text_selected {
            if sel > 0 && sel <= app.render_args.overlays.len() {
                let ov = &app.render_args.overlays[sel - 1];
                let (box_w, box_h) = text_overlay_box_dimensions(ov);
                if app.text_drag_mode == TextDragMode::Sizing {
                    // While establishing dimensions, show the exact cells that
                    // will belong to the box instead of an ambiguous outer border.
                    let sizing_style = Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::DIM);
                    for dy in 0..box_h {
                        for dx in 0..box_w {
                            set_output_preview_cell(
                                f,
                                app,
                                ov.x + dx,
                                ov.y + dy,
                                '▒',
                                sizing_style,
                            );
                        }
                    }
                }
            }
        }

        // A box outline is transient: it appears only while that box is hovered.
        if app.text_drag_mode != TextDragMode::Sizing {
            if let Some(idx) = app.text_hovered {
                if let Some(ov) = app.render_args.overlays.get(idx) {
                    let (box_w, box_h) = text_overlay_box_dimensions(ov);
                    let border_style = Style::default().fg(if app.text_resize_hover == Some(idx) {
                        Color::Yellow
                    } else {
                        Color::Cyan
                    });
                    set_output_preview_cell(f, app, ov.x - 1, ov.y - 1, '┌', border_style);
                    set_output_preview_cell(f, app, ov.x + box_w, ov.y - 1, '┐', border_style);
                    set_output_preview_cell(f, app, ov.x - 1, ov.y + box_h, '└', border_style);
                    set_output_preview_cell(f, app, ov.x + box_w, ov.y + box_h, '┘', border_style);
                    for dx in 0..box_w {
                        set_output_preview_cell(f, app, ov.x + dx, ov.y - 1, '─', border_style);
                        set_output_preview_cell(f, app, ov.x + dx, ov.y + box_h, '─', border_style);
                    }
                    for dy in 0..box_h {
                        set_output_preview_cell(f, app, ov.x - 1, ov.y + dy, '│', border_style);
                        set_output_preview_cell(f, app, ov.x + box_w, ov.y + dy, '│', border_style);
                    }
                }
            }
        }

        // Hovering either side of the lower-right corner reveals the diagonal
        // resize affordance one cell down and right from the box corner.
        if app.text_drag_mode != TextDragMode::Sizing {
            if let Some(idx) = app.text_resize_hover {
                if let Some(ov) = app.render_args.overlays.get(idx) {
                    let (box_w, box_h) = text_overlay_box_dimensions(ov);
                    set_output_preview_cell(
                        f,
                        app,
                        ov.x + box_w + 1,
                        ov.y + box_h + 1,
                        '🢆',
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    );
                }
            }
        }

        if (app.render_tick / 8).is_multiple_of(2) {
            if let Some((cursor_x, cursor_y)) = app.text_cursor_anchor {
                set_output_preview_cell(
                    f,
                    app,
                    cursor_x,
                    cursor_y,
                    '_',
                    Style::default()
                        .fg(Color::White)
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                );
            } else if app.input_mode == InputMode::TextEdit {
                if let Some(sel) = app.text_selected {
                    if let Some(ov) = sel
                        .checked_sub(1)
                        .and_then(|idx| app.render_args.overlays.get(idx))
                    {
                        let (cursor_x, cursor_y) =
                            text_cursor_grid_position(ov, &app.value_edit_buffer);
                        set_output_preview_cell(
                            f,
                            app,
                            cursor_x,
                            cursor_y,
                            '_',
                            Style::default()
                                .fg(Color::White)
                                .bg(Color::DarkGray)
                                .add_modifier(Modifier::BOLD),
                        );
                    }
                }
            }
        }
    }
}

fn render_title_bar(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    let layout = Layout::horizontal([
        Constraint::Length(10), // app title
        Constraint::Length(40), // tabs
        Constraint::Min(0),     // file + spinner
    ])
    .split(area);

    // App title
    let title = Span::styled(
        " img2irc ",
        Style::new()
            .fg(Color::White)
            .bg(ACCENT)
            .add_modifier(Modifier::BOLD),
    );
    f.render_widget(Paragraph::new(Line::from(title)), layout[0]);

    // Tabs
    let tab_names = [" General ", " Pipeline ", " Glyphs ", " Text "];
    let tabs = Tabs::new(
        tab_names
            .iter()
            .cloned()
            .map(Line::from)
            .collect::<Vec<_>>(),
    )
    .select(app.current_tab)
    .style(Style::default().fg(TEXT_DIM).bg(BG))
    .highlight_style(
        Style::default()
            .fg(Color::White)
            .bg(TAB_ACTIVE)
            .add_modifier(Modifier::BOLD),
    )
    .divider("");

    f.render_widget(tabs, layout[1]);

    // Update tab_areas for mouse clicks
    let mut tab_rects = Vec::new();
    let mut x = layout[1].x;
    for name in tab_names {
        let w = (name.len() + 2) as u16; // +2 for ratatui default left/right padding
        tab_rects.push(Rect::new(x, layout[1].y, w, 1));
        x += w;
    }
    app.tab_areas = tab_rects;

    // Show image path after tabs
    let x = layout[2].x + 1;
    let path_display = format!("📁 {}", app.image_path);
    let right_edge = area.x + area.width;

    // Build right text (timings + spinner)
    let mut right_text = String::new();
    if let Some(end_to_end) = app.last_render_latency {
        let elapsed = if end_to_end.as_secs_f64() >= 10.0 {
            format!("{:.1}s", end_to_end.as_secs_f64())
        } else {
            format!("{}ms", end_to_end.as_millis())
        };
        right_text.push_str(&format!(" {}", elapsed));
        if let Some(res) = &app.cached_result {
            let initial_ms = res.initial_time.as_millis();
            if initial_ms > 0 {
                right_text.push_str(&format!(" ({}ms)", initial_ms));
            }
        }
        right_text.push(' ');
    }

    let is_rendering = app.is_rendering.load(Ordering::Relaxed);
    if is_rendering {
        let spinners = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let spinner_ch = spinners[(app.render_tick / 3) % spinners.len()];
        right_text.push_str(&format!(" {} Rendering ", spinner_ch));
    }

    let right_text_len = right_text.chars().count() as u16; // use chars().count() instead of len() for unicode
    let avail = right_edge.saturating_sub(x).saturating_sub(right_text_len) as usize;

    let truncated = if path_display.len() > avail {
        if avail > 3 {
            format!(
                "…{}",
                &app.image_path[app.image_path.len().saturating_sub(avail.saturating_sub(3))..]
            )
        } else {
            "…".to_string()
        }
    } else {
        path_display
    };
    let path_area = Rect::new(x, area.y, truncated.chars().count() as u16, 1);
    app.path_area = path_area;
    f.buffer_mut()
        .set_string(x, area.y, &truncated, Style::new().fg(TEXT_DIM).bg(BG));

    if !right_text.is_empty() {
        let sx = right_edge.saturating_sub(right_text_len);
        f.buffer_mut().set_string(
            sx,
            area.y,
            &right_text,
            Style::new()
                .fg(if is_rendering {
                    Color::Yellow
                } else {
                    TEXT_DIM
                })
                .bg(BG)
                .add_modifier(if is_rendering {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        );
    }
}

fn render_output(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    app.output_area = area;

    // 1. Render normal background content
    render_normal_output_content(f, app, area);

    // 2. Overlay Explorer if active
    if app.explorer_dialog.is_some() {
        let mut dialog = app.explorer_dialog.take().unwrap();
        let layout =
            Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).split(area);
        let overlay_area = layout[1];

        f.render_widget(Clear, overlay_area);
        render_explorer_dialog(f, app, &mut dialog, overlay_area);
        app.explorer_dialog = Some(dialog);
    }

    // 3. Overlay Metadata Prompt if active
    if app.input_mode == InputMode::MetadataPrompt {
        let layout =
            Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).split(area);
        let overlay_area = layout[1];
        f.render_widget(Clear, overlay_area);
        render_metadata_prompt(f, app, overlay_area);
    }
    if let Some(dialog) = &mut app.replace_colour_dialog {
        render_replace_colour(f, dialog);
    }
    if let Some(picker) = &mut app.figlet_picker {
        render_figlet_picker(f, picker);
    }
}

fn render_explorer_dialog(
    f: &mut ratatui::Frame,
    app: &mut App,
    dialog: &mut ExplorerDialog,
    area: Rect,
) {
    let main_chunks = if matches!(dialog.mode, ExplorerMode::Open) {
        vec![area]
    } else {
        Layout::vertical([Constraint::Min(0), Constraint::Length(3)])
            .split(area)
            .to_vec()
    };

    let explorer_area = main_chunks[0];
    let chunks = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(explorer_area);
    app.explorer_area = chunks[0];

    // Explorer border
    let mut explorer_block = Block::default()
        .borders(Borders::ALL)
        .border_set(symbols::border::ROUNDED)
        .title(" Explorer ");

    if !dialog.focus_on_input {
        explorer_block = explorer_block.border_style(Style::default().fg(ACCENT));
    }

    f.render_widget(&dialog.explorer.widget(), chunks[0]);
    f.render_widget(explorer_block, chunks[0]);

    // Preview in right half
    let file = dialog.explorer.current();
    let path = file.path();
    let is_image = path.to_string_lossy().to_lowercase().ends_with(".png")
        || path.to_string_lossy().to_lowercase().ends_with(".jpg")
        || path.to_string_lossy().to_lowercase().ends_with(".jpeg")
        || path.to_string_lossy().to_lowercase().ends_with(".gif")
        || path.to_string_lossy().to_lowercase().ends_with(".bmp")
        || path.to_string_lossy().to_lowercase().ends_with(".webp");

    if is_image {
        let path_str = path.to_string_lossy().into_owned();
        if app.last_preview_path.as_ref() != Some(&path_str) {
            if let Ok(img) = image::open(&path_str) {
                let protocol = app.preview_picker.new_resize_protocol(img);
                app.image_preview_protocol = Some(protocol);
                app.last_preview_path = Some(path_str);
            }
        }

        if let Some(protocol) = &mut app.image_preview_protocol {
            let inner_area = chunks[1].inner(ratatui::layout::Margin {
                vertical: 1,
                horizontal: 1,
            });
            protocol.resize_encode(&Resize::Fit(None), inner_area);
            let widget = StatefulImage::default();
            f.render_stateful_widget(widget, inner_area, protocol);
        }
    } else {
        f.render_widget(
            Paragraph::new("No preview available").alignment(Alignment::Center),
            chunks[1],
        );
    }

    if !matches!(dialog.mode, ExplorerMode::Open) {
        let input_area = main_chunks[1];
        if dialog.focus_on_input {
            dialog.name_input.set_block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(ACCENT))
                    .title(" Filename (Enter to Save) "),
            );
        } else {
            dialog
                .name_input
                .set_block(Block::default().borders(Borders::ALL).title(" Filename "));
        }
        f.render_widget(&dialog.name_input, input_area);
    }
}

fn render_metadata_prompt(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT))
        .title(" Metadata Found! ");

    let target = &app.metadata_prompt_path;
    let message = format!(
        "This image contains img2irc metadata!\n\n\
        The original input image can be found at:\n\
        {}\n\n\
        Would you like to load this into the workspace?\n\n\
        [Y/Enter] Load Settings    [N/Esc] Load Image Only",
        target
    );

    let p = ratatui::widgets::Paragraph::new(message)
        .block(block)
        .alignment(ratatui::layout::Alignment::Center)
        .wrap(ratatui::widgets::Wrap { trim: true });

    f.render_widget(p, area);
}

fn render_normal_output_content(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    let is_rendering = app.is_rendering.load(Ordering::Relaxed);
    // Title with status
    let title_status = if is_rendering {
        let spinners = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let spinner = spinners[(app.render_tick / 3) % spinners.len()];
        format!(" Output {} ", spinner)
    } else {
        " Output ".to_string()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(BORDER))
        .border_set(symbols::border::ROUNDED)
        .title(Span::styled(
            title_status,
            Style::new().fg(TEXT).add_modifier(Modifier::BOLD),
        ));

    if app.cached_render.is_empty() && is_rendering {
        // Only show full-panel spinner if no content yet
        let loading_text = "\n\n\n      Rendering...".to_string();
        let p = Paragraph::new(loading_text)
            .block(block)
            .alignment(Alignment::Center)
            .style(Style::new().fg(ACCENT));
        f.render_widget(p, area);
    } else {
        // Use save_content (IRC) stats when available, otherwise cached_render (ANSI)
        let stats_content = app
            .cached_result
            .as_ref()
            .and_then(|r| r.save_content.as_ref())
            .unwrap_or(&app.cached_render);
        //let max_bytes = stats_content.lines().map(|l| l.len()).max().unwrap_or(0);

        // also get the index of the longest line
        let (longest_line_idx, max_bytes) = stats_content
            .lines()
            .enumerate()
            .map(|(i, l)| (i, l.len()))
            .max_by_key(|&(_, len)| len)
            .unwrap_or((0, 0));

        let width_chars = app
            .cached_result
            .as_ref()
            .and_then(|r| r.grid.first())
            .map_or(0, |row| row.len());
        let height_chars = app.cached_result.as_ref().map_or(0, |r| r.grid.len());

        let (cw, ch) = app.arc_glyphs.as_ref().map_or((0, 0), |g| g.metrics);
        let (fw, fh) = app
            .arc_glyphs
            .as_ref()
            .map_or((0.0, 0.0), |g| g.float_metrics);
        let w_px = width_chars * cw;
        let h_px = height_chars * ch;
        let p_score = app.cached_result.as_ref().map_or(0.0, |r| r.score);

        let mut title_right = format!(
            " Output: {}x{} chars ({}x{} px) | Cell: {}x{} px | Advance: {:.1}px | Line height: {:.1}px | Line {} ({} bytes) | Contour: {:.3} ",
            width_chars, height_chars, w_px, h_px, cw, ch, fw, fh, longest_line_idx, max_bytes, p_score
        );
        if app.graphics_preview {
            title_right.push_str(&format!(
                "| Pixel preview: {:?} ",
                app.preview_picker.protocol_type()
            ));
        }

        if let Some((r, g, b)) = app.eyedropper_color {
            let col_u32 = ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
            let irc_code = crate::draw::nearest_hex_colour_fast(col_u32, &crate::palette::IRC99);
            let ansi_code = crate::draw::nearest_hex_colour_fast(col_u32, &crate::palette::ANSI256);
            title_right.push_str(&format!(
                "| #{:02X}{:02X}{:02X} IRC:{:02} ANSI:{:03} ",
                r, g, b, irc_code, ansi_code
            ));
        }

        let block_with_bottom_title = block
            .title_bottom(Line::from(Span::styled(
                title_right,
                Style::new().fg(TEXT_DIM),
            )))
            .style(Style::new().bg(BG));

        let inner = block_with_bottom_title.inner(area);
        f.render_widget(block_with_bottom_title, area);

        if app.graphics_preview {
            if app.output_preview_dirty {
                let image = app
                    .cached_result
                    .as_ref()
                    .zip(app.arc_glyphs.as_deref())
                    .and_then(|(result, glyphs)| draw::render_result_image(result, glyphs));
                app.output_preview_protocol =
                    image.map(|image| app.preview_picker.new_resize_protocol(image));
                app.output_preview_dirty = false;
            }

            if let Some(protocol) = &mut app.output_preview_protocol {
                let resize = Resize::Crop(None);
                let preview_area = centered_rect(inner, protocol.size_for(resize.clone(), inner));
                protocol.resize_encode(&resize, preview_area);
                f.render_stateful_widget(StatefulImage::default(), preview_area, protocol);
            } else {
                f.render_widget(
                    Paragraph::new("Graphics preview is waiting for rendered output")
                        .alignment(Alignment::Center)
                        .style(Style::new().fg(TEXT_DIM)),
                    inner,
                );
            }
        } else {
            let text = app
                .cached_render
                .as_bytes()
                .into_text()
                .unwrap_or_else(|_| ratatui::text::Text::from(app.cached_render.clone()));

            // Vertical center calculation
            let line_count = app.cached_render.lines().count() as u16;
            app.scroll = clamp_output_scroll(app.scroll, line_count, inner.height);
            let pad_y = if line_count < inner.height {
                inner.height.saturating_sub(line_count).saturating_div(2)
            } else {
                0
            };

            let layout =
                Layout::vertical([Constraint::Length(pad_y), Constraint::Min(0)]).split(inner);

            let output = Paragraph::new(text)
                .alignment(Alignment::Center)
                .scroll((app.scroll, 0));

            f.render_widget(output, layout[1]);
        }

        // Selection Pane at bottom of output
        render_options_panel(f, app, inner);
    }
}

#[derive(PartialEq, Eq)]
enum TextDragMode {
    None,
    Moving,
    Sizing,
}

fn text_content_dimensions(text: &str, wrap_width: Option<usize>) -> (i32, i32) {
    let lines = crate::draw::layout_overlay_text(text, wrap_width);
    let width = lines
        .iter()
        .map(|line| line.len() as i32)
        .max()
        .unwrap_or(1)
        .max(1);
    (width, lines.len().max(1) as i32)
}

fn default_text_foreground(render_mode_idx: usize) -> crate::args::ColorSpec {
    match render_mode_idx {
        0 => crate::args::ColorSpec::Index(0),  // IRC white
        1 => crate::args::ColorSpec::Index(15), // ANSI bright white
        _ => crate::args::ColorSpec::Rgb([255, 255, 255]),
    }
}

fn new_text_overlay(
    render_mode_idx: usize,
    x: i32,
    y: i32,
    auto_grow: bool,
) -> crate::args::TextOverlay {
    crate::args::TextOverlay {
        text: String::new(),
        source_text: None,
        figlet_font: None,
        x,
        y,
        w: 1,
        h: 1,
        fg: Some(default_text_foreground(render_mode_idx)),
        bg: None,
        wrap: true,
        auto_grow,
        transparent_spaces: false,
        bold: false,
        italic: false,
        underline: false,
    }
}

fn begin_auto_growing_text_at_anchor(app: &mut App, initial_text: String) {
    let Some((x, y)) = app.text_cursor_anchor.take() else {
        return;
    };
    app.last_text_click = None;
    let mut overlay = new_text_overlay(app.render_mode_idx, x, y, true);
    overlay.text = initial_text.clone();
    sync_text_overlay_auto_grow(&mut overlay);
    app.render_args.overlays.push(overlay);
    app.text_selected = Some(app.render_args.overlays.len());
    app.text_list_state.select(app.text_selected);
    app.text_field_focus = TextFieldFocus::TextInput;
    app.value_edit_buffer = initial_text;
    app.input_mode = InputMode::TextEdit;
    app.reapply_overlays();
}

fn figlet_font_lists(app: &App) -> Vec<crate::args::OcrFigletFontList> {
    if app.render_args.ocr_figlet_fonts.is_empty() {
        crate::args::default_ocr_figlet_font_lists()
    } else {
        app.render_args.ocr_figlet_fonts.clone()
    }
}

fn figlet_font_names(app: &App) -> Vec<String> {
    let mut names = Vec::new();
    for font in figlet_font_lists(app)
        .into_iter()
        .flat_map(|list| list.fonts)
    {
        if !font.eq_ignore_ascii_case("plain") && !names.contains(&font) {
            names.push(font);
        }
    }
    names
}

fn refresh_text_overlay_figlet(app: &mut App, index: usize) {
    let font_lists = figlet_font_lists(app);
    if let Some(overlay) = app.render_args.overlays.get_mut(index) {
        crate::ocr::refresh_figlet_overlay(overlay, &font_lists);
    }
}

fn toggle_text_overlay_figlet(app: &mut App, index: usize) {
    if !cfg!(feature = "ocr") {
        return;
    }
    let Some(overlay) = app.render_args.overlays.get(index) else {
        return;
    };
    let enabled = overlay.figlet_enabled();
    let was_auto_grow = overlay.auto_grow;
    let default_font = figlet_font_names(app).into_iter().next();
    let available = app.cached_result.as_ref().map(|result| {
        (
            result.grid.first().map_or(1, Vec::len) as i32,
            result.grid.len() as i32,
        )
    });

    let overlay = &mut app.render_args.overlays[index];
    if enabled {
        if let Some(source) = overlay.source_text.take() {
            overlay.text = source;
        }
        overlay.figlet_font = None;
        overlay.auto_grow = true;
        overlay.wrap = true;
        overlay.transparent_spaces = false;
        sync_text_overlay_auto_grow(overlay);
    } else {
        overlay.source_text = Some(overlay.text.clone());
        overlay.figlet_font = default_font;
        overlay.auto_grow = false;
        overlay.wrap = false;
        overlay.transparent_spaces = true;
        if was_auto_grow {
            let (grid_w, grid_h) = available.unwrap_or((40, 8));
            overlay.w = overlay.w.max(24.min((grid_w - overlay.x).max(1)));
            overlay.h = overlay.h.max(6.min((grid_h - overlay.y).max(1)));
        }
    }
    if !enabled {
        refresh_text_overlay_figlet(app, index);
    }
    app.reapply_overlays();
}

fn cycle_text_overlay_figlet_font(app: &mut App, index: usize, delta: i32) {
    let names = figlet_font_names(app);
    if names.is_empty()
        || !app
            .render_args
            .overlays
            .get(index)
            .is_some_and(crate::args::TextOverlay::figlet_enabled)
    {
        return;
    }
    let current = app.render_args.overlays[index]
        .figlet_font
        .as_deref()
        .and_then(|font| names.iter().position(|candidate| candidate == font))
        .unwrap_or(0) as i32;
    let next = (current + delta).rem_euclid(names.len() as i32) as usize;
    app.render_args.overlays[index].figlet_font = Some(names[next].clone());
    refresh_text_overlay_figlet(app, index);
    app.reapply_overlays();
}

fn sync_text_overlay_auto_grow(overlay: &mut crate::args::TextOverlay) {
    if overlay.auto_grow {
        let (width, height) = text_content_dimensions(&overlay.text, None);
        overlay.w = width.max(1);
        overlay.h = height.max(1);
    }
}

fn apply_text_edit_buffer(app: &mut App) {
    let Some(sel) = app.text_selected else {
        return;
    };
    if sel == 0 || sel > app.render_args.overlays.len() {
        return;
    }
    let index = sel - 1;
    let figlet = app.render_args.overlays[index].figlet_enabled();
    let overlay = &mut app.render_args.overlays[index];
    if figlet {
        overlay.source_text = Some(app.value_edit_buffer.clone());
    } else {
        overlay.text = app.value_edit_buffer.clone();
        sync_text_overlay_auto_grow(overlay);
    }
    if figlet {
        refresh_text_overlay_figlet(app, index);
    }
    app.reapply_overlays();
}

fn finish_text_edit(app: &mut App) {
    apply_text_edit_buffer(app);
    app.input_mode = InputMode::Normal;
    app.value_edit_buffer.clear();
}

fn text_overlay_box_dimensions(overlay: &crate::args::TextOverlay) -> (i32, i32) {
    let wrap_width = (overlay.wrap && overlay.w > 0).then_some(overlay.w as usize);
    let (text_width, text_height) = text_content_dimensions(&overlay.text, wrap_width);
    (
        if overlay.w > 0 {
            overlay.w
        } else {
            text_width.max(1)
        },
        if overlay.h > 0 {
            overlay.h
        } else {
            text_height.max(1)
        },
    )
}

fn text_overlay_contains(overlay: &crate::args::TextOverlay, cell_x: i32, cell_y: i32) -> bool {
    let (width, height) = text_overlay_box_dimensions(overlay);
    cell_x >= overlay.x
        && cell_x < overlay.x + width
        && cell_y >= overlay.y
        && cell_y < overlay.y + height
}

fn text_overlay_hover_contains(
    overlay: &crate::args::TextOverlay,
    cell_x: i32,
    cell_y: i32,
) -> bool {
    let (width, height) = text_overlay_box_dimensions(overlay);
    cell_x >= overlay.x - 1
        && cell_x <= overlay.x + width
        && cell_y >= overlay.y - 1
        && cell_y <= overlay.y + height
}

fn text_overlay_resize_hotspot(
    overlay: &crate::args::TextOverlay,
    cell_x: i32,
    cell_y: i32,
) -> bool {
    let (width, height) = text_overlay_box_dimensions(overlay);
    let inner_corner = cell_x == overlay.x + width - 1 && cell_y == overlay.y + height - 1;
    let right_of_corner = cell_x == overlay.x + width && cell_y == overlay.y + height - 1;
    let below_corner = cell_x == overlay.x + width - 1 && cell_y == overlay.y + height;
    inner_corner || right_of_corner || below_corner
}

fn output_grid_position(app: &App, screen_x: u16, screen_y: u16) -> Option<(i32, i32)> {
    let result = app.cached_result.as_ref()?;
    let inner_height = app.output_area.height.saturating_sub(2);
    let inner_width = app.output_area.width.saturating_sub(2);
    let grid_height = result.grid.len() as u16;
    let pad_y = if grid_height < inner_height {
        inner_height.saturating_sub(grid_height) / 2
    } else {
        0
    };
    let content_y = screen_y
        .checked_sub(app.output_area.y + 1 + pad_y)?
        .saturating_add(app.scroll) as usize;
    let row = result.grid.get(content_y)?;
    let row_width = row.len() as u16;
    let pad_x = if row_width < inner_width {
        (inner_width - row_width) / 2
    } else {
        0
    };
    let content_x = screen_x.checked_sub(app.output_area.x + 1 + pad_x)? as usize;
    if content_x >= row.len() {
        return None;
    }
    Some((content_x as i32, content_y as i32))
}

fn grid_output_position(app: &App, grid_x: i32, grid_y: i32) -> Option<(u16, u16)> {
    if grid_x < 0 || grid_y < 0 || grid_y < app.scroll as i32 {
        return None;
    }
    let result = app.cached_result.as_ref()?;
    let inner_height = app.output_area.height.saturating_sub(2);
    let inner_width = app.output_area.width.saturating_sub(2);
    let grid_height = result.grid.len() as u16;
    let grid_width = result.grid.first().map_or(0, |row| row.len()) as u16;
    let pad_y = if grid_height < inner_height {
        inner_height.saturating_sub(grid_height) / 2
    } else {
        0
    };
    let pad_x = if grid_width < inner_width {
        (inner_width - grid_width) / 2
    } else {
        0
    };
    let screen_x = app.output_area.x + 1 + pad_x + grid_x as u16;
    let screen_y = app.output_area.y + 1 + pad_y + (grid_y as u16).saturating_sub(app.scroll);
    let right = app.output_area.x + app.output_area.width.saturating_sub(1);
    let bottom = app.output_area.y + app.output_area.height.saturating_sub(1);
    (screen_x < right && screen_y < bottom).then_some((screen_x, screen_y))
}

fn text_cursor_grid_position(overlay: &crate::args::TextOverlay, text: &str) -> (i32, i32) {
    let wrap_width =
        (overlay.wrap && !overlay.auto_grow && overlay.w > 0).then_some(overlay.w as usize);
    let lines = crate::draw::layout_overlay_text(text, wrap_width);
    let mut line_y = lines.len().saturating_sub(1) as i32;
    let mut line_x = lines.last().map_or(0, |line| line.len()) as i32;
    if !overlay.auto_grow && overlay.w > 0 && line_x >= overlay.w {
        line_x = 0;
        line_y += 1;
    }
    (overlay.x + line_x, overlay.y + line_y)
}

fn set_output_preview_cell(
    frame: &mut ratatui::Frame,
    app: &App,
    grid_x: i32,
    grid_y: i32,
    ch: char,
    style: Style,
) {
    let Some((screen_x, screen_y)) = grid_output_position(app, grid_x, grid_y) else {
        return;
    };
    if let Some(cell) = frame
        .buffer_mut()
        .cell_mut(ratatui::layout::Position::new(screen_x, screen_y))
    {
        cell.set_style(style);
        cell.set_char(ch);
    }
}

fn render_status_bar(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    let keys = [
        ("Tab", "Switch"),
        ("↑↓", "Navigate"),
        ("←→", "Adjust"),
        ("Space", "Toggle"),
        ("Del", "Reset"),
        ("Shift", "×10"),
        ("g/p/y", "Panes"),
        ("Ctrl+O", "Open"),
        ("Ctrl+V", "Paste"),
        ("Ctrl+S", "Save"),
        ("Ctrl+P", "Png"),
        ("Ctrl+Q", "Quit"),
    ];

    let spans: Vec<Span> = keys
        .iter()
        .flat_map(|(key, desc)| {
            vec![
                Span::styled(format!(" {} ", key), Style::new().fg(SURFACE).bg(ACCENT)),
                Span::styled(format!(" {} ", desc), Style::new().fg(TEXT).bg(SURFACE)),
                Span::raw(" "),
            ]
        })
        .collect();

    let mut full_spans = vec![];
    if let Some(res) = &app.cached_result {
        let display_score = if app.is_optimizing {
            app.optimization_best_score.unwrap_or(res.score)
        } else {
            res.score
        };
        full_spans.push(Span::styled(
            format!(" Contour {:.3} ", display_score),
            Style::new()
                .fg(Color::White)
                .bg(ACCENT_DIM)
                .add_modifier(Modifier::BOLD),
        ));
        full_spans.push(Span::raw(" "));
    }
    if let Some(status) = &app.clipboard_paste_status {
        full_spans.push(Span::styled(
            format!(" {} ", status),
            Style::new().fg(Color::White).bg(STATUS_BG),
        ));
        full_spans.push(Span::raw(" "));
    }
    full_spans.extend(spans);

    let line = Line::from(full_spans);
    let bar = Paragraph::new(line).style(Style::new().bg(STATUS_BG));
    f.render_widget(bar, area);
}
