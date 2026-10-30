//! Owned per-video history, scoped to a render even when its worker changes threads.
use super::{Options, Rgb};
use crate::draw::{oklab_distance_squared, rgb_to_oklab, Oklab};
use std::cell::RefCell;
use std::collections::HashMap;

const MAX_BYTES: usize = 128 * 1024 * 1024;

pub(super) struct Layer {
    pub(super) options: Options,
    pub(super) width: usize,
    pub(super) height: usize,
    pub(super) source: Vec<Oklab>,
    pub(super) labels: Vec<usize>,
    pub(super) colours: HashMap<usize, Rgb>,
}

impl Layer {
    pub(super) fn stable_colour(
        &self,
        region: &[usize],
        source: &[Oklab],
        mean: [f64; 3],
    ) -> Option<Rgb> {
        let tolerance =
            (self.options.temporal_tolerance.min(self.options.tolerance) / 100.0).powi(2);
        let mean = Oklab {
            l: mean[0] as f32,
            a: mean[1] as f32,
            b: mean[2] as f32,
        };
        let mut overlaps = HashMap::<usize, usize>::new();
        for &pixel in region {
            let label = self.labels[pixel];
            if self.colours.contains_key(&label)
                && oklab_distance_squared(source[pixel], self.source[pixel]) <= tolerance
            {
                *overlaps.entry(label).or_default() += 1;
            }
        }
        let (label, overlap) = overlaps
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))?;
        if overlap * 10 < region.len() * 7 {
            return None;
        }
        let colour = self.colours[&label];
        let lab = rgb_to_oklab(colour);
        (crate::colour_quantization::chroma_compatible(mean, lab)
            && oklab_distance_squared(lab, mean) <= tolerance)
            .then_some(colour)
    }

    fn bytes(&self) -> usize {
        self.source.len() * std::mem::size_of::<Oklab>()
            + self.labels.len() * std::mem::size_of::<usize>()
            + self.colours.len() * 64
    }
}

#[derive(Default)]
pub(crate) struct History {
    layers: Vec<Option<Layer>>,
}

struct Context {
    history: History,
    next: Vec<Option<Layer>>,
}

thread_local! { static CONTEXT: RefCell<Option<Context>> = const { RefCell::new(None) }; }

pub(super) fn with_layer(apply: impl FnOnce(Option<&Layer>, bool) -> Option<Layer>) {
    CONTEXT.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(context) = slot.as_mut() {
            let previous = context
                .history
                .layers
                .get(context.next.len())
                .and_then(Option::as_ref);
            let next = apply(previous, true);
            context.next.push(next);
        } else {
            apply(None, false);
        }
    });
}

pub(super) fn can_record(pixels: usize) -> bool {
    pixels.saturating_mul(std::mem::size_of::<Oklab>() + std::mem::size_of::<usize>()) <= MAX_BYTES
}

/// A cancelled render keeps its previous history. A panic restores the scope too.
pub(crate) fn with_history<T>(history: &mut History, render: impl FnOnce() -> (T, bool)) -> T {
    struct Restore<'a> {
        target: &'a mut History,
        previous: Option<Context>,
    }
    impl Drop for Restore<'_> {
        fn drop(&mut self) {
            let current = CONTEXT.with(|slot| slot.replace(self.previous.take()));
            *self.target = current.map(|context| context.history).unwrap_or_default();
        }
    }
    let previous = CONTEXT.with(|slot| {
        slot.replace(Some(Context {
            history: std::mem::take(history),
            next: Vec::new(),
        }))
    });
    let _restore = Restore {
        target: history,
        previous,
    };
    let (result, commit) = render();
    if commit {
        CONTEXT.with(|slot| {
            if let Some(context) = slot.borrow_mut().as_mut() {
                let bytes: usize = context.next.iter().flatten().map(Layer::bytes).sum();
                context.history.layers = if bytes <= MAX_BYTES {
                    std::mem::take(&mut context.next)
                } else {
                    Vec::new()
                };
            }
        });
    }
    result
}
