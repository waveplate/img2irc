//! Reuse local rankings across frames. Exact by default; an opt-in per-channel
//! tolerance accepts compression noise. Global scores are never stored here.
use super::*;
use std::{
    cell::RefCell,
    collections::VecDeque,
    hash::{Hash, Hasher},
};

#[derive(Default)]
struct Cache {
    entries: HashMap<u64, Entry>,
    contexts: HashMap<u64, u64>,
    order: VecDeque<u64>,
    bytes: usize,
    budget: usize,
    hits: u64,
    misses: u64,
    tolerance: u8,
}
struct Entry {
    key: Vec<u8>,
    trials: Vec<CachedTrial>,
    layout_len: usize,
    layout_hash: u64,
}
impl Entry {
    fn bytes(&self) -> usize {
        self.key.capacity() + self.trials.capacity() * std::mem::size_of::<CachedTrial>() + 128
    }
}
thread_local! { static CACHE: RefCell<Cache> = RefCell::new(Cache::default()); }

pub(crate) fn configure(bytes: usize, tolerance: u8) {
    CACHE.with_borrow_mut(|cache| {
        cache.budget = bytes;
        cache.tolerance = tolerance;
        cache.evict(0);
    });
}
pub(crate) fn stats() -> (u64, u64) {
    CACHE.with_borrow(|c| (c.hits, c.misses))
}
pub(super) fn tolerance() -> u8 {
    CACHE.with_borrow(|c| c.tolerance)
}
impl Cache {
    fn evict(&mut self, incoming: usize) {
        while self.bytes + incoming > self.budget || (self.budget == 0 && !self.entries.is_empty())
        {
            let Some(hash) = self.order.pop_front() else {
                break;
            };
            if let Some(entry) = self.entries.remove(&hash) {
                self.bytes -= entry.bytes();
                if self.contexts.get(&entry.layout_hash) == Some(&hash) {
                    self.contexts.remove(&entry.layout_hash);
                }
            }
        }
    }
    fn get(&mut self, hash: u64, key: &[u8]) -> Option<Vec<CachedTrial>> {
        let found = self
            .entries
            .get(&hash)
            .filter(|e| e.key == key)
            .map(|e| e.trials.clone());
        if found.is_some() {
            self.hits += 1;
        } else {
            self.misses += 1;
        }
        found
    }
    fn insert(&mut self, hash: u64, entry: Entry) {
        if self.entries.contains_key(&hash) || entry.bytes() > self.budget {
            return;
        }
        self.evict(entry.bytes());
        self.bytes += entry.bytes();
        self.order.push_back(hash);
        self.contexts.insert(entry.layout_hash, hash);
        self.entries.insert(hash, entry);
    }
}

pub(super) struct Key {
    bytes: Vec<u8>,
    hash: u64,
    layout_len: usize,
    layout_hash: u64,
    choices: [Vec<(u128, Choice)>; 2],
}

#[derive(Clone)]
struct CachedTrial {
    masks: [u128; 2],
    local: f64,
    lower: f64,
    upper: f64,
}

// (mask, fg, bg) and (!mask, bg, fg) describe identical pixels. The key
// follows rendered masks, while hits map back to the caller's current glyphs.
fn canonical_choices(
    pool: &[Choice],
    masks: &[u128],
    fg: [u8; 3],
    bg: [u8; 3],
) -> Vec<(u128, Choice)> {
    let swap = fg > bg;
    let mut choices: Vec<_> = pool
        .iter()
        .map(|&choice| {
            let mask = if fg == bg {
                0
            } else {
                masks[choice.0] ^ if choice.1 ^ swap { u128::MAX } else { 0 }
            };
            (mask, choice)
        })
        .collect();
    choices.sort_unstable_by_key(|c| c.0);
    choices
}
fn number(key: &mut Vec<u8>, value: usize) {
    key.extend_from_slice(&(value as u64).to_le_bytes());
}
// Run encoding is lossless and keeps flat backgrounds from exhausting the
// budget. The raw form avoids expansion on noisy pictures.
fn pixels(key: &mut Vec<u8>, pixels: impl Iterator<Item = [u8; 3]>) {
    let raw: Vec<_> = pixels.collect();
    let mut runs = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let mut end = i + 1;
        while end < raw.len() && raw[end] == raw[i] && end - i < u16::MAX as usize {
            end += 1;
        }
        runs.extend_from_slice(&((end - i) as u16).to_le_bytes());
        runs.extend_from_slice(&raw[i]);
        i = end;
    }
    let compressed = runs.len() < raw.len() * 3;
    key.push(u8::from(compressed));
    number(
        key,
        if compressed {
            runs.len()
        } else {
            raw.len() * 3
        },
    );
    if compressed {
        key.extend(runs);
    } else {
        key.extend(raw.into_iter().flatten());
    }
}

pub(super) fn key(
    source: &[[u8; 3]],
    output: &[[u8; 3]],
    painter: PairPainter<'_>,
    masks: &[u128],
    pools: &[Vec<Choice>],
    old: (Choice, Choice),
    grid_h: usize,
    finalists: usize,
    workers: usize,
) -> Option<Key> {
    if !CACHE.with_borrow(|c| c.budget > 0) {
        return None;
    }
    let [x0, y0, x1, y1] = Patch::bounds(painter.a, painter.b, painter.grid_w, grid_h);
    let mut bytes = Vec::new();
    for v in [
        x1 - x0,
        y1 - y0,
        painter.a % painter.grid_w - x0,
        painter.a / painter.grid_w - y0,
        painter.b % painter.grid_w - x0,
        painter.b / painter.grid_w - y0,
        painter.gw,
        painter.gh,
        finalists,
        workers,
        ranking_batch_size(),
    ] {
        number(&mut bytes, v);
    }
    let choices = [painter.a, painter.b].map(|cell| {
        canonical_choices(
            &pools[cell],
            masks,
            painter.data[cell].fg,
            painter.data[cell].bg,
        )
    });
    for (i, old) in [old.0, old.1].into_iter().enumerate() {
        let mask = choices[i].iter().find(|c| c.1 == old)?.0;
        bytes.extend_from_slice(&mask.to_le_bytes());
        number(&mut bytes, choices[i].len());
        for &(mask, _) in &choices[i] {
            bytes.extend_from_slice(&mask.to_le_bytes());
        }
    }
    let layout_len = bytes.len();
    let mut layout_hash = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut layout_hash);
    // Approximate lookup follows this pair through time. Unrelated pairs with
    // the same glyph pools must not displace its most recent neighbourhood.
    (painter.a, painter.b, painter.grid_w, grid_h).hash(&mut layout_hash);
    let layout_hash = layout_hash.finish();
    pixels(
        &mut bytes,
        [painter.a, painter.b].into_iter().flat_map(|cell| {
            let (fg, bg) = (painter.data[cell].fg, painter.data[cell].bg);
            [fg.min(bg), fg.max(bg)]
        }),
    );
    for raster in [source, output] {
        pixels(
            &mut bytes,
            (y0 * CH..y1 * CH).flat_map(|y| {
                let begin = y * painter.grid_w * CW + x0 * CW;
                raster[begin..begin + (x1 - x0) * CW].iter().copied()
            }),
        );
    }
    bytes.shrink_to_fit();
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hash);
    Some(Key {
        bytes,
        hash: hash.finish(),
        layout_len,
        layout_hash,
        choices,
    })
}
pub(super) fn get(key: &Key) -> Option<Vec<Trial>> {
    let cached = CACHE.with_borrow_mut(|c| {
        let exact = c.get(key.hash, &key.bytes);
        if exact.is_some() || c.tolerance == 0 {
            return exact;
        }
        let entry = c
            .contexts
            .get(&key.layout_hash)
            .and_then(|h| c.entries.get(h))?;
        // Hash collisions and changed glyph pools cannot relax this check.
        if entry.layout_len != key.layout_len
            || entry.key[..entry.layout_len] != key.bytes[..key.layout_len]
            || !close_colours(
                &entry.key[entry.layout_len..],
                &key.bytes[key.layout_len..],
                c.tolerance,
            )
        {
            return None;
        }
        let trials = entry.trials.clone();
        c.misses -= 1;
        c.hits += 1;
        Some(trials)
    })?;
    let mut trials: Vec<_> = cached
        .into_iter()
        .map(|cached| {
            let choice = |i: usize| {
                key.choices[i]
                    .binary_search_by_key(&cached.masks[i], |c| c.0)
                    .ok()
                    .map(|j| key.choices[i][j].1)
            };
            Some(Trial {
                a: choice(0)?,
                b: choice(1)?,
                local: cached.local,
                local_lower: cached.lower,
                local_upper: cached.upper,
            })
        })
        .collect::<Option<_>>()?;
    // Colour swapping can reverse raw glyph-index tie order. Store all local
    // trials, then restore the caller's ordering before choosing finalists.
    trials.sort_unstable_by(|a, b| {
        a.local
            .total_cmp(&b.local)
            .then_with(|| (a.a, a.b).cmp(&(b.a, b.b)))
    });
    Some(trials)
}

fn decode_colours(mut encoded: &[u8]) -> Vec<[u8; 3]> {
    let mut result = Vec::new();
    while !encoded.is_empty() {
        let run = encoded[0] != 0;
        let len = u64::from_le_bytes(encoded[1..9].try_into().unwrap()) as usize;
        let payload = &encoded[9..9 + len];
        if run {
            for p in payload.chunks_exact(5) {
                let n = u16::from_le_bytes([p[0], p[1]]) as usize;
                result.extend(std::iter::repeat_n([p[2], p[3], p[4]], n));
            }
        } else {
            result.extend(payload.chunks_exact(3).map(|p| [p[0], p[1], p[2]]));
        }
        encoded = &encoded[9 + len..];
    }
    result
}
fn close_colours(a: &[u8], b: &[u8], tolerance: u8) -> bool {
    let (a, b) = (decode_colours(a), decode_colours(b));
    a.len() == b.len()
        && a.iter()
            .flatten()
            .zip(b.iter().flatten())
            .all(|(&a, &b)| a.abs_diff(b) <= tolerance)
}
pub(super) fn insert(key: Key, trials: &[Trial]) {
    let cached = trials
        .iter()
        .map(|trial| {
            let a = key.choices[0].iter().find(|c| c.1 == trial.a)?.0;
            let b = key.choices[1].iter().find(|c| c.1 == trial.b)?.0;
            Some(CachedTrial {
                masks: [a, b],
                local: trial.local,
                lower: trial.local_lower,
                upper: trial.local_upper,
            })
        })
        .collect::<Option<Vec<_>>>();
    let Some(cached) = cached else {
        return;
    };
    CACHE.with_borrow_mut(|c| {
        c.insert(
            key.hash,
            Entry {
                key: key.bytes,
                trials: cached,
                layout_len: key.layout_len,
                layout_hash: key.layout_hash,
            },
        )
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn collisions_never_reuse_a_different_patch_and_budget_is_enforced() {
        let mut c = Cache {
            budget: 512,
            ..Cache::default()
        };
        c.insert(
            1,
            Entry {
                key: vec![1; 32],
                trials: Vec::new(),
                layout_len: 0,
                layout_hash: 0,
            },
        );
        assert!(c.get(1, &[2; 32]).is_none());
        assert!(c.get(1, &[1; 32]).is_some());
        for n in 2..20 {
            c.insert(
                n,
                Entry {
                    key: vec![n as u8; 32],
                    trials: Vec::new(),
                    layout_len: 0,
                    layout_hash: 0,
                },
            );
        }
        assert!(c.bytes <= c.budget);
        c.budget = 0;
        c.evict(0);
        assert!(c.entries.is_empty());
    }

    #[test]
    fn tolerance_checks_every_channel_and_does_not_hide_moving_edges() {
        let mut a = Vec::new();
        let mut b = Vec::new();
        pixels(&mut a, [[40, 80, 120]; 100].into_iter());
        pixels(&mut b, (0..100).map(|p| [40 + (p % 3) as u8, 80, 120]));
        assert!(close_colours(&a, &b, 2));
        assert!(!close_colours(&a, &b, 1));
        let mut edge = Vec::new();
        pixels(
            &mut edge,
            (0..100).map(|p| if p == 99 { [0, 0, 0] } else { [40, 80, 120] }),
        );
        assert!(!close_colours(&a, &edge, 8));
    }

    #[test]
    fn inverse_glyphs_and_swapped_colours_share_rankings_but_return_current_choices() {
        let masks = [0x0000ffff_u128, !0x0000ffff_u128];
        let light = [220; 3];
        let dark = [30; 3];
        let original = canonical_choices(&[(0, false), (0, true)], &masks, light, dark);
        let inverse = canonical_choices(&[(1, true), (1, false)], &masks, light, dark);
        let swapped = canonical_choices(&[(0, true), (0, false)], &masks, dark, light);
        assert_eq!(
            original.iter().map(|c| c.0).collect::<Vec<_>>(),
            inverse.iter().map(|c| c.0).collect::<Vec<_>>()
        );
        assert_eq!(
            original.iter().map(|c| c.0).collect::<Vec<_>>(),
            swapped.iter().map(|c| c.0).collect::<Vec<_>>()
        );
        let key = |choices: Vec<(u128, Choice)>| Key {
            bytes: vec![17],
            hash: 12345,
            layout_len: 1,
            layout_hash: 12345,
            choices: [choices.clone(), choices],
        };
        configure(1024 * 1024, 0);
        insert(
            key(original),
            &[Trial {
                a: (0, false),
                b: (0, true),
                local: 1.0,
                local_lower: 1.0,
                local_upper: 1.0,
            }],
        );
        let hit = get(&key(inverse)).unwrap();
        assert_eq!((hit[0].a, hit[0].b), ((1, true), (1, false)));
        let hit = get(&key(swapped)).unwrap();
        assert_eq!((hit[0].a, hit[0].b), ((0, true), (0, false)));
        configure(0, 0);
    }
}
