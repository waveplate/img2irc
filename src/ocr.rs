use crate::args::{ColorSpec, OcrModelTier, RenderArgs, TextOverlay};
use crate::font::GlyphStore;
use image::RgbImage;
use photon_rs::PhotonImage;
use ppocr_rs::{
    DocOrientationClassifier, ModelHub, OcrLite, OcrOptions, PpOcrVersion, PpStructureModel,
};
use std::error::Error;
use std::io;
use std::path::Path;

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
        suppress_stdout(|| -> Result<(), Box<dyn Error>> {
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
        suppress_stdout(|| -> Result<(), Box<dyn Error>> {
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
        suppress_stdout(|| -> Result<(), Box<dyn Error>> {
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

    let result = suppress_stdout(|| -> Result<_, Box<dyn Error>> {
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

    let min_w = args.min_width.unwrap_or(20);
    let max_w = args.max_width.unwrap_or(100);

    let (cols, rows) = match (args.width, args.height) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, rows_for_cols(w)),
        (None, Some(h)) => (cols_for_rows(h), h),
        (None, None) => {
            if let Some((cols, rows, char_pitch, line_pitch)) =
                infer_text_grid_geometry(source, args, glyphs, detections)
            {
                let clamped_cols = cols.clamp(min_w, max_w);
                let clamped_rows = if clamped_cols != cols {
                    rows_for_cols(clamped_cols)
                } else {
                    rows
                };
                log::info!(
                    "OCR-inferred text grid: {clamped_cols}x{clamped_rows} cells (char_pitch={:.2}px line_pitch={:.2}px input_aspect={:.4} output_aspect={:.4})",
                    char_pitch,
                    line_pitch,
                    image_aspect,
                    (clamped_cols as f32 * gw) / (clamped_rows as f32 * gh)
                );
                args.width = Some(clamped_cols);
                args.height = Some(clamped_rows);
                return;
            }

            let mut min_cols = 1u32;
            let mut min_rows = 1u32;

            for det in renderable_detections(detections, args, source.get_width(), source.get_height()) {
                let Some((min_x, min_y, _max_x, max_y)) =
                    polygon_bounds(&det.polygon, source.get_width(), source.get_height())
                else {
                    continue;
                };
                let text_cols = det.text.chars().count().max(1) as u32;
                let text_rows = (((max_y - min_y).max(1) as f32 / gh).ceil().max(1.0)) as u32;
                let left_frac = (min_x as f32 / image_w).clamp(0.0, 0.98);
                let top_frac = (min_y as f32 / image_h).clamp(0.0, 0.98);
                min_cols = min_cols.max(((text_cols + 1) as f32 / (1.0 - left_frac)).ceil() as u32);
                min_rows = min_rows.max(((text_rows + 1) as f32 / (1.0 - top_frac)).ceil() as u32);
            }

            if min_cols == 1 && min_rows == 1 {
                min_cols = 80;
            }

            let mut cols = min_cols;
            let mut rows = min_rows.max(rows_for_cols(cols));
            cols = cols.max(cols_for_rows(rows));
            rows = rows.max(rows_for_cols(cols));

            let clamped_cols = cols.clamp(min_w, max_w);
            let clamped_rows = if clamped_cols != cols {
                rows_for_cols(clamped_cols)
            } else {
                rows
            };
            (clamped_cols, clamped_rows)
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
    y0: f32,
    x1: f32,
    y1: f32,
    text_cols: usize,
}

impl TextBoxMetric {
    fn width(&self) -> f32 {
        (self.x1 - self.x0).max(1.0)
    }

    fn height(&self) -> f32 {
        (self.y1 - self.y0).max(1.0)
    }

    fn center_y(&self) -> f32 {
        (self.y0 + self.y1) * 0.5
    }
}

fn infer_text_grid_geometry(
    source: &PhotonImage,
    args: &RenderArgs,
    glyphs: &GlyphStore,
    detections: &[OcrDetection],
) -> Option<(u32, u32, f32, f32)> {
    let image_w = source.get_width().max(1) as f32;
    let image_h = source.get_height().max(1) as f32;
    let (gw, gh) = glyphs.metrics;
    let gw = gw.max(1) as f32;
    let gh = gh.max(1) as f32;
    let boxes = collect_text_box_metrics(source, args, detections);
    if boxes.len() < 2 {
        return None;
    }

    let mut char_pitch_candidates = boxes
        .iter()
        .filter(|b| b.text_cols >= 3)
        .map(|b| b.width() / b.text_cols as f32)
        .filter(|pitch| pitch.is_finite() && *pitch >= 2.0 && *pitch <= image_w * 0.5)
        .collect::<Vec<_>>();
    if char_pitch_candidates.len() < 2 {
        return None;
    }
    let char_pitch = dominant_f32(&mut char_pitch_candidates, 1.0)?;

    let mut heights = boxes.iter().map(TextBoxMetric::height).collect::<Vec<_>>();
    let median_height = median_f32(&mut heights)?;
    let line_centers = clustered_line_centers(&boxes, median_height);
    if line_centers.len() < 2 {
        return None;
    }

    let mut line_pitch_candidates = line_centers
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .filter(|pitch| {
            pitch.is_finite()
                && *pitch >= (median_height * 0.75).max(2.0)
                && *pitch <= image_h * 0.5
        })
        .collect::<Vec<_>>();
    if line_pitch_candidates.is_empty() {
        return None;
    }
    let line_pitch = dominant_f32(&mut line_pitch_candidates, 2.0)?;

    let mut cols = (image_w / char_pitch).ceil().max(1.0) as u32;
    let mut rows = (image_h / line_pitch).ceil().max(1.0) as u32;
    for b in &boxes {
        let x = (b.x0 / char_pitch).floor().max(0.0) as u32;
        let y = (b.y0 / line_pitch).floor().max(0.0) as u32;
        cols = cols.max(x.saturating_add(b.text_cols as u32).saturating_add(1));
        rows = rows.max(y.saturating_add(1));
    }

    let image_aspect = image_w / image_h;
    let output_aspect = (cols as f32 * gw) / (rows as f32 * gh);
    let aspect_error = ((output_aspect / image_aspect) - 1.0).abs();
    if aspect_error > 0.35 {
        log::info!(
            "Ignoring OCR-inferred text grid because aspect drift is too high: cols={cols} rows={rows} input_aspect={:.4} output_aspect={:.4}",
            image_aspect,
            output_aspect
        );
        return None;
    }

    Some((cols, rows, char_pitch, line_pitch))
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
            let (x0, y0, x1, y1) = polygon_bounds(&det.polygon, image_w, image_h)?;
            let text_cols = det.text.chars().count();
            if text_cols == 0 {
                return None;
            }
            Some(TextBoxMetric {
                x0: x0 as f32,
                y0: y0 as f32,
                x1: x1 as f32,
                y1: y1 as f32,
                text_cols,
            })
        })
        .collect()
}

fn clustered_line_centers(boxes: &[TextBoxMetric], median_height: f32) -> Vec<f32> {
    let mut centers = boxes
        .iter()
        .map(TextBoxMetric::center_y)
        .filter(|center| center.is_finite())
        .collect::<Vec<_>>();
    centers.sort_by(|a, b| a.total_cmp(b));
    let threshold = (median_height * 0.65).clamp(2.0, median_height.max(2.0));
    let mut clusters: Vec<(f32, usize)> = Vec::new();

    for center in centers {
        if let Some((sum, count)) = clusters.last_mut() {
            let mean = *sum / *count as f32;
            if (center - mean).abs() <= threshold {
                *sum += center;
                *count += 1;
                continue;
            }
        }
        clusters.push((center, 1));
    }

    clusters
        .into_iter()
        .map(|(sum, count)| sum / count as f32)
        .collect()
}

fn overlay_line_rows(
    source: &PhotonImage,
    args: &RenderArgs,
    detections: &[OcrDetection],
    grid_h: i32,
) -> Vec<(f32, i32)> {
    if grid_h <= 0 {
        return Vec::new();
    }

    let boxes = collect_text_box_metrics(source, args, detections);
    if boxes.is_empty() {
        return Vec::new();
    }

    let mut heights = boxes.iter().map(TextBoxMetric::height).collect::<Vec<_>>();
    let Some(median_height) = median_f32(&mut heights) else {
        return Vec::new();
    };
    let threshold = (median_height * 0.65).clamp(2.0, median_height.max(2.0));
    let mut sorted = boxes;
    sorted.sort_by(|a, b| a.center_y().total_cmp(&b.center_y()));

    let mut clusters: Vec<(f32, usize, f32, f32)> = Vec::new(); // (sum_of_centers, count, min_y, max_y)
    for b in sorted {
        let center = b.center_y();
        if let Some((sum, count, min_y, max_y)) = clusters.last_mut() {
            let mean = *sum / *count as f32;
            if (center - mean).abs() <= threshold {
                *sum += center;
                *count += 1;
                *min_y = min_y.min(b.y0);
                *max_y = max_y.max(b.y1);
                continue;
            }
        }
        clusters.push((center, 1, b.y0, b.y1));
    }

    let image_h = source.get_height().max(1) as f32;

    // Segment clusters into groups of consecutive lines based on overlap or proximity of their bounding boxes
    let mut groups: Vec<Vec<(f32, f32, f32)>> = Vec::new(); // each group is a Vec of (center, min_y, max_y)
    let threshold_gap = median_height * 0.25;

    for (sum, count, min_y, max_y) in clusters {
        let center = sum / count as f32;
        if let Some(last_group) = groups.last_mut() {
            if let Some(&(_, _, last_max_y)) = last_group.last() {
                // If the top of the current line is very close to or overlaps with the bottom of the previous line
                if min_y - last_max_y <= threshold_gap {
                    last_group.push((center, min_y, max_y));
                    continue;
                }
            }
        }
        groups.push(vec![(center, min_y, max_y)]);
    }

    // Map each group's lines to rows, centering around the middle line
    let mut rows_to_adjust = Vec::new();
    for group in groups {
        let n = group.len();
        if n == 1 {
            let (center, _, _) = group[0];
            let row = ((center / image_h) * grid_h as f32).floor() as i32;
            rows_to_adjust.push((center, row));
        } else {
            let c_first = group[0].0;
            let c_last = group[n - 1].0;
            let d = (c_last - c_first) / (n - 1) as f32;
            let d_rows = (d / image_h) * grid_h as f32;
            let s = d_rows.round().max(1.0) as i32;

            let m = n / 2;
            let c_m = group[m].0;
            let row_m = ((c_m / image_h) * grid_h as f32).floor() as i32;

            for (idx, &(center, _, _)) in group.iter().enumerate() {
                let row = row_m + (idx as i32 - m as i32) * s;
                rows_to_adjust.push((center, row));
            }
        }
    }

    // Perform sequential adjustment to prevent overlap and clamp to grid boundaries
    let mut rows = Vec::with_capacity(rows_to_adjust.len());
    let mut last_row = -1;
    for (center, mut row) in rows_to_adjust {
        row = row.clamp(0, grid_h - 1);
        if row <= last_row && last_row + 1 < grid_h {
            row = last_row + 1;
        }
        last_row = row;
        rows.push((center, row));
    }
    rows
}

fn median_f32(values: &mut Vec<f32>) -> Option<f32> {
    values.retain(|value| value.is_finite());
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    Some(values[values.len() / 2])
}

fn dominant_f32(values: &mut Vec<f32>, tolerance: f32) -> Option<f32> {
    values.retain(|value| value.is_finite());
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.total_cmp(b));

    let mut best_sum = 0.0;
    let mut best_count = 0usize;
    let mut best_center = f32::INFINITY;
    let mut sum = 0.0;
    let mut count = 0usize;
    let mut center = 0.0;

    for &value in values.iter() {
        if count == 0 {
            sum = value;
            count = 1;
            center = value;
            continue;
        }

        if (value - center).abs() <= tolerance {
            sum += value;
            count += 1;
            center = sum / count as f32;
            continue;
        }

        if count > best_count || (count == best_count && center < best_center) {
            best_sum = sum;
            best_count = count;
            best_center = center;
        }
        sum = value;
        count = 1;
        center = value;
    }

    if count > best_count || (count == best_count && center < best_center) {
        best_sum = sum;
        best_count = count;
    }

    if best_count == 0 {
        None
    } else {
        Some(best_sum / best_count as f32)
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
    let line_rows = overlay_line_rows(source, args, detections, grid_h);

    let mut overlays = Vec::new();
    let mut readability_adjustments = 0usize;
    for det in renderable_detections(detections, args, source.get_width(), source.get_height()) {
        let Some(bounds @ (min_x, min_y, max_x, max_y)) =
            polygon_bounds(&det.polygon, source.get_width(), source.get_height())
        else {
            continue;
        };

        let image_w = source.get_width().max(1) as f32;
        let image_h = source.get_height().max(1) as f32;
        let x = ((min_x as f32 / image_w) * grid_w as f32).round() as i32;
        let center_y = (min_y + max_y) as f32 * 0.5;
        let y = line_rows
            .iter()
            .min_by(|(a, _), (b, _)| (a - center_y).abs().total_cmp(&(b - center_y).abs()))
            .map(|(_, row)| *row)
            .unwrap_or_else(|| ((center_y / image_h) * grid_h as f32).floor() as i32);
        if x >= grid_w || y >= grid_h {
            continue;
        }

        let x2 = ((max_x as f32 / image_w) * grid_w as f32).round() as i32;
        let text_width = det.text.chars().count().max(1) as i32;
        let w = (x2 - x)
            .max(text_width)
            .min(grid_w.saturating_sub(x).max(1));
        let fg = estimate_text_color(
            &rgb,
            source.get_width(),
            source.get_height(),
            bounds,
            global_bg,
        );
        let line_bg = dominant_rgb_in_rect(
            &background_rgb,
            background_source.get_width(),
            background_source.get_height(),
            expand_text_bounds(
                bounds,
                background_source.get_width(),
                background_source.get_height(),
            ),
        )
        .unwrap_or(global_bg);

        let chars: Vec<char> = det.text.chars().take(w.max(0) as usize).collect();
        if chars.is_empty() {
            continue;
        }

        let mut cell_backgrounds = Vec::with_capacity(chars.len());
        let mut readable_backgrounds = Vec::new();
        for (idx, ch) in chars.iter().enumerate() {
            let cell_x = x + idx as i32;
            if cell_x < 0 || cell_x >= grid_w {
                cell_backgrounds.push(line_bg);
                continue;
            }
            let bg = sample_cell_background(
                &background_rgb,
                background_source.get_width(),
                background_source.get_height(),
                grid_w,
                grid_h,
                cell_x,
                y,
                line_bg,
            );
            if !ch.is_whitespace() && is_readable_text_pair(fg, bg) {
                readable_backgrounds.push((idx, bg));
            }
            cell_backgrounds.push(bg);
        }

        let mut resolved_backgrounds = Vec::with_capacity(chars.len());
        for (idx, ch) in chars.iter().enumerate() {
            let mut bg = cell_backgrounds[idx];
            if !ch.is_whitespace() && !is_readable_text_pair(fg, bg) {
                if let Some((_, readable_bg)) = readable_backgrounds
                    .iter()
                    .min_by_key(|(readable_idx, _)| readable_idx.abs_diff(idx))
                {
                    bg = *readable_bg;
                } else {
                    bg = strongest_contrast_background(fg);
                }
                readability_adjustments += 1;
            }
            resolved_backgrounds.push(bg);
        }

        push_overlay_runs(
            &mut overlays,
            &chars,
            &resolved_backgrounds,
            x,
            y.max(0),
            grid_w,
            fg,
        );
    }

    overlays.sort_by_key(|ov| (ov.y, ov.x));
    if readability_adjustments > 0 {
        log::info!(
            "Adjusted {readability_adjustments} OCR overlay cell backgrounds for text readability"
        );
    }
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
            let paths = suppress_stdout(|| {
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
            None => suppress_stdout(|| hub.ensure_single(PpStructureModel::DocOrientation))?.onnx,
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
    if image_w == 0 || image_h == 0 || grid_w <= 0 || grid_h <= 0 {
        return fallback;
    }

    let image_wf = image_w as f32;
    let image_hf = image_h as f32;
    let grid_wf = grid_w as f32;
    let grid_hf = grid_h as f32;
    let x0 = ((cell_x.max(0) as f32 / grid_wf) * image_wf).floor() as u32;
    let x1 = ((((cell_x + 1).max(1) as f32 / grid_wf) * image_wf).ceil() as u32)
        .clamp((x0 + 1).min(image_w), image_w);
    let y0 = ((cell_y.max(0) as f32 / grid_hf) * image_hf).floor() as u32;
    let y1 = ((((cell_y + 1).max(1) as f32 / grid_hf) * image_hf).ceil() as u32)
        .clamp((y0 + 1).min(image_h), image_h);

    let pad_x = ((x1 - x0).max(1) / 4).clamp(1, 4);
    let pad_y = ((y1 - y0).max(1) / 4).clamp(1, 4);
    let sample_bounds = (
        x0.saturating_sub(pad_x),
        y0.saturating_sub(pad_y),
        x1.saturating_add(pad_x).min(image_w),
        y1.saturating_add(pad_y).min(image_h),
    );

    dominant_rgb_in_rect(rgb, image_w, image_h, sample_bounds).unwrap_or(fallback)
}

fn dominant_rgb_in_rect(
    rgb: &[[u8; 3]],
    image_w: u32,
    image_h: u32,
    bounds: (u32, u32, u32, u32),
) -> Option<[u8; 3]> {
    let (x0, y0, x1, y1) = bounds;
    if x1 <= x0 || y1 <= y0 || image_w == 0 || image_h == 0 {
        return None;
    }

    let mut bins = std::collections::HashMap::<u16, (u32, [u64; 3])>::new();
    for y in y0..y1.min(image_h) {
        let row = y as usize * image_w as usize;
        for x in x0..x1.min(image_w) {
            let Some(px) = rgb.get(row + x as usize).copied() else {
                continue;
            };
            let key = quantized_rgb3_key(px);
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

fn quantized_rgb3_key(px: [u8; 3]) -> u16 {
    ((px[0] as u16 >> 4) << 8) | ((px[1] as u16 >> 4) << 4) | (px[2] as u16 >> 4)
}

fn is_readable_text_pair(fg: [u8; 3], bg: [u8; 3]) -> bool {
    contrast_ratio(fg, bg) >= 3.0
}

fn strongest_contrast_background(fg: [u8; 3]) -> [u8; 3] {
    let black = [0, 0, 0];
    let white = [255, 255, 255];
    if contrast_ratio(fg, black) >= contrast_ratio(fg, white) {
        black
    } else {
        white
    }
}

fn contrast_ratio(a: [u8; 3], b: [u8; 3]) -> f32 {
    let l1 = relative_luminance(a);
    let l2 = relative_luminance(b);
    let (hi, lo) = if l1 >= l2 { (l1, l2) } else { (l2, l1) };
    (hi + 0.05) / (lo + 0.05)
}

fn relative_luminance(rgb: [u8; 3]) -> f32 {
    fn channel(v: u8) -> f32 {
        let s = v as f32 / 255.0;
        if s <= 0.03928 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(rgb[0]) + 0.7152 * channel(rgb[1]) + 0.0722 * channel(rgb[2])
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
