use crate::args::{Args, ColourSpace, Param, Render, SamplingFilter};
use crate::{args_to_render_args, draw, font};
use clap::Parser;
use photon_rs::PhotonImage;
use serde::{Deserialize, Serialize};
use std::hash::{DefaultHasher, Hash, Hasher};
use wasm_bindgen::prelude::*;

const DEFAULT_WIDTH: u32 = 80;
const DEFAULT_FONT_SIZE: f32 = 16.0;
const MAX_INPUT_PIXELS: u64 = 64 * 1024 * 1024;
const GLYPH_CACHE_CAPACITY: usize = 2;

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
    prepareCanvasMs: number;
    glyphMatchMs: number;
    smoothingPrepareMs: number;
    smoothingSearchMs: number;
    shapeRefineMs: number;
    finalScoreMs: number;
    encodeMs: number;
    previewMs: number;
    contourCacheHits: number;
    contourCacheMisses: number;
}

export interface Img2Irc {
    setFont(fontData: Uint8Array): void;
    setImage(imageData: Uint8Array): void;
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
        }
    }
}

impl WebOptions {
    fn validate(&self) -> Result<(), JsValue> {
        if self.width == Some(0) || self.height == Some(0) {
            return Err(js_error("width and height must be greater than zero"));
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
struct WebRenderTimings {
    canvas_cache_hit: bool,
    prepare_canvas_ms: f64,
    glyph_match_ms: f64,
    smoothing_prepare_ms: f64,
    smoothing_search_ms: f64,
    shape_refine_ms: f64,
    final_score_ms: f64,
    encode_ms: f64,
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

/// A reusable img2irc renderer for browser applications.
///
/// Construct it with a monospace OpenType/TrueType font as a `Uint8Array`, then
/// load encoded PNG/JPEG/GIF/WebP bytes and render them with an options object.
#[wasm_bindgen(js_name = Img2Irc)]
pub struct WebRenderer {
    font_data: Vec<u8>,
    glyph_caches: Vec<GlyphCache>,
    canvas_cache: Option<CanvasCache>,
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
        Ok(())
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
        crate::contour_score::clear_cache();
    }
}

impl WebRenderer {
    fn render_loaded(&mut self, options: JsValue) -> Result<JsValue, JsValue> {
        let options = if options.is_null() || options.is_undefined() {
            WebOptions::default()
        } else {
            serde_wasm_bindgen::from_value(options)
                .map_err(|error| js_error(format!("invalid render options: {error}")))?
        };
        options.validate()?;

        if self.source_image.is_none() {
            return Err(js_error("no image is loaded; call setImage first"));
        }

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
        args.ocr = false;
        args.save = None;
        args.fft_debug_dir = None;

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
        let mut result = crate::render_from_canvas(
            &args,
            &canvas_cache.canvas,
            canvas_cache.luma_canvas.as_ref(),
            &cache.fast_glyphs,
            None,
        );
        result.profile.prepare_canvas = prepare_canvas_time;
        let preview_started = draw::RenderInstant::now();
        let preview = draw::render_result_rgba_indexed(&result, &cache.store, &cache.bitmap_index);
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
            prepare_canvas_ms: duration_ms(result.profile.prepare_canvas),
            glyph_match_ms: duration_ms(result.profile.glyph_match),
            smoothing_prepare_ms: duration_ms(result.profile.smoothing_prepare),
            smoothing_search_ms: duration_ms(result.profile.smoothing_search),
            shape_refine_ms: duration_ms(result.profile.shape_refine),
            final_score_ms: duration_ms(result.profile.final_score),
            encode_ms: duration_ms(result.profile.encode),
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
