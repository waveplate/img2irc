use crate::args::{ColorSpec, OcrFigletFontList, OcrModelTier, RenderArgs, TextOverlay};
use crate::font::GlyphStore;
use figlet_rs::FIGlet;
use image::RgbImage;
use photon_rs::PhotonImage;
use ppocr_rs::{
    DocOrientationClassifier, ModelHub, OcrLite, OcrOptions, PpOcrVersion, PpStructureModel,
};
use std::error::Error;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use unicode_width::UnicodeWidthChar;

#[derive(Clone, Debug)]
pub struct OcrDetection {
    pub text: String,
    pub confidence: f32,
    pub polygon: Vec<[f32; 2]>,
}

pub fn detect_text(
    source: &PhotonImage,
    args: &RenderArgs,
) -> Result<Vec<OcrDetection>, Box<dyn Error>> {
    let rgb = photon_to_rgb_image(source)?;
    let mut engine = OcrLite::new();
    let model_paths = resolve_model_paths(args)?;

    if args.ocr_textline_orientation {
        let cls = model_paths.cls.as_ref().ok_or_else(|| {
            "OCR text-line orientation is enabled but no classifier model is available; pass --ocr-cls-model or --ocr-textline-orientation=false"
        })?;
        maybe_suppress_ocr_output(args, || -> Result<(), Box<dyn Error>> {
            engine.init_models_with_dict(
                path_str(&model_paths.det)?,
                path_str(cls)?,
                path_str(&model_paths.rec)?,
                path_str(&model_paths.dict)?,
                args.ocr_threads.max(1),
            )?;
            Ok(())
        })?;
    } else {
        maybe_suppress_ocr_output(args, || -> Result<(), Box<dyn Error>> {
            engine.init_models_no_angle(
                path_str(&model_paths.det)?,
                path_str(&model_paths.rec)?,
                path_str(&model_paths.dict)?,
                args.ocr_threads.max(1),
            )?;
            Ok(())
        })?;
    }

    if args.ocr_doc_orientation {
        let doc_orientation = model_paths.doc_orientation.as_ref().ok_or_else(|| {
            "OCR document orientation is enabled but no document orientation model is available"
        })?;
        maybe_suppress_ocr_output(args, || -> Result<(), Box<dyn Error>> {
            engine.set_doc_orientation_model(DocOrientationClassifier::from_path(doc_orientation)?);
            Ok(())
        })?;
    }

    let options = OcrOptions {
        return_word_box: false,
        lang: Some("en".to_string()),
        use_doc_orientation: args.ocr_doc_orientation,
        use_doc_unwarping: args.ocr_doc_unwarping,
        use_seal: false,
        use_formula: false,
        use_chart: false,
    };

    let result = maybe_suppress_ocr_output(args, || -> Result<_, Box<dyn Error>> {
        Ok(engine.detect_with_options(
            &rgb,
            10,
            args.ocr_max_side_len,
            args.ocr_box_score_threshold,
            args.ocr_box_threshold,
            args.ocr_unclip_ratio,
            args.ocr_textline_orientation,
            args.ocr_most_angle,
            options,
        )?)
    })?;

    if result.page_angle != 0 {
        log::info!(
            "OCR corrected page orientation by {} degrees",
            result.page_angle
        );
    }

    let detections = result
        .text_blocks
        .into_iter()
        .map(|block| OcrDetection {
            text: block.text,
            confidence: block.text_score.min(block.box_score),
            polygon: block
                .box_points
                .into_iter()
                .map(|p| [p.x as f32, p.y as f32])
                .collect(),
        })
        .collect();

    Ok(detections)
}

pub fn valid_detections<'a>(
    detections: &'a [OcrDetection],
    args: &'a RenderArgs,
) -> impl Iterator<Item = &'a OcrDetection> + 'a {
    detections.iter().filter(move |det| {
        is_valid_text(
            &det.text,
            det.confidence,
            args.ocr_min_confidence,
            args.ocr_min_ascii_ratio,
        ) && det.polygon.len() >= 4
    })
}

fn renderable_detections<'a>(
    detections: &'a [OcrDetection],
    args: &'a RenderArgs,
    image_w: u32,
    image_h: u32,
) -> Vec<&'a OcrDetection> {
    let mut filtered: Vec<&OcrDetection> = valid_detections(detections, args).collect();
    if args.ocr_max_text_height_ratio <= 0.0 || filtered.len() < 2 {
        return filtered;
    }

    let mut heights = filtered
        .iter()
        .filter_map(|det| ocr_detection_height(det, image_w, image_h))
        .collect::<Vec<_>>();
    let Some(median_height) = median_f32(&mut heights) else {
        return filtered;
    };
    let max_height = median_height * args.ocr_max_text_height_ratio.max(0.0);
    filtered.retain(|det| {
        ocr_detection_height(det, image_w, image_h)
            .map(|height| height <= max_height)
            .unwrap_or(false)
    });
    filtered
}

fn ocr_detection_height(det: &OcrDetection, image_w: u32, image_h: u32) -> Option<f32> {
    polygon_bounds(&det.polygon, image_w, image_h).map(|(_, y0, _, y1)| (y1 - y0).max(1) as f32)
}

pub fn log_detections(detections: &[OcrDetection], args: &RenderArgs) {
    let mut kept = 0usize;
    for det in detections {
        let valid = is_valid_text(
            &det.text,
            det.confidence,
            args.ocr_min_confidence,
            args.ocr_min_ascii_ratio,
        ) && det.polygon.len() >= 4;
        if valid {
            kept += 1;
            log::info!(
                "OCR text: conf={:.3} text={:?} polygon={:?}",
                det.confidence,
                det.text,
                det.polygon
            );
        } else {
            log::info!(
                "OCR rejected: conf={:.3} text={:?} polygon_points={}",
                det.confidence,
                det.text,
                det.polygon.len()
            );
        }
    }
    log::info!(
        "OCR accepted {kept}/{} detected text regions",
        detections.len()
    );
}

pub fn choose_text_render_geometry(
    args: &mut RenderArgs,
    source: &PhotonImage,
    glyphs: &GlyphStore,
    detections: &[OcrDetection],
) {
    let image_w = source.get_width().max(1) as f32;
    let image_h = source.get_height().max(1) as f32;
    let (gw, gh) = glyphs.metrics;
    let gw = gw.max(1) as f32;
    let gh = gh.max(1) as f32;
    let image_aspect = image_w / image_h;

    let cell_aspect = gw / gh;
    let rows_for_cols = |cols: u32| -> u32 {
        ((cols as f32 * cell_aspect / image_aspect).round())
            .max(1.0)
            .ceil() as u32
    };
    let cols_for_rows = |rows: u32| -> u32 {
        ((rows as f32 * image_aspect / cell_aspect).round())
            .max(1.0)
            .ceil() as u32
    };

    let min_w = args.min_width.unwrap_or(1).max(1);

    let (cols, rows) = match (args.width, args.height) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, rows_for_cols(w)),
        (None, Some(h)) => (cols_for_rows(h), h),
        (None, None) => {
            // Start at the grid whose rendered pixel dimensions most closely
            // match the source image. OCR may grow that grid, but must never
            // make the entire image smaller merely because its text is large.
            let text_boxes = collect_text_box_metrics(source, args, detections);
            let (cols, native_cols, required_cols, max_w) = automatic_ocr_columns(
                source.get_width(),
                gw,
                min_w,
                args.max_width,
                &text_boxes,
            );
            let rows = rows_for_cols(cols);

            log::info!(
                "OCR automatic grid: {cols}x{rows} cells (native={native_cols} cols, text-fit={required_cols} cols, limit={max_w})"
            );
            (cols, rows)
        }
    };

    args.width = Some(cols);
    args.height = Some(rows);
    let output_aspect = (cols as f32 * gw) / (rows as f32 * gh);
    log::info!(
        "OCR-derived output size: {cols}x{rows} cells (input_aspect={:.4}, output_aspect={:.4})",
        image_aspect,
        output_aspect
    );
}

pub fn has_renderable_detections(
    detections: &[OcrDetection],
    args: &RenderArgs,
    image_w: u32,
    image_h: u32,
) -> bool {
    !renderable_detections(detections, args, image_w, image_h).is_empty()
}

pub fn choose_text_font_size(args: &mut RenderArgs, detections: &[OcrDetection], image_h: u32) {
    if args.font_size > 0.0 {
        return;
    }

    let mut heights: Vec<f32> = renderable_detections(detections, args, u32::MAX, image_h)
        .into_iter()
        .filter_map(|det| ocr_detection_height(det, u32::MAX, image_h))
        .collect();

    if heights.is_empty() {
        return;
    }

    heights.sort_by(|a, b| a.total_cmp(b));
    let median = heights[heights.len() / 2];
    args.font_size = median.round().clamp(6.0, 256.0);
    log::info!("OCR-derived font size: {}", args.font_size);
}

#[derive(Clone, Debug)]
struct TextBoxMetric {
    x0: f32,
    x1: f32,
    text_cols: usize,
}

impl TextBoxMetric {
    fn width(&self) -> f32 {
        (self.x1 - self.x0).max(1.0)
    }
}

fn automatic_ocr_columns(
    image_w: u32,
    glyph_w: f32,
    min_width: u32,
    max_width: Option<u32>,
    text_boxes: &[TextBoxMetric],
) -> (u32, u32, u32, u32) {
    let image_wf = image_w.max(1) as f32;
    let native_cols = (image_wf / glyph_w.max(1.0)).round().max(1.0) as u32;
    let required_cols = text_boxes
        .iter()
        .map(|text_box| {
            ((text_box.text_cols as f32 * image_wf) / text_box.width())
                .ceil()
                .max(1.0) as u32
        })
        .max()
        .unwrap_or(1);
    let desired_cols = native_cols.max(required_cols).max(min_width.max(1));

    // A one-pixel-wide false-positive box should not request an unbounded
    // allocation. An explicit maximum takes precedence; otherwise allow up to
    // one cell per source pixel, capped at a generous 4096 columns.
    let automatic_safety_max = native_cols
        .max(image_w.min(4096))
        .max(min_width.max(1));
    let max_width = max_width.unwrap_or(automatic_safety_max).max(1);
    (
        desired_cols.min(max_width),
        native_cols,
        required_cols,
        max_width,
    )
}

fn collect_text_box_metrics(
    source: &PhotonImage,
    args: &RenderArgs,
    detections: &[OcrDetection],
) -> Vec<TextBoxMetric> {
    let image_w = source.get_width();
    let image_h = source.get_height();
    renderable_detections(detections, args, image_w, image_h)
        .into_iter()
        .filter_map(|det| {
            let (x0, _, x1, _) = polygon_bounds(&det.polygon, image_w, image_h)?;
            let text_cols = det.text.chars().count();
            if text_cols == 0 {
                return None;
            }
            Some(TextBoxMetric {
                x0: x0 as f32,
                x1: x1 as f32,
                text_cols,
            })
        })
        .collect()
}

fn median_f32(values: &mut Vec<f32>) -> Option<f32> {
    values.retain(|value| value.is_finite());
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    Some(values[values.len() / 2])
}

const PREFERRED_OCR_FIGLET_FONTS: &[&str] =
    &["phm-minecraft", "phm-largetype", "phm-lcdmatrix"];

#[derive(Debug)]
struct OcrFigletFont {
    name: String,
    configured_height: u32,
    priority: usize,
    font: FIGlet,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TextArt {
    source: String,
    configured_height: u32,
    priority: usize,
    lines: Vec<Vec<char>>,
}

impl TextArt {
    fn width(&self) -> usize {
        self.lines
            .iter()
            .map(|line| {
                line.iter()
                    .map(|ch| UnicodeWidthChar::width(*ch).unwrap_or(1))
                    .sum()
            })
            .max()
            .unwrap_or(0)
    }

    fn height(&self) -> usize {
        self.lines.len()
    }

    fn fill_key(&self) -> (usize, usize, usize, std::cmp::Reverse<usize>) {
        (
            self.width() * self.height(),
            self.height(),
            self.width(),
            std::cmp::Reverse(self.priority),
        )
    }
}

fn figlet_font_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for variable in ["IMG2IRC_FIGLET_FONT_DIR", "FIGLET_FONTDIR"] {
        if let Some(paths) = std::env::var_os(variable) {
            dirs.extend(std::env::split_paths(&paths));
        }
    }

    if let Ok(output) = Command::new("figlet").arg("-I2").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                dirs.push(PathBuf::from(path));
            }
        }
    }

    dirs.extend([
        PathBuf::from("/usr/share/figlet/fonts"),
        PathBuf::from("/usr/local/share/figlet/fonts"),
        PathBuf::from("/opt/homebrew/share/figlet/fonts"),
        PathBuf::from("FIGfonts/fonts"),
        PathBuf::from("figfonts/fonts"),
    ]);
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local/share/figlet/fonts"));
    }

    let mut unique = Vec::new();
    for dir in dirs {
        if !unique.contains(&dir) {
            unique.push(dir);
        }
    }
    unique
}

fn load_ocr_figlet_fonts(font_lists: &[OcrFigletFontList]) -> Vec<OcrFigletFont> {
    let dirs = figlet_font_dirs();
    let mut fonts = Vec::new();
    let mut loaded = std::collections::HashSet::new();

    for list in font_lists {
        for (priority, name) in list.fonts.iter().enumerate() {
            if name.eq_ignore_ascii_case("plain")
                || !loaded.insert((list.height, name.to_lowercase()))
            {
                continue;
            }
            let Some(path) = dirs
                .iter()
                .map(|dir| dir.join(format!("{name}.flf")))
                .find(|path| path.is_file())
            else {
                log::debug!("OCR FIGfont {name:?} was not found for {} lines", list.height);
                continue;
            };
            let Some(path_str) = path.to_str() else {
                continue;
            };

            match FIGlet::from_file(path_str) {
                Ok(font) if font.header_line.height as u32 == list.height => {
                    fonts.push(OcrFigletFont {
                        name: name.clone(),
                        configured_height: list.height,
                        priority,
                        font,
                    });
                }
                Ok(font) => log::warn!(
                    "Ignoring OCR FIGfont {} in the {}-line list because it declares {} lines",
                    name,
                    list.height,
                    font.header_line.height
                ),
                Err(error) => log::debug!(
                    "Ignoring OCR FIGfont {} at {}: {}",
                    name,
                    path.display(),
                    error
                ),
            }
        }
    }

    if !fonts.is_empty() {
        log::info!(
            "Loaded {} compact OCR FIGfonts: {}",
            fonts.len(),
            fonts
                .iter()
                .map(|font| format!("{}:{}", font.configured_height, font.name))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    fonts
}

#[cfg(test)]
fn normalize_text_art(source: impl Into<String>, rows: Vec<String>) -> Option<TextArt> {
    normalize_text_art_with_metadata(source, 1, usize::MAX, rows)
}

fn normalize_text_art_with_metadata(
    source: impl Into<String>,
    configured_height: u32,
    priority: usize,
    rows: Vec<String>,
) -> Option<TextArt> {
    let rows = rows
        .into_iter()
        .map(|row| row.chars().collect::<Vec<_>>())
        .collect::<Vec<_>>();
    if rows.is_empty()
        || rows
            .iter()
            .flatten()
            .any(|ch| {
                ch.is_control()
                    || *ch == '\u{1b}'
                    || matches!(UnicodeWidthChar::width(*ch), Some(width) if width != 1)
            })
    {
        return None;
    }

    let mut min_x = usize::MAX;
    let mut max_x = 0usize;
    let mut min_y = usize::MAX;
    let mut max_y = 0usize;
    for (y, row) in rows.iter().enumerate() {
        for (x, ch) in row.iter().enumerate() {
            if !ch.is_whitespace() {
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
                .collect::<Vec<_>>()
        })
        .collect();
    Some(TextArt {
        source: source.into(),
        configured_height,
        priority,
        lines,
    })
}

fn render_figlet_text(font: &OcrFigletFont, text: &str) -> Option<TextArt> {
    let figure = font.font.convert(text)?;
    if figure.characters.len() != text.chars().count() {
        return None;
    }
    normalize_text_art_with_metadata(
        font.name.clone(),
        font.configured_height,
        font.priority,
        figure.as_str().lines().map(str::to_string).collect(),
    )
}

fn select_best_text_art(
    candidates: impl IntoIterator<Item = TextArt>,
    max_width: usize,
    max_height: usize,
) -> Option<TextArt> {
    let fitting = candidates
        .into_iter()
        .filter(|art| {
            art.width() > 0
                && art.height() > 0
                && art.width() <= max_width
                && art.height() <= max_height
        })
        .collect::<Vec<_>>();
    let has_preferred = fitting.iter().any(|art| {
        PREFERRED_OCR_FIGLET_FONTS
            .iter()
            .any(|font| *font == art.source)
    });

    fitting
        .into_iter()
        .filter(|art| {
            !has_preferred
                || PREFERRED_OCR_FIGLET_FONTS
                    .iter()
                    .any(|font| *font == art.source)
        })
        .max_by_key(TextArt::fill_key)
}

#[cfg(test)]
fn choose_text_art(text: &str, max_width: usize, max_height: usize) -> Option<TextArt> {
    let font_lists = crate::args::default_ocr_figlet_font_lists();
    let fonts = load_ocr_figlet_fonts(&font_lists);
    choose_text_art_from_fonts(text, max_width, max_height, &font_lists, &fonts)
}

fn choose_text_art_from_fonts(
    text: &str,
    max_width: usize,
    max_height: usize,
    font_lists: &[OcrFigletFontList],
    fonts: &[OcrFigletFont],
) -> Option<TextArt> {
    let text = text.trim();
    if text.is_empty() || max_width == 0 || max_height == 0 {
        return None;
    }

    let mut heights = font_lists
        .iter()
        .map(|list| list.height)
        .filter(|height| *height > 0 && *height as usize <= max_height)
        .collect::<Vec<_>>();
    heights.sort_unstable_by(|a, b| b.cmp(a));
    heights.dedup();

    for height in heights {
        let mut candidates = fonts
            .iter()
            .filter(|font| font.configured_height == height)
            .filter_map(|font| render_figlet_text(font, text))
            .collect::<Vec<_>>();

        if height == 1 && text.chars().count() <= max_width {
            if let Some(priority) = font_lists
                .iter()
                .filter(|list| list.height == 1)
                .flat_map(|list| list.fonts.iter())
                .position(|font| font.eq_ignore_ascii_case("plain"))
            {
                if let Some(plain) = normalize_text_art_with_metadata(
                    "plain",
                    1,
                    priority,
                    vec![text.to_string()],
                ) {
                    candidates.push(plain);
                }
            }
        }

        if let Some(selected) = select_best_text_art(candidates, max_width, max_height) {
            return Some(selected);
        }
    }

    None
}

fn plain_text_art(text: &str, max_width: usize) -> Option<TextArt> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > max_width {
        return None;
    }
    normalize_text_art_with_metadata("plain", 1, 0, vec![text.to_string()])
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ScaledTextBox {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

impl ScaledTextBox {
    fn height(self) -> f64 {
        (self.bottom - self.top).max(0.0)
    }

    fn available_width(self) -> usize {
        ((self.right - self.left + 1e-9).floor() as usize).max(1)
    }

    fn available_height(self) -> usize {
        ((self.bottom - self.top + 1e-9).floor() as usize).max(1)
    }

    fn place(self, art_height: usize) -> (i32, i32) {
        // The OCR box's left edge is the layout anchor. Centering each FIGlet
        // rendering independently makes text that was aligned in the source
        // drift according to the selected font's width.
        let x = self.left.round();
        let y = (self.top + (self.bottom - self.top - art_height as f64) * 0.5).round();
        (x as i32, y as i32)
    }
}

fn figlet_allowed(
    args: &RenderArgs,
    text_box: ScaledTextBox,
    detection_height: f32,
    median_detection_height: f32,
) -> bool {
    if !args.ocr_figlet || text_box.height() < args.ocr_figlet_min_height.max(0.0) as f64 {
        return false;
    }

    let min_ratio = args.ocr_figlet_min_height_ratio.max(0.0);
    min_ratio <= 0.0
        || median_detection_height <= 0.0
        || detection_height >= median_detection_height * min_ratio
}

fn boxes_overlap_vertically(a: ScaledTextBox, b: ScaledTextBox) -> bool {
    a.bottom.min(b.bottom) > a.top.max(b.top)
}

fn available_layout_width(
    index: usize,
    boxes: &[ScaledTextBox],
    grid_width: i32,
) -> usize {
    let text_box = boxes[index];
    let right_limit = boxes
        .iter()
        .enumerate()
        .filter(|(other_index, other)| {
            *other_index != index
                && other.left >= text_box.right
                && boxes_overlap_vertically(text_box, **other)
        })
        .map(|(_, other)| other.left)
        .fold(grid_width.max(1) as f64, f64::min);

    ((right_limit - text_box.left + 1e-9).floor() as usize)
        .max(text_box.available_width())
        .max(1)
}

fn removed_text_left_edge(
    source: &[[u8; 3]],
    background: &[[u8; 3]],
    image_width: u32,
    bounds: (u32, u32, u32, u32),
) -> Option<u32> {
    let (x0, y0, x1, y1) = bounds;
    for x in x0..x1 {
        for y in y0..y1 {
            let index = y as usize * image_width as usize + x as usize;
            let (Some(source), Some(background)) =
                (source.get(index), background.get(index))
            else {
                continue;
            };
            let distance = source
                .iter()
                .zip(background)
                .map(|(a, b)| (*a as i32 - *b as i32).pow(2) as u32)
                .sum::<u32>();
            if distance >= 8 * 8 {
                return Some(x);
            }
        }
    }
    None
}

fn scale_image_box_to_grid(
    (x0, y0, x1, y1): (u32, u32, u32, u32),
    image_w: u32,
    image_h: u32,
    grid_w: i32,
    grid_h: i32,
) -> ScaledTextBox {
    let image_w = image_w.max(1) as f64;
    let image_h = image_h.max(1) as f64;
    let grid_wf = grid_w.max(1) as f64;
    let grid_hf = grid_h.max(1) as f64;

    ScaledTextBox {
        left: x0 as f64 * grid_wf / image_w,
        top: y0 as f64 * grid_hf / image_h,
        right: x1 as f64 * grid_wf / image_w,
        bottom: y1 as f64 * grid_hf / image_h,
    }
}

pub fn detections_to_overlays(
    source: &PhotonImage,
    background_source: &PhotonImage,
    args: &RenderArgs,
    glyphs: &GlyphStore,
    detections: &[OcrDetection],
) -> Vec<TextOverlay> {
    let (gw, gh) = glyphs.metrics;
    let fallback_cols = (source.get_width() as f32 / gw.max(1) as f32)
        .ceil()
        .max(1.0) as i32;
    let fallback_rows = (source.get_height() as f32 / gh.max(1) as f32)
        .ceil()
        .max(1.0) as i32;
    let grid_w = args.width.unwrap_or(fallback_cols as u32).max(1) as i32;
    let grid_h = args.height.unwrap_or(fallback_rows as u32).max(1) as i32;
    let rgb = photon_rgb(source);
    let background_rgb = photon_rgb(background_source);
    let global_bg = rgb.first().copied().unwrap_or([0, 0, 0]);
    let figlet_fonts = if args.ocr_figlet {
        load_ocr_figlet_fonts(&args.ocr_figlet_fonts)
    } else {
        Vec::new()
    };

    let mut overlays = Vec::new();
    let mut occupied_cells = std::collections::HashSet::new();
    let mut renderable = renderable_detections(
        detections,
        args,
        source.get_width(),
        source.get_height(),
    )
    .into_iter()
    .filter_map(|det| {
        let bounds = polygon_bounds(&det.polygon, source.get_width(), source.get_height())?;
        let mut text_box = scale_image_box_to_grid(
            bounds,
            source.get_width(),
            source.get_height(),
            grid_w,
            grid_h,
        );
        if let Some(left) = removed_text_left_edge(
            &rgb,
            &background_rgb,
            source.get_width(),
            bounds,
        ) {
            text_box.left = left as f64 * grid_w.max(1) as f64
                / source.get_width().max(1) as f64;
        }
        Some((det, bounds, text_box))
    })
    .collect::<Vec<_>>();
    renderable.sort_by(|a, b| b.0.confidence.total_cmp(&a.0.confidence));
    let mut detection_heights = renderable
        .iter()
        .map(|(_, bounds, _)| bounds.3.saturating_sub(bounds.1) as f32)
        .filter(|height| *height > 0.0)
        .collect::<Vec<_>>();
    // A relative threshold is useful only when the page has enough lines to
    // establish a typical text height. For one-off labels, the absolute grid
    // height remains the deciding gate.
    let median_detection_height = if detection_heights.len() >= 3 {
        median_f32(&mut detection_heights).unwrap_or(0.0)
    } else {
        0.0
    };
    let layout_boxes = renderable
        .iter()
        .map(|(_, _, text_box)| *text_box)
        .collect::<Vec<_>>();

    for (index, (det, bounds, text_box)) in renderable.into_iter().enumerate() {
        let target_w = available_layout_width(index, &layout_boxes, grid_w);
        let target_h = text_box.available_height();
        let detection_height = bounds.3.saturating_sub(bounds.1) as f32;
        let allow_figlet = figlet_allowed(
            args,
            text_box,
            detection_height,
            median_detection_height,
        );
        let art = if allow_figlet {
            choose_text_art_from_fonts(
                &det.text,
                target_w,
                target_h,
                &args.ocr_figlet_fonts,
                &figlet_fonts,
            )
        } else {
            plain_text_art(&det.text, target_w)
        };
        let Some(art) = art else {
            log::debug!(
                "No complete OCR text rendering fits {:?} in {}x{} cells",
                det.text,
                target_w,
                target_h
            );
            continue;
        };
        let art_w = art.width() as i32;
        let art_h = art.height() as i32;
        let (art_x, art_y) = text_box.place(art_h as usize);
        let art_x = art_x.clamp(0, grid_w.saturating_sub(art_w).max(0));
        let art_y = art_y.clamp(0, grid_h.saturating_sub(art_h).max(0));
        let art_cells = art
            .lines
            .iter()
            .enumerate()
            .flat_map(|(row, line)| {
                line.iter()
                    .enumerate()
                    .map(move |(col, _)| (art_x + col as i32, art_y + row as i32))
            })
            .collect::<Vec<_>>();
        if art_cells.iter().any(|cell| occupied_cells.contains(cell)) {
            log::debug!("Skipping overlapping OCR rendering for {:?}", det.text);
            continue;
        }
        occupied_cells.extend(art_cells);
        let fg = estimate_text_color(
            &rgb,
            source.get_width(),
            source.get_height(),
            bounds,
            global_bg,
        );
        let fallback_bg = dominant_rgb_in_rect(
            &background_rgb,
            background_source.get_width(),
            background_source.get_height(),
            bounds,
        )
        .unwrap_or(global_bg);
        if art.source != "plain" {
            log::debug!(
                "OCR FIGfont {} ({}-line list) selected for {:?}: {}x{} in {}x{} cells",
                art.source,
                art.configured_height,
                det.text,
                art_w,
                art_h,
                target_w,
                target_h
            );
        }
        push_text_art_overlay_runs(
            &mut overlays,
            &art,
            &background_rgb,
            background_source.get_width(),
            background_source.get_height(),
            art_x,
            art_y,
            grid_w,
            grid_h,
            fg,
            fallback_bg,
        );
    }

    overlays.sort_by_key(|ov| (ov.y, ov.x));
    overlays
}

pub fn remove_detected_text(
    source: &PhotonImage,
    args: &RenderArgs,
    detections: &[OcrDetection],
) -> PhotonImage {
    let width = source.get_width();
    let height = source.get_height();
    if width == 0 || height == 0 {
        return source.clone();
    }

    let mut raw = source.get_raw_pixels();
    let mut removed = 0usize;
    for det in renderable_detections(detections, args, width, height) {
        let Some(bounds) = polygon_bounds(&det.polygon, width, height) else {
            continue;
        };
        if fill_text_region_from_background_mask(&mut raw, width, height, bounds) {
            removed += 1;
        }
    }
    if removed > 0 {
        log::info!("Removed {removed} OCR text regions from source bitmap");
    }
    PhotonImage::new(raw, width, height)
}

struct ModelPaths {
    det: std::path::PathBuf,
    cls: Option<std::path::PathBuf>,
    rec: std::path::PathBuf,
    dict: std::path::PathBuf,
    doc_orientation: Option<std::path::PathBuf>,
}

fn resolve_model_paths(args: &RenderArgs) -> Result<ModelPaths, Box<dyn Error>> {
    let hub = if let Some(cache) = &args.ocr_model_cache {
        ModelHub::new(cache)
    } else {
        ModelHub::with_default_cache()?
    };

    let (det, rec, dict) = match (&args.ocr_det_model, &args.ocr_rec_model, &args.ocr_dict) {
        (Some(det), Some(rec), Some(dict)) => (det.into(), rec.into(), dict.into()),
        (None, None, None) => {
            log::info!("Resolving PP-OCR {:?} model files", args.ocr_model_tier);
            let paths = maybe_suppress_ocr_output(args, || {
                hub.ensure(match args.ocr_model_tier {
                    OcrModelTier::Tiny => PpOcrVersion::V6Tiny,
                    OcrModelTier::Small => PpOcrVersion::V6Small,
                    OcrModelTier::Medium => PpOcrVersion::V6Medium,
                })
            })?;
            (paths.det_onnx, paths.rec_onnx, paths.dict_txt)
        }
        _ => {
            return Err("OCR model paths must be provided together: --ocr-det-model, --ocr-rec-model, and --ocr-dict".into());
        }
    };

    let cls = if args.ocr_textline_orientation {
        Some(match &args.ocr_cls_model {
            Some(path) => path.into(),
            None => ensure_textline_orientation_model(&hub)?,
        })
    } else {
        args.ocr_cls_model.as_ref().map(Into::into)
    };

    let doc_orientation = if args.ocr_doc_orientation {
        Some(match &args.ocr_doc_orientation_model {
            Some(path) => path.into(),
            None => maybe_suppress_ocr_output(args, || {
                hub.ensure_single(PpStructureModel::DocOrientation)
            })?
            .onnx,
        })
    } else {
        args.ocr_doc_orientation_model.as_ref().map(Into::into)
    };

    log::info!(
        "Using OCR models det={}{} rec={} dict={}{}",
        det.display(),
        cls.as_ref()
            .map(|p| format!(" cls={}", p.display()))
            .unwrap_or_default(),
        rec.display(),
        dict.display(),
        doc_orientation
            .as_ref()
            .map(|p| format!(" doc_orientation={}", p.display()))
            .unwrap_or_default()
    );

    Ok(ModelPaths {
        det,
        cls,
        rec,
        dict,
        doc_orientation,
    })
}

fn ensure_textline_orientation_model(hub: &ModelHub) -> Result<std::path::PathBuf, Box<dyn Error>> {
    let path = hub
        .cache_dir()
        .join("textline_orientation")
        .join("ppocrv5_cls.onnx");
    if path.exists() && path.metadata()?.len() > 0 {
        return Ok(path);
    }

    let url =
        "https://github.com/jingsongliujing/OnnxOCR/raw/main/onnxocr/models/ppocrv5/cls/cls.onnx";
    log::info!(
        "Downloading OCR text-line orientation model to {}",
        path.display()
    );
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp_path = path.with_extension("onnx.tmp");
    {
        let mut file = std::fs::File::create(&tmp_path)?;
        let response = ureq::get(url).call()?;
        io::copy(&mut response.into_reader(), &mut file)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp_path, &path)?;
    Ok(path)
}

fn maybe_suppress_ocr_output<F, T>(args: &RenderArgs, f: F) -> T
where
    F: FnOnce() -> T,
{
    // Redirecting these process-wide file descriptors while Ratatui is active
    // also redirects parts of its terminal frames, leaving intermittent black
    // or stale regions. The OCR library is normally quiet after model setup,
    // so keep output attached in preview/TUI mode.
    if args.as_preview {
        f()
    } else {
        suppress_stdout(f)
    }
}

#[cfg(unix)]
fn suppress_stdout<F, T>(f: F) -> T
where
    F: FnOnce() -> T,
{
    use std::fs::OpenOptions;
    use std::os::fd::AsRawFd;

    let Ok(dev_null) = OpenOptions::new().write(true).open("/dev/null") else {
        return f();
    };

    unsafe {
        let saved_stdout = libc::dup(libc::STDOUT_FILENO);
        let saved_stderr = libc::dup(libc::STDERR_FILENO);
        if saved_stdout < 0 || saved_stderr < 0 {
            if saved_stdout >= 0 {
                let _ = libc::close(saved_stdout);
            }
            if saved_stderr >= 0 {
                let _ = libc::close(saved_stderr);
            }
            return f();
        }
        let _ = libc::dup2(dev_null.as_raw_fd(), libc::STDOUT_FILENO);
        let _ = libc::dup2(dev_null.as_raw_fd(), libc::STDERR_FILENO);
        let result = f();
        let _ = libc::dup2(saved_stdout, libc::STDOUT_FILENO);
        let _ = libc::dup2(saved_stderr, libc::STDERR_FILENO);
        let _ = libc::close(saved_stdout);
        let _ = libc::close(saved_stderr);
        result
    }
}

#[cfg(not(unix))]
fn suppress_stdout<F, T>(f: F) -> T
where
    F: FnOnce() -> T,
{
    f()
}

fn path_str(path: &Path) -> Result<&str, Box<dyn Error>> {
    path.to_str()
        .ok_or_else(|| format!("path is not valid UTF-8: {}", path.display()).into())
}

fn photon_to_rgb_image(image: &PhotonImage) -> Result<RgbImage, Box<dyn Error>> {
    let width = image.get_width();
    let height = image.get_height();
    let raw = image.get_raw_pixels();
    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    for px in raw.chunks_exact(4) {
        rgb.extend_from_slice(&px[0..3]);
    }
    RgbImage::from_raw(width, height, rgb).ok_or_else(|| "failed to build RGB OCR image".into())
}

fn photon_rgb(image: &PhotonImage) -> Vec<[u8; 3]> {
    image
        .get_raw_pixels()
        .chunks_exact(4)
        .map(|px| [px[0], px[1], px[2]])
        .collect()
}

fn push_overlay_runs(
    overlays: &mut Vec<TextOverlay>,
    chars: &[char],
    backgrounds: &[[u8; 3]],
    x: i32,
    y: i32,
    grid_w: i32,
    fg: [u8; 3],
) {
    let mut run_text = String::new();
    let mut run_x = 0i32;
    let mut run_bg: Option<[u8; 3]> = None;

    let flush_run = |overlays: &mut Vec<TextOverlay>,
                     run_text: &mut String,
                     run_x: i32,
                     run_bg: Option<[u8; 3]>| {
        if run_text.is_empty() {
            return;
        }
        let w = run_text.chars().count() as i32;
        overlays.push(TextOverlay {
            text: std::mem::take(run_text),
            x: run_x,
            y,
            w,
            h: 1,
            fg: Some(ColorSpec::Rgb(fg)),
            bg: run_bg.map(ColorSpec::Rgb),
            wrap: false,
            auto_grow: false,
            bold: false,
            italic: false,
            underline: false,
        });
    };

    for (idx, (&ch, &bg)) in chars.iter().zip(backgrounds.iter()).enumerate() {
        if ch.is_control() {
            continue;
        }
        let cell_x = x + idx as i32;
        if cell_x < 0 || cell_x >= grid_w {
            flush_run(overlays, &mut run_text, run_x, run_bg);
            run_bg = None;
            continue;
        }
        if run_text.is_empty() {
            run_x = cell_x;
            run_bg = Some(bg);
        } else if run_bg != Some(bg) || run_x + run_text.chars().count() as i32 != cell_x {
            flush_run(overlays, &mut run_text, run_x, run_bg);
            run_x = cell_x;
            run_bg = Some(bg);
        }
        run_text.push(ch);
    }

    flush_run(overlays, &mut run_text, run_x, run_bg);
}

#[allow(clippy::too_many_arguments)]
fn push_text_art_overlay_runs(
    overlays: &mut Vec<TextOverlay>,
    art: &TextArt,
    background_rgb: &[[u8; 3]],
    background_width: u32,
    background_height: u32,
    x: i32,
    y: i32,
    grid_w: i32,
    grid_h: i32,
    fg: [u8; 3],
    fallback_bg: [u8; 3],
) {
    for (row_idx, chars) in art.lines.iter().enumerate() {
        let cell_y = y + row_idx as i32;
        if cell_y < 0 || cell_y >= grid_h {
            continue;
        }

        let backgrounds = chars
            .iter()
            .enumerate()
            .map(|(idx, _)| {
                sample_cell_background(
                    background_rgb,
                    background_width,
                    background_height,
                    grid_w,
                    grid_h,
                    x + idx as i32,
                    cell_y,
                    fallback_bg,
                )
            })
            .collect::<Vec<_>>();
        push_overlay_runs(overlays, chars, &backgrounds, x, cell_y, grid_w, fg);
    }
}

fn sample_cell_background(
    rgb: &[[u8; 3]],
    image_w: u32,
    image_h: u32,
    grid_w: i32,
    grid_h: i32,
    cell_x: i32,
    cell_y: i32,
    fallback: [u8; 3],
) -> [u8; 3] {
    if image_w == 0
        || image_h == 0
        || grid_w <= 0
        || grid_h <= 0
        || cell_x < 0
        || cell_y < 0
        || cell_x >= grid_w
        || cell_y >= grid_h
    {
        return fallback;
    }

    let x0 = (cell_x as f64 * image_w as f64 / grid_w as f64).floor() as u32;
    let x1 = ((cell_x + 1) as f64 * image_w as f64 / grid_w as f64).ceil() as u32;
    let y0 = (cell_y as f64 * image_h as f64 / grid_h as f64).floor() as u32;
    let y1 = ((cell_y + 1) as f64 * image_h as f64 / grid_h as f64).ceil() as u32;
    dominant_rgb_in_rect(rgb, image_w, image_h, (x0, y0, x1, y1)).unwrap_or(fallback)
}

fn dominant_rgb_in_rect(
    rgb: &[[u8; 3]],
    image_w: u32,
    image_h: u32,
    (x0, y0, x1, y1): (u32, u32, u32, u32),
) -> Option<[u8; 3]> {
    if x1 <= x0 || y1 <= y0 || image_w == 0 || image_h == 0 {
        return None;
    }

    let mut bins = std::collections::HashMap::<u16, (u32, [u64; 3])>::new();
    for y in y0..y1.min(image_h) {
        let row = y as usize * image_w as usize;
        for x in x0..x1.min(image_w) {
            let px = *rgb.get(row + x as usize)?;
            let key = ((px[0] as u16 >> 4) << 8)
                | ((px[1] as u16 >> 4) << 4)
                | (px[2] as u16 >> 4);
            let entry = bins.entry(key).or_insert((0, [0; 3]));
            entry.0 += 1;
            entry.1[0] += px[0] as u64;
            entry.1[1] += px[1] as u64;
            entry.1[2] += px[2] as u64;
        }
    }

    let (_, (count, sum)) = bins.into_iter().max_by_key(|(_, (count, _))| *count)?;
    Some([
        (sum[0] / count as u64) as u8,
        (sum[1] / count as u64) as u8,
        (sum[2] / count as u64) as u8,
    ])
}

fn fill_text_region_from_background_mask(
    raw: &mut [u8],
    image_w: u32,
    image_h: u32,
    bounds: (u32, u32, u32, u32),
) -> bool {
    let (x0, y0, x1, y1) = expand_text_bounds(bounds, image_w, image_h);
    if x1 <= x0 || y1 <= y0 {
        return false;
    }

    let Some(background) =
        dominant_background_color(raw, image_w, image_h, bounds, (x0, y0, x1, y1))
    else {
        return false;
    };
    let threshold = foreground_threshold(raw, image_w, image_h, background, bounds);
    let bw = (x1 - x0) as usize;
    let bh = (y1 - y0) as usize;
    let mut mask = vec![false; bw * bh];

    for y in y0..y1 {
        for x in x0..x1 {
            let Some(px) = rgba_at(raw, image_w, x, y) else {
                continue;
            };
            if color_distance_sq(px, background) >= threshold {
                let idx = (y - y0) as usize * bw + (x - x0) as usize;
                mask[idx] = true;
            }
        }
    }

    let mask = dilate_mask(&mask, bw, bh);
    let mut changed = false;
    for y in y0..y1 {
        for x in x0..x1 {
            let idx = (y - y0) as usize * bw + (x - x0) as usize;
            if mask[idx] {
                set_rgba(raw, image_w, x, y, background);
                changed = true;
            }
        }
    }

    changed
}

fn expand_text_bounds(
    bounds: (u32, u32, u32, u32),
    image_w: u32,
    image_h: u32,
) -> (u32, u32, u32, u32) {
    let (x0, y0, x1, y1) = bounds;
    let box_h = y1.saturating_sub(y0);
    let pad = ((box_h as f32 * 0.08).ceil() as u32).clamp(1, 3);
    (
        x0.saturating_sub(pad),
        y0.saturating_sub(pad),
        x1.saturating_add(pad).min(image_w),
        y1.saturating_add(pad).min(image_h),
    )
}

fn dominant_background_color(
    raw: &[u8],
    image_w: u32,
    image_h: u32,
    bounds: (u32, u32, u32, u32),
    erase_bounds: (u32, u32, u32, u32),
) -> Option<[u8; 4]> {
    let (x0, y0, x1, y1) = bounds;
    let (ex0, ey0, ex1, ey1) = erase_bounds;
    let ring = (y1.saturating_sub(y0) / 4).clamp(3, 8);
    let sx0 = ex0.saturating_sub(ring);
    let sy0 = ey0.saturating_sub(ring);
    let sx1 = ex1.saturating_add(ring).min(image_w);
    let sy1 = ey1.saturating_add(ring).min(image_h);

    let mut bins = std::collections::HashMap::<u16, (u32, [u64; 4])>::new();
    for y in sy0..sy1 {
        for x in sx0..sx1 {
            let in_erased_rect = x >= ex0 && x < ex1 && y >= ey0 && y < ey1;
            let in_original_rect = x >= x0 && x < x1 && y >= y0 && y < y1;
            if in_erased_rect && !in_original_rect {
                continue;
            }
            let Some(px) = rgba_at(raw, image_w, x, y) else {
                continue;
            };
            let key = quantized_rgb_key(px);
            let entry = bins.entry(key).or_insert((0, [0; 4]));
            entry.0 += 1;
            entry.1[0] += px[0] as u64;
            entry.1[1] += px[1] as u64;
            entry.1[2] += px[2] as u64;
            entry.1[3] += px[3] as u64;
        }
    }

    let (_, (count, sum)) = bins.into_iter().max_by_key(|(_, (count, _))| *count)?;
    Some([
        (sum[0] / count as u64) as u8,
        (sum[1] / count as u64) as u8,
        (sum[2] / count as u64) as u8,
        (sum[3] / count as u64) as u8,
    ])
}

fn foreground_threshold(
    raw: &[u8],
    image_w: u32,
    image_h: u32,
    background: [u8; 4],
    bounds: (u32, u32, u32, u32),
) -> u32 {
    let (x0, y0, x1, y1) = bounds;
    let ring = (y1.saturating_sub(y0) / 4).clamp(3, 8);
    let sx0 = x0.saturating_sub(ring);
    let sy0 = y0.saturating_sub(ring);
    let sx1 = x1.saturating_add(ring).min(image_w);
    let sy1 = y1.saturating_add(ring).min(image_h);
    let mut distances = Vec::new();

    for y in sy0..sy1 {
        for x in sx0..sx1 {
            if x >= x0 && x < x1 && y >= y0 && y < y1 {
                continue;
            }
            if let Some(px) = rgba_at(raw, image_w, x, y) {
                distances.push(color_distance_sq(px, background));
            }
        }
    }

    if distances.is_empty() {
        return 18 * 18;
    }
    distances.sort_unstable();
    let p85 = distances[(distances.len() * 85 / 100).min(distances.len() - 1)];
    p85.saturating_mul(4).clamp(18 * 18, 80 * 80)
}

fn dilate_mask(mask: &[bool], width: usize, height: usize) -> Vec<bool> {
    let mut out = mask.to_vec();
    for y in 0..height {
        for x in 0..width {
            if !mask[y * width + x] {
                continue;
            }
            let y0 = y.saturating_sub(1);
            let y1 = (y + 1).min(height - 1);
            let x0 = x.saturating_sub(1);
            let x1 = (x + 1).min(width - 1);
            for ny in y0..=y1 {
                for nx in x0..=x1 {
                    out[ny * width + nx] = true;
                }
            }
        }
    }
    out
}

fn quantized_rgb_key(px: [u8; 4]) -> u16 {
    ((px[0] as u16 >> 4) << 8) | ((px[1] as u16 >> 4) << 4) | (px[2] as u16 >> 4)
}

fn color_distance_sq(a: [u8; 4], b: [u8; 4]) -> u32 {
    let dr = a[0] as i32 - b[0] as i32;
    let dg = a[1] as i32 - b[1] as i32;
    let db = a[2] as i32 - b[2] as i32;
    (dr * dr + dg * dg + db * db) as u32
}

fn rgba_at(raw: &[u8], image_w: u32, x: u32, y: u32) -> Option<[u8; 4]> {
    let idx = ((y as usize)
        .checked_mul(image_w as usize)?
        .checked_add(x as usize)?)
    .checked_mul(4)?;
    Some([
        *raw.get(idx)?,
        *raw.get(idx + 1)?,
        *raw.get(idx + 2)?,
        *raw.get(idx + 3)?,
    ])
}

fn set_rgba(raw: &mut [u8], image_w: u32, x: u32, y: u32, px: [u8; 4]) {
    let idx = ((y as usize * image_w as usize) + x as usize) * 4;
    if idx + 3 < raw.len() {
        raw[idx] = px[0];
        raw[idx + 1] = px[1];
        raw[idx + 2] = px[2];
        raw[idx + 3] = px[3];
    }
}

fn is_valid_text(text: &str, confidence: f32, min_confidence: f32, min_ascii_ratio: f32) -> bool {
    if confidence < min_confidence || text.trim().is_empty() {
        return false;
    }
    let total = text.chars().count();
    if total == 0 {
        return false;
    }
    let printable_ascii = text
        .chars()
        .filter(|c| matches!(*c as u32, 32..=126))
        .count();
    printable_ascii as f32 / total as f32 >= min_ascii_ratio
}

fn polygon_bounds(
    polygon: &[[f32; 2]],
    image_w: u32,
    image_h: u32,
) -> Option<(u32, u32, u32, u32)> {
    if image_w == 0 || image_h == 0 {
        return None;
    }
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for [x, y] in polygon {
        if x.is_finite() && y.is_finite() {
            min_x = min_x.min(*x);
            min_y = min_y.min(*y);
            max_x = max_x.max(*x);
            max_y = max_y.max(*y);
        }
    }
    if !min_x.is_finite() || !min_y.is_finite() || !max_x.is_finite() || !max_y.is_finite() {
        return None;
    }

    let x0 = min_x.floor().clamp(0.0, image_w.saturating_sub(1) as f32) as u32;
    let y0 = min_y.floor().clamp(0.0, image_h.saturating_sub(1) as f32) as u32;
    let x1 = max_x
        .ceil()
        .clamp((x0 + 1).min(image_w) as f32, image_w as f32) as u32;
    let y1 = max_y
        .ceil()
        .clamp((y0 + 1).min(image_h) as f32, image_h as f32) as u32;
    Some((x0, y0, x1, y1))
}

fn estimate_text_color(
    rgb: &[[u8; 3]],
    image_w: u32,
    image_h: u32,
    bounds: (u32, u32, u32, u32),
    global_bg: [u8; 3],
) -> [u8; 3] {
    let (x0, y0, x1, y1) = bounds;
    if x1 <= x0 || y1 <= y0 || image_w == 0 || image_h == 0 {
        return [0, 0, 0];
    }

    let mut pixels = Vec::with_capacity(((x1 - x0) * (y1 - y0)) as usize);
    for y in y0..y1 {
        for x in x0..x1 {
            if let Some(px) = rgb
                .get((y as usize).saturating_mul(image_w as usize) + x as usize)
                .copied()
            {
                pixels.push(px);
            }
        }
    }

    if pixels.is_empty() {
        return global_bg;
    }

    let bg = median_rgb(&pixels);
    let mut ranked: Vec<(u32, [u8; 3])> = pixels
        .into_iter()
        .map(|px| {
            let dr = px[0] as i32 - bg[0] as i32;
            let dg = px[1] as i32 - bg[1] as i32;
            let db = px[2] as i32 - bg[2] as i32;
            ((dr * dr + dg * dg + db * db) as u32, px)
        })
        .collect();
    ranked.sort_unstable_by_key(|(distance, _)| std::cmp::Reverse(*distance));
    let keep = ((ranked.len() as f32) * 0.15).ceil().max(1.0) as usize;
    let mut sum = [0u64; 3];
    for (_, px) in ranked.iter().take(keep) {
        sum[0] += px[0] as u64;
        sum[1] += px[1] as u64;
        sum[2] += px[2] as u64;
    }
    [
        (sum[0] / keep as u64) as u8,
        (sum[1] / keep as u64) as u8,
        (sum[2] / keep as u64) as u8,
    ]
}

fn median_rgb(pixels: &[[u8; 3]]) -> [u8; 3] {
    let mut r = Vec::with_capacity(pixels.len());
    let mut g = Vec::with_capacity(pixels.len());
    let mut b = Vec::with_capacity(pixels.len());
    for px in pixels {
        r.push(px[0]);
        g.push(px[1]);
        b.push(px[2]);
    }
    r.sort_unstable();
    g.sort_unstable();
    b.sort_unstable();
    let mid = pixels.len() / 2;
    [r[mid], g[mid], b[mid]]
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn art(source: &str, rows: &[&str]) -> TextArt {
        normalize_text_art(source, rows.iter().map(|row| (*row).to_string()).collect()).unwrap()
    }

    fn default_render_args() -> RenderArgs {
        let args = crate::args::Args::parse_from(["img2irc"]);
        crate::args_to_render_args(&args)
    }

    #[test]
    fn text_art_normalization_crops_blank_outer_rows_and_columns() {
        let normalized = art("compact", &["      ", "  AB  ", "  CD  ", "      "]);

        assert_eq!(normalized.lines, vec![vec!['A', 'B'], vec!['C', 'D']]);
        assert_eq!(normalized.width(), 2);
        assert_eq!(normalized.height(), 2);
    }

    #[test]
    fn adaptive_text_art_uses_the_largest_candidate_that_fits() {
        let selected = select_best_text_art(
            [
                art("plain", &["HI"]),
                art("compact", &["ABC", "DEF"]),
                art("too-wide", &["ABCD", "EFGH"]),
            ],
            3,
            2,
        )
        .unwrap();

        assert_eq!(selected.source, "compact");
    }

    #[test]
    fn adaptive_text_art_falls_back_to_plain_for_a_single_row() {
        let selected =
            select_best_text_art([art("plain", &["HI"]), art("compact", &["HI", "HI"])], 2, 1)
                .unwrap();

        assert_eq!(selected.source, "plain");
    }

    #[test]
    fn figlet_gate_honors_enabled_and_minimum_grid_height() {
        let text_box = ScaledTextBox {
            left: 0.0,
            top: 0.0,
            right: 20.0,
            bottom: 3.0,
        };
        let mut args = default_render_args();

        args.ocr_figlet = false;
        assert!(!figlet_allowed(&args, text_box, 30.0, 10.0));

        args.ocr_figlet = true;
        args.ocr_figlet_min_height = 4.0;
        assert!(!figlet_allowed(&args, text_box, 30.0, 10.0));

        args.ocr_figlet_min_height = 3.0;
        assert!(figlet_allowed(&args, text_box, 30.0, 10.0));
    }

    #[test]
    fn figlet_gate_can_require_text_larger_than_the_typical_ocr_line() {
        let text_box = ScaledTextBox {
            left: 0.0,
            top: 0.0,
            right: 20.0,
            bottom: 3.0,
        };
        let mut args = default_render_args();
        args.ocr_figlet_min_height_ratio = 1.5;

        assert!(!figlet_allowed(&args, text_box, 14.0, 10.0));
        assert!(figlet_allowed(&args, text_box, 15.0, 10.0));

        args.ocr_figlet_min_height_ratio = 0.0;
        assert!(figlet_allowed(&args, text_box, 10.0, 10.0));
    }

    #[test]
    fn automatic_ocr_width_starts_at_native_size_and_grows_for_text() {
        let no_text = automatic_ocr_columns(200, 10.0, 1, None, &[]);
        assert_eq!(no_text.0, 20);

        let boxes = [TextBoxMetric {
            x0: 20.0,
            x1: 40.0,
            text_cols: 10,
        }];
        let with_text = automatic_ocr_columns(200, 10.0, 1, None, &boxes);
        assert_eq!((with_text.0, with_text.1, with_text.2), (100, 20, 100));

        let capped = automatic_ocr_columns(200, 10.0, 1, Some(30), &boxes);
        assert_eq!(capped.0, 30);
    }

    #[test]
    fn image_box_mapping_scales_without_changing_alignment() {
        let small = scale_image_box_to_grid((25, 20, 75, 40), 100, 100, 20, 10);
        let large = scale_image_box_to_grid((25, 20, 75, 40), 100, 100, 40, 20);

        assert_eq!(small, ScaledTextBox {
            left: 5.0,
            top: 2.0,
            right: 15.0,
            bottom: 4.0,
        });
        assert_eq!(large, ScaledTextBox {
            left: 10.0,
            top: 4.0,
            right: 30.0,
            bottom: 8.0,
        });
        assert_eq!(small.place(2), (5, 2));
        assert_eq!(large.place(4), (10, 4));
    }

    #[test]
    fn horizontal_fit_uses_space_until_the_next_aligned_text_box() {
        let boxes = [
            ScaledTextBox {
                left: 10.0,
                top: 5.0,
                right: 30.0,
                bottom: 8.0,
            },
            ScaledTextBox {
                left: 60.0,
                top: 6.0,
                right: 75.0,
                bottom: 9.0,
            },
            ScaledTextBox {
                left: 40.0,
                top: 12.0,
                right: 55.0,
                bottom: 14.0,
            },
        ];

        assert_eq!(available_layout_width(0, &boxes, 90), 50);
        assert_eq!(available_layout_width(1, &boxes, 90), 30);
    }

    #[test]
    fn overlay_anchor_uses_the_removed_text_edge_not_detector_padding() {
        let background = vec![[10, 20, 30]; 10];
        let mut source = background.clone();
        source[2] = [240, 240, 240];
        source[7] = [240, 240, 240];

        assert_eq!(
            removed_text_left_edge(&source, &background, 5, (0, 0, 5, 2)),
            Some(2)
        );
    }

    #[test]
    fn figlet_fit_uses_continuous_box_size_not_touched_cells() {
        // This box crosses two row boundaries, but it is only 0.8 rows tall
        // and 4.8 columns wide. It must not admit two-line or five-column art.
        let text_box = scale_image_box_to_grid((3, 15, 27, 23), 100, 100, 20, 10);

        assert_eq!(text_box.available_width(), 4);
        assert_eq!(text_box.available_height(), 1);
    }

    #[test]
    fn plain_fallback_never_truncates_recognized_text() {
        assert!(choose_text_art("Hello", 3, 1).is_none());
        assert_eq!(choose_text_art("Hello", 5, 1).unwrap().source, "plain");
    }

    #[test]
    fn preferred_phmajerus_font_wins_when_it_fits() {
        let selected = select_best_text_art(
            [
                art("Three Point", &["ABCD", "EFGH"]),
                art("phm-minecraft", &["AB", "CD"]),
            ],
            4,
            2,
        )
        .unwrap();

        assert_eq!(selected.source, "phm-minecraft");
    }

    #[test]
    fn discovered_compact_figfont_renders_when_one_is_installed() {
        let font_lists = crate::args::default_ocr_figlet_font_lists();
        let fonts = load_ocr_figlet_fonts(&font_lists);
        let Some(font) = fonts.first() else {
            return;
        };
        let rendered = render_figlet_text(font, "Hello").unwrap();

        assert!((2..=3).contains(&rendered.height()));
        assert!(rendered.width() >= "Hello".len());
    }

    #[test]
    fn installed_preferred_font_is_selected_for_a_large_enough_box() {
        let font_lists = crate::args::default_ocr_figlet_font_lists();
        let fonts = load_ocr_figlet_fonts(&font_lists);
        if !fonts.iter().any(|font| {
            PREFERRED_OCR_FIGLET_FONTS
                .iter()
                .any(|preferred| *preferred == font.name)
        }) {
            return;
        }

        let rendered =
            choose_text_art_from_fonts("Hello", 512, 3, &font_lists, &fonts).unwrap();
        assert!(
            PREFERRED_OCR_FIGLET_FONTS
                .iter()
                .any(|preferred| *preferred == rendered.source),
            "selected {}",
            rendered.source
        );
    }

    #[test]
    fn multiline_text_art_creates_overlay_runs_on_each_row() {
        let mut overlays = Vec::new();
        let background = vec![[7, 8, 9]; 16];

        push_text_art_overlay_runs(
            &mut overlays,
            &art("compact", &["AB", "CD"]),
            &background,
            4,
            4,
            1,
            1,
            4,
            4,
            [255, 255, 255],
            [0, 0, 0],
        );

        assert_eq!(overlays.len(), 2);
        assert_eq!((overlays[0].text.as_str(), overlays[0].y), ("AB", 1));
        assert_eq!((overlays[1].text.as_str(), overlays[1].y), ("CD", 2));
        assert!(overlays
            .iter()
            .all(|overlay| overlay.bg == Some(ColorSpec::Rgb([7, 8, 9]))));
    }
}
