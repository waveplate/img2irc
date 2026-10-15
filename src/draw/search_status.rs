//! Search diagnostics and a scoped observer for synchronous render calls.

use super::{RenderInstant, RenderResult, RenderedCell};
use serde::Serialize;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlyphDefectMap {
    pub columns: usize,
    pub rows: usize,
    pub threshold: f32,
    pub priorities: Vec<f32>,
    pub targets: Vec<bool>,
    pub active: Vec<bool>,
}

impl GlyphDefectMap {
    pub fn target_count(&self) -> usize {
        self.targets.iter().filter(|&&value| value).count()
    }

    pub fn neighbor_count(&self) -> usize {
        self.active
            .iter()
            .zip(&self.targets)
            .filter(|&(&active, &target)| active && !target)
            .count()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GlyphSearchStage {
    Analyzing,
    Candidates,
    Searching,
    Verifying,
    Finished,
}

pub struct GlyphSearchPreview {
    pub defects: Arc<GlyphDefectMap>,
    pub result: RenderResult,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlyphSearchProgress {
    pub stage: GlyphSearchStage,
    pub pass: u32,
    pub passes: u32,
    pub pairs_done: usize,
    pub pairs_total: usize,
    pub pair_work_done: usize,
    pub pair_work_total: usize,
    pub combinations: usize,
    pub full_checks: usize,
    pub moves: usize,
    pub skipped: usize,
    pub score: f64,
    pub elapsed_ms: f64,
    pub message: String,
    #[serde(skip)]
    pub preview: Option<Arc<GlyphSearchPreview>>,
}

impl GlyphSearchProgress {
    pub(super) fn new(passes: u32) -> Self {
        Self {
            stage: GlyphSearchStage::Analyzing,
            pass: 1,
            passes,
            pairs_done: 0,
            pairs_total: 0,
            pair_work_done: 0,
            pair_work_total: 0,
            combinations: 0,
            full_checks: 0,
            moves: 0,
            skipped: 0,
            score: 0.0,
            elapsed_ms: 0.0,
            message: String::new(),
            preview: None,
        }
    }

    /// Fraction of the current pass, including work inside a large pair pool.
    /// The number of passes is a maximum: early convergence can finish sooner.
    pub fn fraction(&self) -> f64 {
        if self.stage == GlyphSearchStage::Finished {
            return 1.0;
        }
        if self.stage != GlyphSearchStage::Searching || self.pairs_total == 0 {
            return 0.0;
        }
        let inside = self.pair_work_done as f64 / self.pair_work_total.max(1) as f64;
        ((self.pairs_done as f64 + inside) / self.pairs_total as f64).clamp(0.0, 1.0)
    }

    pub(super) fn publish(&mut self, started: &RenderInstant) {
        self.elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        let stage = match self.stage {
            GlyphSearchStage::Analyzing => "mapping defects".to_string(),
            GlyphSearchStage::Candidates => "preparing candidates".to_string(),
            GlyphSearchStage::Searching => format!(
                "{:.0}% ({}/{} pairs)",
                self.fraction() * 100.0,
                self.pairs_done,
                self.pairs_total,
            ),
            GlyphSearchStage::Verifying => "verifying result".to_string(),
            GlyphSearchStage::Finished => "finished".to_string(),
        };
        self.message = format!(
            "Edge repair {}/{}: {} | {:.1}s | score {:.3} | {} moves",
            self.pass,
            self.passes,
            stage,
            self.elapsed_ms / 1000.0,
            self.score,
            self.moves,
        );
        OBSERVER.with(|slot| {
            let observer = slot.borrow().clone();
            if let Some(observer) = observer {
                (observer.callback)(self);
            }
        });
    }
}

#[derive(Clone)]
struct Observer {
    callback: Rc<dyn Fn(&GlyphSearchProgress)>,
    include_preview: bool,
}

thread_local! {
    static OBSERVER: RefCell<Option<Observer>> = RefCell::new(None);
}

pub(super) fn wants_preview() -> bool {
    OBSERVER.with(|slot| slot.borrow().as_ref().is_some_and(|observer| observer.include_preview))
}

/// Observe a synchronous render on this thread. The previous observer is
/// restored even on panic, and callbacks run outside the thread-local borrow.
/// Parallel glyph preparation does not call the observer from worker threads.
pub fn observe_glyph_search<T>(
    observer: impl Fn(&GlyphSearchProgress) + 'static,
    render: impl FnOnce() -> T,
) -> T {
    observe_glyph_search_with_preview(observer, true, render)
}

/// Progress-only consumers avoid building live grid snapshots they do not use.
pub(crate) fn observe_glyph_search_with_preview<T>(
    observer: impl Fn(&GlyphSearchProgress) + 'static,
    include_preview: bool,
    render: impl FnOnce() -> T,
) -> T {
    struct Restore(Option<Observer>);
    impl Drop for Restore {
        fn drop(&mut self) {
            OBSERVER.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(OBSERVER.with(|slot| slot.replace(Some(Observer {
        callback: Rc::new(observer), include_preview,
    }))));
    render()
}

/// Diagnostic colours apply to a display copy, preserving the normal result
/// for scoring, copy/save and image export. Red marks targets; amber the halo.
pub fn highlight_glyph_defects(base: &RenderResult, map: &GlyphDefectMap) -> RenderResult {
    if base.grid.len() != map.rows
        || base.grid.iter().any(|row| row.len() != map.columns)
        || map.targets.len() != map.columns * map.rows
        || map.active.len() != map.targets.len()
    {
        return base.clone();
    }
    let mut result = base.clone();
    let mix = |colour: [u8; 3], tint: [u8; 3]| {
        std::array::from_fn(|c| ((colour[c] as u16 * 2 + tint[c] as u16 * 3) / 5) as u8)
    };
    for (i, cell) in result.grid.iter_mut().flatten().enumerate() {
        let tint = if map.targets[i] {
            [255, 45, 65]
        } else if map.active[i] {
            [235, 165, 35]
        } else {
            continue;
        };
        cell.fg = mix(cell.fg, tint);
        cell.bg = Some(mix(cell.bg.unwrap_or([0; 3]), tint));
    }
    super::encode_output_colours(&result, crate::args::Render::Ansi24, true, 0)
}

pub(super) fn preview_cell(ch: char, fg: [u8; 3], bg: [u8; 3], inverted: bool) -> RenderedCell {
    RenderedCell {
        char: ch,
        fg: if inverted { bg } else { fg },
        bg: Some(if inverted { fg } else { bg }),
        // The colours above are already in display order, just like the
        // completed render. Structured viewers must not swap them again.
        inverted: false,
        bold: false,
        italic: false,
        underline: false,
    }
}
