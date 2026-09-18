use crate::args::{Args, ColorSpec, ColourSpace, Param, Render, SamplingFilter, TextOverlay};
use crate::{args_to_render_args, draw, font};
use clap::Parser;
use figlet_rs::FIGlet;
use photon_rs::PhotonImage;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{Cursor, Read};
use unicode_width::UnicodeWidthChar;
use wasm_bindgen::prelude::*;

const DEFAULT_WIDTH: u32 = 80;
const DEFAULT_FONT_SIZE: f32 = 16.0;
const MAX_INPUT_PIXELS: u64 = 64 * 1024 * 1024;
const GLYPH_CACHE_CAPACITY: usize = 2;
const MAX_FIGLET_FONTS: usize = 128;
const MAX_FIGLET_FONT_BYTES: usize = 4 * 1024 * 1024;
const MAX_FIGLET_TOTAL_BYTES: usize = 32 * 1024 * 1024;
const MAX_FIGLET_REQUESTS: usize = 1024;

#[wasm_bindgen(typescript_custom_section)]
const TYPESCRIPT_TYPES: &str = r#"
export type Img2IrcColorMode = "irc" | "ansi" | "ansi24";
export type Img2IrcSamplingFilter =
    | "nearest"
    | "triangle"
    | "catmull-rom"
    | "gaussian"
    | "lanczos3";
export type Img2IrcColourSpace = "hsl" | "hsv" | "hsluv" | "lch";

export interface Img2IrcRenderOptions {
    width?: number;
    height?: number;
    fontSize?: number;
    render?: Img2IrcColorMode;
    braille?: boolean;
    glyphs?: string;
    include?: string;
    includeRanges?: string[];
    exclude?: string;
    excludeRanges?: string[];
    filter?: Img2IrcSamplingFilter;
    scaleX?: number;
    scaleY?: number;
    rotate?: number;
    flipHorizontal?: boolean;
    flipVertical?: boolean;
    grayscaleTolerance?: number;
    colourSpace?: Img2IrcColourSpace;
    brightness?: number;
    contrast?: number;
    lumaContrast?: number;
    medianBlur?: number;
    lineThickness?: number;
    lightLines?: boolean;
    replaceFrom?: [number, number, number];
    replaceTo?: [number, number, number];
    replaceTolerance?: number;
    gamma?: number;
    saturation?: number;
    hue?: number;
    invert?: boolean;
    dither?: number;
    grayscale?: boolean;
    noGrayscale?: boolean;
    pixelize?: number;
    boxBlur?: boolean;
    gaussianBlur?: number;
    smooth?: boolean;
    smoothCandidates?: number;
    smoothOrders?: number;
    smoothShapes?: boolean;
    smoothNeighbors?: boolean;
    includeCells?: boolean;
    includePreview?: boolean;
    overlays?: Img2IrcTextOverlay[];
}

export interface Img2IrcTextOverlay {
    text: string;
    x: number;
    y: number;
    width?: number;
    height?: number;
    foreground?: [number, number, number];
    background?: [number, number, number];
    wrap?: boolean;
    autoGrow?: boolean;
    transparentSpaces?: boolean;
    bold?: boolean;
    italic?: boolean;
    underline?: boolean;
}

export interface Img2IrcFigletFont {
    name: string;
    height: number;
    data: Uint8Array;
}

export interface Img2IrcFigletRequest {
    text: string;
    baseWidth?: number;
    baseHeight?: number;
    fillAvailable?: boolean;
    maxWidth: number;
    maxHeight: number;
    useFiglet?: boolean;
    fontName?: string;
}

export interface Img2IrcFigletArt {
    source: string;
    text: string;
    width: number;
    height: number;
}

export interface Img2IrcCell {
    character: string;
    foreground: [number, number, number];
    background?: [number, number, number];
    inverted: boolean;
    bold: boolean;
    italic: boolean;
    underline: boolean;
}

export interface Img2IrcRenderResult {
    content: string;
    columns: number;
    rows: number;
    cells?: Img2IrcCell[][];
    errorCount: number;
    totalPixels: number;
    score: number;
    previewWidth: number;
    previewHeight: number;
    fontSize: number;
    cellWidth: number;
    cellHeight: number;
    cellAdvance: number;
    lineHeight: number;
    longestLineBytes: number;
    timings: Img2IrcRenderTimings;
    previewRgba?: Uint8Array;
}

export interface Img2IrcRenderTimings {
    canvasCacheHit: boolean;
    resultCacheHit: boolean;
    prepareCanvasMs: number;
    glyphMatchMs: number;
    smoothingPrepareMs: number;
    smoothingSearchMs: number;
    shapeRefineMs: number;
    finalScoreMs: number;
    encodeMs: number;
    overlayMs: number;
    previewMs: number;
    contourCacheHits: number;
    contourCacheMisses: number;
}

export interface Img2IrcTransformedImage {
    width: number;
    height: number;
    rgba: Uint8Array;
}

export interface Img2Irc {
    setFont(fontData: Uint8Array): void;
    setImage(imageData: Uint8Array): void;
    setFigletFonts(fonts: Img2IrcFigletFont[]): void;
    renderFiglet(requests: Img2IrcFigletRequest[]): (Img2IrcFigletArt | undefined)[];
    getTransformedImage(options: Img2IrcRenderOptions): Img2IrcTransformedImage;
    clearCache(): void;
    renderCurrent(options: Img2IrcRenderOptions): Img2IrcRenderResult;
    render(imageData: Uint8Array, options: Img2IrcRenderOptions): Img2IrcRenderResult;
}
"#;

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum WebRender {
    #[default]
    Irc,
    Ansi,
    Ansi24,
}

impl From<WebRender> for Render {
    fn from(value: WebRender) -> Self {
        match value {
            WebRender::Irc => Render::Irc,
            WebRender::Ansi => Render::Ansi,
            WebRender::Ansi24 => Render::Ansi24,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum WebSamplingFilter {
    #[default]
    Nearest,
    Triangle,
    CatmullRom,
    Gaussian,
    Lanczos3,
}

impl From<WebSamplingFilter> for SamplingFilter {
    fn from(value: WebSamplingFilter) -> Self {
        match value {
            WebSamplingFilter::Nearest => SamplingFilter::Nearest,
            WebSamplingFilter::Triangle => SamplingFilter::Triangle,
            WebSamplingFilter::CatmullRom => SamplingFilter::CatmullRom,
            WebSamplingFilter::Gaussian => SamplingFilter::Gaussian,
            WebSamplingFilter::Lanczos3 => SamplingFilter::Lanczos3,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum WebColourSpace {
    Hsl,
    #[default]
    Hsv,
    Hsluv,
    Lch,
}

impl From<WebColourSpace> for ColourSpace {
    fn from(value: WebColourSpace) -> Self {
        match value {
            WebColourSpace::Hsl => ColourSpace::HSL,
            WebColourSpace::Hsv => ColourSpace::HSV,
            WebColourSpace::Hsluv => ColourSpace::HSLUV,
            WebColourSpace::Lch => ColourSpace::LCH,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct WebOptions {
    width: Option<u32>,
    height: Option<u32>,
    font_size: f32,
    render: WebRender,
    braille: bool,
    glyphs: Option<String>,
    include: Option<String>,
    include_ranges: Vec<String>,
    exclude: String,
    exclude_ranges: Vec<String>,
    filter: WebSamplingFilter,
    scale_x: f32,
    scale_y: f32,
    rotate: f32,
    flip_horizontal: bool,
    flip_vertical: bool,
    grayscale_tolerance: u8,
    colour_space: WebColourSpace,
    brightness: f32,
    contrast: f32,
    luma_contrast: f32,
    median_blur: i32,
    line_thickness: i32,
    light_lines: bool,
    replace_from: Option<[u8; 3]>,
    replace_to: [u8; 3],
    replace_tolerance: f32,
    gamma: f32,
    saturation: f32,
    hue: f32,
    invert: bool,
    dither: u32,
    grayscale: bool,
    no_grayscale: bool,
    pixelize: i32,
    box_blur: bool,
    gaussian_blur: i32,
    smooth: bool,
    smooth_candidates: u32,
    smooth_orders: u32,
    smooth_shapes: bool,
    smooth_neighbors: bool,
    include_cells: bool,
    include_preview: bool,
    overlays: Vec<WebTextOverlay>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct WebTextOverlay {
    text: String,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    foreground: Option<[u8; 3]>,
    background: Option<[u8; 3]>,
    wrap: bool,
    auto_grow: bool,
    transparent_spaces: bool,
    bold: bool,
    italic: bool,
    underline: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WebFigletFontInput {
    name: String,
    height: u32,
    data: Vec<u8>,
}

#[derive(Debug)]
struct WebFigletFont {
    name: String,
    height: u32,
    priority: usize,
    font: FIGlet,
}

#[derive(Debug, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct WebFigletRequest {
    text: String,
    fill_available: bool,
    base_width: usize,
    base_height: usize,
    max_width: usize,
    max_height: usize,
    use_figlet: bool,
    font_name: Option<String>,
}

impl Default for WebFigletRequest {
    fn default() -> Self {
        Self {
            text: String::new(),
            fill_available: false,
            base_width: 0,
            base_height: 0,
            max_width: 1,
            max_height: 1,
            use_figlet: false,
            font_name: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebFigletArt {
    source: String,
    text: String,
    width: usize,
    height: usize,
}

impl WebTextOverlay {
    fn into_render_overlay(self) -> TextOverlay {
        let (width, height) = if self.auto_grow {
            let mut width = 0usize;
            let mut height = 0usize;
            for line in self.text.split('\n') {
                width = width.max(line.trim_end_matches('\r').chars().count());
                height += 1;
            }
            (width.max(1) as i32, height.max(1) as i32)
        } else {
            (self.width.max(0), self.height.max(0))
        };
        TextOverlay {
            text: self.text,
            source_text: None,
            figlet_font: None,
            x: self.x,
            y: self.y,
            w: width,
            h: height,
            fg: self.foreground.map(ColorSpec::Rgb),
            bg: self.background.map(ColorSpec::Rgb),
            wrap: self.wrap,
            auto_grow: self.auto_grow,
            transparent_spaces: self.transparent_spaces,
            bold: self.bold,
            italic: self.italic,
            underline: self.underline,
        }
    }
}

impl Default for WebOptions {
    fn default() -> Self {
        Self {
            width: Some(DEFAULT_WIDTH),
            height: None,
            font_size: DEFAULT_FONT_SIZE,
            render: WebRender::Irc,
            braille: false,
            glyphs: None,
            include: None,
            include_ranges: Vec::new(),
            exclude: String::new(),
            exclude_ranges: Vec::new(),
            filter: WebSamplingFilter::Nearest,
            scale_x: 1.0,
            scale_y: 1.0,
            rotate: 0.0,
            flip_horizontal: false,
            flip_vertical: false,
            grayscale_tolerance: 32,
            colour_space: WebColourSpace::Hsv,
            brightness: 0.0,
            contrast: 0.0,
            luma_contrast: 0.0,
            median_blur: 0,
            line_thickness: 0,
            light_lines: false,
            replace_from: None,
            replace_to: [0; 3],
            replace_tolerance: 5.0,
            gamma: 0.0,
            saturation: 0.0,
            hue: 0.0,
            invert: false,
            dither: 0,
            grayscale: false,
            no_grayscale: false,
            pixelize: 0,
            box_blur: false,
            gaussian_blur: 0,
            smooth: false,
            smooth_candidates: 12,
            smooth_orders: 1,
            smooth_shapes: false,
            smooth_neighbors: true,
            include_cells: false,
            include_preview: true,
            overlays: Vec::new(),
        }
    }
}

impl WebOptions {
    fn validate(&self) -> Result<(), JsValue> {
        if self.height == Some(0) {
            return Err(js_error("height must be greater than zero"));
        }
        if !self.font_size.is_finite() || self.font_size < 6.0 || self.font_size > 256.0 {
            return Err(js_error("fontSize must be between 6 and 256"));
        }
        if !self.scale_x.is_finite()
            || !self.scale_y.is_finite()
            || self.scale_x <= 0.0
            || self.scale_y <= 0.0
        {
            return Err(js_error("scaleX and scaleY must be positive numbers"));
        }
        if !self.rotate.is_finite() {
            return Err(js_error("rotate must be a finite number"));
        }
        let has_explicit_glyphs = self.glyphs.as_ref().is_some_and(|value| !value.is_empty())
            || self.include.as_ref().is_some_and(|value| !value.is_empty())
            || !self.include_ranges.is_empty();
        if !has_explicit_glyphs {
            return Err(js_error(
                "glyphs must be supplied from a runtime catalog or explicit includes",
            ));
        }
        if self.overlays.len() > 1024 {
            return Err(js_error("at most 1024 text overlays may be rendered"));
        }
        if self.overlays.iter().any(|overlay| {
            overlay.text.chars().count() > 65_536
                || overlay.width < 0
                || overlay.height < 0
                || overlay.width > 16_384
                || overlay.height > 16_384
        }) {
            return Err(js_error("a text overlay has invalid text or dimensions"));
        }
        Ok(())
    }

    fn glyph_cache_key(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.font_size.to_bits().hash(&mut hasher);
        self.braille.hash(&mut hasher);
        self.glyphs.hash(&mut hasher);
        self.include.hash(&mut hasher);
        self.include_ranges.hash(&mut hasher);
        self.exclude.hash(&mut hasher);
        self.exclude_ranges.hash(&mut hasher);
        hasher.finish()
    }

    fn canvas_cache_key(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.glyph_cache_key().hash(&mut hasher);
        self.width.hash(&mut hasher);
        self.height.hash(&mut hasher);
        (self.filter as u8).hash(&mut hasher);
        self.scale_x.to_bits().hash(&mut hasher);
        self.scale_y.to_bits().hash(&mut hasher);
        self.rotate.to_bits().hash(&mut hasher);
        self.flip_horizontal.hash(&mut hasher);
        self.flip_vertical.hash(&mut hasher);
        self.grayscale_tolerance.hash(&mut hasher);
        (self.colour_space as u8).hash(&mut hasher);
        self.brightness.to_bits().hash(&mut hasher);
        self.contrast.to_bits().hash(&mut hasher);
        self.luma_contrast.to_bits().hash(&mut hasher);
        self.median_blur.hash(&mut hasher);
        self.line_thickness.hash(&mut hasher);
        self.light_lines.hash(&mut hasher);
        self.replace_from.hash(&mut hasher);
        self.replace_to.hash(&mut hasher);
        self.replace_tolerance.to_bits().hash(&mut hasher);
        self.gamma.to_bits().hash(&mut hasher);
        self.saturation.to_bits().hash(&mut hasher);
        self.hue.to_bits().hash(&mut hasher);
        self.invert.hash(&mut hasher);
        self.dither.hash(&mut hasher);
        self.grayscale.hash(&mut hasher);
        self.no_grayscale.hash(&mut hasher);
        self.pixelize.hash(&mut hasher);
        self.box_blur.hash(&mut hasher);
        self.gaussian_blur.hash(&mut hasher);
        hasher.finish()
    }

    fn result_cache_key(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.canvas_cache_key().hash(&mut hasher);
        (self.render as u8).hash(&mut hasher);
        self.smooth.hash(&mut hasher);
        self.smooth_candidates.hash(&mut hasher);
        self.smooth_orders.hash(&mut hasher);
        self.smooth_shapes.hash(&mut hasher);
        self.smooth_neighbors.hash(&mut hasher);
        hasher.finish()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebCell {
    character: String,
    foreground: [u8; 3],
    background: Option<[u8; 3]>,
    inverted: bool,
    bold: bool,
    italic: bool,
    underline: bool,
}

impl From<&draw::RenderedCell> for WebCell {
    fn from(cell: &draw::RenderedCell) -> Self {
        Self {
            character: cell.char.to_string(),
            foreground: cell.fg,
            background: cell.bg,
            inverted: cell.inverted,
            bold: cell.bold,
            italic: cell.italic,
            underline: cell.underline,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebRenderResult {
    content: String,
    columns: usize,
    rows: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    cells: Option<Vec<Vec<WebCell>>>,
    error_count: u64,
    total_pixels: u64,
    score: f64,
    preview_width: u32,
    preview_height: u32,
    font_size: f32,
    cell_width: usize,
    cell_height: usize,
    cell_advance: f32,
    line_height: f32,
    longest_line_bytes: usize,
    timings: WebRenderTimings,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebTransformedImage {
    width: u32,
    height: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebRenderTimings {
    canvas_cache_hit: bool,
    result_cache_hit: bool,
    prepare_canvas_ms: f64,
    glyph_match_ms: f64,
    smoothing_prepare_ms: f64,
    smoothing_search_ms: f64,
    shape_refine_ms: f64,
    final_score_ms: f64,
    encode_ms: f64,
    overlay_ms: f64,
    preview_ms: f64,
    contour_cache_hits: u64,
    contour_cache_misses: u64,
}

fn duration_ms(duration: std::time::Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

struct GlyphCache {
    key: u64,
    store: font::GlyphStore,
    fast_glyphs: Vec<draw::FastGlyph>,
    bitmap_index: draw::GlyphBitmapIndex,
}

struct CanvasCache {
    key: u64,
    canvas: draw::AnsiImage,
    luma_canvas: Option<draw::AnsiImage>,
}

struct ResultCache {
    key: u64,
    result: draw::RenderResult,
}

fn normalize_figlet_art(source: String, rendered: &str) -> Option<WebFigletArt> {
    let rows = rendered
        .lines()
        .map(|line| line.chars().collect::<Vec<_>>())
        .collect::<Vec<_>>();
    if rows.is_empty()
        || rows.iter().flatten().any(|character| {
            character.is_control()
                || *character == '\u{1b}'
                || matches!(UnicodeWidthChar::width(*character), Some(width) if width != 1)
        })
    {
        return None;
    }

    let mut min_x = usize::MAX;
    let mut max_x = 0usize;
    let mut min_y = usize::MAX;
    let mut max_y = 0usize;
    for (y, row) in rows.iter().enumerate() {
        for (x, character) in row.iter().enumerate() {
            if !character.is_whitespace() {
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
    }
    if min_x == usize::MAX || min_y == usize::MAX {
        return None;
    }

    let lines = rows[min_y..=max_y]
        .iter()
        .map(|row| {
            (min_x..=max_x)
                .map(|x| row.get(x).copied().unwrap_or(' '))
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    Some(WebFigletArt {
        source,
        text: lines.join("\n"),
        width: max_x - min_x + 1,
        height: max_y - min_y + 1,
    })
}

fn plain_figlet_art(text: &str, max_width: usize, max_height: usize) -> Option<WebFigletArt> {
    let text = text.trim();
    let width = text.lines().map(|line| line.chars().count()).max().unwrap_or(0);
    let height = text.lines().count();
    if text.is_empty() || width > max_width || height > max_height {
        return None;
    }
    Some(WebFigletArt {
        source: "plain".to_string(),
        text: text.to_string(),
        width,
        height,
    })
}

fn render_figlet_font(font: &WebFigletFont, text: &str) -> Option<WebFigletArt> {
    let rendered = crate::figlet_text::render(&font.font, text)?;
    normalize_figlet_art(font.name.clone(), &rendered)
}

fn decode_figlet_font(data: &[u8]) -> Result<String, String> {
    if data.starts_with(b"PK\x03\x04") {
        let mut archive =
            zip::ZipArchive::new(Cursor::new(data)).map_err(|error| error.to_string())?;
        if archive.is_empty() {
            return Err("ZIP archive is empty".to_string());
        }
        let mut file = archive.by_index(0).map_err(|error| error.to_string())?;
        if file.size() > MAX_FIGLET_FONT_BYTES as u64 {
            return Err("decompressed FIGlet font exceeds the browser limit".to_string());
        }
        let mut contents = String::new();
        (&mut file)
            .take(MAX_FIGLET_FONT_BYTES as u64 + 1)
            .read_to_string(&mut contents)
            .map_err(|error| error.to_string())?;
        if contents.len() > MAX_FIGLET_FONT_BYTES {
            return Err("decompressed FIGlet font exceeds the browser limit".to_string());
        }
        Ok(contents)
    } else {
        String::from_utf8(data.to_vec()).map_err(|error| error.to_string())
    }
}

fn select_figlet_art(fonts: &[WebFigletFont], request: &WebFigletRequest) -> Option<WebFigletArt> {
    let text = request.text.trim();
    if text.is_empty() || request.max_width == 0 || request.max_height == 0 {
        return None;
    }
    let base_w = if request.base_width > 0 { request.base_width } else { request.max_width };
    let base_h = if request.base_height > 0 { request.base_height } else { request.max_height };

    if request.use_figlet {
        if let Some(font_name) = request.font_name.as_deref() {
            if font_name.eq_ignore_ascii_case("plain") {
                return plain_figlet_art(text, request.max_width, request.max_height);
            }
            return fonts
                .iter()
                .find(|font| font.name.eq_ignore_ascii_case(font_name))
                .and_then(|font| render_figlet_font(font, text))
                .filter(|art| {
                    art.width <= request.max_width && art.height <= request.max_height
                })
                .or_else(|| plain_figlet_art(text, request.max_width, request.max_height));
        }

        if request.fill_available {
            return fonts.iter().filter_map(|font| {
                render_figlet_font(font, text)
                    .filter(|art| art.width <= request.max_width && art.height <= request.max_height)
                    .map(|art| (art, font.priority))
            }).max_by_key(|(art, priority)| (art.height, art.width, std::cmp::Reverse(*priority)))
                .map(|(art, _)| art)
                .or_else(|| plain_figlet_art(text, request.max_width, request.max_height));
        }

        // Step 1: Check if any FIGlet font fits in original base box without enlargement
        let mut original_heights = fonts
            .iter()
            .map(|font| font.height)
            .filter(|height| *height > 0 && *height as usize <= base_h)
            .collect::<Vec<_>>();
        original_heights.sort_unstable_by(|left, right| right.cmp(left));
        original_heights.dedup();

        for height in original_heights {
            let mut list_fonts = fonts.iter().filter(|f| f.height == height).collect::<Vec<_>>();
            list_fonts.sort_by_key(|f| f.priority);
            for font in list_fonts {
                if let Some(rendered) = render_figlet_font(font, text) {
                    if rendered.width <= base_w && rendered.height <= base_h {
                        return Some(rendered);
                    }
                }
            }
        }

        // Step 2: Only if no font fit in original box, check candidate fonts up to max_width x max_height
        if request.max_width > base_w || request.max_height > base_h {
            let mut candidates = Vec::new();
            for font in fonts {
                if font.height == 0 || font.height as usize > request.max_height {
                    continue;
                }
                if let Some(rendered) = render_figlet_font(font, text) {
                    if rendered.width <= request.max_width && rendered.height <= request.max_height {
                        let expand_w = rendered.width.saturating_sub(base_w);
                        let expand_h = rendered.height.saturating_sub(base_h);
                        candidates.push((rendered, expand_w, expand_h, font.height, font.priority));
                    }
                }
            }
            if !candidates.is_empty() {
                candidates.sort_by(|a, b| {
                    (a.1 + a.2)
                        .cmp(&(b.1 + b.2))
                        .then_with(|| b.3.cmp(&a.3))
                        .then_with(|| a.4.cmp(&b.4))
                });
                return Some(candidates.into_iter().next().unwrap().0);
            }
        }
    }
    plain_figlet_art(text, request.max_width, request.max_height)
}

/// A reusable img2irc renderer for browser applications.
///
/// Construct it with a monospace OpenType/TrueType font as a `Uint8Array`, then
/// load encoded PNG/JPEG/GIF/WebP bytes and render them with an options object.
#[wasm_bindgen(js_name = Img2Irc)]
pub struct WebRenderer {
    font_data: Vec<u8>,
    glyph_caches: Vec<GlyphCache>,
    canvas_cache: Option<CanvasCache>,
    result_cache: Option<ResultCache>,
    figlet_fonts: Vec<WebFigletFont>,
    source_image: Option<PhotonImage>,
    cli: Args,
}

#[wasm_bindgen(js_class = Img2Irc)]
impl WebRenderer {
    #[wasm_bindgen(constructor)]
    pub fn new(font_data: &[u8]) -> Result<WebRenderer, JsValue> {
        if font_data.is_empty() {
            return Err(js_error("font data is empty"));
        }

        // Validate eagerly so bad font uploads fail at construction time.
        font::validate_monospace_font_bytes(font_data).map_err(js_error)?;

        Ok(Self {
            font_data: font_data.to_vec(),
            glyph_caches: Vec::with_capacity(GLYPH_CACHE_CAPACITY),
            canvas_cache: None,
            result_cache: None,
            figlet_fonts: Vec::new(),
            source_image: None,
            // Parsing Clap defaults is relatively expensive in WASM. Do it
            // once per font-backed renderer instead of once per image render.
            cli: Args::parse_from(["img2irc"]),
        })
    }

    /// Decode and retain an image for repeated renders. Effect and layout
    /// changes can then call `renderCurrent` without transferring or decoding
    /// the original image again.
    #[wasm_bindgen(js_name = setImage)]
    pub fn set_image(&mut self, image_data: &[u8]) -> Result<(), JsValue> {
        self.source_image = Some(decode_image(image_data)?);
        self.canvas_cache = None;
        self.result_cache = None;
        Ok(())
    }

    /// Replace the render font while retaining the decoded source image.
    /// Glyph raster caches depend on the font and are discarded.
    #[wasm_bindgen(js_name = setFont)]
    pub fn set_font(&mut self, font_data: &[u8]) -> Result<(), JsValue> {
        if font_data.is_empty() {
            return Err(js_error("font data is empty"));
        }
        font::validate_monospace_font_bytes(font_data).map_err(js_error)?;
        self.font_data.clear();
        self.font_data.extend_from_slice(font_data);
        self.glyph_caches.clear();
        self.canvas_cache = None;
        self.result_cache = None;
        Ok(())
    }

    /// Parse and cache externally hosted FIGlet fonts. Font data remains a
    /// deployment asset and is supplied by JavaScript instead of being
    /// embedded in the WebAssembly module.
    #[wasm_bindgen(js_name = setFigletFonts)]
    pub fn set_figlet_fonts(&mut self, fonts: JsValue) -> Result<(), JsValue> {
        let inputs: Vec<WebFigletFontInput> = serde_wasm_bindgen::from_value(fonts)
            .map_err(|error| js_error(format!("invalid FIGlet font list: {error}")))?;
        if inputs.len() > MAX_FIGLET_FONTS {
            return Err(js_error(format!(
                "at most {MAX_FIGLET_FONTS} FIGlet fonts may be loaded"
            )));
        }
        let total_bytes = inputs.iter().map(|font| font.data.len()).sum::<usize>();
        if total_bytes > MAX_FIGLET_TOTAL_BYTES
            || inputs
                .iter()
                .any(|font| font.data.len() > MAX_FIGLET_FONT_BYTES)
        {
            return Err(js_error("FIGlet font data exceeds the browser limit"));
        }

        let mut names = HashSet::new();
        let mut parsed = Vec::with_capacity(inputs.len());
        let mut decoded_bytes = 0usize;
        for (priority, input) in inputs.into_iter().enumerate() {
            let name = input.name.trim();
            if name.is_empty() || name.len() > 256 || name.eq_ignore_ascii_case("plain") {
                return Err(js_error("a FIGlet font has an invalid or reserved name"));
            }
            if input.height == 0 || input.height > 64 {
                return Err(js_error(format!(
                    "FIGlet font {name:?} has an invalid configured height"
                )));
            }
            if !names.insert(name.to_lowercase()) {
                return Err(js_error(format!("duplicate FIGlet font {name:?}")));
            }
            let contents = decode_figlet_font(&input.data).map_err(|error| {
                js_error(format!("could not decode FIGlet font {name:?}: {error}"))
            })?;
            decoded_bytes = decoded_bytes.saturating_add(contents.len());
            if decoded_bytes > MAX_FIGLET_TOTAL_BYTES {
                return Err(js_error(
                    "decompressed FIGlet font data exceeds the browser limit",
                ));
            }
            let font = FIGlet::from_content(&contents).map_err(|error| {
                js_error(format!("could not parse FIGlet font {name:?}: {error}"))
            })?;
            let actual_height = font.header_line.height as u32;
            parsed.push(WebFigletFont {
                name: name.to_string(),
                height: actual_height,
                priority,
                font,
            });
        }
        self.figlet_fonts = parsed;
        Ok(())
    }

    /// Render text labels with an explicitly requested hosted FIGlet font, or
    /// automatically choose the best fit when no font name is supplied. Falls
    /// back to ordinary one-row text when the requested rendering cannot fit.
    #[wasm_bindgen(js_name = renderFiglet)]
    pub fn render_figlet(&self, requests: JsValue) -> Result<JsValue, JsValue> {
        let requests: Vec<WebFigletRequest> = serde_wasm_bindgen::from_value(requests)
            .map_err(|error| js_error(format!("invalid FIGlet requests: {error}")))?;
        if requests.len() > MAX_FIGLET_REQUESTS {
            return Err(js_error(format!(
                "at most {MAX_FIGLET_REQUESTS} FIGlet labels may be rendered"
            )));
        }
        if requests.iter().any(|request| {
            request.text.chars().count() > 4096
                || request.max_width > 16_384
                || request.max_height > 16_384
        }) {
            return Err(js_error("a FIGlet request exceeds the browser limit"));
        }
        let rendered = requests
            .iter()
            .map(|request| select_figlet_art(&self.figlet_fonts, request))
            .collect::<Vec<_>>();
        serde_wasm_bindgen::to_value(&rendered)
            .map_err(|error| js_error(format!("could not serialize FIGlet output: {error}")))
    }

    /// Return the image with all configured pipeline effects (rotation, crop,
    /// scale, brightness, contrast, hue, saturation, etc.) applied.
    #[wasm_bindgen(js_name = getTransformedImage)]
    pub fn get_transformed_image(&mut self, options: JsValue) -> Result<JsValue, JsValue> {
        let mut options = if options.is_null() || options.is_undefined() {
            WebOptions::default()
        } else {
            serde_wasm_bindgen::from_value(options)
                .map_err(|error| js_error(format!("invalid render options: {error}")))?
        };
        if options.width == Some(0) {
            options.width = None;
        }
        options.validate()?;

        let Some(source) = self.source_image.clone() else {
            return Err(js_error("no image is loaded; call setImage first"));
        };

        let args = self.options_to_render_args(&options);
        let key = options.glyph_cache_key();
        let store = if let Some(cache) = self.glyph_caches.iter().find(|cache| cache.key == key) {
            cache.store.clone()
        } else {
            font::glyph_store_from_font_bytes(
                &self.font_data,
                &args.blocks,
                &args.exclude_range,
                &args.exclude,
                &args.include_range,
                args.include.as_ref(),
                args.font_size,
                false,
            )
            .map_err(js_error)?
        };

        let transformed = crate::effects::apply_effects(&args, &store, source.clone());
        let width = transformed.get_width();
        let height = transformed.get_height();
        let rgba = transformed.get_raw_pixels();

        let result = WebTransformedImage {
            width,
            height,
        };
        let value = serde_wasm_bindgen::to_value(&result)
            .map_err(|error| js_error(format!("could not serialize transformed image: {error}")))?;
        let bytes = js_sys::Uint8Array::from(rgba.as_slice());
        js_sys::Reflect::set(&value, &JsValue::from_str("rgba"), &bytes)
            .map_err(|_| js_error("could not attach transformed image pixels"))?;
        Ok(value)
    }

    /// Render the image most recently supplied to `setImage`.
    #[wasm_bindgen(js_name = renderCurrent)]
    pub fn render_current(&mut self, options: JsValue) -> Result<JsValue, JsValue> {
        self.render_loaded(options)
    }

    /// Compatibility entry point which loads and renders an image in one call.
    /// Repeated browser renders should prefer `setImage` plus `renderCurrent`.
    pub fn render(&mut self, image_data: &[u8], options: JsValue) -> Result<JsValue, JsValue> {
        self.set_image(image_data)?;
        self.render_loaded(options)
    }

    /// Drop cached rasterized glyphs after changing the supplied font outside
    /// this renderer or when the application wants to reclaim memory.
    #[wasm_bindgen(js_name = clearCache)]
    pub fn clear_cache(&mut self) {
        self.glyph_caches.clear();
        self.canvas_cache = None;
        self.result_cache = None;
        crate::contour_score::clear_cache();
    }
}

impl WebRenderer {
    fn options_to_render_args(&mut self, options: &WebOptions) -> crate::RenderArgs {
        let cli = &mut self.cli;
        cli.width = options.width.map(|value| Param::Fixed(value as f32));
        cli.height = options.height.map(|value| Param::Fixed(value as f32));
        cli.font_size = Param::Fixed(options.font_size);
        cli.render = options.render.into();
        cli.braille = options.braille;
        // Browser glyph membership comes from the external JSON catalog and
        // is passed as an explicit character string.
        cli.blocks = Vec::new();
        let mut explicit_glyphs = options.glyphs.clone().unwrap_or_default();
        if let Some(include) = &options.include {
            explicit_glyphs.push_str(include);
        }
        cli.include = (!explicit_glyphs.is_empty()).then_some(explicit_glyphs);
        cli.include_range = options.include_ranges.clone();
        cli.exclude = options.exclude.chars().collect();
        cli.exclude_range = options.exclude_ranges.clone();
        cli.filter = options.filter.into();
        cli.scale = Some((options.scale_x, options.scale_y));
        cli.rotate = Param::Fixed(options.rotate);
        cli.fliph = options.flip_horizontal;
        cli.flipv = options.flip_vertical;
        cli.grayscale_tolerance = Param::Fixed(f32::from(options.grayscale_tolerance));
        cli.colorspace = options.colour_space.into();
        cli.brightness = Param::Fixed(options.brightness);
        cli.contrast = Param::Fixed(options.contrast);
        cli.gamma = Param::Fixed(options.gamma);
        cli.saturation = Param::Fixed(options.saturation);
        cli.hue = Param::Fixed(options.hue);
        cli.invert = options.invert;
        cli.dither = options.dither;
        cli.grayscale = options.grayscale;
        cli.nograyscale = options.no_grayscale;
        cli.pixelize = Param::Fixed(options.pixelize as f32);
        cli.box_blur = options.box_blur;
        cli.gaussian_blur = Param::Fixed(options.gaussian_blur as f32);
        cli.smooth = options.smooth;
        cli.smooth_candidates = options.smooth_candidates.max(1);
        cli.smooth_orders = options.smooth_orders.clamp(1, 5);
        cli.smooth_shapes = options.smooth_shapes;
        cli.smooth_neighbors = options.smooth_neighbors;

        let mut args = args_to_render_args(&cli);
        use crate::pipeline::ImageEffect;
        args.pipeline = crate::effects::default_pipeline(&args);
        if let Some(from) = options.replace_from {
            args.pipeline.insert(0, ImageEffect::ReplaceColour {
                from,
                to: options.replace_to,
                tolerance: options.replace_tolerance,
            });
        }
        args.pipeline.push(ImageEffect::LumaContrast(options.luma_contrast));
        args.pipeline.push(ImageEffect::MedianBlur(options.median_blur));
        let thickness = options.line_thickness.clamp(-10, 10);
        args.pipeline.push(ImageEffect::DarkLines(if options.light_lines { -thickness } else { thickness }));
        args.ocr = false;
        args.save = None;
        args.fft_debug_dir = None;
        args.overlays = options
            .overlays
            .clone()
            .into_iter()
            .map(WebTextOverlay::into_render_overlay)
            .collect();
        args
    }

    fn render_loaded(&mut self, options: JsValue) -> Result<JsValue, JsValue> {
        let mut options = if options.is_null() || options.is_undefined() {
            WebOptions::default()
        } else {
            serde_wasm_bindgen::from_value(options)
                .map_err(|error| js_error(format!("invalid render options: {error}")))?
        };
        if options.width == Some(0) {
            options.width = None;
        }
        options.validate()?;

        if self.source_image.is_none() {
            return Err(js_error("no image is loaded; call setImage first"));
        }

        let args = self.options_to_render_args(&options);

        let key = options.glyph_cache_key();
        if let Some(position) = self.glyph_caches.iter().position(|cache| cache.key == key) {
            // Keep two common configurations (for example default and smooth)
            // hot without retaining an unbounded number of rasterized sets.
            let cache = self.glyph_caches.remove(position);
            self.glyph_caches.push(cache);
        } else {
            let store = font::glyph_store_from_font_bytes(
                &self.font_data,
                &args.blocks,
                &args.exclude_range,
                &args.exclude,
                &args.include_range,
                args.include.as_ref(),
                args.font_size,
                false,
            )
            .map_err(js_error)?;
            let fast_glyphs = draw::prepare_glyphs(&store, &args);
            if fast_glyphs.is_empty() && !args.braille {
                return Err(js_error(
                    "the selected characters produced no usable glyphs",
                ));
            }
            if self.glyph_caches.len() == GLYPH_CACHE_CAPACITY {
                self.glyph_caches.remove(0);
            }
            let bitmap_index = draw::prepare_glyph_bitmap_index(&store);
            self.glyph_caches.push(GlyphCache {
                key,
                store,
                fast_glyphs,
                bitmap_index,
            });
        }

        let canvas_key = options.canvas_cache_key();
        let canvas_cache_hit =
            self.canvas_cache.as_ref().map(|cache| cache.key) == Some(canvas_key);
        let prepare_started = draw::RenderInstant::now();
        if !canvas_cache_hit {
            let cache = self.glyph_caches.last().expect("glyph cache initialized");
            let image = self.source_image.as_ref().expect("source checked").clone();
            let (canvas, luma_canvas) = crate::prepare_canvas(&args, &cache.store, image);
            self.canvas_cache = Some(CanvasCache {
                key: canvas_key,
                canvas,
                luma_canvas,
            });
        }

        let cache = self.glyph_caches.last().expect("glyph cache initialized");
        let canvas_cache = self
            .canvas_cache
            .as_ref()
            .expect("canvas cache initialized");
        let prepare_canvas_time = prepare_started.elapsed();
        let result_key = options.result_cache_key();
        let result_cache_hit =
            self.result_cache.as_ref().map(|cache| cache.key) == Some(result_key);
        let mut base_result = if result_cache_hit {
            self.result_cache
                .as_ref()
                .expect("result cache key checked")
                .result
                .clone()
        } else {
            let mut base_args = args.clone();
            base_args.overlays.clear();
            let mut rendered = crate::render_from_canvas(
                &base_args,
                &canvas_cache.canvas,
                canvas_cache.luma_canvas.as_ref(),
                &cache.fast_glyphs,
                None,
            );
            rendered.profile.prepare_canvas = prepare_canvas_time;
            self.result_cache = Some(ResultCache {
                key: result_key,
                result: rendered.clone(),
            });
            rendered
        };
        if result_cache_hit {
            base_result.profile = draw::RenderProfile::default();
            base_result.profile.prepare_canvas = prepare_canvas_time;
        }
        let overlay_started = draw::RenderInstant::now();
        let result = draw::apply_overlays_to_result(
            &base_result,
            &args.overlays,
            args.render,
            false,
            args.grayscale_tolerance,
        );
        let overlay_time = overlay_started.elapsed();
        let preview_started = draw::RenderInstant::now();
        let preview = options
            .include_preview
            .then(|| draw::render_result_rgba_indexed(&result, &cache.store, &cache.bitmap_index))
            .flatten();
        let preview_time = preview_started.elapsed();
        let (preview_width, preview_height) = preview
            .as_ref()
            .map_or((0, 0), |(width, height, _)| (*width, *height));
        let rows = result.grid.len();
        let columns = result.grid.first().map_or(0, Vec::len);
        let cells = options.include_cells.then(|| {
            result
                .grid
                .iter()
                .map(|row| row.iter().map(WebCell::from).collect())
                .collect()
        });
        let timings = WebRenderTimings {
            canvas_cache_hit,
            result_cache_hit,
            prepare_canvas_ms: duration_ms(result.profile.prepare_canvas),
            glyph_match_ms: duration_ms(result.profile.glyph_match),
            smoothing_prepare_ms: duration_ms(result.profile.smoothing_prepare),
            smoothing_search_ms: duration_ms(result.profile.smoothing_search),
            shape_refine_ms: duration_ms(result.profile.shape_refine),
            final_score_ms: duration_ms(result.profile.final_score),
            encode_ms: duration_ms(result.profile.encode),
            overlay_ms: duration_ms(overlay_time),
            preview_ms: duration_ms(preview_time),
            contour_cache_hits: result.profile.contour_cache_hits,
            contour_cache_misses: result.profile.contour_cache_misses,
        };
        let output = WebRenderResult {
            content: result.content,
            columns,
            rows,
            cells,
            error_count: result.error_count,
            total_pixels: result.total_pixels,
            score: result.score,
            preview_width,
            preview_height,
            font_size: options.font_size,
            cell_width: cache.store.metrics.0,
            cell_height: cache.store.metrics.1,
            cell_advance: cache.store.float_metrics.0,
            line_height: cache.store.float_metrics.1,
            longest_line_bytes: result.longest_line_bytes,
            timings,
        };

        let value = serde_wasm_bindgen::to_value(&output)
            .map_err(|error| js_error(format!("could not serialize render result: {error}")))?;
        if let Some((_, _, pixels)) = preview {
            let bytes = js_sys::Uint8Array::from(pixels.as_slice());
            js_sys::Reflect::set(&value, &JsValue::from_str("previewRgba"), &bytes)
                .map_err(|_| js_error("could not attach browser preview"))?;
        }
        Ok(value)
    }
}

fn decode_image(image_data: &[u8]) -> Result<PhotonImage, JsValue> {
    let decoded = image::load_from_memory(image_data)
        .map_err(|error| js_error(format!("could not decode image: {error}")))?;
    let pixel_count = u64::from(decoded.width()) * u64::from(decoded.height());
    if pixel_count > MAX_INPUT_PIXELS {
        return Err(js_error(format!(
            "image has {pixel_count} pixels; the browser limit is {MAX_INPUT_PIXELS}"
        )));
    }
    let width = decoded.width();
    let height = decoded.height();
    Ok(PhotonImage::new(
        decoded.into_rgba8().into_raw(),
        width,
        height,
    ))
}

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

fn js_error(message: impl ToString) -> JsValue {
    JsValue::from_str(&message.to_string())
}
