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

fn box_iou(a: (u32, u32, u32, u32), b: (u32, u32, u32, u32)) -> f32 {
    let x0 = a.0.max(b.0);
    let y0 = a.1.max(b.1);
    let x1 = a.2.min(b.2);
    let y1 = a.3.min(b.3);
    if x1 <= x0 || y1 <= y0 {
        return 0.0;
    }
    let inter = ((x1 - x0) * (y1 - y0)) as f32;
    let area_a = ((a.2 - a.0) * (a.3 - a.1)) as f32;
    let area_b = ((b.2 - b.0) * (b.3 - b.1)) as f32;
    inter / (area_a + area_b - inter)
}

fn renderable_detections<'a>(
    detections: &'a [OcrDetection],
    args: &'a RenderArgs,
    image_w: u32,
    image_h: u32,
) -> Vec<&'a OcrDetection> {
    let mut filtered: Vec<&OcrDetection> = valid_detections(detections, args).collect();
    if args.ocr_max_text_height_ratio > 0.0 && filtered.len() >= 2 {
        let mut heights = filtered
            .iter()
            .filter_map(|det| ocr_detection_height(det, image_w, image_h))
            .collect::<Vec<_>>();
        if let Some(median_height) = median_f32(&mut heights) {
            let max_height = median_height * args.ocr_max_text_height_ratio.max(0.0);
            filtered.retain(|det| {
                ocr_detection_height(det, image_w, image_h)
                    .map(|height| height <= max_height)
                    .unwrap_or(false)
            });
        }
    }

    filtered.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
    let mut deduped: Vec<&OcrDetection> = Vec::new();
    for det in filtered {
        let bounds = polygon_bounds(&det.polygon, image_w, image_h);
        if let Some(b) = bounds {
            if !deduped.iter().any(|kept| {
                polygon_bounds(&kept.polygon, image_w, image_h)
                    .map(|kb| box_iou(kb, b) > 0.75)
                    .unwrap_or(false)
            }) {
                deduped.push(det);
            }
        } else {
            deduped.push(det);
        }
    }
    deduped
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

    #[cfg(test)]
    fn fill_key(&self) -> (usize, usize, usize, std::cmp::Reverse<usize>) {
        (
            self.width() * self.height(),
            self.height(),
            self.width(),
            std::cmp::Reverse(self.priority),
        )
    }
}

#[cfg(test)]
const PREFERRED_OCR_FIGLET_FONTS: &[&str] =
    &["phm-minecraft", "phm-largetype", "phm-lcdmatrix"];

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
        PathBuf::from("/usr/share/figlet"),
        PathBuf::from("/usr/local/share/figlet/fonts"),
        PathBuf::from("/usr/local/share/figlet"),
        PathBuf::from("/opt/homebrew/share/figlet/fonts"),
        PathBuf::from("/opt/homebrew/share/figlet"),
        PathBuf::from("static/figlet"),
        PathBuf::from("FIGfonts/fonts"),
        PathBuf::from("figfonts/fonts"),
    ]);
    if let Some(home) = std::env::var_os("HOME") {
        let home_path = PathBuf::from(home);
        dirs.push(home_path.join(".local/share/figlet/fonts"));
        dirs.push(home_path.join(".local/share/figlet"));
    }

    let mut unique = Vec::new();
    for dir in dirs {
        if !unique.contains(&dir) {
            unique.push(dir);
        }
    }
    unique
}

fn load_figlet_from_path(path: &Path) -> Result<FIGlet, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if bytes.starts_with(b"PK\x03\x04") {
        #[cfg(feature = "zip")]
        {
            use std::io::Read;
            let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes))
                .map_err(|e| e.to_string())?;
            if archive.is_empty() {
                return Err("ZIP archive is empty".to_string());
            }
            let mut file = archive.by_index(0).map_err(|e| e.to_string())?;
            let mut contents = String::new();
            file.read_to_string(&mut contents).map_err(|e| e.to_string())?;
            FIGlet::from_content(&contents).map_err(|e| e.to_string())
        }
        #[cfg(not(feature = "zip"))]
        {
            Err("ZIP-compressed FIGlet fonts require zip feature".to_string())
        }
    } else {
        let contents = String::from_utf8(bytes).map_err(|e| e.to_string())?;
        FIGlet::from_content(&contents).map_err(|e| e.to_string())
    }
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
            let name_variants = [
                name.clone(),
                name.replace('_', " "),
                name.replace('_', "-"),
                name.replace('-', " "),
                name.replace('-', "_"),
            ];
            let Some(path) = dirs
                .iter()
                .flat_map(|dir| {
                    name_variants.iter().flat_map(move |variant| {
                        ["flf", "tlf"]
                            .into_iter()
                            .map(move |extension| dir.join(format!("{variant}.{extension}")))
                    })
                })
                .find(|path| path.is_file())
            else {
                log::debug!(
                    "OCR FIGlet/Toilet font {name:?} was not found for {} lines",
                    list.height
                );
                continue;
            };

            match load_figlet_from_path(&path) {
                Ok(font) => {
                    let actual_height = font.header_line.height as u32;
                    fonts.push(OcrFigletFont {
                        name: name.clone(),
                        configured_height: actual_height,
                        priority,
                        font,
                    });
                }
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

thread_local! {
    static INTERACTIVE_FIGLET_FONTS: std::cell::RefCell<Option<(
        Vec<OcrFigletFontList>,
        Vec<OcrFigletFont>,
    )>> = const { std::cell::RefCell::new(None) };
}

/// Rebuild a monolithic FIGlet overlay from its editable source text and box.
/// The parsed font set is retained per UI thread so editing and resizing do not
/// repeatedly read and parse every configured font.
pub fn refresh_figlet_overlay(
    overlay: &mut TextOverlay,
    font_lists: &[OcrFigletFontList],
) -> bool {
    let Some(source) = overlay.source_text.clone() else {
        return false;
    };
    overlay.transparent_spaces = true;
    if source.trim().is_empty() {
        overlay.text = source;
        return true;
    }
    let max_width = overlay.w.max(1) as usize;
    let max_height = overlay.h.max(1) as usize;
    let requested_font = overlay
        .figlet_font
        .clone()
        .filter(|name| !name.eq_ignore_ascii_case("plain"));
    let art = INTERACTIVE_FIGLET_FONTS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache
            .as_ref()
            .is_none_or(|(cached_lists, _)| cached_lists != font_lists)
        {
            *cache = Some((font_lists.to_vec(), load_ocr_figlet_fonts(font_lists)));
        }
        let (_, fonts) = cache.as_ref().expect("interactive FIGlet cache initialized");
        if let Some(name) = requested_font.as_deref() {
            fonts
                .iter()
                .find(|font| font.name.eq_ignore_ascii_case(name))
                .and_then(|font| render_figlet_text(font, &source))
                .filter(|art| art.width() <= max_width && art.height() <= max_height)
        } else {
            choose_text_art_from_fonts(&source, max_width, max_height, max_width, max_height, font_lists, fonts)
        }
    });
    if let Some(art) = art {
        overlay.text = art
            .lines
            .iter()
            .map(|line| line.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        overlay.figlet_font = Some(requested_font.unwrap_or(art.source));
        true
    } else {
        overlay.text = source;
        if requested_font.is_none() {
            overlay.figlet_font = Some("plain".to_string());
        }
        false
    }
}

fn text_art_overlay(
    source_text: &str,
    art: &TextArt,
    figlet_enabled: bool,
    x: i32,
    y: i32,
    fg: [u8; 3],
) -> TextOverlay {
    TextOverlay {
        text: art
            .lines
            .iter()
            .map(|line| line.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("\n"),
        source_text: figlet_enabled.then(|| source_text.to_string()),
        figlet_font: figlet_enabled.then(|| art.source.clone()),
        x,
        y,
        w: art.width() as i32,
        h: art.height() as i32,
        fg: Some(ColorSpec::Rgb(fg)),
        bg: None,
        wrap: false,
        auto_grow: false,
        transparent_spaces: figlet_enabled,
        bold: false,
        italic: false,
        underline: false,
    }
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

#[cfg(test)]
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
fn choose_text_art(text: &str, base_width: usize, base_height: usize) -> Option<TextArt> {
    let font_lists = crate::args::default_ocr_figlet_font_lists();
    let fonts = load_ocr_figlet_fonts(&font_lists);
    choose_text_art_from_fonts(
        text,
        base_width,
        base_height,
        base_width,
        base_height,
        &font_lists,
        &fonts,
    )
}

fn choose_text_art_from_fonts(
    text: &str,
    base_width: usize,
    base_height: usize,
    max_w_allowed: usize,
    max_h_allowed: usize,
    font_lists: &[OcrFigletFontList],
    fonts: &[OcrFigletFont],
) -> Option<TextArt> {
    let text = text.trim();
    if text.is_empty() || base_width == 0 || base_height == 0 || max_w_allowed == 0 || max_h_allowed == 0 {
        return None;
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Step 1 (Strict 0% Expansion Priority):
    // Check if ANY configured FIGlet font fits in the UNMODIFIED bounding box.
    // Heights are checked descending (largest font that fits in the original box first).
    // ─────────────────────────────────────────────────────────────────────────
    let mut candidate_heights = fonts
        .iter()
        .map(|f| f.configured_height)
        .chain(font_lists.iter().map(|l| l.height))
        .filter(|&h| h > 0 && h as usize <= base_height)
        .collect::<Vec<_>>();
    candidate_heights.sort_unstable_by(|a, b| b.cmp(a));
    candidate_heights.dedup();

    for height in candidate_heights {
        if height == 1
            && font_lists.iter().any(|l| {
                l.height == 1 && l.fonts.iter().any(|n| n.eq_ignore_ascii_case("plain"))
            })
        {
            if text.chars().count() <= base_width {
                if let Some(plain) =
                    normalize_text_art_with_metadata("plain", 1, 0, vec![text.to_string()])
                {
                    return Some(plain);
                }
            }
        }

        let mut list_fonts: Vec<&OcrFigletFont> = fonts
            .iter()
            .filter(|f| f.configured_height == height)
            .collect();
        list_fonts.sort_by_key(|f| f.priority);

        for font in list_fonts {
            if let Some(rendered) = render_figlet_text(font, text) {
                if rendered.width() > 0
                    && rendered.height() > 0
                    && rendered.width() <= base_width
                    && rendered.height() <= base_height
                {
                    return Some(rendered);
                }
            }
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Step 2 (Enlargement ONLY if Step 1 found no fitting font):
    // Only if no font fit within the original bounding box, check if enlarging
    // the bounding box (up to max_w_allowed and max_h_allowed) allows a font to fit.
    // Pick the font that requires the SMALLEST enlargement.
    // ─────────────────────────────────────────────────────────────────────────
    if max_w_allowed > base_width || max_h_allowed > base_height {
        let mut candidates = Vec::new();
        for font in fonts {
            if font.configured_height == 0 || font.configured_height as usize > max_h_allowed {
                continue;
            }
            if let Some(rendered) = render_figlet_text(font, text) {
                let art_w = rendered.width();
                let art_h = rendered.height();
                if art_w > 0 && art_h > 0 && art_w <= max_w_allowed && art_h <= max_h_allowed {
                    let expand_w = art_w.saturating_sub(base_width);
                    let expand_h = art_h.saturating_sub(base_height);
                    candidates.push((
                        rendered,
                        expand_w,
                        expand_h,
                        font.configured_height,
                        font.priority,
                    ));
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

    // ─────────────────────────────────────────────────────────────────────────
    // Step 3 (Fallback):
    // Fall back to plain single-line text within max_w_allowed.
    // ─────────────────────────────────────────────────────────────────────────
    if text.chars().count() <= max_w_allowed {
        plain_text_art(text, max_w_allowed)
    } else {
        None
    }
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

    fn place(self, art_width: usize, art_height: usize) -> (i32, i32) {
        let box_w = (self.right - self.left).max(1.0);
        let box_h = (self.bottom - self.top).max(1.0);
        let x = self.left + (box_w - art_width as f64) * 0.5;
        let y = self.top + (box_h - art_height as f64) * 0.5;
        (x.round() as i32, y.round() as i32)
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
    _background_source: &PhotonImage,
    args: &RenderArgs,
    glyphs: &GlyphStore,
    detections: &[OcrDetection],
) -> Vec<TextOverlay> {
    let (dim_w, dim_h) = crate::effects::calculate_dimensions(
        args,
        glyphs,
        source.get_width() as f32,
        source.get_height() as f32,
    );
    let (gw, gh) = if args.braille {
        (2, 4)
    } else {
        glyphs.metrics
    };
    let grid_w = (dim_w / gw.max(1) as u32).max(1) as i32;
    let grid_h = (dim_h / gh.max(1) as u32).max(1) as i32;
    let rgb = photon_rgb(source);
    let global_bg = rgb.first().copied().unwrap_or([0, 0, 0]);
    let figlet_fonts = if args.ocr_figlet {
        load_ocr_figlet_fonts(&args.ocr_figlet_fonts)
    } else {
        Vec::new()
    };

    let mut overlays = Vec::new();
    let mut renderable = renderable_detections(
        detections,
        args,
        source.get_width(),
        source.get_height(),
    )
    .into_iter()
    .filter_map(|det| {
        let bounds = polygon_bounds(&det.polygon, source.get_width(), source.get_height())?;
        let text_box = scale_image_box_to_grid(
            bounds,
            source.get_width(),
            source.get_height(),
            grid_w,
            grid_h,
        );
        Some((det, bounds, text_box))
    })
    .collect::<Vec<_>>();
    renderable.sort_by(|a, b| {
        a.2.top.total_cmp(&b.2.top).then_with(|| a.2.left.total_cmp(&b.2.left))
    });

    let mut detection_heights = renderable
        .iter()
        .map(|(_, bounds, _)| bounds.3.saturating_sub(bounds.1) as f32)
        .filter(|height| *height > 0.0)
        .collect::<Vec<_>>();
    let median_detection_height = if detection_heights.len() >= 3 {
        median_f32(&mut detection_heights).unwrap_or(0.0)
    } else {
        0.0
    };

    let mut occupied = vec![vec![false; grid_w as usize]; grid_h as usize];

    for i in 0..renderable.len() {
        let (det, bounds, text_box) = &renderable[i];
        let target_w = text_box.available_width();
        let target_h = text_box.available_height();
        let left_i = (text_box.left.round() as usize).min(grid_w as usize);
        let top_i = (text_box.top.round() as usize).min(grid_h as usize);
        let right_i = (text_box.right.round() as usize).min(grid_w as usize);
        let bottom_i = (text_box.bottom.round() as usize).min(grid_h as usize);

        let canvas_max_w = (grid_w as usize).saturating_sub(left_i);
        let canvas_max_h = (grid_h as usize).saturating_sub(top_i);

        let mut max_w_allowed = ((target_w as f32 * (1.0 + args.ocr_figlet_max_width_ratio.max(0.0))).floor() as usize).min(canvas_max_w).max(1);
        let mut max_h_allowed = ((target_h as f32 * (1.0 + args.ocr_figlet_max_height_ratio.max(0.0))).floor() as usize).min(canvas_max_h).max(1);

        // Constrain max enlargement against neighboring text boxes so they do not collide
        for j in 0..renderable.len() {
            if i == j {
                continue;
            }
            let (_, _, other_box) = &renderable[j];
            let left_j = (other_box.left.round() as usize).min(grid_w as usize);
            let top_j = (other_box.top.round() as usize).min(grid_h as usize);
            let right_j = (other_box.right.round() as usize).min(grid_w as usize);
            let bottom_j = (other_box.bottom.round() as usize).min(grid_h as usize);

            let horiz_overlap = !(right_j <= left_i || left_j >= right_i);
            let vert_overlap = !(bottom_j <= top_i || top_j >= bottom_i);

            if horiz_overlap && top_j >= bottom_i {
                let max_down = top_j.saturating_sub(top_i);
                max_h_allowed = max_h_allowed.min(max_down);
            }
            if vert_overlap && left_j >= right_i {
                let max_right = left_j.saturating_sub(left_i);
                max_w_allowed = max_w_allowed.min(max_right);
            }
        }
        max_w_allowed = max_w_allowed.max(1);
        max_h_allowed = max_h_allowed.max(1);

        let detection_height = bounds.3.saturating_sub(bounds.1) as f32;
        let allow_figlet = figlet_allowed(
            args,
            *text_box,
            detection_height,
            median_detection_height,
        ) && target_h >= args.ocr_figlet_min_height as usize;

        let art = if allow_figlet {
            choose_text_art_from_fonts(
                &det.text,
                target_w,
                target_h,
                max_w_allowed,
                max_h_allowed,
                &args.ocr_figlet_fonts,
                &figlet_fonts,
            )
        } else {
            plain_text_art(&det.text, max_w_allowed)
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
        let (placed_x, placed_y) = text_box.place(art_w as usize, art_h as usize);
        let art_x = placed_x.clamp(0, grid_w.saturating_sub(art_w).max(0));
        let art_y = placed_y.clamp(0, grid_h.saturating_sub(art_h).max(0));

        // Overlap check against occupied cells
        let mut has_overlap = false;
        for r in (art_y as usize)..((art_y + art_h) as usize).min(grid_h as usize) {
            for c in (art_x as usize)..((art_x + art_w) as usize).min(grid_w as usize) {
                if occupied[r][c] {
                    has_overlap = true;
                    break;
                }
            }
            if has_overlap {
                break;
            }
        }

        let (final_art, final_x, final_y, final_w, final_h) = if has_overlap && art.source != "plain" {
            if let Some(plain) = plain_text_art(&det.text, max_w_allowed) {
                let pw = plain.width() as i32;
                let ph = plain.height() as i32;
                let (px, py) = text_box.place(pw as usize, ph as usize);
                (plain, px.clamp(0, grid_w.saturating_sub(pw).max(0)), py.clamp(0, grid_h.saturating_sub(ph).max(0)), pw, ph)
            } else {
                (art, art_x, art_y, art_w, art_h)
            }
        } else {
            (art, art_x, art_y, art_w, art_h)
        };

        for r in (final_y as usize)..((final_y + final_h) as usize).min(grid_h as usize) {
            for c in (final_x as usize)..((final_x + final_w) as usize).min(grid_w as usize) {
                occupied[r][c] = true;
            }
        }

        let fg = estimate_text_color(
            &rgb,
            source.get_width(),
            source.get_height(),
            *bounds,
            global_bg,
        );
        if final_art.source != "plain" {
            log::debug!(
                "OCR FIGfont {} ({}-line list) selected for {:?}: {}x{} in {}x{} cells",
                final_art.source,
                final_art.configured_height,
                det.text,
                final_w,
                final_h,
                target_w,
                target_h
            );
        }
        overlays.push(text_art_overlay(
            &det.text,
            &final_art,
            allow_figlet,
            final_x,
            final_y,
            fg,
        ));
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
    _bounds: (u32, u32, u32, u32),
    erase_bounds: (u32, u32, u32, u32),
) -> Option<[u8; 4]> {
    let (ex0, ey0, ex1, ey1) = erase_bounds;
    let ring = (erase_bounds.3.saturating_sub(erase_bounds.1) / 3).clamp(3, 16);
    let sx0 = ex0.saturating_sub(ring);
    let sy0 = ey0.saturating_sub(ring);
    let sx1 = ex1.saturating_add(ring).min(image_w);
    let sy1 = ey1.saturating_add(ring).min(image_h);

    let mut bins = std::collections::HashMap::<u16, (u32, [u64; 4])>::new();
    for y in sy0..sy1 {
        for x in sx0..sx1 {
            if x >= ex0 && x < ex1 && y >= ey0 && y < ey1 {
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

    if let Some((_, (count, sum))) = bins.into_iter().max_by_key(|(_, (count, _))| *count) {
        Some([
            (sum[0] / count as u64) as u8,
            (sum[1] / count as u64) as u8,
            (sum[2] / count as u64) as u8,
            (sum[3] / count as u64) as u8,
        ])
    } else {
        rgba_at(raw, image_w, 0, 0).or(Some([0, 0, 0, 255]))
    }
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
        return [255, 255, 255];
    }
    let pad = ((((y1 - y0) as f32) * 0.08).ceil() as u32).clamp(1, 3);
    let erase_x0 = x0.saturating_sub(pad);
    let erase_y0 = y0.saturating_sub(pad);
    let erase_x1 = (x1 + pad).min(image_w);
    let erase_y1 = (y1 + pad).min(image_h);
    let ring = (((y1 - y0) / 4) as u32).clamp(3, 8);
    let sample_x0 = erase_x0.saturating_sub(ring);
    let sample_y0 = erase_y0.saturating_sub(ring);
    let sample_x1 = (erase_x1 + ring).min(image_w);
    let sample_y1 = (erase_y1 + ring).min(image_h);

    let mut bg_pixels = Vec::new();
    for y in sample_y0..sample_y1 {
        for x in sample_x0..sample_x1 {
            let inside = x >= erase_x0 && x < erase_x1 && y >= erase_y0 && y < erase_y1;
            if !inside {
                let idx = (y as usize) * (image_w as usize) + (x as usize);
                if let Some(px) = rgb.get(idx) {
                    bg_pixels.push(*px);
                }
            }
        }
    }
    let bg = if !bg_pixels.is_empty() {
        dominant_rgb(&bg_pixels)
    } else {
        global_bg
    };

    let mut bbox_pixels = Vec::with_capacity(((x1 - x0) * (y1 - y0)) as usize);
    for y in y0..y1 {
        for x in x0..x1 {
            let idx = (y as usize) * (image_w as usize) + (x as usize);
            if let Some(px) = rgb.get(idx).copied() {
                bbox_pixels.push(px);
            }
        }
    }

    best_contrast_foreground_color(&bbox_pixels, bg)
}

fn dominant_rgb(pixels: &[[u8; 3]]) -> [u8; 3] {
    if pixels.is_empty() {
        return [0, 0, 0];
    }
    let mut bins: std::collections::HashMap<(u8, u8, u8), (usize, [u64; 3])> =
        std::collections::HashMap::new();
    for px in pixels {
        let key = (px[0] >> 4, px[1] >> 4, px[2] >> 4);
        let entry = bins.entry(key).or_insert((0, [0, 0, 0]));
        entry.0 += 1;
        entry.1[0] += px[0] as u64;
        entry.1[1] += px[1] as u64;
        entry.1[2] += px[2] as u64;
    }
    bins.into_values()
        .max_by_key(|(count, _)| *count)
        .map(|(count, sums)| [
            (sums[0] / count as u64) as u8,
            (sums[1] / count as u64) as u8,
            (sums[2] / count as u64) as u8,
        ])
        .unwrap_or(pixels[0])
}

#[inline]
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[inline]
pub fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Standard CIE relative luminance Y in [0.0, 1.0] from sRGB [u8; 3].
pub fn relative_luminance(rgb: [u8; 3]) -> f32 {
    let r = srgb_to_linear(rgb[0] as f32 / 255.0);
    let g = srgb_to_linear(rgb[1] as f32 / 255.0);
    let b = srgb_to_linear(rgb[2] as f32 / 255.0);
    0.2126729 * r + 0.7151522 * g + 0.0721750 * b
}

/// WCAG 2.1 Contrast Ratio in range [1.0, 21.0].
pub fn wcag_contrast_ratio(fg: [u8; 3], bg: [u8; 3]) -> f32 {
    let y_fg = relative_luminance(fg);
    let y_bg = relative_luminance(bg);
    let (l1, l2) = if y_fg > y_bg {
        (y_fg, y_bg)
    } else {
        (y_bg, y_fg)
    };
    (l1 + 0.05) / (l2 + 0.05)
}

/// APCA (Accessible Perceptual Contrast Algorithm - APCA-W3 v0.98G).
/// Returns signed Lc value (-108..+106).
/// Positive indicates dark text on light background; negative indicates light text on dark background.
/// The magnitude `apca_contrast(fg, bg).abs()` gives the perceptual contrast score.
pub fn apca_contrast(fg: [u8; 3], bg: [u8; 3]) -> f32 {
    let r_txt = (fg[0] as f32 / 255.0).powf(2.4);
    let g_txt = (fg[1] as f32 / 255.0).powf(2.4);
    let b_txt = (fg[2] as f32 / 255.0).powf(2.4);
    let y_txt = 0.2126729 * r_txt + 0.7151522 * g_txt + 0.0721750 * b_txt;

    let r_bg = (bg[0] as f32 / 255.0).powf(2.4);
    let g_bg = (bg[1] as f32 / 255.0).powf(2.4);
    let b_bg = (bg[2] as f32 / 255.0).powf(2.4);
    let y_bg = 0.2126729 * r_bg + 0.7151522 * g_bg + 0.0721750 * b_bg;

    const BLK_THRESH: f32 = 0.022;
    const BLK_EXP: f32 = 1.414;

    let y_txt_c = if y_txt > BLK_THRESH {
        y_txt
    } else {
        y_txt + (BLK_THRESH - y_txt).powf(BLK_EXP)
    };

    let y_bg_c = if y_bg > BLK_THRESH {
        y_bg
    } else {
        y_bg + (BLK_THRESH - y_bg).powf(BLK_EXP)
    };

    if (y_bg_c - y_txt_c).abs() < 0.0005 {
        return 0.0;
    }

    let lc = if y_bg_c > y_txt_c {
        let s_bg = y_bg_c.powf(0.56);
        let s_txt = y_txt_c.powf(0.57);
        (s_bg - s_txt) * 1.1414
    } else {
        let s_bg = y_bg_c.powf(0.65);
        let s_txt = y_txt_c.powf(0.62);
        (s_bg - s_txt) * 1.1414
    };

    if lc.abs() < 0.1 {
        0.0
    } else if lc > 0.0 {
        (lc - 0.027) * 100.0
    } else {
        (lc + 0.027) * 100.0
    }
}

/// Evaluates whether foreground text `fg` has readable contrast against background `bg`
/// using APCA perceptual contrast, WCAG contrast ratio, and Oklab color difference.
pub fn is_readable_contrast(fg: [u8; 3], bg: [u8; 3]) -> bool {
    let apca = apca_contrast(fg, bg).abs();
    let wcag = wcag_contrast_ratio(fg, bg);
    let oklab_de = crate::draw::oklab_distance(
        crate::draw::rgb_to_oklab(fg),
        crate::draw::rgb_to_oklab(bg),
    );

    // Readable if:
    // - APCA score >= 40.0 (good perceptual readability for text)
    // - OR WCAG ratio >= 3.0:1 (minimum AA standard for large/bold text)
    // - OR strong chromatic contrast (Oklab DeltaE >= 0.35 with WCAG >= 2.2)
    apca >= 40.0 || wcag >= 3.0 || (oklab_de >= 0.35 && wcag >= 2.2)
}

/// Selects the best foreground text color from bounding box pixels against `bg`,
/// automatically filtering out hard-to-read colors (e.g. anti-aliasing edge blur,
/// background bleed, compression artifacts) using APCA and WCAG contrast.
/// When the original text color is readable, it is preserved without modification.
pub fn best_contrast_foreground_color(pixels: &[[u8; 3]], bg: [u8; 3]) -> [u8; 3] {
    if pixels.is_empty() {
        let bg_lum = relative_luminance(bg);
        return if bg_lum < 0.5 {
            [255, 255, 255]
        } else {
            [0, 0, 0]
        };
    }

    // 1. Cluster bounding box pixels into 4-bit RGB buckets (4096 bins)
    let mut bins: std::collections::HashMap<(u8, u8, u8), (usize, [u64; 3])> =
        std::collections::HashMap::new();
    for px in pixels {
        let key = (px[0] >> 4, px[1] >> 4, px[2] >> 4);
        let entry = bins.entry(key).or_insert((0, [0, 0, 0]));
        entry.0 += 1;
        entry.1[0] += px[0] as u64;
        entry.1[1] += px[1] as u64;
        entry.1[2] += px[2] as u64;
    }

    let total_pixels = pixels.len() as f32;
    let bg_oklab = crate::draw::rgb_to_oklab(bg);

    struct Candidate {
        color: [u8; 3],
        count: usize,
        wcag_cr: f32,
        apca_lc: f32,
        oklab_de: f32,
        chroma: f32,
        is_readable: bool,
    }

    let mut candidates = Vec::new();

    for (count, sums) in bins.values() {
        let avg_px = [
            (sums[0] / *count as u64) as u8,
            (sums[1] / *count as u64) as u8,
            (sums[2] / *count as u64) as u8,
        ];
        let fraction = *count as f32 / total_pixels;

        // Skip negligible noise (< 0.8% and count < 3)
        if fraction < 0.008 && *count < 3 {
            continue;
        }

        let px_oklab = crate::draw::rgb_to_oklab(avg_px);
        let oklab_de = crate::draw::oklab_distance(px_oklab, bg_oklab);

        // Skip pixels that are essentially identical to background (pure background pixels)
        if oklab_de < 0.06 {
            continue;
        }

        let wcag_cr = wcag_contrast_ratio(avg_px, bg);
        let apca_lc = apca_contrast(avg_px, bg);
        let is_readable = is_readable_contrast(avg_px, bg);
        let chroma = (px_oklab.a * px_oklab.a + px_oklab.b * px_oklab.b).sqrt();

        candidates.push(Candidate {
            color: avg_px,
            count: *count,
            wcag_cr,
            apca_lc,
            oklab_de,
            chroma,
            is_readable,
        });
    }

    // 2. Filter out hard-to-read colors automatically
    let readable_candidates: Vec<&Candidate> =
        candidates.iter().filter(|c| c.is_readable).collect();

    if !readable_candidates.is_empty() {
        let mut best_cand = readable_candidates[0];
        let mut best_score = -1.0f32;

        for cand in readable_candidates {
            let apca_mag = cand.apca_lc.abs();
            // Contrast quality: rewards comfortable readability with smooth plateau
            let contrast_quality = (cand.wcag_cr.min(7.0) / 4.5).clamp(0.75, 1.4)
                * (apca_mag.min(90.0) / 60.0).clamp(0.75, 1.4);
            // Population factor: true text glyphs form a substantial foreground cluster
            let pop_factor = (cand.count as f32).powf(0.65);
            // Saturation / chroma bonus: favors vivid text over desaturated edge blur
            let chroma_bonus = 1.0 + 0.6 * cand.chroma;

            let score = pop_factor * contrast_quality * chroma_bonus;
            if score > best_score {
                best_score = score;
                best_cand = cand;
            }
        }

        // Return the true original text color
        return best_cand.color;
    }

    // 3. Fallback when no candidate passed the initial contrast threshold:
    // (e.g. low-contrast watermark, blurry or faint text)
    if !candidates.is_empty() {
        let best_low_contrast = candidates
            .iter()
            .max_by(|a, b| {
                let score_a = a.apca_lc.abs() * 0.5
                    + a.wcag_cr * 10.0
                    + a.oklab_de * 30.0
                    + (a.count as f32).sqrt();
                let score_b = b.apca_lc.abs() * 0.5
                    + b.wcag_cr * 10.0
                    + b.oklab_de * 30.0
                    + (b.count as f32).sqrt();
                score_a.total_cmp(&score_b)
            })
            .unwrap();

        return adjust_color_for_readability(best_low_contrast.color, bg);
    }

    // 4. Fallback for completely uniform / empty boxes
    let bg_lum = relative_luminance(bg);
    if bg_lum < 0.5 {
        [255, 255, 255]
    } else {
        [0, 0, 0]
    }
}

/// Adjusts the lightness of `fg` towards readable contrast against `bg`
/// while preserving the exact original hue and saturation.
pub fn adjust_color_for_readability(fg: [u8; 3], bg: [u8; 3]) -> [u8; 3] {
    if is_readable_contrast(fg, bg) {
        return fg;
    }

    let bg_lum = relative_luminance(bg);
    let make_lighter = bg_lum < 0.5;
    let target_val: f32 = if make_lighter { 1.0 } else { 0.0 };

    let lin_r = srgb_to_linear(fg[0] as f32 / 255.0);
    let lin_g = srgb_to_linear(fg[1] as f32 / 255.0);
    let lin_b = srgb_to_linear(fg[2] as f32 / 255.0);

    for step in 1..=25 {
        let t = step as f32 / 25.0;
        let blend_r = (1.0 - t) * lin_r + t * target_val;
        let blend_g = (1.0 - t) * lin_g + t * target_val;
        let blend_b = (1.0 - t) * lin_b + t * target_val;

        let r = (linear_to_srgb(blend_r) * 255.0).round().clamp(0.0, 255.0) as u8;
        let g = (linear_to_srgb(blend_g) * 255.0).round().clamp(0.0, 255.0) as u8;
        let b = (linear_to_srgb(blend_b) * 255.0).round().clamp(0.0, 255.0) as u8;

        let cand = [r, g, b];
        if is_readable_contrast(cand, bg) {
            return cand;
        }
    }

    if make_lighter {
        [255, 255, 255]
    } else {
        [0, 0, 0]
    }
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
        assert_eq!(small.place(10, 2), (5, 2));
        assert_eq!(large.place(20, 4), (10, 4));
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

        assert!(rendered.height() > 0);
        assert!(rendered.width() >= "Hello".len());
    }

    #[test]
    fn first_fitting_font_in_list_is_selected() {
        let font_lists = crate::args::default_ocr_figlet_font_lists();
        let fonts = load_ocr_figlet_fonts(&font_lists);
        let Some(first_3_font) = font_lists
            .iter()
            .find(|l| l.height == 3)
            .and_then(|l| l.fonts.first())
        else {
            return;
        };

        if !fonts.iter().any(|f| f.name == *first_3_font) {
            return;
        }

        let rendered =
            choose_text_art_from_fonts("Hello", 512, 3, 512, 3, &font_lists, &fonts).unwrap();
        assert_eq!(rendered.source, *first_3_font);
    }

    #[test]
    fn figlet_font_renders_in_taller_box_as_long_as_it_fits() {
        let font_lists = crate::args::default_ocr_figlet_font_lists();
        let fonts = load_ocr_figlet_fonts(&font_lists);
        let Some(font) = fonts.iter().find(|f| f.name == "phm-minecraft") else {
            return;
        };

        let mut overlay = TextOverlay {
            text: "Hello".to_string(),
            source_text: Some("Hello".to_string()),
            x: 0,
            y: 0,
            w: 40,
            h: 10, // Much taller than phm-minecraft (height 2)
            fg: None,
            bg: None,
            transparent_spaces: true,
            wrap: false,
            auto_grow: false,
            bold: false,
            italic: false,
            underline: false,
            figlet_font: Some("phm-minecraft".to_string()),
        };

        let refreshed = refresh_figlet_overlay(&mut overlay, &font_lists);
        assert!(refreshed);
        assert_eq!(overlay.figlet_font.as_deref(), Some("phm-minecraft"));
        assert!(overlay.text.contains('\n')); // Multi-line FIGlet rendered text
    }

    #[test]
    fn multiline_figlet_art_is_one_editable_overlay() {
        let overlay = text_art_overlay(
            "Hello",
            &art("compact", &["AB", "CD"]),
            true,
            1,
            2,
            [255, 255, 255],
        );

        assert_eq!(overlay.text, "AB\nCD");
        assert_eq!(overlay.source_text.as_deref(), Some("Hello"));
        assert_eq!(overlay.figlet_font.as_deref(), Some("compact"));
        assert_eq!((overlay.x, overlay.y, overlay.w, overlay.h), (1, 2, 2, 2));
        assert_eq!(overlay.bg, None);
    }

    #[test]
    fn wcag_contrast_ratio_computes_correct_ratios() {
        // Black vs White
        let cr_bw = wcag_contrast_ratio([255, 255, 255], [0, 0, 0]);
        assert!((cr_bw - 21.0).abs() < 0.01);

        // Same colors -> 1.0
        let cr_same = wcag_contrast_ratio([128, 128, 128], [128, 128, 128]);
        assert!((cr_same - 1.0).abs() < 0.01);

        // Yellow on Black -> very high contrast (> 18:1)
        let cr_yb = wcag_contrast_ratio([255, 255, 0], [0, 0, 0]);
        assert!(cr_yb > 18.0);

        // Dark gray on White -> high contrast (> 13:1)
        let cr_gw = wcag_contrast_ratio([40, 40, 40], [255, 255, 255]);
        assert!(cr_gw > 13.0);

        // Light gray on White -> low contrast (< 2:1)
        let cr_lw = wcag_contrast_ratio([210, 210, 210], [255, 255, 255]);
        assert!(cr_lw < 2.0);
    }

    #[test]
    fn apca_contrast_computes_perceptual_scores() {
        // Light text on dark bg (negative score with large magnitude)
        let apca_wb = apca_contrast([255, 255, 255], [0, 0, 0]);
        assert!(apca_wb.abs() > 100.0);
        assert!(apca_wb < 0.0); // light on dark is negative polarity in APCA

        // Dark text on light bg (positive score with large magnitude)
        let apca_bw = apca_contrast([0, 0, 0], [255, 255, 255]);
        assert!(apca_bw > 100.0); // dark on light is positive polarity in APCA

        // Faint difference -> low score (< 20)
        let apca_faint = apca_contrast([200, 200, 200], [220, 220, 220]);
        assert!(apca_faint.abs() < 20.0);
    }

    #[test]
    fn is_readable_contrast_filters_unreadable_combinations() {
        // High contrast readable pairs
        assert!(is_readable_contrast([255, 255, 255], [0, 0, 0]));
        assert!(is_readable_contrast([0, 0, 0], [255, 255, 255]));
        assert!(is_readable_contrast([255, 240, 0], [20, 20, 20]));
        assert!(is_readable_contrast([220, 30, 40], [245, 245, 245]));

        // Low contrast unreadable pairs
        assert!(!is_readable_contrast([210, 210, 210], [240, 240, 240]));
        assert!(!is_readable_contrast([30, 30, 30], [20, 20, 20]));
        assert!(!is_readable_contrast([120, 120, 120], [130, 130, 130]));
    }

    #[test]
    fn best_contrast_foreground_color_preserves_readable_original_colors() {
        let bg = [20, 20, 30]; // dark blue-gray background
        let mut pixels = Vec::new();
        // 70% background pixels
        pixels.extend(vec![bg; 70]);
        // 10% transitional / anti-aliasing pixels (low contrast with bg)
        pixels.extend(vec![[50, 60, 80]; 10]);
        // 20% bright yellow text pixels
        let yellow_text = [255, 230, 10];
        pixels.extend(vec![yellow_text; 20]);

        let chosen = best_contrast_foreground_color(&pixels, bg);
        // Should accurately pick the yellow text color, filtering out the anti-aliasing pixels
        assert_eq!(chosen, yellow_text);
    }

    #[test]
    fn best_contrast_foreground_color_filters_edge_blur_on_light_background() {
        let bg = [250, 250, 250]; // near white
        let mut pixels = Vec::new();
        // 70% background
        pixels.extend(vec![bg; 70]);
        // 10% pinkish / light red anti-aliasing edge pixels
        pixels.extend(vec![[240, 180, 185]; 10]);
        // 20% deep red text pixels
        let red_text = [180, 20, 30];
        pixels.extend(vec![red_text; 20]);

        let chosen = best_contrast_foreground_color(&pixels, bg);
        assert_eq!(chosen, red_text);
    }

    #[test]
    fn adjust_color_for_readability_boosts_faint_colors_while_preserving_hue() {
        // Faint pastel green on white background
        let faint_green = [180, 230, 180];
        let bg = [255, 255, 255];
        assert!(!is_readable_contrast(faint_green, bg));

        let adjusted = adjust_color_for_readability(faint_green, bg);
        assert!(is_readable_contrast(adjusted, bg));
        // Should maintain the green dominance (G > R and G > B)
        assert!(adjusted[1] >= adjusted[0] && adjusted[1] >= adjusted[2]);
        // Should be significantly darker than the faint green
        assert!(adjusted[1] < faint_green[1]);
    }
}
