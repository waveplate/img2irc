//! Glyphs with the same boundary pixels have the same connection costs, even
//! when their interiors differ. Cache those equivalence classes, rather than
//! a quadratic table of glyph pairs. Colours and source gradients stay dynamic.
use super::*;
use std::sync::{Mutex, OnceLock};

pub(super) struct Connections {
    masks: Vec<u128>,
    ids: Vec<[usize; 4]>,
    profiles: [Vec<u16>; 4],
}

fn edge_pixel(side: usize, t: usize) -> (usize, usize) {
    match side {
        0 => (0, t),
        1 => (CW - 1, t),
        2 => (t, 0),
        _ => (t, CH - 1),
    }
}

impl Connections {
    pub(super) fn cached(masks: &[u128]) -> Arc<Self> {
        // Retain one glyph set between renders/frames; compare the actual masks
        // so a changed font or resolution cannot reuse stale relationships.
        static LAST: OnceLock<Mutex<Option<Arc<Connections>>>> = OnceLock::new();
        let cache = LAST.get_or_init(|| Mutex::new(None));
        if let Some(found) = cache.lock().unwrap().as_ref().filter(|c| c.masks == masks) {
            return found.clone();
        }
        let mut result = Self {
            masks: masks.to_vec(),
            ids: Vec::with_capacity(masks.len() * 2),
            profiles: std::array::from_fn(|_| Vec::new()),
        };
        let mut indices: [HashMap<u16, usize>; 4] = std::array::from_fn(|_| HashMap::new());
        for &mask in masks {
            for mask in [mask, !mask] {
                result.ids.push(std::array::from_fn(|side| {
                    let mut profile = 0;
                    for t in 0..if side < 2 { CH } else { CW } {
                        let (x, y) = edge_pixel(side, t);
                        profile |= (((mask >> (y * CW + x)) & 1) as u16) << t;
                    }
                    *indices[side].entry(profile).or_insert_with(|| {
                        result.profiles[side].push(profile);
                        result.profiles[side].len() - 1
                    })
                }));
            }
        }
        let result = Arc::new(result);
        *cache.lock().unwrap() = Some(result.clone());
        result
    }

    pub(super) fn costs(
        &self,
        cell: usize,
        data: &BlockData,
        source: &[[u8; 3]],
        output: &[[u8; 3]],
        grid_w: usize,
        grid_h: usize,
    ) -> Costs {
        let (cx, cy) = (cell % grid_w, cell / grid_w);
        let width = grid_w * CW;
        let neighbors = [cx > 0, cx + 1 < grid_w, cy > 0, cy + 1 < grid_h];
        let mut comparisons = 0;
        let mut current: [Vec<i64>; 4] =
            std::array::from_fn(|side| vec![0; self.profiles[side].len()]);
        for (side, neighbor) in neighbors.into_iter().enumerate() {
            if !neighbor {
                continue;
            }
            let length = if side < 2 { CH } else { CW };
            comparisons += length;
            let mut base = 0;
            let mut delta = [0i64; CH];
            for t in 0..length {
                let (x, y) = edge_pixel(side, t);
                let p = (cy * CH + y) * width + cx * CW + x;
                let q = match side {
                    0 => p - 1,
                    1 => p + 1,
                    2 => p - width,
                    _ => p + width,
                };
                let other = output[q];
                let error = |colour: [u8; 3]| -> i64 {
                    (0..3)
                        .map(|c| {
                            let d = colour[c] as i64
                                - other[c] as i64
                                - (source[p][c] as i64 - source[q][c] as i64);
                            d * d
                        })
                        .sum()
                };
                let clear = error(data.bg);
                base += clear;
                delta[t] = error(data.fg) - clear;
            }
            for (id, &profile) in self.profiles[side].iter().enumerate() {
                let mut bits = profile;
                let mut error = base;
                while bits != 0 {
                    error += delta[bits.trailing_zeros() as usize];
                    bits &= bits - 1;
                }
                current[side][id] = error;
            }
        }
        Costs {
            current,
            comparisons: comparisons.max(1),
        }
    }

    pub(super) fn error(&self, costs: &Costs, choice: Choice) -> f64 {
        let ids = self.ids[choice.0 * 2 + usize::from(choice.1)];
        let sum = |values: &[Vec<i64>; 4]| -> f64 {
            (0..4).map(|side| values[side][ids[side]]).sum::<i64>() as f64
                / costs.comparisons as f64
        };
        sum(&costs.current)
    }
}

pub(super) struct Costs {
    current: [Vec<i64>; 4],
    comparisons: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> BlockData {
        BlockData {
            fg: [230, 20, 100],
            bg: [5, 240, 30],
            fg_col: None,
            bg_col: None,
            alt_fg_col: None,
            alt_bg_col: None,
        }
    }

    #[test]
    fn cached_connections_preserve_source_gradients_colours_and_inversions() {
        let masks = [
            0,
            u128::MAX,
            0x35d53fc813ab13ef,
            u128::from_le_bytes([0x81; CH]),
        ];
        let connections = Connections::cached(&masks);
        assert_eq!(connections.ids, Connections::cached(&masks).ids);
        assert_ne!(connections.masks, Connections::cached(&[1]).masks);
        for (grid_w, grid_h) in [(1, 1), (1, 3), (3, 1), (3, 3)] {
            let pixels = grid_w * CW * grid_h * CH;
            let source: Vec<_> = (0..pixels)
                .map(|p| [(p * 31 % 256) as u8, (p * 17 % 256) as u8, 190])
                .collect();
            let output: Vec<_> = (0..pixels)
                .map(|p| [(p * 7 % 256) as u8, (p * 13 % 256) as u8, 20])
                .collect();
            let cells = vec![data(); grid_w * grid_h];
            for cell in 0..cells.len() {
                let costs = connections.costs(cell, &cells[cell], &source, &output, grid_w, grid_h);
                for (gi, &mask) in masks.iter().enumerate() {
                    for inverted in [false, true] {
                        let exact = super::super::seam_error(
                            cell,
                            mask ^ if inverted { u128::MAX } else { 0 },
                            &cells[cell],
                            &source,
                            &output,
                            grid_w,
                            grid_h,
                        );
                        let current = connections.error(&costs, (gi, inverted));
                        assert_eq!(current.to_bits(), exact.to_bits());
                    }
                }
            }
        }
    }

    #[test]
    fn different_interiors_reuse_the_same_connection_profiles() {
        let masks = [0, 1 << (6 * CW + 3)];
        let connections = Connections::cached(&masks);
        assert_eq!(connections.ids[0], connections.ids[2]);
        assert_eq!(connections.ids[1], connections.ids[3]);
        for side in 0..4 {
            assert_eq!(connections.profiles[side].len(), 2);
        }
    }
}
