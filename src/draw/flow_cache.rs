//! Exact memoization of the expensive, source-independent median-flow score.
//! Shared by Rayon workers and successive video frames, with bounded memory.
use once_cell::sync::Lazy;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_ENTRIES: usize = 4096;

#[derive(Default)]
struct Cache {
    entries: HashMap<u64, Entry>,
    order: VecDeque<u64>,
    bytes: usize,
}

struct Entry {
    pixels: Vec<[u8; 3]>,
    width: usize,
    height: usize,
    score: f64,
}

impl Entry {
    fn bytes(&self) -> usize {
        self.pixels.len() * std::mem::size_of::<[u8; 3]>() + std::mem::size_of::<Self>()
    }
}

impl Cache {
    fn get(&self, hash: u64, pixels: &[[u8; 3]], width: usize, height: usize) -> Option<f64> {
        self.entries
            .get(&hash)
            .filter(|entry| {
                entry.width == width && entry.height == height && entry.pixels == pixels
            })
            .map(|entry| entry.score)
    }

    fn insert(&mut self, hash: u64, entry: Entry) {
        // Do not trust a matching hash alone, or displace an entry on collision.
        if self.entries.contains_key(&hash) || entry.bytes() > MAX_BYTES {
            return;
        }
        while self.bytes + entry.bytes() > MAX_BYTES || self.entries.len() >= MAX_ENTRIES {
            if let Some(oldest) = self
                .order
                .pop_front()
                .and_then(|key| self.entries.remove(&key))
            {
                self.bytes -= oldest.bytes();
            }
        }
        self.bytes += entry.bytes();
        self.entries.insert(hash, entry);
        self.order.push_back(hash);
    }
}

static CACHE: Lazy<Mutex<Cache>> = Lazy::new(|| Mutex::new(Cache::default()));

pub(super) fn score(
    pixels: &[[u8; 3]],
    width: usize,
    height: usize,
    compute: impl FnOnce() -> f64,
) -> f64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    width.hash(&mut hash);
    height.hash(&mut hash);
    pixels.hash(&mut hash);
    let hash = hash.finish();
    if let Some(score) = CACHE
        .lock()
        .ok()
        .and_then(|cache| cache.get(hash, pixels, width, height))
    {
        return score;
    }
    // Never hold the lock through median passes: independent patches run in parallel.
    let score = compute();
    if let Ok(mut cache) = CACHE.lock() {
        cache.insert(
            hash,
            Entry {
                pixels: pixels.to_vec(),
                width,
                height,
                score,
            },
        );
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collisions_and_patch_geometry_cannot_reuse_the_wrong_score() {
        let mut cache = Cache::default();
        let pixels = vec![[12, 34, 56]; 6];
        cache.insert(
            123,
            Entry {
                pixels: pixels.clone(),
                width: 3,
                height: 2,
                score: 4.5,
            },
        );
        assert_eq!(cache.get(123, &pixels, 3, 2), Some(4.5));
        assert_eq!(cache.get(123, &pixels, 2, 3), None);
        let mut changed = pixels.clone();
        changed[0][0] += 1;
        assert_eq!(cache.get(123, &changed, 3, 2), None);
        cache.insert(
            123,
            Entry {
                pixels: changed,
                width: 3,
                height: 2,
                score: 99.0,
            },
        );
        assert_eq!(cache.get(123, &pixels, 3, 2), Some(4.5));
    }

    #[test]
    fn cache_evicts_old_entries_with_bounded_memory() {
        let mut cache = Cache::default();
        for hash in 0..5000 {
            cache.insert(
                hash,
                Entry {
                    pixels: vec![[0, 0, 0]; 2048],
                    width: 32,
                    height: 64,
                    score: 0.0,
                },
            );
        }
        assert!(cache.bytes <= MAX_BYTES);
        assert!(cache.entries.len() <= MAX_ENTRIES);
        assert_eq!(cache.entries.len(), cache.order.len());
        assert!(!cache.entries.contains_key(&0));
        assert!(cache.entries.contains_key(&4999));
    }

    #[test]
    fn cached_flow_scores_match_uncached_scores_for_changing_patches() {
        for (width, height) in [(8, 16), (24, 48), (16, 8)] {
            for seed in 0..4 {
                let pixels: Vec<_> = (0..width * height)
                    .map(|i| {
                        if (i / width + i % width * 3 + seed) % 7 < 3 {
                            [10, 20, 30]
                        } else {
                            [200, 180, 160]
                        }
                    })
                    .collect();
                let expected =
                    super::super::curvature_flow_regularity_uncached(&pixels, width, height);
                assert_eq!(score(&pixels, width, height, || expected), expected);
                assert_eq!(
                    score(&pixels, width, height, || panic!(
                        "unchanged patches should be reused"
                    )),
                    expected
                );
            }
        }
    }
}
