//! Optional video warm start: carry repaired glyphs through unchanged areas and
//! spend the next search on changed cells plus a two-cell surrounding margin.
//! The caller validates the whole warm start and final image against the fresh
//! initial fit, so a stale guess never bypasses the perceptual acceptance rule.
use super::*;
use std::cell::RefCell;

#[derive(Default)]
struct History {
    enabled: bool,
    frame: Option<Frame>,
    reused: usize,
}
struct Frame {
    key: Vec<u8>,
    source: Vec<[u8; 3]>,
    colours: Vec<([u8; 3], [u8; 3])>,
    states: Vec<Choice>,
}
thread_local! { static HISTORY: RefCell<History> = RefCell::new(History::default()); }

pub(crate) fn configure(enabled: bool) {
    HISTORY.with_borrow_mut(|h| {
        h.enabled = enabled;
        h.reused = 0;
        if !enabled {
            h.frame = None;
        }
    });
}
pub(crate) fn reused() -> usize {
    HISTORY.with_borrow(|h| h.reused)
}
pub(super) fn record_reused(cells: usize) {
    HISTORY.with_borrow_mut(|h| h.reused = cells);
}

pub(super) fn key(
    args: &RenderArgs,
    glyphs: &[FastGlyph],
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
) -> Option<Vec<u8>> {
    if !HISTORY.with_borrow(|h| h.enabled) {
        return None;
    }
    let mut key = serde_json::to_vec(&(args, grid_w, grid_h, gw, gh)).ok()?;
    for glyph in glyphs {
        key.extend_from_slice(&glyph.bitmap);
    }
    Some(key)
}
fn close(a: [u8; 3], b: [u8; 3], tolerance: u8) -> bool {
    a.into_iter()
        .zip(b)
        .all(|(a, b)| a.abs_diff(b) <= tolerance)
}
pub(super) fn reuse(
    key: &[u8],
    source: &[[u8; 3]],
    data: &[BlockData],
    states: &mut [Choice],
    grid_w: usize,
    grid_h: usize,
    allow_inversion: bool,
) -> Option<Vec<bool>> {
    let tolerance = super::rank_cache::tolerance();
    HISTORY.with_borrow(|history| {
        let previous = history.frame.as_ref().filter(|f| {
            f.key == key && f.source.len() == source.len() && f.states.len() == states.len()
        })?;
        let mut changed = vec![false; states.len()];
        let mut swapped = vec![false; states.len()];
        for cell in 0..states.len() {
            let (fg, bg) = previous.colours[cell];
            let same = close(data[cell].fg, fg, tolerance) && close(data[cell].bg, bg, tolerance);
            let swap = allow_inversion
                && close(data[cell].fg, bg, tolerance) && close(data[cell].bg, fg, tolerance);
            swapped[cell] = !same && swap;
            changed[cell] = !(same || swap);
            let (x, y) = (cell % grid_w * CW, cell / grid_w * CH);
            for yy in y..y + CH {
                let begin = yy * grid_w * CW + x;
                changed[cell] |= source[begin..begin + CW]
                    .iter()
                    .zip(&previous.source[begin..begin + CW])
                    .any(|(&a, &b)| !close(a, b, tolerance));
            }
        }
        let mut targets = changed.clone();
        for (cell, &dirty) in changed.iter().enumerate() {
            if !dirty {
                continue;
            }
            let (x, y) = (cell % grid_w, cell / grid_w);
            for yy in y.saturating_sub(2)..=(y + 2).min(grid_h - 1) {
                for xx in x.saturating_sub(2)..=(x + 2).min(grid_w - 1) {
                    targets[yy * grid_w + xx] = true;
                }
            }
        }
        if targets.iter().all(|&t| t) {
            return None;
        }
        for cell in 0..states.len() {
            if !targets[cell] {
                states[cell] = (
                    previous.states[cell].0,
                    previous.states[cell].1 ^ swapped[cell],
                );
            }
        }
        Some(targets)
    })
}
pub(super) fn remember(key: Vec<u8>, source: &[[u8; 3]], data: &[BlockData], states: &[Choice]) {
    if source.len() * 3 + key.len() + states.len() * 32 > 64 * 1024 * 1024 {
        return;
    }
    HISTORY.with_borrow_mut(|h| {
        h.frame = Some(Frame {
            key,
            source: source.to_vec(),
            colours: data.iter().map(|d| (d.fg, d.bg)).collect(),
            states: states.to_vec(),
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn motion_invalidates_a_margin_and_configuration_changes_reject_reuse() {
        configure(true);
        super::super::rank_cache::configure(0, 2);
        let (w, h) = (9, 9);
        let mut source = vec![[10; 3]; w * h * CW * CH];
        let data: Vec<_> = (0..w * h)
            .map(|_| BlockData {
                fg: [240; 3],
                bg: [20; 3],
                fg_col: None,
                bg_col: None,
                alt_fg_col: None,
                alt_bg_col: None,
            })
            .collect();
        remember(vec![1], &source, &data, &vec![(1, false); w * h]);
        let mut states = vec![(0, false); w * h];
        let mask = reuse(&[1], &source, &data, &mut states, w, h, true).unwrap();
        assert!(mask.iter().all(|&t| !t));
        assert!(states.iter().all(|&s| s == (1, false)));
        source[(4 * CH) * w * CW + 4 * CW] = [200; 3];
        states.fill((0, false));
        let mask = reuse(&[1], &source, &data, &mut states, w, h, true).unwrap();
        assert_eq!(mask.iter().filter(|&&t| t).count(), 25);
        assert_eq!(states[4 * w + 4], (0, false));
        assert_eq!(states[0], (1, false));
        assert!(reuse(&[2], &source, &data, &mut states, w, h, true).is_none());
        configure(false);
    }
}
