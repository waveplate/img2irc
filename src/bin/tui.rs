use std::{error::Error, io, time::Duration, sync::Arc};
use std::sync::atomic::{AtomicBool, Ordering};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers,
            MouseEventKind, MouseButton},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    buffer::Buffer,
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Layout, Rect, Alignment},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
    Terminal,
    symbols,
};
use img2irc_rs::{args::{RenderArgs, Render, SamplingFilter, ColourSpace, Encoding}, font, draw};
use clap::Parser;
use ansi_to_tui::IntoText;
use tokio::sync::{mpsc, watch};

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
const STATUS_KEY: Color = Color::Rgb(130, 140, 255);
const STATUS_DESC: Color = Color::Rgb(140, 140, 160);

// ── CLI ──────────────────────────────────────────────────────────────────────
#[derive(Parser)]
struct Cli {
    #[arg(required = true)]
    image: String,
}

// ── Control types ────────────────────────────────────────────────────────────
#[derive(Clone)]
enum ControlKind {
    Slider { min: i32, max: i32 },
    Toggle,
    Enum { options: Vec<&'static str> },
}

#[derive(Clone)]
struct ControlDef {
    label: &'static str,
    kind: ControlKind,
}

// ── App State ────────────────────────────────────────────────────────────────
enum InputMode { Normal, Editing }

struct App {
    render_args: RenderArgs,
    input_mode: InputMode,
    selected_control: usize,
    current_tab: usize,
    scroll: u16,

    tx_args: watch::Sender<RenderArgs>,
    rx_result: mpsc::Receiver<String>,
    cached_render: String,
    is_rendering: Arc<AtomicBool>,
    render_tick: usize,

    show_save_dialog: bool,
    save_input: String,

    // Image path
    image_path: String,

    // Control values
    width: i32,
    render_mode_idx: usize,
    dither: i32,
    smooth: bool,
    invert: bool,
    grayscale: bool,
    brightness: i32,
    contrast: i32,
    saturation: i32,
    gamma: i32,
    hue: i32,
    crop_x1: i32,
    crop_y1: i32,
    crop_x2: i32,
    crop_y2: i32,
    fliph: bool,
    flipv: bool,
    rotate: i32,
    scalex: i32,  // percentage, 100 = 1.0
    scaley: i32,
    font_size: i32,
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

    // Track rendered control areas for mouse
    control_areas: Vec<Rect>,
    tab_areas: Vec<Rect>,
}

impl App {
    fn new(render_args: RenderArgs, tx_args: watch::Sender<RenderArgs>, rx_result: mpsc::Receiver<String>) -> App {
        let (blocks_map, _) = img2irc_rs::font::load_blocks_config();
        let mut available_blocks: Vec<String> = blocks_map.keys().cloned().collect();
        available_blocks.sort();

        App {
            render_args: render_args.clone(),
            input_mode: InputMode::Normal,
            selected_control: 0,
            current_tab: 0,
            scroll: 0,
            tx_args,
            rx_result,
            cached_render: String::new(),
            is_rendering: Arc::new(AtomicBool::new(true)),
            render_tick: 0,
            show_save_dialog: false,
            save_input: String::new(),

            image_path: render_args.image.clone().unwrap_or_default(),
            width: render_args.width.unwrap_or(80) as i32,
            render_mode_idx: match render_args.render {
                Render::Irc => 0, Render::Ansi => 1, Render::Ansi24 => 2, Render::Preview => 3,
            },
            dither: render_args.dither as i32,
            smooth: render_args.smooth,
            invert: render_args.invert,
            grayscale: render_args.grayscale,
            brightness: 0,
            contrast: 0,
            saturation: 0,
            gamma: render_args.gamma as i32,
            hue: render_args.hue as i32,
            crop_x1: render_args.crop.map(|c| c.0 as i32).unwrap_or(0),
            crop_y1: render_args.crop.map(|c| c.1 as i32).unwrap_or(0),
            crop_x2: render_args.crop.map(|c| c.2 as i32).unwrap_or(0),
            crop_y2: render_args.crop.map(|c| c.3 as i32).unwrap_or(0),
            fliph: render_args.fliph,
            flipv: render_args.flipv,
            rotate: render_args.rotate as i32,
            scalex: render_args.scale.map(|s| (s.0 * 100.0) as i32).unwrap_or(100),
            scaley: render_args.scale.map(|s| (s.1 * 100.0) as i32).unwrap_or(100),
            font_size: render_args.font_size as i32,
            filter_idx: match render_args.filter {
                SamplingFilter::Nearest => 0, SamplingFilter::Triangle => 1,
                SamplingFilter::CatmullRom => 2, SamplingFilter::Gaussian => 3,
                SamplingFilter::Lanczos3 => 4,
            },
            grayscale_tolerance: render_args.grayscale_tolerance as i32,
            colorspace_idx: match render_args.colorspace {
                ColourSpace::HSL => 0, ColourSpace::HSV => 1,
                ColourSpace::HSLUV => 2, ColourSpace::LCH => 3,
            },
            encoding_idx: match render_args.encoding {
                Encoding::Utf8 => 0, Encoding::Utf16 => 1,
                Encoding::Utf16be => 2, Encoding::Utf16le => 3,
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

            available_blocks,
            control_areas: Vec::new(),
            tab_areas: Vec::new(),
        }
    }

    fn tab_controls(&self, tab: usize) -> Vec<ControlDef> {
        match tab {
            0 => vec![
                ControlDef { label: "Width",      kind: ControlKind::Slider { min: 10, max: 300 } },
                ControlDef { label: "Mode",       kind: ControlKind::Enum { options: vec!["IRC", "ANSI", "ANSI24", "PREVIEW"] } },
                ControlDef { label: "Dither",     kind: ControlKind::Slider { min: 0, max: 8 } },
                ControlDef { label: "Smooth",     kind: ControlKind::Toggle },
                ControlDef { label: "Invert",     kind: ControlKind::Toggle },
                ControlDef { label: "Grayscale",  kind: ControlKind::Toggle },
                ControlDef { label: "Brightness", kind: ControlKind::Slider { min: -255, max: 255 } },
                ControlDef { label: "Contrast",   kind: ControlKind::Slider { min: -255, max: 255 } },
                ControlDef { label: "Saturation", kind: ControlKind::Slider { min: -255, max: 255 } },
                ControlDef { label: "Gamma",      kind: ControlKind::Slider { min: 0, max: 255 } },
                ControlDef { label: "Hue",        kind: ControlKind::Slider { min: 0, max: 360 } },
                ControlDef { label: "Crop X1",    kind: ControlKind::Slider { min: 0, max: 4000 } },
                ControlDef { label: "Crop Y1",    kind: ControlKind::Slider { min: 0, max: 4000 } },
                ControlDef { label: "Crop X2",    kind: ControlKind::Slider { min: 0, max: 4000 } },
                ControlDef { label: "Crop Y2",    kind: ControlKind::Slider { min: 0, max: 4000 } },
                ControlDef { label: "Flip H",     kind: ControlKind::Toggle },
                ControlDef { label: "Flip V",     kind: ControlKind::Toggle },
                ControlDef { label: "Rotate",     kind: ControlKind::Slider { min: 0, max: 360 } },
                ControlDef { label: "Scale X %",  kind: ControlKind::Slider { min: 10, max: 500 } },
                ControlDef { label: "Scale Y %",  kind: ControlKind::Slider { min: 10, max: 500 } },
                ControlDef { label: "Font Size",  kind: ControlKind::Slider { min: 8, max: 64 } },
                ControlDef { label: "Filter",     kind: ControlKind::Enum { options: vec!["Nearest", "Triangle", "CatmullRom", "Gaussian", "Lanczos3"] } },
                ControlDef { label: "Gray Tol",   kind: ControlKind::Slider { min: 0, max: 255 } },
                ControlDef { label: "Colorspace", kind: ControlKind::Enum { options: vec!["HSL", "HSV", "HSLUV", "LCH"] } },
                ControlDef { label: "Encoding",   kind: ControlKind::Enum { options: vec!["UTF-8", "UTF-16", "UTF-16BE", "UTF-16LE", "CESU-8"] } },
            ],
            1 => {
                let mut v = vec![
                    ControlDef { label: "Pixelize",    kind: ControlKind::Slider { min: 0, max: 100 } },
                    ControlDef { label: "Gauss Blur",  kind: ControlKind::Slider { min: 0, max: 100 } },
                ];
                let toggles = [
                    "Box Blur", "Oil", "Halftone", "Sepia", "Normalize", "Noise",
                    "Emboss", "Identity", "Laplace", "Denoise", "Sharpen", "Cali",
                    "Dramatic", "Firenze", "Golden", "Lix", "Lofi", "Neue",
                    "Obsidian", "PastelPink", "Ryo", "FrostGlass", "Solarize", "EdgeDetect",
                ];
                for t in toggles {
                    v.push(ControlDef { label: t, kind: ControlKind::Toggle });
                }
                v
            },
            2 => {
                self.available_blocks.iter().map(|_name| {
                    ControlDef { label: "block", kind: ControlKind::Toggle }
                }).collect()
            },
            _ => vec![],
        }
    }

    fn control_count(&self) -> usize {
        match self.current_tab {
            0 => 25,
            1 => 26,
            2 => self.available_blocks.len(),
            _ => 0,
        }
    }

    fn get_control_value(&self, tab: usize, idx: usize) -> i32 {
        if tab == 0 {
            match idx {
                0 => self.width,
                1 => self.render_mode_idx as i32,
                2 => self.dither,
                3 => self.smooth as i32,
                4 => self.invert as i32,
                5 => self.grayscale as i32,
                6 => self.brightness,
                7 => self.contrast,
                8 => self.saturation,
                9 => self.gamma,
                10 => self.hue,
                11 => self.crop_x1,
                12 => self.crop_y1,
                13 => self.crop_x2,
                14 => self.crop_y2,
                15 => self.fliph as i32,
                16 => self.flipv as i32,
                17 => self.rotate,
                18 => self.scalex,
                19 => self.scaley,
                20 => self.font_size,
                21 => self.filter_idx as i32,
                22 => self.grayscale_tolerance,
                23 => self.colorspace_idx as i32,
                24 => self.encoding_idx as i32,
                _ => 0,
            }
        } else if tab == 1 {
            match idx {
                0 => self.pixelize,
                1 => self.gaussian_blur,
                2 => self.box_blur as i32,
                3 => self.oil as i32,
                4 => self.halftone as i32,
                5 => self.sepia as i32,
                6 => self.normalize as i32,
                7 => self.noise as i32,
                8 => self.emboss as i32,
                9 => self.identity as i32,
                10 => self.laplace as i32,
                11 => self.noise_reduction as i32,
                12 => self.sharpen as i32,
                13 => self.cali as i32,
                14 => self.dramatic as i32,
                15 => self.firenze as i32,
                16 => self.golden as i32,
                17 => self.lix as i32,
                18 => self.lofi as i32,
                19 => self.neue as i32,
                20 => self.obsidian as i32,
                21 => self.pastel_pink as i32,
                22 => self.ryo as i32,
                23 => self.frosted_glass as i32,
                24 => self.solarize as i32,
                25 => self.edge_detection as i32,
                _ => 0,
            }
        } else {
            // For blocks tab, checked = 1
            if idx < self.available_blocks.len() {
                let name = &self.available_blocks[idx];
                if self.render_args.blocks.is_empty() { 1 } // all on
                else if self.render_args.blocks.contains(name) { 1 }
                else { 0 }
            } else { 0 }
        }
    }

    fn update_render_args(&mut self) {
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
        self.render_args.pixelize = self.pixelize;
        self.render_args.gaussian_blur = self.gaussian_blur;
        self.render_args.box_blur = self.box_blur;
        self.render_args.oil = if self.oil { Some("2,20.0".to_string()) } else { None };
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
            0 => Render::Irc, 1 => Render::Ansi, 2 => Render::Ansi24, _ => Render::Preview,
        };
        self.render_args.fliph = self.fliph;
        self.render_args.flipv = self.flipv;
        self.render_args.rotate = self.rotate as f32;
        self.render_args.scale = Some((self.scalex as f32 / 100.0, self.scaley as f32 / 100.0));
        self.render_args.font_size = self.font_size as u32;
        self.render_args.filter = match self.filter_idx {
            0 => SamplingFilter::Nearest, 1 => SamplingFilter::Triangle,
            2 => SamplingFilter::CatmullRom, 3 => SamplingFilter::Gaussian,
            _ => SamplingFilter::Lanczos3,
        };
        self.render_args.grayscale_tolerance = self.grayscale_tolerance as u8;
        self.render_args.colorspace = match self.colorspace_idx {
            0 => ColourSpace::HSL, 1 => ColourSpace::HSV,
            2 => ColourSpace::HSLUV, _ => ColourSpace::LCH,
        };
        self.render_args.encoding = match self.encoding_idx {
            0 => Encoding::Utf8, 1 => Encoding::Utf16,
            2 => Encoding::Utf16be, 3 => Encoding::Utf16le,
            _ => Encoding::Cesu8,
        };
        if self.crop_x2 > 0 || self.crop_y2 > 0 {
            self.render_args.crop = Some((self.crop_x1 as u32, self.crop_y1 as u32, self.crop_x2 as u32, self.crop_y2 as u32));
        } else {
            self.render_args.crop = None;
        }
        let _ = self.tx_args.send(self.render_args.clone());
    }
}

// ── Custom Widgets ───────────────────────────────────────────────────────────

/// A gauge-style slider bar. Value is rendered inside the filled portion.
fn render_bar(buf: &mut Buffer, area: Rect, label: &str, value: i32, min: i32, max: i32, selected: bool) {
    if area.height < 1 || area.width < 4 { return; }

    let range = (max - min).max(1) as f64;
    let ratio = ((value - min) as f64 / range).clamp(0.0, 1.0);
    let filled_width = ((area.width as f64) * ratio) as u16;

    let bg_style = if selected { Style::new().bg(SELECTED_BG) } else { Style::new().bg(SURFACE) };
    buf.set_style(area, bg_style);

    // Draw filled portion
    let bar_style = Style::new().bg(if selected { BAR_FG } else { ACCENT_DIM }).fg(Color::White);
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
        if x >= area.x + area.width { break; }
        let cell = &mut buf[(x, ly)];
        cell.set_char(ch);
        cell.set_style(Style::new().fg(Color::White).add_modifier(Modifier::BOLD));
    }

    // Draw value on the right
    let value_text = format!("{} ", value);
    let vx = area.x + area.width - value_text.len() as u16;
    for (i, ch) in value_text.chars().enumerate() {
        let x = vx + i as u16;
        if x < area.x || x >= area.x + area.width { continue; }
        let cell = &mut buf[(x, ly)];
        cell.set_char(ch);
        cell.set_style(Style::new().fg(TEXT).add_modifier(Modifier::BOLD));
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
fn render_enum(buf: &mut Buffer, area: Rect, label: &str, options: &[&str], value: usize, selected: bool) {
    if area.height < 1 || area.width < 4 { return; }

    let bg = if selected { SELECTED_BG } else { SURFACE };
    buf.set_style(area, Style::new().bg(bg));

    let ly = area.y + area.height.saturating_sub(1) / 2;
    let label_text = format!(" {}", label);
    for (i, ch) in label_text.chars().enumerate() {
        let x = area.x + 1 + i as u16;
        if x >= area.x + area.width { break; }
        let cell = &mut buf[(x, ly)];
        cell.set_char(ch);
        cell.set_style(Style::new().fg(if selected { Color::White } else { TEXT }).add_modifier(Modifier::BOLD));
    }

    let opt = options.get(value).unwrap_or(&"?");
    let val_text = format!("◂ {} ▸ ", opt);
    let vx = area.x + area.width - val_text.len() as u16;
    for (i, ch) in val_text.chars().enumerate() {
        let x = vx + i as u16;
        if x < area.x || x >= area.x + area.width { continue; }
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

/// A toggle switch
fn render_toggle(buf: &mut Buffer, area: Rect, label: &str, on: bool, selected: bool) {
    if area.height < 1 || area.width < 4 { return; }

    let bg = if selected { SELECTED_BG } else { SURFACE };
    buf.set_style(area, Style::new().bg(bg));

    let ly = area.y + area.height.saturating_sub(1) / 2;

    // Label
    let label_text = format!(" {}", label);
    for (i, ch) in label_text.chars().enumerate() {
        let x = area.x + 1 + i as u16;
        if x >= area.x + area.width { break; }
        let cell = &mut buf[(x, ly)];
        cell.set_char(ch);
        cell.set_style(Style::new().fg(if selected { Color::White } else { TEXT }));
    }

    // Toggle indicator on the right
    let (indicator, color) = if on {
        (" ● ON ", TOGGLE_ON)
    } else {
        (" ○ OFF", TOGGLE_OFF)
    };
    let vx = area.x + area.width - indicator.len() as u16 - 1;
    for (i, ch) in indicator.chars().enumerate() {
        let x = vx + i as u16;
        if x < area.x || x >= area.x + area.width { continue; }
        let cell = &mut buf[(x, ly)];
        cell.set_char(ch);
        cell.set_style(Style::new().fg(color).add_modifier(Modifier::BOLD));
    }

    if selected {
        let accent_style = Style::new().bg(ACCENT).fg(ACCENT);
        for y in area.y..area.y + area.height {
            buf.set_string(area.x, y, "▎", accent_style);
        }
    }
}

// ── Main ─────────────────────────────────────────────────────────────────────
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    let mut render_args = img2irc_rs::args_to_render_args(
        &img2irc_rs::args::Args::parse_from(&["img2irc", &cli.image])
    );
    if render_args.width.is_none() { render_args.width = Some(80); }
    render_args.render = Render::Ansi;

    let glyph_store = font::glyph_store(
        &render_args.blocks,
        &render_args.exclude_range,
        &render_args.exclude,
        &render_args.include_range,
        render_args.include.as_ref(),
        &render_args.font,
        render_args.font_size,
    );

    let img_res = img2irc_rs::load_image_from_url_or_path(&cli.image, &render_args, &glyph_store).await;
    let image = match img_res {
        Ok(img) => img,
        Err(e) => { eprintln!("Failed to load image: {}", e); return Ok(()); }
    };

    let arc_image = Arc::new(image);
    let arc_glyphs = Arc::new(glyph_store);

    let (tx_args, mut rx_args) = watch::channel(render_args.clone());
    let (tx_result, rx_result) = mpsc::channel(1);
    let rendering_flag = Arc::new(AtomicBool::new(true));
    let rendering_flag2 = rendering_flag.clone();

    tokio::spawn(async move {
        let mut fast_glyphs_store: Option<Vec<draw::FastGlyph>> = None;
        loop {
            rendering_flag2.store(true, Ordering::Relaxed);
            let current_args = { rx_args.borrow_and_update().clone() };
            if fast_glyphs_store.is_none() {
                fast_glyphs_store = Some(draw::prepare_glyphs(&arc_glyphs, &current_args));
            }
            let glyphs = fast_glyphs_store.as_ref().unwrap();
            let result = img2irc_rs::render_output(&current_args, &arc_glyphs, (*arc_image).clone(), glyphs);
            rendering_flag2.store(false, Ordering::Relaxed);
            if tx_result.send(result.content).await.is_err() { break; }
            if rx_args.changed().await.is_err() { break; }
        }
    });

    let mut app = App::new(render_args, tx_args, rx_result);
    app.is_rendering = rendering_flag;
    app.update_render_args();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_app(&mut terminal, app).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;

    if let Err(err) = res { println!("{:?}", err); }
    Ok(())
}

// ── App Loop ─────────────────────────────────────────────────────────────────
async fn run_app<B: Backend>(terminal: &mut Terminal<B>, mut app: App) -> io::Result<()> {
    loop {
        app.render_tick = app.render_tick.wrapping_add(1);
        while let Ok(content) = app.rx_result.try_recv() {
            app.cached_render = content;
        }

        terminal.draw(|f| ui(f, &mut app))?;

        if crossterm::event::poll(Duration::from_millis(50))? {
            let event = event::read()?;
            match event {
                Event::Mouse(mouse) => {
                    match mouse.kind {
                        MouseEventKind::ScrollUp => {
                            app.scroll = app.scroll.saturating_sub(3);
                        }
                        MouseEventKind::ScrollDown => {
                            app.scroll = app.scroll.saturating_add(3);
                        }
                        MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Left) => {
                            // Check tab clicks (only on Down, not Drag)
                            if matches!(mouse.kind, MouseEventKind::Down(_)) {
                                for (i, area) in app.tab_areas.iter().enumerate() {
                                    if mouse.column >= area.x && mouse.column < area.x + area.width
                                        && mouse.row >= area.y && mouse.row < area.y + area.height
                                    {
                                        app.current_tab = i;
                                        app.selected_control = 0;
                                        break;
                                    }
                                }
                            }
                            // Check control clicks/drags
                            for (i, area) in app.control_areas.iter().enumerate() {
                                if area.width == 0 { continue; }
                                if mouse.column >= area.x && mouse.column < area.x + area.width
                                    && mouse.row >= area.y && mouse.row < area.y + area.height
                                {
                                    app.selected_control = i;
                                    let controls = app.tab_controls(app.current_tab);
                                    if let Some(ctrl) = controls.get(i) {
                                        match &ctrl.kind {
                                            ControlKind::Slider { min, max } => {
                                                // Map mouse x position to value
                                                let rel_x = (mouse.column - area.x) as f64;
                                                let ratio = (rel_x / area.width as f64).clamp(0.0, 1.0);
                                                let new_val = *min + (((*max - *min) as f64) * ratio) as i32;
                                                set_control_value(&mut app, i, new_val.clamp(*min, *max));
                                            }
                                            ControlKind::Toggle => {
                                                if matches!(mouse.kind, MouseEventKind::Down(_)) {
                                                    toggle_control(&mut app);
                                                }
                                            }
                                            ControlKind::Enum { .. } => {
                                                if matches!(mouse.kind, MouseEventKind::Down(_)) {
                                                    toggle_control(&mut app);
                                                }
                                            }
                                        }
                                    }
                                    break;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Event::Key(key) => {
                    match app.input_mode {
                        InputMode::Editing => {
                            match key.code {
                                KeyCode::Esc => {
                                    app.input_mode = InputMode::Normal;
                                    app.show_save_dialog = false;
                                }
                                KeyCode::Enter => {
                                    if !app.save_input.is_empty() {
                                        let path = &app.save_input;
                                        if let Err(e) = std::fs::write(path, &app.cached_render) {
                                            eprintln!("Failed to save: {}", e);
                                        }
                                    }
                                    app.input_mode = InputMode::Normal;
                                    app.show_save_dialog = false;
                                    app.save_input.clear();
                                }
                                KeyCode::Char(c) => { app.save_input.push(c); }
                                KeyCode::Backspace => { app.save_input.pop(); }
                                _ => {}
                            }
                        }
                        InputMode::Normal => {
                            let speed = if key.modifiers.contains(KeyModifiers::SHIFT) { 10 } else { 1 };
                            match key.code {
                                KeyCode::Char('q') => return Ok(()),
                                KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                    app.input_mode = InputMode::Editing;
                                    app.show_save_dialog = true;
                                    app.save_input.clear();
                                }
                                KeyCode::Char('r') | KeyCode::Delete => {
                                    reset_control(&mut app);
                                }
                                KeyCode::Tab => {
                                    app.current_tab = (app.current_tab + 1) % 3;
                                    app.selected_control = 0;
                                }
                                KeyCode::BackTab => {
                                    app.current_tab = (app.current_tab + 2) % 3;
                                    app.selected_control = 0;
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    let count = app.control_count();
                                    if count > 0 {
                                        app.selected_control = (app.selected_control + 1) % count;
                                    }
                                }
                                KeyCode::Up | KeyCode::Char('k') => {
                                    let count = app.control_count();
                                    if count > 0 {
                                        app.selected_control = if app.selected_control == 0 { count - 1 } else { app.selected_control - 1 };
                                    }
                                }
                                KeyCode::Left | KeyCode::Char('h') => {
                                    adjust_control(&mut app, -speed);
                                }
                                KeyCode::Right | KeyCode::Char('l') => {
                                    adjust_control(&mut app, speed);
                                }
                                KeyCode::Char(' ') | KeyCode::Enter => {
                                    toggle_control(&mut app);
                                }
                                KeyCode::PageUp => {
                                    app.scroll = app.scroll.saturating_sub(10);
                                }
                                KeyCode::PageDown => {
                                    app.scroll = app.scroll.saturating_add(10);
                                }
                                _ => {}
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

// ── Control Logic ────────────────────────────────────────────────────────────
fn adjust_control(app: &mut App, delta: i32) {
    if app.current_tab == 0 {
        match app.selected_control {
            0 => app.width = (app.width + delta).clamp(10, 300),
            1 => {
                if delta > 0 { app.render_mode_idx = (app.render_mode_idx + 1) % 4; }
                else { app.render_mode_idx = (app.render_mode_idx + 3) % 4; }
            }
            2 => app.dither = (app.dither + delta).clamp(0, 8),
            6 => app.brightness = (app.brightness + delta).clamp(-255, 255),
            7 => app.contrast = (app.contrast + delta).clamp(-255, 255),
            8 => app.saturation = (app.saturation + delta).clamp(-255, 255),
            9 => app.gamma = (app.gamma + delta).clamp(0, 255),
            10 => app.hue = (app.hue + delta).clamp(0, 360),
            11 => app.crop_x1 = (app.crop_x1 + delta).clamp(0, 4000),
            12 => app.crop_y1 = (app.crop_y1 + delta).clamp(0, 4000),
            13 => app.crop_x2 = (app.crop_x2 + delta).clamp(0, 4000),
            14 => app.crop_y2 = (app.crop_y2 + delta).clamp(0, 4000),
            17 => app.rotate = (app.rotate + delta).clamp(0, 360),
            18 => app.scalex = (app.scalex + delta).clamp(10, 500),
            19 => app.scaley = (app.scaley + delta).clamp(10, 500),
            20 => app.font_size = (app.font_size + delta).clamp(8, 64),
            21 => {
                if delta > 0 { app.filter_idx = (app.filter_idx + 1) % 5; }
                else { app.filter_idx = (app.filter_idx + 4) % 5; }
            }
            22 => app.grayscale_tolerance = (app.grayscale_tolerance + delta).clamp(0, 255),
            23 => {
                if delta > 0 { app.colorspace_idx = (app.colorspace_idx + 1) % 4; }
                else { app.colorspace_idx = (app.colorspace_idx + 3) % 4; }
            }
            24 => {
                if delta > 0 { app.encoding_idx = (app.encoding_idx + 1) % 5; }
                else { app.encoding_idx = (app.encoding_idx + 4) % 5; }
            }
            _ => {} // toggles ignore adjust
        }
    } else if app.current_tab == 1 {
        match app.selected_control {
            0 => app.pixelize = (app.pixelize + delta).clamp(0, 100),
            1 => app.gaussian_blur = (app.gaussian_blur + delta).clamp(0, 100),
            _ => {} // toggles
        }
    }
    app.update_render_args();
}

fn set_control_value(app: &mut App, idx: usize, val: i32) {
    if app.current_tab == 0 {
        match idx {
            0 => app.width = val,
            2 => app.dither = val,
            6 => app.brightness = val,
            7 => app.contrast = val,
            8 => app.saturation = val,
            9 => app.gamma = val,
            10 => app.hue = val,
            11 => app.crop_x1 = val,
            12 => app.crop_y1 = val,
            13 => app.crop_x2 = val,
            14 => app.crop_y2 = val,
            17 => app.rotate = val,
            18 => app.scalex = val,
            19 => app.scaley = val,
            20 => app.font_size = val,
            22 => app.grayscale_tolerance = val,
            _ => {}
        }
    } else if app.current_tab == 1 {
        match idx {
            0 => app.pixelize = val,
            1 => app.gaussian_blur = val,
            _ => {}
        }
    }
    app.update_render_args();
}

fn toggle_control(app: &mut App) {
    if app.current_tab == 0 {
        match app.selected_control {
            1 => { app.render_mode_idx = (app.render_mode_idx + 1) % 4; }
            3 => app.smooth = !app.smooth,
            4 => app.invert = !app.invert,
            5 => app.grayscale = !app.grayscale,
            15 => app.fliph = !app.fliph,
            16 => app.flipv = !app.flipv,
            21 => { app.filter_idx = (app.filter_idx + 1) % 5; }
            23 => { app.colorspace_idx = (app.colorspace_idx + 1) % 4; }
            24 => { app.encoding_idx = (app.encoding_idx + 1) % 5; }
            _ => {}
        }
    } else if app.current_tab == 1 {
        match app.selected_control {
            2 => app.box_blur = !app.box_blur,
            3 => app.oil = !app.oil,
            4 => app.halftone = !app.halftone,
            5 => app.sepia = !app.sepia,
            6 => app.normalize = !app.normalize,
            7 => app.noise = !app.noise,
            8 => app.emboss = !app.emboss,
            9 => app.identity = !app.identity,
            10 => app.laplace = !app.laplace,
            11 => app.noise_reduction = !app.noise_reduction,
            12 => app.sharpen = !app.sharpen,
            13 => app.cali = !app.cali,
            14 => app.dramatic = !app.dramatic,
            15 => app.firenze = !app.firenze,
            16 => app.golden = !app.golden,
            17 => app.lix = !app.lix,
            18 => app.lofi = !app.lofi,
            19 => app.neue = !app.neue,
            20 => app.obsidian = !app.obsidian,
            21 => app.pastel_pink = !app.pastel_pink,
            22 => app.ryo = !app.ryo,
            23 => app.frosted_glass = !app.frosted_glass,
            24 => app.solarize = !app.solarize,
            25 => app.edge_detection = !app.edge_detection,
            _ => {}
        }
    } else if app.current_tab == 2 {
        if app.selected_control < app.available_blocks.len() {
            let name = app.available_blocks[app.selected_control].clone();
            if app.render_args.blocks.is_empty() {
                app.render_args.blocks = vec![name];
            } else {
                if let Some(pos) = app.render_args.blocks.iter().position(|x| *x == name) {
                    app.render_args.blocks.remove(pos);
                } else {
                    app.render_args.blocks.push(name);
                }
            }
        }
    }
    app.update_render_args();
}

fn reset_control(app: &mut App) {
    if app.current_tab == 0 {
        match app.selected_control {
            0 => app.width = 80,
            1 => app.render_mode_idx = 1,
            2 => app.dither = 0,
            3 => app.smooth = false,
            4 => app.invert = false,
            5 => app.grayscale = false,
            6 => app.brightness = 0,
            7 => app.contrast = 0,
            8 => app.saturation = 0,
            9 => app.gamma = 0,
            10 => app.hue = 0,
            11 => app.crop_x1 = 0,
            12 => app.crop_y1 = 0,
            13 => app.crop_x2 = 0,
            14 => app.crop_y2 = 0,
            15 => app.fliph = false,
            16 => app.flipv = false,
            17 => app.rotate = 0,
            18 => app.scalex = 100,
            19 => app.scaley = 100,
            20 => app.font_size = 24,
            21 => app.filter_idx = 0,
            22 => app.grayscale_tolerance = 0,
            23 => app.colorspace_idx = 1,
            24 => app.encoding_idx = 0,
            _ => {}
        }
    } else if app.current_tab == 1 {
        match app.selected_control {
            0 => app.pixelize = 0,
            1 => app.gaussian_blur = 0,
            2 => app.box_blur = false,
            3 => app.oil = false,
            4 => app.halftone = false,
            5 => app.sepia = false,
            6 => app.normalize = false,
            7 => app.noise = false,
            8 => app.emboss = false,
            9 => app.identity = false,
            10 => app.laplace = false,
            11 => app.noise_reduction = false,
            12 => app.sharpen = false,
            13 => app.cali = false,
            14 => app.dramatic = false,
            15 => app.firenze = false,
            16 => app.golden = false,
            17 => app.lix = false,
            18 => app.lofi = false,
            19 => app.neue = false,
            20 => app.obsidian = false,
            21 => app.pastel_pink = false,
            22 => app.ryo = false,
            23 => app.frosted_glass = false,
            24 => app.solarize = false,
            25 => app.edge_detection = false,
            _ => {}
        }
    }
    app.update_render_args();
}

// ── UI Rendering ─────────────────────────────────────────────────────────────
fn ui(f: &mut ratatui::Frame, app: &mut App) {
    let area = f.area();

    // Background
    f.render_widget(Block::default().style(Style::new().bg(BG)), area);

    // Layout: [title_bar | main_content | status_bar]
    let root = Layout::vertical([
        Constraint::Length(1), // title/tabs
        Constraint::Min(0),   // content
        Constraint::Length(1), // status bar
    ]).split(area);

    // ── Title bar with tabs ──
    render_title_bar(f, app, root[0]);

    // ── Main content: left panel + output ──
    let main = Layout::horizontal([
        Constraint::Length(44), // left panel
        Constraint::Min(0),     // output
    ]).split(root[1]);

    // ── Left panel: controls ──
    render_controls(f, app, main[0]);

    // ── Right panel: output ──
    render_output(f, app, main[1]);

    // ── Status bar ──
    render_status_bar(f, app, root[2]);

    // ── Save dialog overlay ──
    if app.show_save_dialog {
        render_save_dialog(f, app);
    }
}

fn render_title_bar(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    let layout = Layout::horizontal([
        Constraint::Length(10),  // app title
        Constraint::Min(0),     // tabs + file
    ]).split(area);

    // App title
    let title = Span::styled(
        " img2irc ",
        Style::new().fg(Color::White).bg(ACCENT).add_modifier(Modifier::BOLD),
    );
    f.render_widget(Paragraph::new(Line::from(title)), layout[0]);

    // Tabs + filename
    let tab_names = ["  Main  ", " Effects ", "  Data  "];
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

        let style = Style::new().fg(fg).bg(bg_color).add_modifier(if i == app.current_tab { Modifier::BOLD } else { Modifier::empty() });
        buf.set_style(tab_area, style);
        buf.set_string(x, tab_area.y, name, style);

        x += w + 1;
    }

    // Show image path after tabs
    x += 1;
    let path_display = format!("📁 {}", app.image_path);
    let avail = (layout[1].x + layout[1].width).saturating_sub(x) as usize;
    let truncated = if path_display.len() > avail {
        format!("…{}", &app.image_path[app.image_path.len().saturating_sub(avail.saturating_sub(3))..])
    } else {
        path_display
    };
    buf.set_string(x, layout[1].y, &truncated, Style::new().fg(TEXT_DIM).bg(BG));

    app.tab_areas = tab_rects;
}

fn render_controls(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    let buf = f.buffer_mut();

    // Panel border
    let panel = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(BORDER))
        .border_set(symbols::border::ROUNDED)
        .style(Style::new().bg(SURFACE));
    let inner = panel.inner(area);
    panel.render(area, buf);

    let controls = app.tab_controls(app.current_tab);
    let avail_height = inner.height as usize;
    let item_height: u16 = 1; // single-row controls
    let max_visible = avail_height / item_height as usize;

    // Scrolling offset for controls
    let scroll_offset = if app.selected_control >= max_visible {
        app.selected_control - max_visible + 1
    } else {
        0
    };

    let mut control_rects = Vec::new();
    let mut y = inner.y;

    for (i, ctrl) in controls.iter().enumerate() {
        if i < scroll_offset { 
            control_rects.push(Rect::default());
            continue; 
        }
        if y + item_height > inner.y + inner.height { 
            control_rects.push(Rect::default());
            continue; 
        }

        let ctrl_area = Rect::new(inner.x, y, inner.width, item_height);
        let selected = i == app.selected_control;

        match &ctrl.kind {
            ControlKind::Slider { min, max } => {
                let value = app.get_control_value(app.current_tab, i);
                render_bar(buf, ctrl_area, ctrl.label, value, *min, *max, selected);
            }
            ControlKind::Enum { options } => {
                let value = app.get_control_value(app.current_tab, i) as usize;
                render_enum(buf, ctrl_area, ctrl.label, options, value, selected);
            }
            ControlKind::Toggle => {
                let on = app.get_control_value(app.current_tab, i) != 0;
                // For blocks tab, show block name
                let label = if app.current_tab == 2 && i < app.available_blocks.len() {
                    &app.available_blocks[i]
                } else {
                    ctrl.label
                };
                render_toggle(buf, ctrl_area, label, on, selected);
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

fn render_output(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let is_loading = app.is_rendering.load(Ordering::Relaxed) && app.cached_render.is_empty();
    let spinners = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    let spinner_ch = spinners[(app.render_tick / 3) % spinners.len()];

    // Compute output stats
    let line_count = app.cached_render.lines().count();
    let max_bytes = app.cached_render.lines().map(|l| l.len()).max().unwrap_or(0);

    let title_right = if is_loading {
        format!(" {} Rendering... ", spinner_ch)
    } else if app.is_rendering.load(Ordering::Relaxed) {
        format!(" {} Updating... │ {}×{} │ {}B ", spinner_ch, max_bytes, line_count, app.cached_render.len())
    } else {
        format!(" {}×{} │ {}B ", max_bytes, line_count, app.cached_render.len())
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(BORDER))
        .border_set(symbols::border::ROUNDED)
        .title(Span::styled(" Output ", Style::new().fg(TEXT).add_modifier(Modifier::BOLD)))
        .title_bottom(Line::from(Span::styled(title_right, Style::new().fg(TEXT_DIM))))
        .style(Style::new().bg(BG));

    if is_loading {
        let loading_text = format!("\n\n\n      {} Rendering image...", spinner_ch);
        let p = Paragraph::new(loading_text)
            .block(block)
            .alignment(Alignment::Center)
            .style(Style::new().fg(ACCENT));
        f.render_widget(p, area);
    } else {
        let text = app.cached_render.as_bytes().into_text().unwrap_or_else(|_| {
            ratatui::text::Text::from(app.cached_render.clone())
        });
        let output = Paragraph::new(text)
            .block(block)
            .alignment(Alignment::Center)
            .scroll((app.scroll, 0));
        f.render_widget(output, area);
    }
}

fn render_status_bar(f: &mut ratatui::Frame, _app: &App, area: Rect) {
    let keys = [
        ("Tab", "Switch"),
        ("↑↓", "Navigate"),
        ("←→", "Adjust"),
        ("Space", "Toggle"),
        ("Del", "Reset"),
        ("Shift", "×10"),
        ("Ctrl+S", "Save"),
        ("Q", "Quit"),
    ];

    let spans: Vec<Span> = keys.iter().flat_map(|(key, desc)| {
        vec![
            Span::styled(format!(" {} ", key), Style::new().fg(Color::White).bg(STATUS_KEY).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" {} ", desc), Style::new().fg(STATUS_DESC).bg(STATUS_BG)),
        ]
    }).collect();

    let line = Line::from(spans);
    let bar = Paragraph::new(line).style(Style::new().bg(STATUS_BG));
    f.render_widget(bar, area);
}

fn render_save_dialog(f: &mut ratatui::Frame, app: &App) {
    let area = centered_rect(50, 20, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .title(Span::styled(" Save as PNG ", Style::new().fg(Color::White).add_modifier(Modifier::BOLD)))
        .borders(Borders::ALL)
        .border_style(Style::new().fg(ACCENT))
        .border_set(symbols::border::ROUNDED)
        .style(Style::new().bg(SURFACE));
    
    let inner = block.inner(area);
    f.render_widget(block, area);

    let input_text = if app.save_input.is_empty() {
        "Type filename and press Enter..."
    } else {
        &app.save_input
    };
    let style = if app.save_input.is_empty() { 
        Style::new().fg(TEXT_DIM) 
    } else { 
        Style::new().fg(ACCENT).add_modifier(Modifier::BOLD) 
    };
    
    let prompt_area = Rect::new(inner.x + 1, inner.y + inner.height / 2, inner.width - 2, 1);
    f.render_widget(Paragraph::new(input_text).style(style), prompt_area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ]).split(r);

    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ]).split(popup[1])[1]
}
