//! A complete, inexpensive font-size sweep shared by native and browser UIs.
use crate::args::RenderArgs;
use serde::Serialize;

pub const MIN_SIZE: u32 = 9;
pub const MAX_SIZE: u32 = 30;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FontScanResult {
    pub font_size: f32,
    pub score: f64,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FontScanProgress {
    pub completed: u32,
    pub total: u32,
    pub font_size: f32,
    pub score: Option<f64>,
    pub best: Option<FontScanResult>,
}

pub fn quick_args(mut args: RenderArgs) -> RenderArgs {
    args.score = true;
    args.perceptual_score = true;
    args.smooth = false;
    args.glyph_search = false;
    args.region_guidance = false;
    args.superpixel_warp = 0;
    args.ocr = false;
    args.show_glyph_defects = false;
    args.overlays.clear();
    args.save = None;
    args
}

/// Evaluate every integer size, accepting only finite scores. Equal scores
/// retain the smaller size. No early exit or cached score can skip a candidate.
pub fn scan<E>(
    mut evaluate: impl FnMut(f32) -> Result<f64, E>,
    mut progress: impl FnMut(FontScanProgress),
) -> Result<Option<FontScanResult>, E> {
    let mut best: Option<FontScanResult> = None;
    for size in MIN_SIZE..=MAX_SIZE {
        let score = evaluate(size as f32)?;
        if score.is_finite() && best.is_none_or(|current| score < current.score) {
            best = Some(FontScanResult {
                font_size: size as f32,
                score,
            });
        }
        progress(FontScanProgress {
            completed: size - MIN_SIZE + 1,
            total: MAX_SIZE - MIN_SIZE + 1,
            font_size: size as f32,
            score: score.is_finite().then_some(score),
            best,
        });
    }
    Ok(best)
}
