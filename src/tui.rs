pub mod general;
pub mod glyphs;
pub mod pipeline;
pub mod text;
pub use general::*;
pub use glyphs::*;
pub use pipeline::*;
pub use text::*;

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{
    error::Error,
    io::{self},
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

use crate::{
    args::{ColourSpace, Encoding, Render, RenderArgs, SamplingFilter},
    draw, font,
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Tabs},
    Terminal,
};
use ratatui_explorer::FileExplorer;
use ratatui_image::{
    picker::Picker, protocol::StatefulProtocol, Resize, ResizeEncodeRender, StatefulImage,
};
use simplelog::*;
use std::fs::File;
use std::process::Command;
use std::time::Instant;
use tokio::sync::{mpsc, watch};
use tui_textarea::TextArea;

// ── Theme ────────────────────────────────────────────────────────────────────
const BG: Color = Color::Rgb(22, 22, 30);
const SURFACE: Color = Color::Rgb(30, 30, 42);
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
const GLYPHS_PANE_WIDTH: u16 = 30;
const TEXT_PANE_WIDTH: u16 = 40;
const ACTIVE_PANE_WIDTH: u16 = 60;
const SLIDER_BAR_CELLS: usize = 32;
const EIGHTH_BLOCKS: [&str; 8] = ["", "▏", "▎", "▍", "▌", "▋", "▊", "▉"];

#[derive(Debug)]
enum OptimizationUpdate {
    Progress(String),
    BestResult(RenderArgs),
    Finished,
}

// ── Control types ────────────────────────────────────────────────────────────
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ControlId {
    Width,
    RenderMode,

    Gpu,
    HighlightDisc,
    FixDisc,
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
    FixDiscPasses,
    FixDiscRadius,
    FixDiscTolRatio,
    FixDiscTolPixels,
    FixDiscPool,
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
            min: 10,
            max: 300,
            default: 80,
        },
    },
    ControlDef {
        id: ControlId::Gpu,
        label: "GPU Mode",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::HighlightDisc,
        label: "Show Disc",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::FixDisc,
        label: "Fix Disc",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::ShowAdvanced,
        label: "Advanced",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::FixDiscPasses,
        label: "Fix Passes",
        kind: ControlKind::Slider {
            min: 1,
            max: 100,
            default: 10,
        },
    },
    ControlDef {
        id: ControlId::FixDiscRadius,
        label: "Fix Radius",
        kind: ControlKind::Slider {
            min: 0,
            max: 10,
            default: 2,
        },
    },
    ControlDef {
        id: ControlId::FixDiscTolRatio,
        label: "Fix Tol %",
        kind: ControlKind::Slider {
            min: 0,
            max: 100,
            default: 20,
        },
    },
    ControlDef {
        id: ControlId::FixDiscTolPixels,
        label: "Fix Tol Px",
        kind: ControlKind::Slider {
            min: 0,
            max: 100,
            default: 2,
        },
    },
    ControlDef {
        id: ControlId::FixDiscPool,
        label: "Fix Pool",
        kind: ControlKind::Slider {
            min: 1,
            max: 100,
            default: 12,
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
        id: ControlId::TrimTop,
        label: "Crop Top",
        kind: ControlKind::Slider {
            min: 0,
            max: 500,
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::TrimBottom,
        label: "Crop Bottom",
        kind: ControlKind::Slider {
            min: 0,
            max: 500,
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::TrimLeft,
        label: "Crop Left",
        kind: ControlKind::Slider {
            min: 0,
            max: 500,
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::TrimRight,
        label: "Crop Right",
        kind: ControlKind::Slider {
            min: 0,
            max: 500,
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::FlipH,
        label: "Flip H",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::FlipV,
        label: "Flip V",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::ScaleX,
        label: "Scale X",
        kind: ControlKind::Slider {
            min: 50,
            max: 200,
            default: 100,
        },
    },
    ControlDef {
        id: ControlId::ScaleY,
        label: "Scale Y",
        kind: ControlKind::Slider {
            min: 50,
            max: 200,
            default: 100,
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
            default: 0,
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

struct CatalogItem {
    label: &'static str,
    effect: Option<crate::pipeline::ImageEffect>,
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
        label: "── Effects ──",
        effect: None,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextFieldFocus {
    List,
    TextInput,
    X,
    Y,
    W,
    H,
    FgButton,
    BgButton,
    DeleteButton,
}

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
    focused_idx: usize,
    optimize_shortest_longest_line: bool,
    batch_size: u32,
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

struct TreeState {
    root: Vec<TreeNode>,
    selected_idx: usize,
    scroll: u16,
}

impl TreeState {
    fn new(blocks: &[String]) -> Self {
        let mut root = Vec::new();
        for path in blocks {
            Self::insert_path(&mut root, path, path, 0);
        }
        Self::sort_tree(&mut root);
        TreeState {
            root,
            selected_idx: 0,
            scroll: 0,
        }
    }

    fn insert_path(nodes: &mut Vec<TreeNode>, path: &str, full_path: &str, level: usize) {
        let (head, tail) = match path.split_once('.') {
            Some((h, t)) => (h, Some(t)),
            None => (path, None),
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
                full_path: if tail.is_none() {
                    full_path.to_string()
                } else {
                    String::new()
                },
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
            Self::insert_path(&mut nodes[node_idx].children, t, full_path, level + 1);
        } else {
            // It's a leaf content. Mark the existing node as leaf.
            nodes[node_idx].is_leaf = true;
            nodes[node_idx].full_path = full_path.to_string();
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
        }
        self.mark_args_dirty();
    }

    fn collect_leaves(node: &TreeNode, leaves: &mut Vec<String>) {
        if node.is_leaf {
            leaves.push(node.full_path.clone());
        } else {
            for child in &node.children {
                Self::collect_leaves(child, leaves);
            }
        }
    }
}

pub(crate) struct App {
    render_args: RenderArgs,
    input_mode: InputMode,
    selected_control: usize,
    current_tab: usize, // Focused pane group: 0=general, 1=pipeline, 2=glyphs
    show_general: bool,
    show_pipeline: bool,
    show_glyphs: bool,
    scroll: u16,

    // Cancellation & Debouncing
    tx_args: watch::Sender<(RenderArgs, Arc<AtomicBool>)>,
    current_cancel_flag: Arc<AtomicBool>,
    args_dirty: bool,
    last_args_change: Option<Instant>,

    rx_result: mpsc::Receiver<(draw::RenderResult, Arc<font::GlyphStore>)>,
    cached_render: String,
    cached_result: Option<draw::RenderResult>,
    /// Raw result from render thread (no overlays applied)
    base_render_result: Option<draw::RenderResult>,
    is_rendering: Arc<AtomicBool>,
    render_tick: usize,
    dragged_control: Option<usize>,
    value_edit_buffer: String,

    pipeline_nodes: Vec<crate::pipeline::ImageEffectNode>,
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

    // Metadata prompt state
    metadata_prompt_settings: Option<RenderArgs>,
    metadata_prompt_path: String,
    metadata_prompt_png_path: String,

    // Image path
    image_path: String,

    // Control values
    width: i32,
    render_mode_idx: usize,
    dither: i32,
    gpu: bool,
    show_discontinuities: bool,
    fix_disc: bool,
    show_advanced: bool,
    fix_disc_passes: i32,
    fix_disc_shift_radius: i32,
    fix_disc_ref_tolerance_ratio: f32,
    fix_disc_ref_tolerance_pixels: i32,
    fix_disc_pool_size: i32,

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
    explorer_area: Rect,
    preview_area: Rect,
    output_area: Rect,
    options_pane_area: Rect,
    options_areas: Vec<Rect>,

    // Image preview for xplr
    image_preview_protocol: Option<StatefulProtocol>,
    preview_picker: Picker,
    last_preview_path: Option<String>,

    // Eyedropper state
    eyedropper_color: Option<(u8, u8, u8)>,
    is_optimizing: bool,
    optimization_progress: String,
    auto_optimize_state: Option<AutoOptimizeState>,
    saved_auto_optimize_inputs: Vec<String>,
    saved_auto_optimize_shortest_longest_line: bool,
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
    base_grid: Option<Vec<Vec<crate::draw::RenderedCell>>>,
    text_field_focus: TextFieldFocus,
    text_field_areas: Vec<Rect>, // areas for each field row in the editor
    text_btn_areas: Vec<Rect>,   // clickable button areas: [fg, bg, delete]
    color_picker_state: Option<crate::colorpicker::ColorPickerState>,
}

impl App {
    fn new(
        render_args: RenderArgs,
        tx_args: watch::Sender<(RenderArgs, Arc<AtomicBool>)>,
        rx_result: mpsc::Receiver<(draw::RenderResult, Arc<font::GlyphStore>)>,
        initial_flag: Arc<AtomicBool>,
    ) -> App {
        let (blocks_map, _) = crate::font::load_blocks_config();
        let mut available_blocks: Vec<String> = blocks_map.keys().cloned().collect();
        available_blocks.sort();
        let tree_state = TreeState::new(&available_blocks);

        let mut ra_final = render_args.clone();
        if ra_final.blocks.is_empty() {
            ra_final.blocks = available_blocks.clone();
        }

        App {
            render_args: ra_final,
            input_mode: InputMode::Normal,
            selected_control: 0,
            current_tab: 0,
            show_general: true,
            show_pipeline: true,
            show_glyphs: true,
            scroll: 0,
            tx_args,
            current_cancel_flag: initial_flag,
            args_dirty: false,
            last_args_change: None,
            rx_result,
            cached_render: String::new(),
            cached_result: None,
            base_render_result: None,
            is_rendering: Arc::new(AtomicBool::new(true)),
            render_tick: 0,
            dragged_control: None,
            value_edit_buffer: String::new(),
            arc_glyphs: None,

            image_preview_protocol: None,
            preview_picker: Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks()),
            last_preview_path: None,

            show_font_picker: false,
            font_search: String::new(),
            font_list_state: ListState::default(),

            metadata_prompt_settings: None,
            metadata_prompt_path: String::new(),
            render_mode_idx: match render_args.render {
                Render::Irc => 0,
                Render::Ansi => 1,
                Render::Ansi24 => 2,
            },
            metadata_prompt_png_path: String::new(),

            tree_state,
            glyph_list_state: ListState::default(),

            image_path: render_args.image.clone().unwrap_or_default(),
            width: render_args.width.unwrap_or(80) as i32,
            dither: render_args.dither as i32,
            gpu: render_args.gpu,
            show_discontinuities: render_args.show_discontinuities,
            fix_disc: render_args.fix_disc,
            show_advanced: false,
            fix_disc_passes: render_args.fix_disc_passes as i32,
            fix_disc_shift_radius: render_args.fix_disc_shift_radius as i32,
            fix_disc_ref_tolerance_ratio: render_args.fix_disc_ref_tolerance_ratio,
            fix_disc_ref_tolerance_pixels: render_args.fix_disc_ref_tolerance_pixels as i32,
            fix_disc_pool_size: render_args.fix_disc_pool_size as i32,

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
            font_size: render_args.font_size,
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
            pipeline_list_state: ratatui::widgets::ListState::default(),
            pipeline_next_id: 1,
            pipeline_dragging: None,
            pipeline_scroll: 0,
            pipeline_selected: None,
            pipeline_expanded: None,
            pipeline_library_selected: 1,
            pipeline_preview_effect: PIPELINE_CATALOG.get(1).and_then(|item| item.effect.clone()),

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
            explorer_area: Rect::default(),
            preview_area: Rect::default(),
            output_area: Rect::default(),
            options_pane_area: Rect::default(),
            options_areas: Vec::new(),
            eyedropper_color: None,
            is_optimizing: false,
            optimization_progress: String::new(),
            rx_optimization: None,
            auto_optimize_state: None,
            saved_auto_optimize_inputs: Vec::new(),
            saved_auto_optimize_shortest_longest_line: false,
            auto_optimize_cancel: None,
            general_pane_width: GENERAL_PANE_WIDTH,
            pipeline_pane_width: PIPELINE_PANE_WIDTH,
            glyphs_pane_width: GLYPHS_PANE_WIDTH,
            glyph_tree_height: 10,
            glyph_list_height: 30,
            splitter_areas: Vec::new(),
            dragging_splitter: None,
            terminal_area: Rect::default(),

            text_pane_width: TEXT_PANE_WIDTH,
            text_area: Rect::default(),
            text_list_state: ratatui::widgets::ListState::default(),
            text_list_area: Rect::default(),
            color_picker_area: Rect::default(),
            text_selected: None,
            text_drag_mode: TextDragMode::None,
            text_drag_start: None,
            last_text_click: None,
            base_grid: None,
            text_field_focus: TextFieldFocus::List,
            text_field_areas: Vec::new(),
            text_btn_areas: Vec::new(),
            color_picker_state: None,
        }
    }

    fn apply_settings(&mut self, render_args: &RenderArgs) {
        self.render_args = render_args.clone();
        self.width = render_args.width.unwrap_or(80) as i32;
        self.render_mode_idx = match render_args.render {
            Render::Irc => 0,
            Render::Ansi => 1,
            Render::Ansi24 => 2,
        };
        self.dither = render_args.dither as i32;
        self.gpu = render_args.gpu;
        self.show_discontinuities = render_args.show_discontinuities;
        self.fix_disc = render_args.fix_disc;
        self.fix_disc_passes = render_args.fix_disc_passes as i32;
        self.fix_disc_shift_radius = render_args.fix_disc_shift_radius as i32;
        self.fix_disc_ref_tolerance_ratio = render_args.fix_disc_ref_tolerance_ratio;
        self.fix_disc_ref_tolerance_pixels = render_args.fix_disc_ref_tolerance_pixels as i32;
        self.fix_disc_pool_size = render_args.fix_disc_pool_size as i32;
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
        self.font_size = render_args.font_size;

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
            2 => vec![FocusTarget::GlyphTree, FocusTarget::GlyphList],
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
                _ => FocusTarget::GlyphList,
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
                let catalog_len = crate::tui::PIPELINE_CATALOG.len();
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

        if self.pipeline_library_selected >= PIPELINE_CATALOG.len() {
            self.pipeline_library_selected = PIPELINE_CATALOG.len().saturating_sub(1);
        }
    }

    fn get_value(&self, id: ControlId) -> i32 {
        match id {
            ControlId::Width => self.width,
            ControlId::RenderMode => self.render_mode_idx as i32,
            ControlId::TrimTop => self.trim_top,
            ControlId::TrimRight => self.trim_right,
            ControlId::TrimBottom => self.trim_bottom,
            ControlId::TrimLeft => self.trim_left,
            ControlId::FlipH => self.fliph as i32,
            ControlId::FlipV => self.flipv as i32,
            ControlId::Rotate => self.rotate,
            ControlId::ScaleX => self.scalex,
            ControlId::ScaleY => self.scaley,
            ControlId::FontSize => (self.font_size * 10.0) as i32,
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
            ControlId::Gpu => self.gpu as i32,
            ControlId::HighlightDisc => self.show_discontinuities as i32,
            ControlId::FixDisc => self.fix_disc as i32,
            ControlId::ShowAdvanced => self.show_advanced as i32,
            ControlId::FixDiscPasses => self.fix_disc_passes,
            ControlId::FixDiscRadius => self.fix_disc_shift_radius,
            ControlId::FixDiscTolRatio => (self.fix_disc_ref_tolerance_ratio * 100.0) as i32,
            ControlId::FixDiscTolPixels => self.fix_disc_ref_tolerance_pixels,
            ControlId::FixDiscPool => self.fix_disc_pool_size,

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
        match id {
            ControlId::Width => {
                let old_w = self.width;
                self.width = val;
                if old_w > 0 && old_w != val && !self.render_args.overlays.is_empty() {
                    let scale = val as f32 / old_w as f32;
                    for ov in &mut self.render_args.overlays {
                        ov.x = (ov.x as f32 * scale).round() as i32;
                        ov.y = (ov.y as f32 * scale).round() as i32;
                    }
                }
            }
            ControlId::RenderMode => self.render_mode_idx = val as usize,
            ControlId::TrimTop => self.trim_top = val,
            ControlId::TrimRight => self.trim_right = val,
            ControlId::TrimBottom => self.trim_bottom = val,
            ControlId::TrimLeft => self.trim_left = val,
            ControlId::FlipH => self.fliph = val != 0,
            ControlId::FlipV => self.flipv = val != 0,
            ControlId::Rotate => self.rotate = val,
            ControlId::ScaleX => self.scalex = val,
            ControlId::ScaleY => self.scaley = val,
            ControlId::FontSize => self.font_size = val as f32 / 10.0,
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
            ControlId::Gpu => self.gpu = val != 0,
            ControlId::HighlightDisc => self.show_discontinuities = val != 0,
            ControlId::FixDisc => self.fix_disc = val != 0,
            ControlId::ShowAdvanced => {
                self.show_advanced = val != 0;
                let count = self.tab_controls(0).len();
                if self.selected_control >= count {
                    self.selected_control = count.saturating_sub(1);
                }
            }
            ControlId::FixDiscPasses => self.fix_disc_passes = val,
            ControlId::FixDiscRadius => self.fix_disc_shift_radius = val,
            ControlId::FixDiscTolRatio => self.fix_disc_ref_tolerance_ratio = val as f32 / 100.0,
            ControlId::FixDiscTolPixels => self.fix_disc_ref_tolerance_pixels = val,
            ControlId::FixDiscPool => self.fix_disc_pool_size = val,

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

    fn tab_controls(&self, tab: usize) -> Vec<ControlDef> {
        match tab {
            0 => {
                let mut ctrls = Vec::new();
                let advanced_ids = [
                    ControlId::FixDiscPasses,
                    ControlId::FixDiscRadius,
                    ControlId::FixDiscTolRatio,
                    ControlId::FixDiscTolPixels,
                    ControlId::FixDiscPool,
                ];
                for ctrl in GENERAL_CONTROLS {
                    if advanced_ids.contains(&ctrl.id) && !self.show_advanced {
                        continue;
                    }
                    ctrls.push(*ctrl);
                }
                ctrls
            }
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
        for i in 0..16 {
            let mut ta = if self.saved_auto_optimize_inputs.len() == 16 {
                TextArea::new(vec![self.saved_auto_optimize_inputs[i].clone()])
            } else {
                TextArea::default()
            };
            inputs.push(ta);
        }

        self.auto_optimize_state = Some(AutoOptimizeState {
            inputs,
            focused_idx: 0,
            optimize_shortest_longest_line: self.saved_auto_optimize_shortest_longest_line,
            batch_size: self.render_args.auto_optimize_batch_size,
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

        fn parse_range_i32(s_min: &str, s_max: &str, current: i32) -> Vec<i32> {
            let p0 = s_min.trim().parse().unwrap_or(current);
            let p1 = if s_max.trim().is_empty() {
                p0
            } else {
                s_max.trim().parse().unwrap_or(p0)
            };
            let min = p0.min(p1);
            let max = p0.max(p1);
            (min..=max).collect()
        }

        let ws = parse_range_i32(
            state.inputs[0].lines()[0].as_str(),
            state.inputs[1].lines()[0].as_str(),
            self.width,
        );
        let ct = parse_range_i32(
            state.inputs[2].lines()[0].as_str(),
            state.inputs[3].lines()[0].as_str(),
            self.trim_top,
        );
        let cr = parse_range_i32(
            state.inputs[4].lines()[0].as_str(),
            state.inputs[5].lines()[0].as_str(),
            self.trim_right,
        );
        let cb = parse_range_i32(
            state.inputs[6].lines()[0].as_str(),
            state.inputs[7].lines()[0].as_str(),
            self.trim_bottom,
        );
        let cl = parse_range_i32(
            state.inputs[8].lines()[0].as_str(),
            state.inputs[9].lines()[0].as_str(),
            self.trim_left,
        );
        let fs: Vec<f32> = parse_range_i32(
            state.inputs[10].lines()[0].as_str(),
            state.inputs[11].lines()[0].as_str(),
            self.font_size.round() as i32,
        )
        .into_iter()
        .map(|x| x as f32)
        .collect();
        let sx = parse_range_i32(
            state.inputs[12].lines()[0].as_str(),
            state.inputs[13].lines()[0].as_str(),
            self.scalex,
        );
        let sy = parse_range_i32(
            state.inputs[14].lines()[0].as_str(),
            state.inputs[15].lines()[0].as_str(),
            self.scaley,
        );

        let total_perms =
            ws.len() * ct.len() * cr.len() * cb.len() * cl.len() * fs.len() * sx.len() * sy.len();

        if total_perms == 0 {
            self.is_optimizing = false;
            return;
        }

        self.saved_auto_optimize_inputs =
            state.inputs.iter().map(|i| i.lines()[0].clone()).collect();
        self.saved_auto_optimize_shortest_longest_line = optimize_shortest_longest_line;

        self.is_optimizing = true;
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

            let num_threads = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1);
            let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(num_threads));
            let (res_tx, mut res_rx) = tokio::sync::mpsc::channel(num_threads * 2);

            let args_len = total_perms;
            let tx_clone = tx.clone();
            let img_clone = image.clone();
            let gs_clone = glyph_store.clone();
            let cancel_clone = cancel_flag.clone();
            let batch_size = args.auto_optimize_batch_size.max(1);

            tokio::spawn(async move {
                let mut aborted = false;
                let mut gs_map = std::collections::HashMap::new();
                for &f in &fs {
                    if tx_clone.is_closed()
                        || cancel_clone.load(std::sync::atomic::Ordering::Relaxed)
                    {
                        aborted = true;
                        break;
                    }
                    if (f - args.font_size).abs() < std::f32::EPSILON {
                        gs_map.insert(f.to_bits(), gs_clone.clone());
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
                        gs_map.insert(f.to_bits(), std::sync::Arc::new(new_store));
                    }
                }

                if aborted {
                    return;
                }

                for &w in &ws {
                    if tx_clone.is_closed() || cancel_clone.load(Ordering::Relaxed) {
                        break;
                    }
                    for &t in &ct {
                        for &r in &cr {
                            for &b in &cb {
                                for &l in &cl {
                                    for &f in &fs {
                                        for &x in &sx {
                                            for &y in &sy {
                                                if tx_clone.is_closed()
                                                    || cancel_clone.load(Ordering::Relaxed)
                                                {
                                                    aborted = true;
                                                    break;
                                                }
                                                let mut a = args.clone();
                                                a.width = Some(w as u32);
                                                a.trim =
                                                    Some((t as u32, r as u32, b as u32, l as u32));
                                                a.font_size = f;
                                                a.scale =
                                                    Some((x as f32 / 100.0, y as f32 / 100.0));

                                                let permit = semaphore
                                                    .clone()
                                                    .acquire_owned()
                                                    .await
                                                    .unwrap();
                                                let img = img_clone.clone();
                                                let gs = gs_map.get(&f.to_bits()).unwrap().clone();
                                                let rtx = res_tx.clone();
                                                let cancel_inner = cancel_clone.clone();
                                                tokio::task::spawn_blocking(move || {
                                                    let fg = crate::draw::prepare_glyphs(&gs, &a);
                                                    let result = crate::render_output(
                                                        a.clone(),
                                                        &gs,
                                                        img,
                                                        &fg,
                                                        Some(cancel_inner),
                                                    );
                                                    if result.content != "Cancelled" {
                                                        let score =
                                                            if optimize_shortest_longest_line {
                                                                result.longest_line_bytes as f64
                                                            } else {
                                                                result.score
                                                            };
                                                        let _ = rtx.blocking_send((a, score));
                                                    }
                                                    drop(permit);
                                                });
                                            }
                                            if aborted {
                                                break;
                                            }
                                        }
                                        if aborted {
                                            break;
                                        }
                                    }
                                    if aborted {
                                        break;
                                    }
                                }
                                if aborted {
                                    break;
                                }
                            }
                            if aborted {
                                break;
                            }
                        }
                        if aborted {
                            break;
                        }
                    }
                    if aborted {
                        break;
                    }
                }
            });

            let optimize_start_time = std::time::Instant::now();

            while let Some((a, score)) = res_rx.recv().await {
                curr += 1;
                if score < best_score {
                    best_score = score;
                    best_args = a;
                    let _ = tx
                        .send(OptimizationUpdate::BestResult(best_args.clone()))
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
            let _ = tx.send(OptimizationUpdate::BestResult(best_args)).await;
            let _ = tx.send(OptimizationUpdate::Finished).await;
        });
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
        self.render_args.width = Some(self.width as u32);
        self.render_args.dither = self.dither as u32;
        self.render_args.invert = self.invert;
        self.render_args.grayscale = self.grayscale;
        self.render_args.gpu = self.gpu;
        self.render_args.show_discontinuities = self.show_discontinuities;
        self.render_args.fix_disc = self.fix_disc;
        self.render_args.fix_disc_passes = self.fix_disc_passes as u32;
        self.render_args.fix_disc_shift_radius = self.fix_disc_shift_radius as u32;
        self.render_args.fix_disc_ref_tolerance_ratio = self.fix_disc_ref_tolerance_ratio;
        self.render_args.fix_disc_ref_tolerance_pixels = self.fix_disc_ref_tolerance_pixels as u32;
        self.render_args.fix_disc_pool_size = self.fix_disc_pool_size as u32;

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
        let _ = self.tx_args.send((args_for_render, new_flag));
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
                let filter = Arc::new(|file: &ratatui_explorer::File| {
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
        self.mark_args_dirty();
    }

    /// Try to paste an image from the system clipboard.
    /// Attempts xclip (X11) first, then wl-paste (Wayland).
    /// Saves the image to a temp file and loads it.
    fn paste_from_clipboard(&mut self) {
        use std::io::Write;

        let clipboard_data = Self::try_clipboard_x11().or_else(|| Self::try_clipboard_wayland());

        if let Some(data) = clipboard_data {
            if data.is_empty() {
                log::info!("Clipboard is empty or contains no image");
                return;
            }

            // Detect format from magic bytes and pick extension
            let ext = if data.starts_with(&[0x89, b'P', b'N', b'G']) {
                "png"
            } else if data.starts_with(&[0xFF, 0xD8]) {
                "jpg"
            } else if data.starts_with(b"GIF") {
                "gif"
            } else if data.starts_with(b"BM") {
                "bmp"
            } else if data.starts_with(b"RIFF") {
                "webp"
            } else {
                "png" // assume PNG as fallback
            };

            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            let tmp_path = format!("/tmp/img2irc_clipboard_{}.{}", ts, ext);
            match std::fs::File::create(&tmp_path) {
                Ok(mut f) => {
                    if let Err(e) = f.write_all(&data) {
                        log::info!("Failed to write clipboard image: {}", e);
                        return;
                    }
                }
                Err(e) => {
                    log::info!("Failed to create temp file: {}", e);
                    return;
                }
            }

            log::info!(
                "Loaded image from clipboard: {} ({} bytes)",
                tmp_path,
                data.len()
            );
            self.image_path = tmp_path.clone();
            self.render_args.image = Some(tmp_path);
            self.mark_args_dirty();
        } else {
            log::info!("No image found in clipboard (tried xclip and wl-paste)");
        }
    }

    fn try_clipboard_x11() -> Option<Vec<u8>> {
        for target in &["image/png", "image/jpeg", "image/bmp"] {
            if let Ok(output) = Command::new("xclip")
                .args(["-selection", "clipboard", "-t", target, "-o"])
                .output()
            {
                if output.status.success() && !output.stdout.is_empty() {
                    return Some(output.stdout);
                }
            }
        }
        None
    }

    fn try_clipboard_wayland() -> Option<Vec<u8>> {
        for mime in &["image/png", "image/jpeg", "image/bmp"] {
            if let Ok(output) = Command::new("wl-paste").args(["--type", mime]).output() {
                if output.status.success() && !output.stdout.is_empty() {
                    return Some(output.stdout);
                }
            }
        }
        None
    }
}

fn list_system_fonts() -> Vec<String> {
    // Try to use fc-list with spacing=100 (monospace)
    if let Ok(output) = Command::new("fc-list").arg(":spacing=100").output() {
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
    vec!["Default".to_string(), "Monospace".to_string()]
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
    let label_width = label_text.chars().take(14).count() as u16;
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
) {
    if area.height < 1 || area.width < 4 {
        return;
    }

    let bg_style = if selected {
        Style::new().bg(SELECTED_BG)
    } else {
        Style::new().bg(SURFACE)
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
    for (i, ch) in label_text.chars().take(14).enumerate() {
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
) {
    if area.height < 1 || area.width < 4 {
        return;
    }

    let bg = if selected { SELECTED_BG } else { SURFACE };
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
) {
    if area.height < 1 || area.width < 4 {
        return;
    }

    let bg = if selected { SELECTED_BG } else { SURFACE };
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
fn render_toggle(buf: &mut Buffer, area: Rect, label: &str, on: bool, selected: bool) {
    if area.height < 1 || area.width < 4 {
        return;
    }

    let bg = if selected { SELECTED_BG } else { SURFACE };
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

fn render_button(buf: &mut Buffer, area: Rect, label: &str, selected: bool, status: &str) {
    let bg = if selected {
        SELECTED_BG
    } else if !status.is_empty() {
        OPTIMIZE_BG
    } else {
        SURFACE
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
pub async fn run(mut render_args: RenderArgs) -> Result<(), Box<dyn Error>> {
    // Initialize logging to file
    let _ = WriteLogger::init(
        LevelFilter::Info,
        Config::default(),
        File::create("img2irc.log").unwrap(),
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

    if render_args.width.is_none() {
        render_args.width = Some(80);
    }
    render_args.render = Render::Ansi;

    let glyph_store = font::glyph_store(
        &render_args.blocks,
        &render_args.exclude_range,
        &render_args.exclude,
        &render_args.include_range,
        render_args.include.as_ref(),
        &render_args.font,
        render_args.font_size,
        // if (render_args.font_size - 0.0).abs() < f32::EPSILON {
        //     24.0
        // } else {
        //     render_args.font_size
        // },
        true,
        None,
    );

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
    let (tx_args, mut rx_args) = watch::channel((render_args.clone(), initial_flag.clone()));
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

        // Use thread_render_args here instead of capturing render_args
        let mut arc_glyphs = arc_glyphs;
        let mut last_font = thread_render_args.font.clone();
        let mut last_font_size = thread_render_args.font_size;
        let mut last_blocks = thread_render_args.blocks.clone();

        loop {
            // Signal rendering start
            rendering_flag2.store(true, Ordering::Relaxed);

            let (current_args, cancel_flag) = { rx_args.borrow_and_update().clone() };

            // Reload image if path changed
            if current_args.image != current_image_path {
                if let Some(path) = &current_args.image {
                    match crate::load_image_from_url_or_path(path, &current_args, &arc_glyphs).await
                    {
                        Ok(img) => {
                            arc_image = Arc::new(img);
                            current_image_path = Some(path.clone());
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
            crate::resolve_auto_font_size(&mut effective_args, &arc_image, &arc_glyphs);

            if effective_args.font != last_font
                || (effective_args.font_size - last_font_size).abs() > ::std::f32::EPSILON
                || effective_args.blocks != last_blocks
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
            }

            if fast_glyphs_store.is_none() {
                fast_glyphs_store = Some(draw::prepare_glyphs(&arc_glyphs, &effective_args));
            }

            // Render!
            let glyphs = fast_glyphs_store.as_ref().unwrap();
            log::info!("Starting render optimization... ({} glyphs)", glyphs.len());
            let start = Instant::now();
            let result = crate::render_output(
                effective_args,
                &arc_glyphs,
                (*arc_image).clone(),
                glyphs,
                Some(cancel_flag.clone()),
            );
            log::info!("Render finished in {:?}", start.elapsed());

            rendering_flag2.store(false, Ordering::Relaxed);

            // Only send if not cancelled (though render output might say cancelled)
            if !cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
                if tx_result.send((result, arc_glyphs.clone())).await.is_err() {
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
        println!("{:?}", err);
    }
    Ok(())
}

// ── App Loop ─────────────────────────────────────────────────────────────────
async fn run_app<B: Backend>(terminal: &mut Terminal<B>, mut app: App) -> io::Result<()> {
    loop {
        app.render_tick = app.render_tick.wrapping_add(1);

        if let Some(mut rx) = app.rx_optimization.take() {
            while let Ok(update) = rx.try_recv() {
                match update {
                    OptimizationUpdate::Progress(p) => app.optimization_progress = p,
                    OptimizationUpdate::BestResult(args) => {
                        app.apply_settings(&args);
                        app.mark_args_dirty();
                    }
                    OptimizationUpdate::Finished => {
                        app.is_optimizing = false;
                        app.optimization_progress = "Finished".to_string();
                    }
                }
            }
            app.rx_optimization = Some(rx);
        }

        while let Ok((result, glyphs)) = app.rx_result.try_recv() {
            if result.content == "Cancelled" {
                continue;
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
        }

        terminal.draw(|f| ui(f, &mut app))?;

        if crossterm::event::poll(Duration::from_millis(50))? {
            let event = event::read()?;

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
                                let _ = dialog.explorer.handle(ratatui_explorer::Input::Up);
                            } else if is_scroll_down {
                                let _ = dialog.explorer.handle(ratatui_explorer::Input::Down);
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
                                            let _ = dialog
                                                .explorer
                                                .handle(ratatui_explorer::Input::Right);
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
                                if let Ok(val) = app.value_edit_buffer.parse::<i32>() {
                                    let controls = app.tab_controls(app.current_tab);
                                    if let Some(ctrl) = controls.get(app.selected_control) {
                                        if let ControlKind::Slider { min, max, .. } = &ctrl.kind {
                                            let clamped = val.clamp(*min, *max);
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
                            KeyCode::Enter => {
                                if key.modifiers.contains(KeyModifiers::SHIFT) {
                                    // Add newline — update overlay and reapply cheaply
                                    app.value_edit_buffer.push('\n');
                                    if let Some(sel) = app.text_selected {
                                        if sel > 0 && sel <= app.render_args.overlays.len() {
                                            app.render_args.overlays[sel - 1].text =
                                                app.value_edit_buffer.clone();
                                            app.reapply_overlays();
                                        }
                                    }
                                } else {
                                    // Commit text — mark dirty for a real re-render
                                    if let Some(sel) = app.text_selected {
                                        if sel > 0 && sel <= app.render_args.overlays.len() {
                                            app.render_args.overlays[sel - 1].text =
                                                app.value_edit_buffer.clone();
                                            app.mark_args_dirty();
                                        }
                                    }
                                    app.input_mode = InputMode::Normal;
                                    app.value_edit_buffer.clear();
                                }
                            }
                            KeyCode::Esc => {
                                // Commit text — mark dirty for a real re-render
                                if let Some(sel) = app.text_selected {
                                    if sel > 0 && sel <= app.render_args.overlays.len() {
                                        app.render_args.overlays[sel - 1].text =
                                            app.value_edit_buffer.clone();
                                        app.mark_args_dirty();
                                    }
                                }
                                app.input_mode = InputMode::Normal;
                                app.value_edit_buffer.clear();
                            }
                            KeyCode::Backspace => {
                                app.value_edit_buffer.pop();
                                // Update overlay text and reapply cheaply (no full re-render)
                                if let Some(sel) = app.text_selected {
                                    if sel > 0 && sel <= app.render_args.overlays.len() {
                                        app.render_args.overlays[sel - 1].text =
                                            app.value_edit_buffer.clone();
                                        app.reapply_overlays();
                                    }
                                }
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
                                if let Some(sel) = app.text_selected {
                                    if sel > 0 && sel <= app.render_args.overlays.len() {
                                        app.render_args.overlays[sel - 1].text =
                                            app.value_edit_buffer.clone();
                                        app.reapply_overlays();
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                continue;
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
                if let Event::Key(key) = event {
                    if key.kind == KeyEventKind::Press {
                        if let Some(state) = &mut app.auto_optimize_state {
                            match key.code {
                                KeyCode::Esc => {
                                    app.input_mode = InputMode::Normal;
                                }
                                KeyCode::Enter => {
                                    app.input_mode = InputMode::Normal;
                                    app.start_auto_optimization();
                                }
                                KeyCode::Up => {
                                    if state.focused_idx == 16 {
                                        state.focused_idx = 14;
                                    } else if state.focused_idx == 17 {
                                        state.focused_idx = 16;
                                    } else if state.focused_idx >= 2 {
                                        state.focused_idx -= 2;
                                    } else {
                                        state.focused_idx = 17;
                                    }
                                }
                                KeyCode::Down => {
                                    if state.focused_idx == 16 {
                                        state.focused_idx = 17;
                                    } else if state.focused_idx == 17 {
                                        state.focused_idx = 0;
                                    } else if state.focused_idx < 14 {
                                        state.focused_idx += 2;
                                    } else {
                                        state.focused_idx = 16;
                                    }
                                }
                                KeyCode::Left => {
                                    if state.focused_idx == 17 {
                                        state.batch_size =
                                            state.batch_size.saturating_sub(1).max(1);
                                    } else if state.focused_idx > 0 {
                                        state.focused_idx -= 1;
                                    } else {
                                        state.focused_idx = 17;
                                    }
                                }
                                KeyCode::Right => {
                                    if state.focused_idx == 17 {
                                        state.batch_size =
                                            state.batch_size.saturating_add(1).min(100);
                                    } else if state.focused_idx < 17 {
                                        state.focused_idx += 1;
                                    } else {
                                        state.focused_idx = 0;
                                    }
                                }
                                KeyCode::BackTab => {
                                    if state.focused_idx > 0 {
                                        state.focused_idx -= 1;
                                    } else {
                                        state.focused_idx = 17;
                                    }
                                }
                                KeyCode::Tab => {
                                    if state.focused_idx < 17 {
                                        state.focused_idx += 1;
                                    } else {
                                        state.focused_idx = 0;
                                    }
                                }
                                KeyCode::Char(' ') if state.focused_idx == 16 => {
                                    state.optimize_shortest_longest_line =
                                        !state.optimize_shortest_longest_line;
                                }
                                KeyCode::Char('-') | KeyCode::Char('_')
                                    if state.focused_idx == 17 =>
                                {
                                    state.batch_size = state.batch_size.saturating_sub(1).max(1);
                                }
                                KeyCode::Char('+') | KeyCode::Char('=')
                                    if state.focused_idx == 17 =>
                                {
                                    state.batch_size = state.batch_size.saturating_add(1).min(100);
                                }
                                _ => {
                                    if state.focused_idx < 16 {
                                        state.inputs[state.focused_idx].input(key);
                                    }
                                }
                            }
                        }
                    }
                }
                continue;
            }

            match event {
                Event::Mouse(mouse) => {
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
                                    false,
                                );
                            } else if mouse.kind
                                == crossterm::event::MouseEventKind::Down(MouseButton::Right)
                            {
                                handled = cp_state.handle_mouse_down(
                                    mouse.column,
                                    mouse.row,
                                    cp_rect,
                                    true,
                                );
                            } else if mouse.kind
                                == crossterm::event::MouseEventKind::Drag(MouseButton::Left)
                            {
                                handled =
                                    cp_state.handle_mouse_drag(mouse.column, mouse.row, cp_rect);
                            } else if mouse.kind
                                == crossterm::event::MouseEventKind::Up(MouseButton::Left)
                            {
                                cp_state.is_dragging_main = false;
                                cp_state.is_dragging_hue = false;
                                handled = true;
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
                                        app.mark_args_dirty();
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
                                let curr = app.glyph_list_state.selected().unwrap_or(0);
                                app.glyph_list_state.select(Some(curr.saturating_sub(1)));
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
                                    1 => {
                                        let curr = app.glyph_list_state.selected().unwrap_or(0);
                                        app.glyph_list_state.select(Some(curr.saturating_sub(1)));
                                    }
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
                                let curr = app.glyph_list_state.selected().unwrap_or(0);
                                app.glyph_list_state.select(Some(curr + 1));
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
                                    1 => {
                                        let curr = app.glyph_list_state.selected().unwrap_or(0);
                                        app.glyph_list_state.select(Some(curr + 1));
                                    }
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

                                                    let mut found = None;
                                                    for (idx, ov) in app
                                                        .render_args
                                                        .overlays
                                                        .iter()
                                                        .enumerate()
                                                        .rev()
                                                    {
                                                        let mut text_w = 1;
                                                        let mut text_h = 1;
                                                        if ov.w == 0 || ov.h == 0 {
                                                            let lines: Vec<&str> =
                                                                ov.text.split('\n').collect();
                                                            text_h = lines.len() as i32;
                                                            text_w = lines
                                                                .iter()
                                                                .map(|l| l.chars().count() as i32)
                                                                .max()
                                                                .unwrap_or(1);
                                                        }
                                                        let check_w = if ov.w > 0 {
                                                            ov.w
                                                        } else {
                                                            text_w.max(2)
                                                        };
                                                        let check_h = if ov.h > 0 {
                                                            ov.h
                                                        } else {
                                                            text_h.max(1)
                                                        };

                                                        if cell_x >= ov.x
                                                            && cell_x < ov.x + check_w
                                                            && cell_y >= ov.y
                                                            && cell_y < ov.y + check_h
                                                        {
                                                            found = Some(idx);
                                                            break;
                                                        }
                                                    }

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
                                                        if is_double {
                                                            app.text_selected = Some(idx + 1);
                                                            app.value_edit_buffer =
                                                                app.render_args.overlays[idx]
                                                                    .text
                                                                    .clone();
                                                            app.input_mode = InputMode::TextEdit;
                                                            app.text_drag_mode = TextDragMode::None;
                                                            app.last_text_click = None;
                                                        } else {
                                                            app.text_selected = Some(idx + 1);
                                                            app.text_drag_mode =
                                                                TextDragMode::Moving;
                                                            app.text_drag_start = Some((
                                                                cell_x
                                                                    - app.render_args.overlays[idx]
                                                                        .x,
                                                                cell_y
                                                                    - app.render_args.overlays[idx]
                                                                        .y,
                                                            ));
                                                            app.last_text_click =
                                                                Some((now, cell_x, cell_y));
                                                        }
                                                    } else {
                                                        // Require double-click to create new overlay
                                                        if is_double {
                                                            app.render_args.overlays.push(
                                                                crate::args::TextOverlay {
                                                                    text: "".to_string(),
                                                                    x: cell_x,
                                                                    y: cell_y,
                                                                    w: 0,
                                                                    h: 0,
                                                                    fg: None,
                                                                    bg: None,
                                                                    wrap: false,
                                                                    bold: false,
                                                                    italic: false,
                                                                    underline: false,
                                                                },
                                                            );
                                                            app.text_selected = Some(
                                                                app.render_args.overlays.len(),
                                                            );
                                                            app.reapply_overlays();
                                                            app.text_drag_mode =
                                                                TextDragMode::Sizing;
                                                            app.text_drag_start =
                                                                Some((cell_x, cell_y));
                                                            app.last_text_click = None;
                                                        } else {
                                                            app.last_text_click =
                                                                Some((now, cell_x, cell_y));
                                                            app.text_selected = None;
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
                                if clicked_idx == app.tree_state.selected_idx {
                                    app.toggle_tree_selection();
                                } else {
                                    app.tree_state.selected_idx = clicked_idx;
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
                                app.glyph_list_state.select(Some(row));
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
                                            app.font_idx = i;
                                        }
                                        app.mark_args_dirty();
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
                                        if PIPELINE_CATALOG[lib_idx].effect.is_some() {
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
                                            }
                                            ControlKind::Action => toggle_control(&mut app),
                                        }
                                    }
                                    break;
                                }
                            }

                            if mouse.column >= app.text_list_area.x
                                && mouse.column < app.text_list_area.x + app.text_list_area.width
                                && mouse.row >= app.text_list_area.y
                                && mouse.row < app.text_list_area.y + app.text_list_area.height
                            {
                                app.current_tab = 3;
                                let row = mouse.row.saturating_sub(app.text_list_area.y) as usize;
                                let text_offset = app.text_list_state.offset();
                                let selected_idx = text_offset + row;

                                let total_len = app.render_args.overlays.len();
                                if selected_idx <= total_len {
                                    if app.text_selected == Some(selected_idx) && selected_idx == 0
                                    {
                                        // Double-click "Add New" - create overlay
                                        app.render_args.overlays.push(crate::args::TextOverlay {
                                            text: "Text".to_string(),
                                            x: 0,
                                            y: 0,
                                            w: 0,
                                            h: 0,
                                            fg: None,
                                            bg: None,
                                            wrap: false,
                                            bold: false,
                                            italic: false,
                                            underline: false,
                                        });
                                        app.text_selected = Some(app.render_args.overlays.len());
                                        app.text_list_state.select(app.text_selected);
                                        app.text_field_focus = TextFieldFocus::TextInput;
                                        app.mark_args_dirty();
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

                            if app.text_btn_areas.len() >= 5 {
                                if app.text_btn_areas[4].contains(pos) {
                                    // Delete button (index 4)
                                    app.current_tab = 3;
                                    if let Some(sel) = app.text_selected {
                                        if sel > 0 && sel <= app.render_args.overlays.len() {
                                            app.render_args.overlays.remove(sel - 1);
                                            let new_len = app.render_args.overlays.len();
                                            if new_len == 0 {
                                                app.text_selected = Some(0);
                                            } else {
                                                app.text_selected = Some(sel.min(new_len));
                                            }
                                            app.text_list_state.select(app.text_selected);
                                            app.text_field_focus = TextFieldFocus::List;
                                            app.mark_args_dirty();
                                        }
                                    }
                                } else if app.text_btn_areas[0].contains(pos) {
                                    // Wrap button (index 0)
                                    app.current_tab = 3;
                                    if let Some(sel) = app.text_selected {
                                        if sel > 0 && sel <= app.render_args.overlays.len() {
                                            app.render_args.overlays[sel - 1].wrap =
                                                !app.render_args.overlays[sel - 1].wrap;
                                            app.mark_args_dirty();
                                        }
                                    }
                                } else if app.text_btn_areas[1].contains(pos) {
                                    // Bold (index 1)
                                    if let Some(sel) = app.text_selected {
                                        if sel > 0 && sel <= app.render_args.overlays.len() {
                                            app.render_args.overlays[sel - 1].bold =
                                                !app.render_args.overlays[sel - 1].bold;
                                            app.reapply_overlays();
                                        }
                                    }
                                } else if app.text_btn_areas[2].contains(pos) {
                                    // Italic (index 2)
                                    if let Some(sel) = app.text_selected {
                                        if sel > 0 && sel <= app.render_args.overlays.len() {
                                            app.render_args.overlays[sel - 1].italic =
                                                !app.render_args.overlays[sel - 1].italic;
                                            app.reapply_overlays();
                                        }
                                    }
                                } else if app.text_btn_areas[3].contains(pos) {
                                    // Underline (index 3)
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
                                                        let cell_x = c_x;
                                                        let cell_y = c_y;

                                                        let ov =
                                                            &mut app.render_args.overlays[sel - 1];
                                                        if app.text_drag_mode
                                                            == TextDragMode::Moving
                                                        {
                                                            // Move
                                                            if let Some((sx, sy)) =
                                                                app.text_drag_start
                                                            {
                                                                ov.x = cell_x - sx;
                                                                ov.y = cell_y - sy;
                                                                app.reapply_overlays();
                                                            }
                                                        } else if app.text_drag_mode
                                                            == TextDragMode::Sizing
                                                        {
                                                            // Resize
                                                            if let Some((sx, sy)) =
                                                                app.text_drag_start
                                                            {
                                                                ov.w = (cell_x - sx).max(0);
                                                                ov.h = (cell_y - sy).max(0);
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
                                            .max(18);
                                        app.text_pane_width = mouse
                                            .column
                                            .saturating_sub(left_edge)
                                            .clamp(18, max_width);
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
                                app.mark_args_dirty();
                                app.text_drag_mode = TextDragMode::None;
                                app.text_drag_start = None;
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
                        if app.current_tab == 3 {
                            if let Some(sel) = app.text_selected {
                                if sel > 0 && sel <= app.render_args.overlays.len() {
                                    let mut handled = true;
                                    let ov = &mut app.render_args.overlays[sel - 1];
                                    match key.code {
                                        KeyCode::Backspace => {
                                            ov.text.pop();
                                            app.mark_args_dirty();
                                        }
                                        KeyCode::Char(c) => {
                                            ov.text.push(c);
                                            app.reapply_overlays();
                                        }
                                        KeyCode::Esc => {
                                            app.text_selected = None;
                                        }
                                        _ => {
                                            handled = false;
                                        }
                                    }
                                    if handled {
                                        continue;
                                    }
                                }
                            }
                        }
                        match key.code {
                            KeyCode::Tab => app.cycle_focus_target(1),
                            KeyCode::BackTab => app.cycle_focus_target(-1),
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
                                app.mark_args_dirty();
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
                            KeyCode::Char(c)
                                if app.current_tab == 0 && (c.is_ascii_digit() || c == '-') =>
                            {
                                let ctrl_id = app.get_current_control_id();
                                let def = get_control_def(ctrl_id);
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
                                if app.is_optimizing {
                                    app.perform_auto_optimize();
                                    continue;
                                }

                                if app.current_tab == 3
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
                                        1 => {
                                            let curr = app.glyph_list_state.selected().unwrap_or(0);
                                            app.glyph_list_state.select(Some(curr + 1));
                                        }
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
                                    } else {
                                        // Cycle through editor fields
                                        app.text_field_focus = match app.text_field_focus {
                                            TextFieldFocus::TextInput => TextFieldFocus::X,
                                            TextFieldFocus::X => TextFieldFocus::Y,
                                            TextFieldFocus::Y => TextFieldFocus::W,
                                            TextFieldFocus::W => TextFieldFocus::H,
                                            TextFieldFocus::H => TextFieldFocus::FgButton,
                                            TextFieldFocus::FgButton => TextFieldFocus::BgButton,
                                            TextFieldFocus::BgButton => {
                                                TextFieldFocus::DeleteButton
                                            }
                                            TextFieldFocus::DeleteButton => TextFieldFocus::List,
                                            _ => TextFieldFocus::TextInput,
                                        };
                                    }
                                } else {
                                    let count = app.control_count();
                                    if count > 0 {
                                        app.selected_control = (app.selected_control + 1) % count;
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
                                        1 => {
                                            let curr = app.glyph_list_state.selected().unwrap_or(0);
                                            app.glyph_list_state
                                                .select(Some(curr.saturating_sub(1)));
                                        }
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
                                    } else {
                                        app.text_field_focus = match app.text_field_focus {
                                            TextFieldFocus::DeleteButton => {
                                                TextFieldFocus::BgButton
                                            }
                                            TextFieldFocus::BgButton => TextFieldFocus::FgButton,
                                            TextFieldFocus::FgButton => TextFieldFocus::H,
                                            TextFieldFocus::H => TextFieldFocus::W,
                                            TextFieldFocus::W => TextFieldFocus::Y,
                                            TextFieldFocus::Y => TextFieldFocus::X,
                                            TextFieldFocus::X => TextFieldFocus::TextInput,
                                            TextFieldFocus::TextInput => TextFieldFocus::List,
                                            _ => TextFieldFocus::List,
                                        };
                                    }
                                } else {
                                    let count = app.control_count();
                                    if count > 0 {
                                        app.selected_control =
                                            (app.selected_control + count - 1) % count;
                                    }
                                }
                            }
                            KeyCode::Left | KeyCode::Char('h') => {
                                if app.current_tab == 3 {
                                    if let Some(sel) = app.text_selected {
                                        if sel > 0 && sel <= app.render_args.overlays.len() {
                                            match app.text_field_focus {
                                                TextFieldFocus::X => {
                                                    app.render_args.overlays[sel - 1].x -= 1;
                                                    app.reapply_overlays();
                                                }
                                                TextFieldFocus::Y => {
                                                    app.render_args.overlays[sel - 1].y -= 1;
                                                    app.reapply_overlays();
                                                }
                                                TextFieldFocus::W => {
                                                    app.render_args.overlays[sel - 1].w =
                                                        (app.render_args.overlays[sel - 1].w - 1)
                                                            .max(0);
                                                    app.reapply_overlays();
                                                }
                                                TextFieldFocus::H => {
                                                    app.render_args.overlays[sel - 1].h =
                                                        (app.render_args.overlays[sel - 1].h - 1)
                                                            .max(0);
                                                    app.reapply_overlays();
                                                }
                                                _ => {}
                                            }
                                        }
                                    }
                                } else if app.current_tab == 2 {
                                    if app.selected_control > 0 {
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
                                    let d = if key.modifiers.contains(KeyModifiers::SHIFT) {
                                        10
                                    } else {
                                        1
                                    };
                                    let ctrl_id = app.get_current_control_id();
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
                                        let multiplier =
                                            2_i32.pow((app.slider_repeat_count / 8).min(6) as u32);
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
                                    if let Some(sel) = app.text_selected {
                                        if sel > 0 && sel <= app.render_args.overlays.len() {
                                            match app.text_field_focus {
                                                TextFieldFocus::X => {
                                                    app.render_args.overlays[sel - 1].x += 1;
                                                    app.reapply_overlays();
                                                }
                                                TextFieldFocus::Y => {
                                                    app.render_args.overlays[sel - 1].y += 1;
                                                    app.reapply_overlays();
                                                }
                                                TextFieldFocus::W => {
                                                    app.render_args.overlays[sel - 1].w += 1;
                                                    app.reapply_overlays();
                                                }
                                                TextFieldFocus::H => {
                                                    app.render_args.overlays[sel - 1].h += 1;
                                                    app.reapply_overlays();
                                                }
                                                _ => {}
                                            }
                                        }
                                    }
                                } else if app.current_tab == 2 {
                                    if app.selected_control < 1 {
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
                                    let d = if key.modifiers.contains(KeyModifiers::SHIFT) {
                                        10
                                    } else {
                                        1
                                    };
                                    let ctrl_id = app.get_current_control_id();
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
                                        let multiplier =
                                            2_i32.pow((app.slider_repeat_count / 8).min(6) as u32);
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
                                    if app.selected_control == 0 {
                                        app.toggle_tree_selection();
                                    }
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
                            KeyCode::Enter => {
                                if app.current_tab == 3 {
                                    if app.text_field_focus == TextFieldFocus::List {
                                        if app.text_selected == Some(0) {
                                            // Add new overlay
                                            app.render_args.overlays.push(
                                                crate::args::TextOverlay {
                                                    text: "Text".to_string(),
                                                    x: 0,
                                                    y: 0,
                                                    w: 0,
                                                    h: 0,
                                                    fg: None,
                                                    bg: None,
                                                    wrap: false,
                                                    bold: false,
                                                    italic: false,
                                                    underline: false,
                                                },
                                            );
                                            app.text_selected =
                                                Some(app.render_args.overlays.len());
                                            app.text_list_state.select(app.text_selected);
                                            app.text_field_focus = TextFieldFocus::TextInput;
                                            app.mark_args_dirty();
                                        } else if let Some(sel) = app.text_selected {
                                            if sel > 0 && sel <= app.render_args.overlays.len() {
                                                // Focus on editor
                                                app.text_field_focus = TextFieldFocus::TextInput;
                                            }
                                        }
                                    } else if app.text_field_focus == TextFieldFocus::TextInput {
                                        if let Some(sel) = app.text_selected {
                                            if sel > 0 && sel <= app.render_args.overlays.len() {
                                                app.value_edit_buffer =
                                                    app.render_args.overlays[sel - 1].text.clone();
                                                app.input_mode = InputMode::TextEdit;
                                            }
                                        }
                                    } else if app.text_field_focus == TextFieldFocus::FgButton {
                                        // Open color picker for FG
                                        let mode = match app.render_mode_idx {
                                            0 => crate::colorpicker::PaletteMode::Irc,
                                            1 => crate::colorpicker::PaletteMode::Ansi,
                                            2 => crate::colorpicker::PaletteMode::Ansi24,
                                            _ => crate::colorpicker::PaletteMode::Irc,
                                        };
                                        let mut cp =
                                            crate::colorpicker::ColorPickerState::new(mode, true);
                                        cp.editing_fg = true;
                                        // Pre-load current fg if set
                                        if let Some(sel) = app.text_selected {
                                            if sel > 0 && sel <= app.render_args.overlays.len() {
                                                if let Some(ref fg) =
                                                    app.render_args.overlays[sel - 1].fg
                                                {
                                                    match fg {
                                                        crate::args::ColorSpec::Rgb(rgb) => {
                                                            cp.set_rgb(rgb[0], rgb[1], rgb[2])
                                                        }
                                                        crate::args::ColorSpec::Index(i) => {
                                                            cp.set_grid_idx(*i as usize)
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        app.color_picker_state = Some(cp);
                                    } else if app.text_field_focus == TextFieldFocus::BgButton {
                                        // Open wcolor picker for BG (only if FG is set)
                                        if let Some(sel) = app.text_selected {
                                            if sel > 0
                                                && sel <= app.render_args.overlays.len()
                                                && app.render_args.overlays[sel - 1].fg.is_some()
                                            {
                                                let mode = match app.render_mode_idx {
                                                    0 => crate::colorpicker::PaletteMode::Irc,
                                                    1 => crate::colorpicker::PaletteMode::Ansi,
                                                    2 => crate::colorpicker::PaletteMode::Ansi24,
                                                    _ => crate::colorpicker::PaletteMode::Irc,
                                                };
                                                let mut cp =
                                                    crate::colorpicker::ColorPickerState::new(
                                                        mode, true,
                                                    );
                                                cp.editing_fg = false;
                                                if let Some(ref bg) =
                                                    app.render_args.overlays[sel - 1].bg
                                                {
                                                    match bg {
                                                        crate::args::ColorSpec::Rgb(rgb) => {
                                                            cp.bg_rgb =
                                                                Some((rgb[0], rgb[1], rgb[2]));
                                                            cp.set_rgb(rgb[0], rgb[1], rgb[2]);
                                                        }
                                                        crate::args::ColorSpec::Index(i) => {
                                                            cp.bg_idx = Some(*i as usize);
                                                            cp.set_grid_idx(*i as usize);
                                                        }
                                                    }
                                                }
                                                app.color_picker_state = Some(cp);
                                            }
                                        }
                                    } else if app.text_field_focus == TextFieldFocus::DeleteButton {
                                        if let Some(sel) = app.text_selected {
                                            if sel > 0 && sel <= app.render_args.overlays.len() {
                                                app.render_args.overlays.remove(sel - 1);
                                                let new_len = app.render_args.overlays.len();
                                                if new_len == 0 {
                                                    app.text_selected = Some(0);
                                                } else {
                                                    app.text_selected = Some(sel.min(new_len));
                                                }
                                                app.text_list_state.select(app.text_selected);
                                                app.text_field_focus = TextFieldFocus::List;
                                                app.mark_args_dirty();
                                            }
                                        }
                                    }
                                } else if app.current_tab == 2 {
                                    if app.selected_control == 0 {
                                        app.toggle_tree_selection();
                                    }
                                } else if app.current_tab == 1 {
                                    if let Some(expanded_idx) = app.pipeline_expanded {
                                        let selected_catalog_idx = app.pipeline_library_selected;
                                        if let Some(effect) =
                                            app.pipeline_preview_effect.take().or_else(|| {
                                                PIPELINE_CATALOG
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
                                    } else {
                                        app.pipeline_expanded = Some(app.selected_control);
                                    }
                                } else {
                                    let controls = app.tab_controls(0);
                                    if let Some(ctrl) = controls.get(app.selected_control) {
                                        if let ControlKind::Slider { .. } = &ctrl.kind {
                                            let current_val = app.get_value(ctrl.id);
                                            app.value_edit_buffer = current_val.to_string();
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
                if last.elapsed() >= std::time::Duration::from_millis(500) {
                    app.flush_render_args();
                }
            }
        }
    }
}

// ── Control Logic ────────────────────────────────────────────────────────────
fn adjust_control(app: &mut App, delta: i32) {
    let id = app.get_current_control_id();
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
            let val = app.get_value(id);
            let count = app.fonts.len() as i32;
            if count > 0 {
                if delta > 0 {
                    app.set_value(id, (val + 1) % count);
                } else {
                    app.set_value(id, (val + count - 1) % count);
                }
            }
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
    app.mark_args_dirty();
}

fn set_control_value(app: &mut App, visual_idx: usize, val: i32) {
    let id = if app.current_tab == 0 {
        let ctrls = app.tab_controls(0);
        ctrls[visual_idx.min(ctrls.len() - 1)].id
    } else {
        return;
    };

    app.set_value(id, val);

    log::debug!(
        "Value set: tab={}, visual={}, id={:?}, val={}",
        app.current_tab,
        visual_idx,
        id,
        val
    );
    app.mark_args_dirty();
}

fn toggle_control(app: &mut App) {
    let id = app.get_current_control_id();
    let def = get_control_def(id);

    match def.kind {
        ControlKind::Toggle { .. } => {
            let val = app.get_value(id);
            app.set_value(id, if val == 0 { 1 } else { 0 });
        }
        ControlKind::Enum { options, .. } => {
            let val = app.get_value(id);
            app.set_value(id, (val + 1) % options.len() as i32);
        }
        ControlKind::FontSelector => {
            if !app.fonts.is_empty() {
                app.font_idx = (app.font_idx + 1) % app.fonts.len();
            }
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
    app.mark_args_dirty();
}

fn reset_control(app: &mut App) {
    let id = app.get_current_control_id();
    let def = get_control_def(id);

    match def.kind {
        ControlKind::Slider { default, .. } => app.set_value(id, default),
        ControlKind::Toggle { default } => app.set_value(id, if default { 1 } else { 0 }),
        ControlKind::Enum { default, .. } => app.set_value(id, default as i32),
        ControlKind::FontSelector => app.font_idx = 0,
        ControlKind::EyedropperWidget | ControlKind::Action => {}
    }

    log::info!("Control reset: tab={}, id={:?}", app.current_tab, id);
    app.mark_args_dirty();
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
        let height = (output_area.height / 2).max(5);
        let area = Rect::new(
            output_area.x,
            output_area.y + output_area.height - height,
            output_area.width,
            height,
        );

        f.render_widget(Clear, area);

        let title = if app.font_search.is_empty() {
            " Select Font ".to_string()
        } else {
            format!(" Select Font (Search: {}) ", app.font_search)
        };

        let block = Block::default()
            .title(title)
            .borders(Borders::TOP | Borders::LEFT | Borders::RIGHT) // Maybe just TOP?
            .style(Style::new().bg(SURFACE));

        // List
        let query = app.font_search.to_lowercase();
        let items: Vec<ListItem> = app
            .fonts
            .iter()
            .enumerate()
            .filter(|(_, f)| f.to_lowercase().contains(&query))
            .map(|(i, f)| {
                let _is_selected = app.font_idx == i; // Visual check? No, font_list_state handles selection.
                                                      // But we want to indicate the *current* font too?
                                                      // The list allows picking a NEW font.
                ListItem::new(f.as_str()).style(Style::new().fg(TEXT))
            })
            .collect();

        let list = List::new(items)
            .block(block)
            .highlight_style(Style::new().add_modifier(Modifier::REVERSED));

        f.render_stateful_widget(list, area, &mut app.font_list_state);
    }

    if app.input_mode == InputMode::AutoOptimize {
        let vert = Layout::vertical([
            Constraint::Percentage(20),
            Constraint::Length(34),
            Constraint::Percentage(20),
        ])
        .split(area);
        let horiz = Layout::horizontal([
            Constraint::Percentage(20),
            Constraint::Length(60),
            Constraint::Percentage(20),
        ])
        .split(vert[1]);

        let overlay_area = horiz[1];
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

    // Render active selection outline around selected text overlay
    if app.current_tab == 3 {
        let inner_height = app.output_area.height.saturating_sub(2);
        let inner_width = app.output_area.width.saturating_sub(2);
        let line_count = app.cached_render.lines().count() as u16;
        let pad_y = if line_count < inner_height {
            inner_height.saturating_sub(line_count) / 2
        } else {
            0
        };

        // Compute pad_x from the rendered grid
        let grid_cols = app
            .cached_result
            .as_ref()
            .map_or(0u16, |r| r.grid.first().map_or(0, |row| row.len()) as u16);
        let pad_x = if inner_width > grid_cols {
            (inner_width - grid_cols) / 2
        } else {
            0
        };

        if let Some(sel) = app.text_selected {
            if sel > 0 && sel <= app.render_args.overlays.len() {
                let ov = &app.render_args.overlays[sel - 1];
                let text_to_draw = if app.input_mode == InputMode::TextEdit {
                    app.value_edit_buffer.clone()
                } else {
                    ov.text.clone()
                };

                let lines: Vec<&str> = text_to_draw.split('\n').collect();
                // Each cell takes 2 terminal columns
                let text_cell_w = lines
                    .iter()
                    .map(|l| l.chars().count() as i32)
                    .max()
                    .unwrap_or(1)
                    .max(1);
                let text_h = lines.len().max(1) as i32;

                let box_w = if ov.w > 0 { ov.w } else { text_cell_w };
                let box_h = if ov.h > 0 { ov.h } else { text_h };

                // Position in terminal coordinates (each cell = 1 column)
                let rx = app.output_area.x + 1 + pad_x + (ov.x as u16);
                let ry = app.output_area.y + 1 + pad_y + ov.y as u16;
                let rw = box_w as u16;
                let rh = box_h as u16;

                if rw > 0 && rh > 0 {
                    // Draw a thin selection outline around the overlay region
                    // without clearing the underlying rendered content
                    let is_dragging = app.text_drag_mode == TextDragMode::Sizing
                        || app.text_drag_mode == TextDragMode::Moving;
                    let border_color = if is_dragging {
                        Color::Yellow
                    } else if app.input_mode == InputMode::TextEdit {
                        Color::Green
                    } else {
                        Color::Cyan
                    };
                    let border_style = Style::default().fg(border_color);

                    // Top border
                    if ry > 0 && ry > app.output_area.y {
                        let top_y = ry.saturating_sub(1);
                        let start_bx = rx.saturating_sub(1);
                        let end_bx = rx + rw;
                        for bx in start_bx..=end_bx {
                            if bx >= app.output_area.x
                                && bx < app.output_area.x + app.output_area.width
                            {
                                if let Some(cell) = f
                                    .buffer_mut()
                                    .cell_mut(ratatui::layout::Position::new(bx, top_y))
                                {
                                    cell.set_style(border_style);
                                    cell.set_char(if bx == start_bx {
                                        '┌'
                                    } else if bx == end_bx {
                                        '┐'
                                    } else {
                                        '─'
                                    });
                                }
                            }
                        }
                    }
                    // Bottom border
                    let bot_y = ry + rh;
                    if bot_y < app.output_area.y + app.output_area.height {
                        let start_bx = rx.saturating_sub(1);
                        let end_bx = rx + rw;
                        for bx in start_bx..=end_bx {
                            if bx >= app.output_area.x
                                && bx < app.output_area.x + app.output_area.width
                            {
                                if let Some(cell) = f
                                    .buffer_mut()
                                    .cell_mut(ratatui::layout::Position::new(bx, bot_y))
                                {
                                    cell.set_style(border_style);
                                    cell.set_char(if bx == start_bx {
                                        '└'
                                    } else if bx == end_bx {
                                        '┘'
                                    } else {
                                        '─'
                                    });
                                }
                            }
                        }
                    }
                    // Left/right borders
                    for dy in 0..rh {
                        let by = ry + dy;
                        if by >= app.output_area.y
                            && by < app.output_area.y + app.output_area.height
                        {
                            // Left
                            let lx = rx.saturating_sub(1);
                            if lx >= app.output_area.x
                                && lx < app.output_area.x + app.output_area.width
                            {
                                if let Some(cell) = f
                                    .buffer_mut()
                                    .cell_mut(ratatui::layout::Position::new(lx, by))
                                {
                                    cell.set_style(border_style);
                                    cell.set_char('│');
                                }
                            }
                            // Right
                            let rrx = rx + rw;
                            if rrx >= app.output_area.x
                                && rrx < app.output_area.x + app.output_area.width
                            {
                                if let Some(cell) = f
                                    .buffer_mut()
                                    .cell_mut(ratatui::layout::Position::new(rrx, by))
                                {
                                    cell.set_style(border_style);
                                    cell.set_char('│');
                                }
                            }
                        }
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
    if let Some(res) = &app.cached_result {
        let initial_ms = res.initial_time.as_millis();
        if initial_ms > 0 || res.fix_disc_time.is_some() {
            right_text.push_str(&format!(" {}ms", initial_ms));
            if let Some(fd_time) = res.fix_disc_time {
                right_text.push_str(&format!(" (+{}ms)", fd_time.as_millis()));
            }
            right_text.push_str(" ");
        }
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
            " Output: {}x{} chars ({}x{} px) | Cell: {}x{} px | Font: {:.1}x{:.1} px | Line {} ({} bytes) | Error: {:.3}% ",
            width_chars, height_chars, w_px, h_px, cw, ch, fw, fh, longest_line_idx, max_bytes, p_score
        );

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

        let text = app
            .cached_render
            .as_bytes()
            .into_text()
            .unwrap_or_else(|_| ratatui::text::Text::from(app.cached_render.clone()));

        // Vertical center calculation
        let line_count = app.cached_render.lines().count() as u16;
        let pad_y = if line_count < inner.height {
            inner.height.saturating_sub(line_count).saturating_div(2)
        } else {
            0
        };

        let layout = Layout::vertical([Constraint::Length(pad_y), Constraint::Min(0)]).split(inner);

        let output = Paragraph::new(text)
            .alignment(Alignment::Center)
            .scroll((app.scroll, 0));

        f.render_widget(output, layout[1]);

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

fn render_status_bar(f: &mut ratatui::Frame, _app: &mut App, area: Rect) {
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
    full_spans.extend(spans);

    let line = Line::from(full_spans);
    let bar = Paragraph::new(line).style(Style::new().bg(STATUS_BG));
    f.render_widget(bar, area);
}
