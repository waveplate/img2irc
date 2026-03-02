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
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
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
const BAR_BG: Color = Color::Rgb(40, 40, 58);
const TOGGLE_ON: Color = Color::Rgb(70, 190, 120);
const TOGGLE_OFF: Color = Color::Rgb(70, 70, 90);
const SELECTED_BG: Color = Color::Rgb(50, 50, 72);
const TAB_ACTIVE: Color = Color::Rgb(130, 140, 255);
const TAB_INACTIVE: Color = Color::Rgb(80, 80, 100);
const STATUS_BG: Color = Color::Rgb(35, 35, 50);

// ── Control types ────────────────────────────────────────────────────────────
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ControlId {
    Width,
    RenderMode,
    Smooth,
    NoGlyphInvert,
    Relax,
    RelaxStrength,
    HighlightDisc,
    FixDisc,
    FixStrength,
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
    Misc1,
    Misc2,
    Misc3,
    Misc4,
    Misc5,
    Misc6,
    Misc7,
    Misc8,
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
}

#[derive(Clone, Copy)]
struct ControlDef {
    id: ControlId,
    label: &'static str,
    kind: ControlKind,
}

const GENERAL_CONTROLS: &[ControlDef] = &[
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
        id: ControlId::RenderMode,
        label: "Mode",
        kind: ControlKind::Enum {
            options: &["IRC", "ANSI", "ANSI24"],
            default: 1,
        },
    },
    ControlDef {
        id: ControlId::Smooth,
        label: "Smooth",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::NoGlyphInvert,
        label: "No Inv Glyphs",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::Relax,
        label: "Relax",
        kind: ControlKind::Toggle { default: false },
    },
    ControlDef {
        id: ControlId::RelaxStrength,
        label: "Relax Str",
        kind: ControlKind::Slider {
            min: 0,
            max: 4000,
            default: 40,
        },
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
        id: ControlId::FixStrength,
        label: "Fix Strength",
        kind: ControlKind::Slider {
            min: 0,
            max: 1000000000,
            default: 4,
        },
    },
    ControlDef {
        id: ControlId::Misc1,
        label: "Smooth Iter",
        kind: ControlKind::Slider {
            min: 0,
            max: 100,
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::Misc2,
        label: "Edge Importance",
        kind: ControlKind::Slider {
            min: 0,
            max: 1000,
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::Misc3,
        label: "Misc3",
        kind: ControlKind::Slider {
            min: 0,
            max: 100000,
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::Misc4,
        label: "Relax Iter",
        kind: ControlKind::Slider {
            min: 0,
            max: 100,
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::Misc5,
        label: "Misc5",
        kind: ControlKind::Slider {
            min: 0,
            max: 100000,
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::Misc6,
        label: "Misc6",
        kind: ControlKind::Slider {
            min: 0,
            max: 100000,
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::Misc7,
        label: "Misc7",
        kind: ControlKind::Slider {
            min: 0,
            max: 100000,
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::Misc8,
        label: "Misc8",
        kind: ControlKind::Slider {
            min: 0,
            max: 100000,
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::TrimTop,
        label: "Crop Top",
        kind: ControlKind::Slider {
            min: 0,
            max: 2000,
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::TrimRight,
        label: "Crop Right",
        kind: ControlKind::Slider {
            min: 0,
            max: 2000,
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::TrimBottom,
        label: "Crop Bottom",
        kind: ControlKind::Slider {
            min: 0,
            max: 2000,
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::TrimLeft,
        label: "Crop Left",
        kind: ControlKind::Slider {
            min: 0,
            max: 2000,
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
        id: ControlId::Rotate,
        label: "Rotate",
        kind: ControlKind::Slider {
            min: -180,
            max: 180,
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::ScaleX,
        label: "Scale X %",
        kind: ControlKind::Slider {
            min: 10,
            max: 500,
            default: 100,
        },
    },
    ControlDef {
        id: ControlId::ScaleY,
        label: "Scale Y %",
        kind: ControlKind::Slider {
            min: 10,
            max: 500,
            default: 100,
        },
    },
    ControlDef {
        id: ControlId::FontSize,
        label: "Font Size",
        kind: ControlKind::Slider {
            min: 0,
            max: 256,
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::Font,
        label: "Font",
        kind: ControlKind::FontSelector,
    },
    ControlDef {
        id: ControlId::Eyedropper,
        label: "Select Color",
        kind: ControlKind::EyedropperWidget,
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
        id: ControlId::Brightness,
        label: "Brightness",
        kind: ControlKind::Slider {
            min: -255,
            max: 255,
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::Contrast,
        label: "Contrast",
        kind: ControlKind::Slider {
            min: -255,
            max: 255,
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::Saturation,
        label: "Saturation",
        kind: ControlKind::Slider {
            min: -255,
            max: 255,
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::Gamma,
        label: "Gamma",
        kind: ControlKind::Slider {
            min: 0,
            max: 255,
            default: 0,
        },
    },
    ControlDef {
        id: ControlId::Hue,
        label: "Hue",
        kind: ControlKind::Slider {
            min: 0,
            max: 360,
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
        .chain(EFFECT_CONTROLS.iter())
        .find(|c| c.id == id)
        .copied()
        .expect("Control ID not found")
}

// ── App State ────────────────────────────────────────────────────────────────
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputMode {
    Normal,
    Explorer,
    ValueEdit,
    MetadataPrompt,
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

struct App {
    render_args: RenderArgs,
    input_mode: InputMode,
    selected_control: usize,
    current_tab: usize,
    scroll: u16,

    // Cancellation & Debouncing
    tx_args: watch::Sender<(RenderArgs, Arc<AtomicBool>)>,
    current_cancel_flag: Arc<AtomicBool>,
    args_dirty: bool,
    last_args_change: Option<Instant>,

    rx_result: mpsc::Receiver<(draw::RenderResult, Arc<font::GlyphStore>)>,
    cached_render: String,
    cached_result: Option<draw::RenderResult>,
    is_rendering: Arc<AtomicBool>,
    render_tick: usize,
    dragged_control: Option<usize>,
    value_edit_buffer: String,

    effects_pipeline: Vec<ControlId>,
    reordering_grabbed: Option<usize>,

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
    smooth: bool,
    no_glyph_invert: bool,
    relax: bool,
    relax_strength: i32,
    show_discontinuities: bool,
    fix_disc: bool,
    fix_strength: u32,

    misc1: i32,
    misc2: i32,
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
            scroll: 0,
            tx_args,
            current_cancel_flag: initial_flag,
            args_dirty: false,
            last_args_change: None,
            rx_result,
            cached_render: String::new(),
            cached_result: None,
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
            metadata_prompt_png_path: String::new(),

            tree_state,
            glyph_list_state: ListState::default(),

            image_path: render_args.image.clone().unwrap_or_default(),
            width: render_args.width.unwrap_or(80) as i32,
            render_mode_idx: match render_args.render {
                Render::Preview => 0,
                Render::Ansi => 1,
                Render::Ansi24 => 2,
                Render::Irc => 0,
            },
            dither: render_args.dither as i32,
            smooth: render_args.smooth,
            no_glyph_invert: render_args.no_glyph_invert,
            relax: render_args.relax,
            relax_strength: (render_args.relax_strength * 10.0) as i32,
            show_discontinuities: render_args.show_discontinuities,
            fix_disc: render_args.fix_disc,
            fix_strength: render_args.fix_strength,
            misc1: render_args.misc1,
            misc2: render_args.misc2,
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

            effects_pipeline: EFFECT_CONTROLS.iter().map(|c| c.id).collect(),
            reordering_grabbed: None,

            available_blocks,
            explorer_dialog: None,
            control_areas: Vec::new(),
            tab_areas: Vec::new(),
            path_area: Rect::default(),
            tree_area: Rect::default(),
            glyph_list_area: Rect::default(),
            explorer_area: Rect::default(),
            preview_area: Rect::default(),
            output_area: Rect::default(),
            options_pane_area: Rect::default(),
            options_areas: Vec::new(),
            eyedropper_color: None,
        }
    }

    fn apply_settings(&mut self, render_args: &RenderArgs) {
        self.render_args = render_args.clone();
        self.width = render_args.width.unwrap_or(80) as i32;
        self.render_mode_idx = match render_args.render {
            Render::Preview => 0,
            Render::Ansi => 1,
            Render::Ansi24 => 2,
            Render::Irc => 0,
        };
        self.dither = render_args.dither as i32;
        self.smooth = render_args.smooth;
        self.no_glyph_invert = render_args.no_glyph_invert;
        self.relax = render_args.relax;
        self.relax_strength = (render_args.relax_strength * 10.0) as i32;
        self.show_discontinuities = render_args.show_discontinuities;
        self.fix_disc = render_args.fix_disc;
        self.fix_strength = render_args.fix_strength;
        self.misc1 = render_args.misc1;
        self.misc2 = render_args.misc2;
        self.misc3 = render_args.misc3;
        self.misc4 = render_args.misc4;
        self.misc5 = render_args.misc5;
        self.misc6 = render_args.misc6;
        self.misc7 = render_args.misc7;
        self.misc8 = render_args.misc8;
        self.invert = render_args.invert;
        self.grayscale = render_args.grayscale;
        self.brightness = 0;
        self.contrast = 0;
        self.saturation = 0;
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
            GENERAL_CONTROLS[self.selected_control.min(GENERAL_CONTROLS.len() - 1)].id
        } else {
            self.effects_pipeline[self.selected_control.min(self.effects_pipeline.len() - 1)]
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
            ControlId::FontSize => self.font_size as i32,
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
            ControlId::Smooth => self.smooth as i32,
            ControlId::NoGlyphInvert => self.no_glyph_invert as i32,
            ControlId::Relax => self.relax as i32,
            ControlId::RelaxStrength => self.relax_strength,
            ControlId::HighlightDisc => self.show_discontinuities as i32,
            ControlId::FixDisc => self.fix_disc as i32,
            ControlId::FixStrength => self.fix_strength as i32,
            ControlId::Misc1 => self.misc1,
            ControlId::Misc2 => self.misc2,
            ControlId::Misc3 => self.misc3,
            ControlId::Misc4 => self.misc4,
            ControlId::Misc5 => self.misc5,
            ControlId::Misc6 => self.misc6,
            ControlId::Misc7 => self.misc7,
            ControlId::Misc8 => self.misc8,
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
        }
    }

    fn set_value(&mut self, id: ControlId, val: i32) {
        match id {
            ControlId::Width => self.width = val,
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
            ControlId::FontSize => self.font_size = (val as f32),
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
            ControlId::Smooth => self.smooth = val != 0,
            ControlId::NoGlyphInvert => self.no_glyph_invert = val != 0,
            ControlId::Relax => self.relax = val != 0,
            ControlId::RelaxStrength => self.relax_strength = val,
            ControlId::HighlightDisc => self.show_discontinuities = val != 0,
            ControlId::FixDisc => self.fix_disc = val != 0,
            ControlId::FixStrength => self.fix_strength = val as u32,
            ControlId::Misc1 => self.misc1 = val,
            ControlId::Misc2 => self.misc2 = val,
            ControlId::Misc3 => self.misc3 = val,
            ControlId::Misc4 => self.misc4 = val,
            ControlId::Misc5 => self.misc5 = val,
            ControlId::Misc6 => self.misc6 = val,
            ControlId::Misc7 => self.misc7 = val,
            ControlId::Misc8 => self.misc8 = val,
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
            ControlId::Eyedropper => {}
        }
    }

    fn tab_controls(&self, tab: usize) -> Vec<ControlDef> {
        match tab {
            0 => GENERAL_CONTROLS.to_vec(),
            1 => self
                .effects_pipeline
                .iter()
                .map(|&id| get_control_def(id))
                .collect(),
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
        if self.current_tab == 2 {
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
            GENERAL_CONTROLS[idx.min(GENERAL_CONTROLS.len() - 1)].id
        } else {
            self.effects_pipeline[idx.min(self.effects_pipeline.len() - 1)]
        };
        self.get_value(id)
    }

    fn mark_args_dirty(&mut self) {
        self.args_dirty = true;
        self.last_args_change = Some(Instant::now());
    }

    fn flush_render_args(&mut self) {
        if !self.args_dirty {
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
        self.render_args.smooth = self.smooth;
        self.render_args.no_glyph_invert = self.no_glyph_invert;
        self.render_args.relax = self.relax;
        self.render_args.relax_strength = self.relax_strength as f32 / 10.0;
        self.render_args.show_discontinuities = self.show_discontinuities;
        self.render_args.fix_disc = self.fix_disc;
        self.render_args.fix_strength = self.fix_strength;
        self.render_args.misc1 = self.misc1;
        self.render_args.misc2 = self.misc2;
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
            0 => Render::Preview,
            1 => Render::Ansi,
            _ => Render::Ansi24,
        };
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
        self.render_args.crop = None;

        // Pipeline
        let mut pipeline = vec![
            "Brightness".to_string(),
            "Saturation".to_string(),
            "Contrast".to_string(),
            "Hue".to_string(),
            "Gamma".to_string(),
        ];
        for &id in &self.effects_pipeline {
            pipeline.push(get_control_def(id).label.to_string());
        }
        self.render_args.pipeline = pipeline;

        // Cancel previous render
        self.current_cancel_flag.store(true, Ordering::Relaxed);
        // Create new flag
        let new_flag = Arc::new(AtomicBool::new(false));
        self.current_cancel_flag = new_flag.clone();

        let _ = self.tx_args.send((self.render_args.clone(), new_flag));
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

            let tmp_path = format!("/tmp/img2irc_clipboard.{}", ext);
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
    // Try to use fc-list
    if let Ok(output) = Command::new("fc-list").arg(":family").output() {
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

/// A gauge-style slider bar. Value is rendered inside the filled portion.
fn render_bar(
    buf: &mut Buffer,
    area: Rect,
    label: &str,
    value: i32,
    min: i32,
    max: i32,
    selected: bool,
) {
    if area.height < 1 || area.width < 4 {
        return;
    }

    let range = (max - min).max(1) as f64;
    let ratio = ((value - min) as f64 / range).clamp(0.0, 1.0);
    let filled_width = ((area.width as f64) * ratio) as u16;

    let bg_style = if selected {
        Style::new().bg(SELECTED_BG)
    } else {
        Style::new().bg(SURFACE)
    };
    buf.set_style(area, bg_style);

    // Draw filled portion
    let bar_style = Style::new()
        .bg(if selected { BAR_FG } else { ACCENT_DIM })
        .fg(Color::White);
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + filled_width.min(area.width) {
            buf.set_string(x, y, " ", bar_style);
        }
    }

    // Draw unfilled portion
    let empty_style = Style::new().bg(BAR_BG).fg(TEXT_DIM);
    for y in area.y..area.y + area.height {
        for x in (area.x + filled_width)..area.x + area.width {
            buf.set_string(x, y, " ", empty_style);
        }
    }

    // Draw label on the left
    let label_text = format!(" {}", label);
    let lx = area.x + 1;
    let ly = area.y + area.height.saturating_sub(1) / 2;
    for (i, ch) in label_text.chars().enumerate() {
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

    // Draw value on the right
    let value_text = if value == 0 {
        "Auto ".to_string()
    } else {
        format!("{} ", value)
    };
    let vx = area.x + area.width - value_text.len() as u16;
    for (i, ch) in value_text.chars().enumerate() {
        let x = vx + i as u16;
        if x < area.x || x >= area.x + area.width {
            continue;
        }
        let cell = &mut buf[(x, ly)];
        cell.set_char(ch);
        let style = if value != 0 {
            Style::new().fg(TEXT).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)
        };
        cell.set_style(style);
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
    let label_text = format!(" {}", label);
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
    let label_text = format!(" {}", label);
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

    let val_text = "🯩███🯫 ";
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
    let label_text = format!(" {}", label);
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
        eprintln!("PANIC at {}: {}", location, message);
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
        if (render_args.font_size - 0.0).abs() < f32::EPSILON {
            24.0
        } else {
            render_args.font_size
        },
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
        while let Ok((result, glyphs)) = app.rx_result.try_recv() {
            if result.content == "Cancelled" {
                continue;
            }
            app.cached_render = result.content.clone();
            app.cached_result = Some(result);
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
                                app.input_mode = InputMode::Normal;
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
                                    app.input_mode = InputMode::Normal;
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
                                                app.input_mode = InputMode::Normal;
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

            // --- Metadata Prompt Mode ---
            if app.input_mode == InputMode::MetadataPrompt {
                if let Event::Key(key) = event {
                    if key.kind == KeyEventKind::Press {
                        match key.code {
                            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                                // Load metadata
                                if let Some(settings) = app.metadata_prompt_settings.take() {
                                    app.image_path = app.metadata_prompt_path.clone();
                                    app.render_args.image = Some(app.metadata_prompt_path.clone());
                                    app.apply_settings(&settings);
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

            match event {
                Event::Mouse(mouse) => {
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

                            if app.current_tab == 2 {
                                // Scroll focused pane in Glyphs tab
                                match app.selected_control {
                                    0 => app.tree_state.select_prev(),
                                    1 => {
                                        let curr = app.glyph_list_state.selected().unwrap_or(0);
                                        app.glyph_list_state.select(Some(curr.saturating_sub(1)));
                                    }
                                    _ => {}
                                }
                            } else if app.selected_control > 0 {
                                app.selected_control -= 1;
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

                            if app.current_tab == 2 {
                                // Scroll focused pane in Glyphs tab
                                match app.selected_control {
                                    0 => app.tree_state.select_next(),
                                    1 => {
                                        let curr = app.glyph_list_state.selected().unwrap_or(0);
                                        app.glyph_list_state.select(Some(curr + 1));
                                    }
                                    _ => {}
                                }
                            } else {
                                app.selected_control += 1;
                            }
                        }
                        MouseEventKind::Down(MouseButton::Left) => {
                            app.dragged_control = None;

                            // Check if over output area for eyedropper
                            if mouse.column >= app.output_area.x
                                && mouse.column < app.output_area.x + app.output_area.width
                                && mouse.row >= app.output_area.y
                                && mouse.row < app.output_area.y + app.output_area.height
                            {
                                if app.get_current_control_id() == ControlId::Eyedropper {
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
                                                            if let Some((_, bitmap)) = g
                                                                .glyphs
                                                                .iter()
                                                                .find(|(c, _)| *c == cell.char)
                                                            {
                                                                let total_pixels =
                                                                    g.metrics.0 * g.metrics.1;
                                                                let set_bits: usize = bitmap
                                                                    .iter()
                                                                    .map(|row| {
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

                            // Check glyph tab special areas
                            if app.current_tab == 2 {
                                if mouse.column >= app.tree_area.x
                                    && mouse.column < app.tree_area.x + app.tree_area.width
                                    && mouse.row >= app.tree_area.y
                                    && mouse.row < app.tree_area.y + app.tree_area.height
                                {
                                    let row = (mouse.row - app.tree_area.y) as usize;
                                    let scroll = app.tree_state.scroll as usize;
                                    let clicked_idx = scroll + row;
                                    if clicked_idx == app.tree_state.selected_idx {
                                        // Click on already-selected item: toggle checkbox
                                        app.toggle_tree_selection();
                                    } else {
                                        app.tree_state.selected_idx = clicked_idx;
                                    }
                                } else if mouse.column >= app.glyph_list_area.x
                                    && mouse.column
                                        < app.glyph_list_area.x + app.glyph_list_area.width
                                    && mouse.row >= app.glyph_list_area.y
                                    && mouse.row
                                        < app.glyph_list_area.y + app.glyph_list_area.height
                                {
                                    let row = (mouse.row - app.glyph_list_area.y) as usize;
                                    app.glyph_list_state.select(Some(row));
                                }
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
                                    app.current_tab = i;
                                    app.selected_control = 0;
                                    break;
                                }
                            }
                            // Check control clicks
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
                                    app.selected_control = i;
                                    if app.current_tab == 1 && mouse.column < area.x + 4 {
                                        app.reordering_grabbed = Some(i);
                                    } else {
                                        app.dragged_control = Some(i);
                                        let controls = app.tab_controls(app.current_tab);
                                        if let Some(ctrl) = controls.get(i) {
                                            match &ctrl.kind {
                                                ControlKind::Slider { min, max, .. } => {
                                                    let rel_x = (mouse.column - area.x) as f64;
                                                    let ratio =
                                                        (rel_x / area.width as f64).clamp(0.0, 1.0);
                                                    let new_val = *min
                                                        + (((*max - *min) as f64) * ratio) as i32;
                                                    set_control_value(
                                                        &mut app,
                                                        i,
                                                        new_val.clamp(*min, *max),
                                                    );
                                                }
                                                ControlKind::Toggle { .. } => {
                                                    toggle_control(&mut app)
                                                }
                                                ControlKind::Enum { .. } => {
                                                    toggle_control(&mut app)
                                                }
                                                ControlKind::EyedropperWidget => {
                                                    // Placeholder behaviour when clicked
                                                }
                                                ControlKind::FontSelector => {
                                                    app.selected_control = i;
                                                }
                                            }
                                        }
                                    }
                                    break;
                                }
                            }
                        }
                        MouseEventKind::Drag(MouseButton::Left) => {
                            if let Some(f) = app.reordering_grabbed {
                                for (t, to_area) in app.control_areas.iter().enumerate() {
                                    if to_area.width > 0
                                        && mouse.row >= to_area.y
                                        && mouse.row < to_area.y + to_area.height
                                    {
                                        if f != t {
                                            let real_item = app.effects_pipeline.remove(f);
                                            app.effects_pipeline.insert(t, real_item);
                                            app.reordering_grabbed = Some(t);
                                            app.selected_control = t;
                                            app.mark_args_dirty();
                                        }
                                        break;
                                    }
                                }
                            } else if let Some(visual_idx) = app.dragged_control {
                                if let Some(area) = app.control_areas.get(visual_idx) {
                                    if area.width > 0 {
                                        let controls = app.tab_controls(app.current_tab);
                                        if let Some(ctrl) = controls.get(visual_idx) {
                                            if let ControlKind::Slider { min, max, .. } = &ctrl.kind
                                            {
                                                let rel_x = (mouse.column as i32 - area.x as i32)
                                                    .max(0)
                                                    as f64;
                                                let ratio =
                                                    (rel_x / area.width as f64).clamp(0.0, 1.0);
                                                let new_val =
                                                    *min + (((*max - *min) as f64) * ratio) as i32;
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
                        MouseEventKind::Up(MouseButton::Left) => {
                            app.dragged_control = None;
                            app.reordering_grabbed = None;
                        }
                        _ => {}
                    }
                }
                Event::Key(key) => {
                    if key.kind == KeyEventKind::Press {
                        match key.code {
                            KeyCode::Tab => {
                                if app.current_tab == 2 {
                                    app.selected_control = (app.selected_control + 1) % 3;
                                }
                            }
                            KeyCode::BackTab => {
                                if app.current_tab == 2 {
                                    app.selected_control = (app.selected_control + 2) % 3;
                                }
                            }
                            KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                return Ok(());
                            }
                            KeyCode::Char('r') | KeyCode::Delete => {
                                reset_control(&mut app);
                            }
                            KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                app.open_explorer(ExplorerMode::Open);
                            }
                            KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                app.paste_from_clipboard();
                            }
                            KeyCode::Char('s') => {
                                app.open_explorer(ExplorerMode::SaveText);
                            }
                            KeyCode::Char('p') => {
                                app.open_explorer(ExplorerMode::SavePng);
                            }
                            KeyCode::Left if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                app.current_tab = (app.current_tab + 2) % 3;
                                app.selected_control = 0;
                            }
                            KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                app.current_tab = (app.current_tab + 1) % 3;
                                app.selected_control = 0;
                            }
                            KeyCode::Esc => {
                                if app.current_tab == 2 {
                                    if app.selected_control > 0 {
                                        app.selected_control = 0;
                                    } else {
                                        app.tree_state.collapse_selected();
                                    }
                                }
                            }
                            KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('J') => {
                                if (key.modifiers.contains(KeyModifiers::SHIFT)
                                    || key.code == KeyCode::Char('J'))
                                    && app.current_tab == 1
                                {
                                    let i = app.selected_control;
                                    if i + 1 < app.effects_pipeline.len() {
                                        app.effects_pipeline.swap(i, i + 1);
                                        app.selected_control += 1;
                                        app.mark_args_dirty();
                                    }
                                } else if app.current_tab == 2 {
                                    match app.selected_control {
                                        0 => app.tree_state.select_next(),
                                        1 => {
                                            let curr = app.glyph_list_state.selected().unwrap_or(0);
                                            app.glyph_list_state.select(Some(curr + 1));
                                        }
                                        2 => {
                                            let line_count =
                                                app.cached_render.lines().count() as u16;
                                            if line_count > app.output_area.height {
                                                app.scroll = app.scroll.saturating_add(3);
                                            }
                                        }
                                        _ => {}
                                    }
                                } else {
                                    let count = app.control_count();
                                    if count > 0 {
                                        app.selected_control = (app.selected_control + 1) % count;
                                    }
                                }
                            }
                            KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('K') => {
                                if (key.modifiers.contains(KeyModifiers::SHIFT)
                                    || key.code == KeyCode::Char('K'))
                                    && app.current_tab == 1
                                {
                                    let i = app.selected_control;
                                    if i > 0 {
                                        app.effects_pipeline.swap(i, i - 1);
                                        app.selected_control -= 1;
                                        app.mark_args_dirty();
                                    }
                                } else if app.current_tab == 2 {
                                    match app.selected_control {
                                        0 => app.tree_state.select_prev(),
                                        1 => {
                                            let curr = app.glyph_list_state.selected().unwrap_or(0);
                                            app.glyph_list_state
                                                .select(Some(curr.saturating_sub(1)));
                                        }
                                        2 => {
                                            let line_count =
                                                app.cached_render.lines().count() as u16;
                                            if line_count > app.output_area.height {
                                                app.scroll = app.scroll.saturating_sub(3);
                                            }
                                        }
                                        _ => {}
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
                                if app.current_tab == 2 {
                                    if app.selected_control == 0 {
                                        app.tree_state.collapse_selected();
                                    }
                                } else {
                                    let d = if key.modifiers.contains(KeyModifiers::SHIFT) {
                                        10
                                    } else {
                                        1
                                    };
                                    adjust_control(&mut app, -d);
                                }
                            }
                            KeyCode::Right | KeyCode::Char('l') => {
                                if app.current_tab == 2 {
                                    if app.selected_control == 0 {
                                        app.tree_state.expand_selected();
                                    }
                                } else {
                                    let d = if key.modifiers.contains(KeyModifiers::SHIFT) {
                                        10
                                    } else {
                                        1
                                    };
                                    adjust_control(&mut app, d);
                                }
                            }
                            KeyCode::Char(' ') | KeyCode::Enter => {
                                if app.current_tab == 2 {
                                    if app.selected_control == 0 {
                                        app.toggle_tree_selection();
                                    }
                                } else {
                                    // Enter on a slider: start direct numeric input
                                    if key.code == KeyCode::Enter {
                                        let controls = app.tab_controls(app.current_tab);
                                        if let Some(ctrl) = controls.get(app.selected_control) {
                                            if let ControlKind::Slider { .. } = &ctrl.kind {
                                                let current_val = app.get_value(ctrl.id);
                                                app.value_edit_buffer = current_val.to_string();
                                                app.input_mode = InputMode::ValueEdit;
                                                continue;
                                            }
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
        ControlKind::EyedropperWidget => {}
    }

    log::info!(
        "TUI Adjusted: tab={}, ctrl={}, id={:?}",
        app.current_tab,
        app.selected_control,
        id
    );
    app.mark_args_dirty();
}

fn set_control_value(app: &mut App, visual_idx: usize, val: i32) {
    let id = if app.current_tab == 0 {
        GENERAL_CONTROLS[visual_idx.min(GENERAL_CONTROLS.len() - 1)].id
    } else {
        app.effects_pipeline[visual_idx.min(app.effects_pipeline.len() - 1)]
    };

    app.set_value(id, val);

    log::info!(
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
        ControlKind::EyedropperWidget => {}
    }

    log::info!("Control reset: tab={}, id={:?}", app.current_tab, id);
    app.mark_args_dirty();
}

// ── UI Rendering ─────────────────────────────────────────────────────────────
fn ui(f: &mut ratatui::Frame, app: &mut App) {
    let area = f.area();

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

    // ── Main content: left panel + output ──
    let main = Layout::horizontal([
        Constraint::Length(44), // left panel
        Constraint::Min(0),     // output
    ])
    .split(root[1]);

    // ── Left panel: controls ──
    render_controls(f, app, main[0]);

    // ── Right panel: output ──
    render_output(f, app, main[1]);

    // ── Status bar ──
    render_status_bar(f, app, root[2]);

    // ── Font Picker Overlay ──
    if app.show_font_picker {
        let output_area = main[1];
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
}

fn render_title_bar(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    let layout = Layout::horizontal([
        Constraint::Length(10), // app title
        Constraint::Min(0),     // tabs + file + spinner
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

    // Tabs + filename + spinner
    let tab_names = [" General ", " Effects ", " Glyphs "];
    let mut tab_rects = Vec::new();

    let mut x = layout[1].x;
    let buf = f.buffer_mut();
    buf.set_style(layout[1], Style::new().bg(BG));

    for (i, name) in tab_names.iter().enumerate() {
        let w = name.len() as u16;
        let tab_area = Rect::new(x, layout[1].y, w, 1);
        tab_rects.push(tab_area);

        let (fg, bg_color) = if i == app.current_tab {
            (Color::White, TAB_ACTIVE)
        } else {
            (TEXT_DIM, TAB_INACTIVE)
        };

        let style = Style::new()
            .fg(fg)
            .bg(bg_color)
            .add_modifier(if i == app.current_tab {
                Modifier::BOLD
            } else {
                Modifier::empty()
            });
        buf.set_style(tab_area, style);
        buf.set_string(x, tab_area.y, name, style);

        x += w + 1;
    }

    // Show image path after tabs
    x += 1;
    let path_display = format!("📁 {}", app.image_path);
    let right_edge = layout[1].x + layout[1].width;
    let avail = right_edge.saturating_sub(x) as usize;
    let truncated = if path_display.len() > avail {
        format!(
            "…{}",
            &app.image_path[app.image_path.len().saturating_sub(avail.saturating_sub(3))..]
        )
    } else {
        path_display
    };
    let path_area = Rect::new(x, layout[1].y, truncated.len() as u16, 1);
    app.path_area = path_area;
    buf.set_string(x, layout[1].y, &truncated, Style::new().fg(TEXT_DIM).bg(BG));

    // Loading spinner in top-right
    let is_rendering = app.is_rendering.load(Ordering::Relaxed);
    if is_rendering {
        let spinners = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let spinner_ch = spinners[(app.render_tick / 3) % spinners.len()];
        let label = format!(" {} Rendering ", spinner_ch);
        let lw = label.len() as u16;
        let sx = right_edge.saturating_sub(lw);
        buf.set_string(
            sx,
            layout[1].y,
            &label,
            Style::new()
                .fg(Color::Yellow)
                .bg(BG)
                .add_modifier(Modifier::BOLD),
        );
    }

    app.tab_areas = tab_rects;
}

fn render_controls(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    // Panel border
    let panel = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(BORDER))
        .border_set(symbols::border::ROUNDED)
        .style(Style::new().bg(SURFACE));
    let inner = panel.inner(area);
    f.render_widget(panel, area);

    if app.current_tab == 2 {
        render_tree(f, app, inner);
        return;
    }

    let buf = f.buffer_mut();

    let controls = app.tab_controls(app.current_tab);
    let avail_height = inner.height as usize;
    let item_height: u16 = 2; // 1 row content + 1 row spacing
    let max_visible = avail_height / item_height as usize;

    // Scrolling offset for controls
    let scroll_offset = if app.selected_control >= max_visible {
        app.selected_control - max_visible + 1
    } else {
        0
    };

    let mut control_rects = Vec::new();
    let mut y = inner.y;

    for (v_idx, ctrl) in controls.iter().enumerate() {
        if v_idx < scroll_offset {
            control_rects.push(Rect::default());
            continue;
        }
        if y + item_height > inner.y + inner.height {
            control_rects.push(Rect::default());
            continue;
        }

        let ctrl_area = Rect::new(inner.x, y, inner.width, 1);
        let selected = v_idx == app.selected_control;

        let label_text = if app.current_tab == 1 {
            format!("≡ {}", ctrl.label)
        } else if app.current_tab == 2 && v_idx < app.available_blocks.len() {
            app.available_blocks[v_idx].clone()
        } else {
            ctrl.label.to_string()
        };

        match ctrl.kind {
            ControlKind::Slider { min, max, .. } => {
                let value = app.get_control_value(app.current_tab, v_idx);
                if selected && app.input_mode == InputMode::ValueEdit {
                    // Show edit buffer with cursor
                    let edit_text = format!("{}: {}▏", label_text, app.value_edit_buffer);
                    let style = Style::new()
                        .fg(Color::Yellow)
                        .bg(SURFACE)
                        .add_modifier(Modifier::BOLD);
                    for (i, ch) in edit_text.chars().enumerate() {
                        if ctrl_area.x + i as u16 >= ctrl_area.x + ctrl_area.width {
                            break;
                        }
                        buf.cell_mut((ctrl_area.x + i as u16, ctrl_area.y))
                            .map(|cell| cell.set_char(ch).set_style(style));
                    }
                } else {
                    render_bar(buf, ctrl_area, &label_text, value, min, max, selected);
                }
            }
            ControlKind::Enum { options, .. } => {
                let value = app.get_control_value(app.current_tab, v_idx) as usize;
                render_enum(buf, ctrl_area, &label_text, options, value, selected);
            }
            ControlKind::Toggle { .. } => {
                let on = app.get_control_value(app.current_tab, v_idx) != 0;
                render_toggle(buf, ctrl_area, &label_text, on, selected);
            }
            ControlKind::FontSelector => {
                let font_name: &str = if app.font_idx < app.fonts.len() {
                    &app.fonts[app.font_idx]
                } else {
                    "Default"
                };
                render_enum(buf, ctrl_area, &label_text, &[font_name], 0, selected);
            }
            ControlKind::EyedropperWidget => {
                render_eyedropper_widget(
                    buf,
                    ctrl_area,
                    &label_text,
                    app.eyedropper_color,
                    selected,
                );
            }
        }

        control_rects.push(ctrl_area);
        y += item_height;
    }

    app.control_areas = control_rects;

    // Draw scroll indicator if needed
    let total = controls.len();
    if total > max_visible {
        let indicator = format!(" {}/{} ", app.selected_control + 1, total);
        let ix = inner.x + inner.width - indicator.len() as u16 - 1;
        let iy = inner.y + inner.height - 1;
        if iy < area.y + area.height {
            buf.set_string(ix, iy, &indicator, Style::new().fg(TEXT_DIM).bg(SURFACE));
        }
    }
}

fn render_tree(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    app.tree_area = area;
    let buf = f.buffer_mut();
    // Get flattened nodes (this might be expensive to do every frame, but fine for now)
    // 1. Update Scroll
    {
        let nodes = app.tree_state.flatten();
        let _total = nodes.len();
        let height = area.height as usize;
        let selected = app.tree_state.selected_idx;
        let current_scroll = app.tree_state.scroll as usize;

        if selected >= current_scroll + height {
            app.tree_state.scroll = (selected - height + 1) as u16;
        } else if selected < current_scroll {
            app.tree_state.scroll = selected as u16;
        }
    }

    // 2. Render
    let nodes = app.tree_state.flatten();
    let total = nodes.len();
    let start_idx = app.tree_state.scroll as usize;
    let end_idx = (start_idx + area.height as usize).min(total);

    let visible_nodes = nodes
        .iter()
        .skip(start_idx)
        .take(end_idx - start_idx)
        .enumerate();

    for (i, node) in visible_nodes {
        let y = area.y + i as u16;
        if y >= area.y + area.height {
            break;
        }

        // Calculate actual index for selection check
        let actual_idx = start_idx + i;
        let selected = actual_idx == app.tree_state.selected_idx;

        let style = if selected {
            Style::new().bg(SURFACE).fg(Color::White)
        } else {
            Style::new().fg(TEXT)
        };

        // Background for the whole line
        for x in area.x..area.x + area.width {
            buf[(x, y)].set_style(style);
        }

        // Indentation
        let indent_len = (node.level as u16) * 2;
        let x = area.x + indent_len;

        // Marker
        let marker = if !node.is_leaf {
            if node.expanded {
                "▼ "
            } else {
                "▶ "
            }
        } else {
            // Check if selected in render_args
            let name = &node.full_path;
            if app.render_args.blocks.contains(name) {
                "☑ "
            } else {
                "☐ "
            }
        };

        // Draw marker
        buf.set_string(x, y, marker, style.fg(ACCENT));

        // Draw name
        // +2 for marker width
        buf.set_string(x + 2, y, &node.name, style);
    }
}

fn render_output(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    app.output_area = area;

    // 1. Render normal background content (Image or Glyphs tab)
    if app.current_tab == 2 {
        // Show glyphs/preview in top 1/3, output in bottom 2/3
        let glyphs_layout =
            Layout::vertical([Constraint::Ratio(1, 3), Constraint::Ratio(2, 3)]).split(area);
        render_glyphs_tab(f, app, glyphs_layout[0]);
        render_normal_output_content(f, app, glyphs_layout[1]);
    } else {
        render_normal_output_content(f, app, area);
    }

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
        let line_count = stats_content.lines().count();
        let max_bytes = stats_content.lines().map(|l| l.len()).max().unwrap_or(0);
        let title_base = format!(" {}×{} │ {}B ", max_bytes, line_count, stats_content.len());

        let mut technical_info = String::new();

        if let Some(res) = &app.cached_result {
            let width_chars = res.grid.first().map_or(0, |row| row.len());
            let height_chars = res.grid.len();
            technical_info.push_str(&format!("Size: {}x{} chars", width_chars, height_chars));
        }

        if let Some(g) = &app.arc_glyphs {
            let (gw, gh) = g.metrics;
            if !technical_info.is_empty() {
                technical_info.push_str(" │ ");
            }
            technical_info.push_str(&format!("Cell: {}x{}", gw, gh));
        }

        if let Some((r, g, b)) = app.eyedropper_color {
            let col_u32 = ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
            let irc_code = crate::draw::nearest_hex_colour_fast(col_u32, &crate::palette::IRC99);
            let ansi_code = crate::draw::nearest_hex_colour_fast(col_u32, &crate::palette::ANSI256);
            if !technical_info.is_empty() {
                technical_info.push_str(" │ ");
            }
            technical_info.push_str(&format!(
                "#{:02X}{:02X}{:02X} IRC:{:02} ANSI:{:03}",
                r, g, b, irc_code, ansi_code
            ));
        }

        let title_right = if technical_info.is_empty() {
            title_base
        } else {
            format!(" {} │ {} ", technical_info, title_base.trim())
        };

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

fn render_options_panel(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    let id = app.get_current_control_id();
    let ctrl = get_control_def(id);

    let (options, current) = match &ctrl.kind {
        ControlKind::Enum { options, .. } => (
            options.iter().map(|&s| s.to_string()).collect::<Vec<_>>(),
            app.get_value(id) as usize,
        ),
        ControlKind::FontSelector => (app.fonts.clone(), app.font_idx),
        _ => return,
    };

    // Use bottom 3 rows of output
    let pane_height = 3;
    let pane_area = Rect::new(
        area.x,
        area.y + area.height.saturating_sub(pane_height),
        area.width,
        pane_height,
    );
    app.options_pane_area = pane_area;
    app.options_areas.clear();
    f.render_widget(Clear, pane_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} Options ", ctrl.label))
        .border_style(Style::new().fg(ACCENT));
    f.render_widget(&block, pane_area);

    let inner = block.inner(pane_area);
    let mut x = inner.x;

    // We want to show options around the current one
    let visible_count = inner.width as usize / 12; // estimate more space
    let start = current.saturating_sub(visible_count / 2);
    let end = (start + visible_count).min(options.len());

    for (i, opt) in options.iter().enumerate().skip(start).take(end - start) {
        let is_selected = i == current;
        let style = if is_selected {
            Style::new()
                .bg(ACCENT)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(TEXT)
        };
        let text = format!(" {} ", opt);
        if x + text.len() as u16 > inner.x + inner.width {
            break;
        }
        let opt_area = Rect::new(x, inner.y, text.len() as u16, 1);
        if app.options_areas.len() <= i {
            app.options_areas.resize(i + 1, Rect::default());
        }
        app.options_areas[i] = opt_area;
        f.render_widget(Paragraph::new(text.clone()).style(style), opt_area);
        x += text.len() as u16 + 1;
    }
}

fn render_glyphs_tab(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    let chunks =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(area);

    app.glyph_list_area = chunks[0];
    app.preview_area = chunks[1];

    let glyph_block = Block::default().borders(Borders::ALL).title(" Glyphs ");

    if let Some(node) = app.tree_state.get_selected_node() {
        let (config_groups, _) = crate::font::load_blocks_config();

        let target_key = if !node.full_path.is_empty() {
            &node.full_path
        } else {
            &node.name
        };

        if let Some(codes) = config_groups.get(target_key) {
            let items: Vec<ListItem> = codes
                .iter()
                .map(|&code| {
                    let ch = char::from_u32(code).unwrap_or('?');
                    ListItem::new(format!("{} U+{:04X}", ch, code))
                })
                .collect();

            let list = List::new(items)
                .block(glyph_block)
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
            f.render_stateful_widget(list, chunks[0], &mut app.glyph_list_state);

            // Preview
            let idx = app.glyph_list_state.selected().unwrap_or(0);
            if let Some(&code) = codes.get(idx) {
                if let Some(ch) = char::from_u32(code) {
                    render_block_bitmap(f, app, chunks[1], ch);
                }
            }
        } else {
            f.render_widget(
                Paragraph::new("No known glyphs").block(glyph_block),
                chunks[0],
            );
        }
    }
}

fn render_block_bitmap(f: &mut ratatui::Frame, app: &mut App, area: Rect, ch: char) {
    let font_name = if app.font_idx < app.fonts.len() {
        &app.fonts[app.font_idx]
    } else {
        "Default"
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Preview: {} ({}) ", ch, font_name));
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Use current store to find bitmap
    if let Some(g) = &app.arc_glyphs {
        if let Some(bitmap) = g.glyphs.iter().find(|(c, _)| *c == ch).map(|(_, b)| b) {
            let mut text = ratatui::text::Text::default();
            for row in bitmap {
                let mut line = Line::from(vec![]);
                for &pixel in row {
                    if pixel > 0 {
                        line.spans
                            .push(Span::styled("██", Style::new().fg(Color::Cyan)));
                    } else {
                        // eye friendly distinct background color
                        line.spans
                            .push(Span::styled("░░", Style::new().fg(Color::Rgb(60, 60, 80))));
                    }
                }
                text.lines.push(line);
            }
            let p = Paragraph::new(text);
            f.render_widget(p, inner);
        } else {
            // Not in current store, maybe need to reload or it's just missing
            let p =
                Paragraph::new("Glyph not in current store.\nPress Space in left panel to reload.")
                    .alignment(Alignment::Center);
            f.render_widget(p, inner);
        }
    }
}

fn render_status_bar(f: &mut ratatui::Frame, _app: &mut App, area: Rect) {
    let keys = [
        ("Tab", "Switch"),
        ("↑↓", "Navigate"),
        ("←→", "Adjust"),
        ("Space", "Toggle"),
        ("Del", "Reset"),
        ("Shift", "×10"),
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
