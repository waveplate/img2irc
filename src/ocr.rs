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

fn detection_dimensions(args: &RenderArgs, width: u32, height: u32) -> (u32, u32, u32) {
    let width = width.max(1);
    let height = height.max(1);
    let budget = args.ocr_detection_budget();
    if let Some(megapixels) = budget {
        let megapixels = if megapixels.is_finite() {
            megapixels.clamp(0.1, 16.0)
        } else {
            1.5
        };
        let scale = ((megapixels as f64 * 1_000_000.0) / (width as f64 * height as f64))
            .sqrt()
            .min(1.0);
        let w = (width as f64 * scale).floor().max(1.0) as u32;
        let h = (height as f64 * scale).floor().max(1.0) as u32;
        // The pixel budget replaces the second, independent longest-side cap.
        return (w, h, w.max(h));
    }
    // Explicit legacy detection sizing is also independent of output sizing.
    let w = args.ocr_width.filter(|w| *w > 0).unwrap_or(width);
    let h = (w as f64 * height as f64 / width as f64).round().max(1.0) as u32;
    (w, h, args.ocr_max_side_len)
}

pub fn detect_text(
    source: &PhotonImage,
    args: &RenderArgs,
) -> Result<Vec<OcrDetection>, Box<dyn Error>> {
    let (width, height, max_side) = detection_dimensions(args, source.get_width(), source.get_height());
    let input = if width != source.get_width() || height != source.get_height() {
        photon_rs::transform::resize(source, width, height, photon_rs::transform::SamplingFilter::Lanczos3)
    } else {
        source.clone()
    };
    let rgb = photon_to_rgb_image(&input)?;
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
            max_side,
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
                .map(|p| [
                    p.x as f32 * source.get_width() as f32 / width as f32,
                    p.y as f32 * source.get_height() as f32 / height as f32,
                ])
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

/// Word boxes remain separate for background removal. Only text fitting uses
/// their combined line box, so pixels between words are never erased by merging.
fn grouped_detections(
    detections: &[OcrDetection],
    args: &RenderArgs,
    image_w: u32,
    image_h: u32,
    multiline: bool,
) -> Vec<OcrDetection> {
    let items: Vec<_> = renderable_detections(detections, args, image_w, image_h)
        .into_iter()
        .filter_map(|det| polygon_bounds(&det.polygon, image_w, image_h).map(|bounds| (det, bounds)))
        .collect();
    let boxes: Vec<_> = items
        .iter()
        .map(|(_, (x0, y0, x1, y1))| [*x0 as f64, *y0 as f64, *x1 as f64, *y1 as f64])
        .collect();
    let groups = if multiline {
        crate::ocr_layout::group_blocks(&boxes)
    } else {
        crate::ocr_layout::group_boxes(&boxes)
            .into_iter()
            .map(|line| vec![line])
            .collect()
    };
    groups
        .into_iter()
        .map(|lines| {
            let group: Vec<usize> = lines.iter().flatten().copied().collect();
            if group.len() == 1 {
                return items[group[0]].0.clone();
            }
            let x0 = group.iter().map(|&i| items[i].1.0).min().unwrap() as f32;
            let y0 = group.iter().map(|&i| items[i].1.1).min().unwrap() as f32;
            let x1 = group.iter().map(|&i| items[i].1.2).max().unwrap() as f32;
            let y1 = group.iter().map(|&i| items[i].1.3).max().unwrap() as f32;
            OcrDetection {
                text: lines
                    .iter()
                    .map(|line| {
                        line.iter()
                            .map(|&i| items[i].0.text.trim())
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
                confidence: group.iter().map(|&i| items[i].0.confidence).fold(1.0, f32::min),
                polygon: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
            }
        })
        .collect()
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
    if !args.ocr || !args.ocr_auto_width {
        return;
    }
    let text_boxes = collect_text_box_metrics(source, args, detections);
    if text_boxes.is_empty() {
        return;
    }
    let gw = if args.braille {
        2
    } else {
        glyphs.metrics.0.max(1)
    } as u32;
    let mut automatic_args = args.clone();
    automatic_args.width = None;
    automatic_args.height = None;
    let (current_pixels, _) = crate::effects::calculate_dimensions(
        &automatic_args,
        glyphs,
        source.get_width() as f32,
        source.get_height() as f32,
    );
    let minimum = args.min_width.unwrap_or(1).max(current_pixels / gw);
    let (cols, _, required, limit) = automatic_ocr_columns(
        source.get_width(),
        gw as f32,
        minimum,
        args.max_width,
        &text_boxes,
    );
    // Dimensions applies scale after selecting the grid. Compensate here so
    // downscaling cannot undo the text-fit calculation.
    let scale_x = args.scale.map_or(1.0, |(x, _)| x);
    let scale_x = if scale_x.is_finite() && scale_x > 0.0 {
        scale_x
    } else {
        1.0
    };
    args.width = Some((cols as f64 / scale_x as f64).ceil().max(1.0) as u32);
    args.height = None;
    if required > limit {
        log::warn!("OCR needs {required} columns to fit all text, but the width limit is {limit}");
    }
    log::info!("OCR automatic render width: {cols} columns (text-fit={required})");
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
    let image_wf = image_w.max(1) as f64;
    let native_cols = (image_wf / glyph_w.max(1.0) as f64).round().max(1.0) as u32;
    let required_cols = text_boxes
        .iter()
        .map(|text_box| {
            ((text_box.text_cols as f64 * image_wf) / text_box.width() as f64)
                .ceil()
                .max(1.0) as u32
        })
        .max()
        .unwrap_or(1);
    let desired_cols = native_cols.max(required_cols).max(min_width.max(1));

    // A one-pixel-wide false-positive box should not request an unbounded
    // allocation. An explicit maximum takes precedence; otherwise allow up to
    // a generous 4096 columns, including when small source text needs upscaling.
    let automatic_safety_max = native_cols
        .max(4096)
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
    grouped_detections(detections, args, image_w, image_h, false)
        .into_iter()
        .filter_map(|det| {
            let (x0, _, x1, _) = polygon_bounds(&det.polygon, image_w, image_h)?;
            let text_cols = det.text.lines().map(|line| line.chars().count()).max().unwrap_or(0);
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

const BUNDLED_FIGLET_FONTS: &[(&str, &[u8])] = &[
    ("abraxas", include_bytes!("../static/figlet/abraxas.flf")),
    ("ansi_regular", include_bytes!("../static/figlet/ansi_regular.flf")),
    ("blocky", include_bytes!("../static/figlet/blocky.flf")),
    ("dos_rebel", include_bytes!("../static/figlet/dos_rebel.flf")),
    ("future", include_bytes!("../static/figlet/future.tlf")),
    ("mono12", include_bytes!("../static/figlet/mono12.tlf")),
    ("mono9", include_bytes!("../static/figlet/mono9.tlf")),
    ("phm-beyondneo-mono", include_bytes!("../static/figlet/phm-beyondneo-mono.flf")),
    ("phm-c64", include_bytes!("../static/figlet/phm-c64.flf")),
    ("phm-cga", include_bytes!("../static/figlet/phm-cga.flf")),
    ("phm-dos-square", include_bytes!("../static/figlet/phm-dos-square.flf")),
    ("phm-dos", include_bytes!("../static/figlet/phm-dos.flf")),
    ("phm-dosv", include_bytes!("../static/figlet/phm-dosv.flf")),
    ("phm-hdos", include_bytes!("../static/figlet/phm-hdos.flf")),
    ("phm-largetype", include_bytes!("../static/figlet/phm-largetype.flf")),
    ("phm-lcdmatrix", include_bytes!("../static/figlet/phm-lcdmatrix.flf")),
    ("phm-minecraft", include_bytes!("../static/figlet/phm-minecraft.flf")),
    ("phm-shinonome", include_bytes!("../static/figlet/phm-shinonome.flf")),
    ("phm-slanted", include_bytes!("../static/figlet/phm-slanted.flf")),
    ("phm-vga-square", include_bytes!("../static/figlet/phm-vga-square.flf")),
    ("phm-vga", include_bytes!("../static/figlet/phm-vga.flf")),
    ("smblock", include_bytes!("../static/figlet/smblock.tlf")),
    ("smmono12", include_bytes!("../static/figlet/smmono12.tlf")),
    ("smmono9", include_bytes!("../static/figlet/smmono9.tlf")),
];

fn load_figlet_from_path(path: &Path) -> Result<FIGlet, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    load_figlet_from_bytes(&bytes)
}

fn load_figlet_from_bytes(bytes: &[u8]) -> Result<FIGlet, String> {
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
        let contents = String::from_utf8(bytes.to_vec()).map_err(|e| e.to_string())?;
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
            let path = dirs
                .iter()
                .flat_map(|dir| {
                    name_variants.iter().flat_map(move |variant| {
                        ["flf", "tlf"]
                            .into_iter()
                            .map(move |extension| dir.join(format!("{variant}.{extension}")))
                    })
                })
                .find(|path| path.is_file());
            let bundled = BUNDLED_FIGLET_FONTS.iter().find(|(font, _)| {
                name_variants.iter().any(|variant| font.eq_ignore_ascii_case(variant))
            });
            let result = if let Some(path) = &path {
                load_figlet_from_path(path)
            } else if let Some((_, data)) = bundled {
                load_figlet_from_bytes(data)
            } else {
                log::debug!(
                    "OCR FIGlet/Toilet font {name:?} was not found for {} lines",
                    list.height
                );
                continue;
            };

            match result {
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
                    path.as_ref().map(|path| path.display().to_string()).unwrap_or_else(|| "bundled".to_string()),
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
    let rendered = crate::figlet_text::render(&font.font, text)?;
    normalize_text_art_with_metadata(
        font.name.clone(),
        font.configured_height,
        font.priority,
        rendered.lines().map(str::to_string).collect(),
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

fn choose_largest_text_art(
    text: &str,
    max_width: usize,
    max_height: usize,
    fonts: &[OcrFigletFont],
) -> Option<TextArt> {
    fonts
        .iter()
        .filter_map(|font| render_figlet_text(font, text))
        .filter(|art| art.width() <= max_width && art.height() <= max_height)
        .max_by_key(|art| (art.height(), art.width(), std::cmp::Reverse(art.priority)))
        .or_else(|| plain_text_art(text, max_width).filter(|art| art.height() <= max_height))
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
            if let Some(plain) = plain_text_art(text, base_width).filter(|art| art.height() <= base_height) {
                return Some(plain);
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
    plain_text_art(text, max_w_allowed).filter(|art| art.height() <= max_h_allowed)
}

fn plain_text_art(text: &str, max_width: usize) -> Option<TextArt> {
    let text = text.trim();
    if text.is_empty() || text.lines().any(|line| line.chars().count() > max_width) {
        return None;
    }
    normalize_text_art_with_metadata("plain", 1, 0, text.lines().map(str::to_string).collect())
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

    fn place(self, art_width: usize, art_height: usize, center_in_box: bool) -> (i32, i32) {
        let box_w = (self.right - self.left).max(1.0);
        let box_h = (self.bottom - self.top).max(1.0);
        let x = if center_in_box && box_w > art_width as f64 {
            self.left + (box_w - art_width as f64) * 0.5
        } else {
            self.left
        };
        let y = self.top + (box_h - art_height as f64) * 0.5;
        (x.round() as i32, y.round() as i32)
    }
}

fn snap_box_alignments(boxes: &mut [ScaledTextBox], snap_threshold: f64) {
    if boxes.len() < 2 {
        return;
    }
    let mut indices: Vec<usize> = (0..boxes.len()).collect();
    indices.sort_by(|&a, &b| boxes[a].left.total_cmp(&boxes[b].left));

    let mut clusters: Vec<Vec<usize>> = Vec::new();
    for &idx in &indices {
        let left = boxes[idx].left;
        if let Some(cluster) = clusters.last_mut() {
            let first_left = boxes[cluster[0]].left;
            let last_left = boxes[*cluster.last().unwrap()].left;
            if (left - first_left).abs() <= snap_threshold
                && (left - last_left).abs() <= snap_threshold
            {
                cluster.push(idx);
                continue;
            }
        }
        clusters.push(vec![idx]);
    }

    for cluster in clusters {
        if cluster.len() >= 2 {
            let mut lefts: Vec<f64> = cluster.iter().map(|&i| boxes[i].left).collect();
            lefts.sort_by(|a, b| a.total_cmp(b));
            let median_left = lefts[lefts.len() / 2];
            for &idx in &cluster {
                let width = boxes[idx].right - boxes[idx].left;
                boxes[idx].left = median_left;
                boxes[idx].right = median_left + width;
            }
        }
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
    let raw = source.get_raw_pixels();
    let figlet_fonts = if args.ocr_figlet {
        load_ocr_figlet_fonts(&args.ocr_figlet_fonts)
    } else {
        Vec::new()
    };

    let mut overlays = Vec::new();
    let mut renderable = grouped_detections(
        detections,
        args,
        source.get_width(),
        source.get_height(),
        true,
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

    let mut text_boxes: Vec<ScaledTextBox> = renderable.iter().map(|(_, _, b)| *b).collect();
    snap_box_alignments(&mut text_boxes, 1.5);
    for (i, (_, _, b)) in renderable.iter_mut().enumerate() {
        *b = text_boxes[i];
    }

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

        let art = if allow_figlet && args.ocr_figlet_fill {
            choose_largest_text_art(&det.text, max_w_allowed, max_h_allowed, &figlet_fonts)
        } else if allow_figlet {
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
        let Some(art) = art.filter(|art| art.height() <= max_h_allowed) else {
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
        let (placed_x, placed_y) = text_box.place(art_w as usize, art_h as usize, allow_figlet && art.source != "plain");
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
                let (px, py) = text_box.place(pw as usize, ph as usize, false);
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
            &raw,
            source.get_width(),
            source.get_height(),
            *bounds,
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
        if args.ocr_debug_boxes {
            log::info!(
                "[OCR Box #{}] text={:?} conf={:.3} img_bounds={:?} grid_box=[left={:.2}, top={:.2}, right={:.2}, bottom={:.2}, w={:.2}, h={:.2}] placed=[x={}, y={}, w={}, h={}] font={:?}",
                i + 1,
                det.text,
                det.confidence,
                bounds,
                text_box.left,
                text_box.top,
                text_box.right,
                text_box.bottom,
                text_box.right - text_box.left,
                text_box.bottom - text_box.top,
                final_x,
                final_y,
                final_w,
                final_h,
                final_art.source,
            );

            let bw = ((text_box.right - text_box.left).round() as usize).max(1);
            let bh = ((text_box.bottom - text_box.top).round() as usize).max(1);
            let bx = (text_box.left.round() as i32).clamp(0, grid_w.saturating_sub(1));
            let by = (text_box.top.round() as i32).clamp(0, grid_h.saturating_sub(1));

            let mut box_lines = Vec::new();
            let tag = format!("[#{}]", i + 1);
            if bh <= 1 {
                let line_str = if bw > tag.len() {
                    format!("{}{}", tag, "─".repeat(bw.saturating_sub(tag.len())))
                } else {
                    tag
                };
                box_lines.push(line_str);
            } else {
                let top_dashes = bw.saturating_sub(tag.len() + 2);
                let top_line = format!("┌{}{}{}", tag, "─".repeat(top_dashes), if bw >= 2 { "┐" } else { "" });
                box_lines.push(top_line);
                for _ in 1..bh.saturating_sub(1) {
                    let mid_line = if bw >= 2 {
                        format!("│{}│", " ".repeat(bw.saturating_sub(2)))
                    } else {
                        "│".to_string()
                    };
                    box_lines.push(mid_line);
                }
                if bh >= 2 {
                    let bot_line = if bw >= 2 {
                        format!("└{}┘", "─".repeat(bw.saturating_sub(2)))
                    } else {
                        "└".to_string()
                    };
                    box_lines.push(bot_line);
                }
            }
            let box_text = box_lines.join("\n");
            overlays.push(TextOverlay {
                text: box_text,
                source_text: None,
                figlet_font: None,
                x: bx,
                y: by,
                w: bw as i32,
                h: bh as i32,
                fg: Some(ColorSpec::Rgb([255, 0, 255])),
                bg: None,
                wrap: false,
                auto_grow: false,
                transparent_spaces: true,
                bold: true,
                italic: false,
                underline: false,
            });
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

    let original = source.get_raw_pixels();
    let mut raw = original.clone();
    let mut removed = 0usize;
    for det in renderable_detections(detections, args, width, height) {
        let Some(bounds) = polygon_bounds(&det.polygon, width, height) else {
            continue;
        };
        if fill_text_region_from_background_mask(&original, &mut raw, width, height, bounds) {
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

fn fill_text_region_from_background_mask(
    original: &[u8],
    raw: &mut [u8],
    image_w: u32,
    image_h: u32,
    bounds: (u32, u32, u32, u32),
) -> bool {
    let (x0, y0, x1, y1) = bounds;
    if x1 <= x0 || y1 <= y0 {
        return false;
    }

    let Some(background) =
        dominant_background_color(original, image_w, image_h, bounds)
    else {
        return false;
    };
    let threshold = foreground_threshold(original, image_w, image_h, background, bounds);
    let bw = (x1 - x0) as usize;
    let bh = (y1 - y0) as usize;
    let mut mask = vec![false; bw * bh];

    for y in y0..y1 {
        for x in x0..x1 {
            let Some(px) = rgba_at(original, image_w, x, y) else {
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

// Score colour families using separate interior, inner-edge, and exterior
// distributions. Nearby scenery must not outvote the actual text surface.
fn dominant_background_color(
    raw: &[u8],
    image_w: u32,
    image_h: u32,
    bounds: (u32, u32, u32, u32),
) -> Option<[u8; 4]> {
    let (x0, y0, x1, y1) = bounds;
    let ring = (y1.saturating_sub(y0) / 4).clamp(1, 4);
    let mut bins = std::collections::BTreeMap::<u16, ([f64; 3], [f64; 4], f64)>::new();
    let mut totals = [0.0; 3];
    for y in y0.saturating_sub(ring)..y1.saturating_add(ring).min(image_h) {
        for x in x0.saturating_sub(ring)..x1.saturating_add(ring).min(image_w) {
            let Some(px) = rgba_at(raw, image_w, x, y) else {
                continue;
            };
            if px[3] == 0 {
                continue;
            }
            let inside = x >= x0 && x < x1 && y >= y0 && y < y1;
            let edge = inside && (x == x0 || x + 1 == x1 || y == y0 || y + 1 == y1);
            let weights = [
                inside as u8 as f64,
                edge as u8 as f64,
                (!inside) as u8 as f64,
            ];
            let bin = bins
                .entry(quantized_rgb_key(px))
                .or_insert(([0.0; 3], [0.0; 4], 0.0));
            for i in 0..3 {
                bin.0[i] += weights[i];
                totals[i] += weights[i];
            }
            // Keep the representative colour anchored to the interior.
            if inside {
                for i in 0..4 {
                    bin.1[i] += px[i] as f64;
                }
                bin.2 += 1.0;
            }
        }
    }
    let score = |counts: [f64; 3]| -> f64 {
        counts[0] / totals[0].max(1.0) * 0.45
            + counts[1] / totals[1].max(1.0) * 0.45
            + counts[2] / totals[2].max(1.0) * 0.10
    };
    let mut candidates = bins
        .values()
        .filter(|bin| bin.2 > 0.0)
        .map(|bin| {
            (
                bin.1.map(|sum| (sum / bin.2).round() as u8),
                bin.0,
                bin.1,
                bin.2,
            )
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|a, b| score(b.1).total_cmp(&score(a.1)));
    let mut best = None;
    let mut best_score = -1.0;
    // Merge neighbouring colour buckets so gradients/JPEG noise do not lose
    // to a small but perfectly uniform black cluster.
    for candidate in candidates.iter().take(32) {
        let mut counts = [0.0; 3];
        let mut sums = [0.0; 4];
        let mut count = 0.0;
        for other in &candidates {
            if color_distance_sq(candidate.0, other.0) <= 48 * 48 {
                for i in 0..3 {
                    counts[i] += other.1[i];
                }
                for i in 0..4 {
                    sums[i] += other.2[i];
                }
                count += other.3;
            }
        }
        let value = score(counts);
        if value > best_score {
            best_score = value;
            best = Some(sums.map(|sum| (sum / count).round() as u8));
        }
    }
    best
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
    raw: &[u8],
    image_w: u32,
    image_h: u32,
    bounds: (u32, u32, u32, u32),
) -> [u8; 3] {
    let (x0, y0, x1, y1) = bounds;
    if x1 <= x0 || y1 <= y0 || image_w == 0 || image_h == 0 {
        return [255, 255, 255];
    }
    let bg = dominant_background_color(raw, image_w, image_h, bounds)
        .map(|px| [px[0], px[1], px[2]])
        .unwrap_or([0, 0, 0]);

    let mut bbox_pixels = Vec::with_capacity(((x1 - x0) * (y1 - y0)) as usize);
    for y in y0..y1 {
        for x in x0..x1 {
            if let Some(px) = rgba_at(raw, image_w, x, y) {
                bbox_pixels.push([px[0], px[1], px[2]]);
            }
        }
    }

    best_contrast_foreground_color(&bbox_pixels, bg)
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
    fn detection_megapixels_preserve_aspect_and_replace_legacy_resizing() {
        let mut args = default_render_args();
        args.ocr_auto_width = false;
        args.ocr_megapixels = Some(2.0);
        args.ocr_width = Some(123);
        args.ocr_max_side_len = 320;
        let (w, h, side) = detection_dimensions(&args, 4000, 2000);
        assert_eq!((w, h, side), (2000, 1000, 2000));
        assert_eq!(detection_dimensions(&args, 2000, 4000), (1000, 2000, 2000));
        assert_eq!(detection_dimensions(&args, 640, 480), (640, 480, 640));
        args.ocr_auto_width = true;
        let automatic = detection_dimensions(&args, 4000, 2000);
        args.ocr_megapixels = Some(16.0);
        assert_eq!(detection_dimensions(&args, 4000, 2000), automatic);
        assert!(automatic.0 as u64 * automatic.1 as u64 <= 3_000_000);
        args.ocr_auto_width = false;
        args.ocr_megapixels = None;
        assert_eq!(detection_dimensions(&args, 4000, 2000), (123, 62, 320));
    }

    #[test]
    fn megapixel_argument_validates_finite_positive_budgets() {
        for invalid in ["NaN", "inf", "0", "-1", "17"] {
            assert!(
                crate::args::Args::try_parse_from(["img2irc", "--ocr-megapixels", invalid]).is_err()
            );
        }
        let args = crate::args::Args::parse_from(["img2irc", "--ocr-megapixels", "2.5"]);
        assert_eq!(args.ocr_megapixels, Some(2.5));
    }

    fn detection(text: &str, bounds: (u32, u32, u32, u32)) -> OcrDetection {
        let (x0, y0, x1, y1) = bounds;
        OcrDetection {
            text: text.to_string(),
            confidence: 1.0,
            polygon: vec![
                [x0 as f32, y0 as f32],
                [x1 as f32, y0 as f32],
                [x1 as f32, y1 as f32],
                [x0 as f32, y1 as f32],
            ],
        }
    }

    fn test_glyphs() -> GlyphStore {
        GlyphStore {
            glyphs: Vec::new(),
            groups: Default::default(),
            selected: Default::default(),
            metrics: (10, 20),
            float_metrics: (9.5, 19.0),
        }
    }

    #[test]
    fn auto_width_grows_manual_dimensions_and_keeps_complete_overlays() {
        let mut args = default_render_args();
        assert!(args.ocr_auto_width);
        args.ocr = true;
        args.ocr_figlet = false;
        args.width = Some(40);
        args.height = Some(10);
        let image = PhotonImage::new(vec![255; 1000 * 300 * 4], 1000, 300);
        let detections = vec![
            detection("Twenty characters!!!", (100, 20, 200, 40)),
            detection("Short", (600, 100, 800, 120)),
        ];
        choose_text_render_geometry(&mut args, &image, &test_glyphs(), &detections);
        assert_eq!(args.width, Some(200));
        assert_eq!(args.height, None);
        let overlays = detections_to_overlays(&image, &image, &args, &test_glyphs(), &detections);
        assert_eq!(overlays.len(), 2);
        assert_eq!(overlays[0].text, detections[0].text);
        assert_eq!(overlays[0].w, 20);
        args.width = Some(999);
        choose_text_render_geometry(&mut args, &image, &test_glyphs(), &detections);
        assert_eq!(args.width, Some(200), "Inactive manual width must not affect automatic fitting");
    }

    #[test]
    fn auto_width_honors_opt_out_empty_detections_and_confidence_filter() {
        let mut args = default_render_args();
        args.ocr = true;
        args.width = Some(12);
        args.height = Some(5);
        let image = PhotonImage::new(vec![255; 100 * 40 * 4], 100, 40);
        let detections = vec![detection("A long line", (0, 0, 5, 10))];
        args.ocr_auto_width = false;
        choose_text_render_geometry(&mut args, &image, &test_glyphs(), &detections);
        assert_eq!((args.width, args.height), (Some(12), Some(5)));
        args.ocr_auto_width = true;
        choose_text_render_geometry(&mut args, &image, &test_glyphs(), &[]);
        assert_eq!((args.width, args.height), (Some(12), Some(5)));
        args.ocr_min_confidence = 1.1;
        choose_text_render_geometry(&mut args, &image, &test_glyphs(), &detections);
        assert_eq!((args.width, args.height), (Some(12), Some(5)));
    }

    #[test]
    fn adjacent_word_detections_form_one_overlay_with_spaces() {
        let words = vec![
            detection("THREE", (42, 5, 67, 15)),
            detection("ONE", (10, 5, 25, 15)),
            detection("TWO", (26, 5, 41, 15)),
        ];
        let mut args = default_render_args();
        args.ocr_figlet = false;
        args.width = Some(100);
        args.height = Some(30);
        let source = PhotonImage::new(vec![255; 100 * 30 * 4], 100, 30);
        let metrics = collect_text_box_metrics(&source, &args, &words);
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].text_cols, 13);
        let overlays = detections_to_overlays(&source, &source, &args, &test_glyphs(), &words);
        assert_eq!(overlays.len(), 1);
        assert_eq!(overlays[0].text, "ONE TWO THREE");
    }

    #[test]
    fn adjacent_lines_form_one_multiline_overlay_but_width_fits_each_line() {
        let words = vec![detection("FIRST LINE", (10, 10, 80, 30)), detection("SECOND LINE", (10, 35, 80, 55))];
        let mut args = default_render_args();
        args.ocr_figlet = false;
        args.width = Some(100);
        args.height = Some(100);
        let source = PhotonImage::new(vec![255; 100 * 100 * 4], 100, 100);
        let metrics = collect_text_box_metrics(&source, &args, &words);
        assert_eq!(metrics.len(), 2);
        let overlays = detections_to_overlays(&source, &source, &args, &test_glyphs(), &words);
        assert_eq!(overlays.len(), 1);
        assert_eq!(
            overlays[0].text.lines().map(str::trim_end).collect::<Vec<_>>(),
            ["FIRST LINE", "SECOND LINE"]
        );
        assert_eq!(overlays[0].h, 2);
    }

    #[test]
    fn grouped_words_leave_the_gap_in_the_source_untouched() {
        let words = vec![detection("ONE", (10, 5, 38, 15)), detection("TWO", (43, 5, 70, 15))];
        let mut raw = vec![255; 100 * 20 * 4];
        set_rgba(&mut raw, 100, 40, 10, [200, 0, 0, 255]);
        let source = PhotonImage::new(raw, 100, 20);
        let args = default_render_args();
        assert_eq!(grouped_detections(&words, &args, 100, 20, true).len(), 1);
        let cleaned = remove_detected_text(&source, &args, &words).get_raw_pixels();
        assert_eq!(&cleaned[(10 * 100 + 40) * 4..][..4], &[200, 0, 0, 255]);
    }

    #[test]
    fn auto_width_accounts_for_downscaling_and_braille_cells() {
        let image = PhotonImage::new(vec![255; 100 * 40 * 4], 100, 40);
        for braille in [false, true] {
            let mut args = default_render_args();
            args.ocr = true;
            args.braille = braille;
            args.width = Some(10);
            args.scale = Some((0.5, 0.5));
            let detections = vec![detection("0123456789", (10, 10, 20, 20))];
            choose_text_render_geometry(&mut args, &image, &test_glyphs(), &detections);
            let (pw, _) = crate::effects::calculate_dimensions(&args, &test_glyphs(), 100.0, 40.0);
            let cols = pw / if braille { 2 } else { 10 };
            assert!(cols >= 100);
        }
    }

    #[test]
    fn background_uses_inner_surface_even_with_dense_text_and_black_surroundings() {
        let bounds = (10, 5, 50, 15);
        for (surface, ink) in [
            ([245, 240, 230, 255], [0, 0, 0, 255]),
            ([15, 20, 25, 255], [255, 255, 255, 255]),
        ] {
            let mut raw = [0, 0, 0, 255].repeat(60 * 20);
            for y in 5..15 {
                for x in 10..50 {
                    let pixel = if y > 5 && y < 14 && x > 10 && x < 49 {
                        ink
                    } else {
                        surface
                    };
                    set_rgba(&mut raw, 60, x, y, pixel);
                }
            }
            assert_eq!(
                dominant_background_color(&raw, 60, 20, bounds),
                Some(surface)
            );
            let source = PhotonImage::new(raw.clone(), 60, 20);
            let cleaned = remove_detected_text(
                &source,
                &default_render_args(),
                &[detection("TEXT", bounds)],
            );
            let result = cleaned.get_raw_pixels();
            assert_eq!(rgba_at(&result, 60, 30, 10), Some(surface));
            assert_eq!(rgba_at(&result, 60, 9, 10), rgba_at(&raw, 60, 9, 10));
            assert_eq!(
                estimate_text_color(&raw, 60, 20, bounds),
                [ink[0], ink[1], ink[2]]
            );
        }
    }

    #[test]
    fn background_at_image_edges_does_not_fall_back_to_the_first_pixel() {
        let surface = [240, 230, 210, 255];
        let mut raw = surface.repeat(20 * 10);
        set_rgba(&mut raw, 20, 0, 0, [0, 0, 0, 255]);
        assert_eq!(
            dominant_background_color(&raw, 20, 10, (0, 0, 20, 10)),
            Some(surface)
        );
        assert_eq!(
            dominant_background_color(&vec![0; 20 * 10 * 4], 20, 10, (0, 0, 20, 10)),
            None
        );
    }

    #[test]
    fn background_merges_nearby_shades_instead_of_choosing_uniform_black_text() {
        let mut raw = Vec::new();
        for y in 0..20 {
            for x in 0..60 {
                let gray = if y > 5 && y < 15 && x % 3 == 0 {
                    0
                } else {
                    190 + (x % 40) as u8
                };
                raw.extend_from_slice(&[gray, gray, gray, 255]);
            }
        }
        let bg = dominant_background_color(&raw, 60, 20, (0, 0, 60, 20)).unwrap();
        assert!(bg[0] >= 190 && bg[0] <= 229, "{bg:?}");
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

        let tiny_box = [TextBoxMetric { x0: 0.0, x1: 2.0, text_cols: 20 }];
        assert_eq!(automatic_ocr_columns(200, 10.0, 1, None, &tiny_box).0, 2000);
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
        // small is centered in a 20-col grid (center = 10, box center = 10)
        assert_eq!(small.place(10, 2, false), (5, 2));
        assert_eq!(large.place(20, 4, false), (10, 4));
    }

    #[test]
    fn test_alignment_snapping_and_selective_centering() {
        // Box spanning 30..50 (width 20)
        let centered_box = ScaledTextBox {
            left: 30.0,
            top: 5.0,
            right: 50.0,
            bottom: 9.0,
        };
        // FIGlet art with width 10 is centered horizontally inside box (30 + (20 - 10)/2 = 35)
        assert_eq!(centered_box.place(10, 4, true), (35, 5));
        // Plain text with width 10 remains left-aligned at box left (30)
        assert_eq!(centered_box.place(10, 4, false), (30, 5));

        // Off-center column box
        let left_box_a = ScaledTextBox {
            left: 5.2,
            top: 10.0,
            right: 18.0,
            bottom: 11.0,
        };
        let left_box_b = ScaledTextBox {
            left: 4.8,
            top: 12.0,
            right: 25.0,
            bottom: 13.0,
        };
        // Plain text is left-aligned with the box's left coordinate
        assert_eq!(left_box_a.place(8, 1, false), (5, 10));
        assert_eq!(left_box_b.place(15, 1, false), (5, 12));

        // Snap alignments
        let mut boxes = vec![left_box_a, left_box_b];
        snap_box_alignments(&mut boxes, 1.5);
        assert_eq!(boxes[0].left, 5.2); // Median left of cluster
        assert_eq!(boxes[1].left, 5.2);
    }

    #[test]
    fn fill_figlet_uses_expansion_even_when_a_smaller_font_already_fits() {
        let fonts = vec![
            OcrFigletFont { name: "small".into(), configured_height: 5, priority: 0, font: FIGlet::small().unwrap() },
            OcrFigletFont { name: "standard".into(), configured_height: 6, priority: 1, font: FIGlet::standard().unwrap() },
        ];
        let small = render_figlet_text(&fonts[0], "HELLO").unwrap();
        let large = render_figlet_text(&fonts[1], "HELLO").unwrap();
        assert!(large.height() > small.height());
        let width = small.width().max(large.width());
        let original = choose_text_art_from_fonts("HELLO", width, 5, width, 6, &[], &fonts).unwrap();
        assert_eq!(original.source, "small");
        let filled = choose_largest_text_art("HELLO", width, 6, &fonts).unwrap();
        assert_eq!(filled.source, "standard");
        assert_eq!(choose_largest_text_art("HELLO", width, small.height(), &fonts).unwrap().source, "small");
        assert_eq!(choose_largest_text_art("HELLO", 5, 1, &fonts).unwrap().source, "plain");
        assert!(choose_largest_text_art("HELLO", 4, 1, &fonts).is_none());
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
