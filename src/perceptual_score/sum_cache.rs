//! Cache block totals in a balanced tree. A local edit changes only its leaves
//! and ancestors, rather than forcing an unchanged raster suffix to be summed
//! again. Leaves and tree nodes accumulate in f64; close search decisions are
//! checked against the original raster-order scorer.

use super::REDUCTION_BLOCK;

pub(super) struct SumCache<const N: usize> {
    nodes: Vec<[f64; N]>,
    leaves: usize,
    listed: Vec<bool>,
    dirty: Vec<usize>,
}

impl<const N: usize> SumCache<N> {
    pub(super) fn new(pixels: usize) -> Self {
        let blocks = pixels.div_ceil(REDUCTION_BLOCK);
        let leaves = blocks.max(1).next_power_of_two();
        Self {
            nodes: vec![[0.0; N]; leaves * 2],
            leaves,
            listed: vec![true; blocks],
            dirty: (0..blocks).collect(),
        }
    }

    pub(super) fn invalidate(&mut self, pixel: usize) {
        self.invalidate_range(pixel, pixel + 1);
    }

    pub(super) fn invalidate_range(&mut self, begin: usize, end: usize) {
        if begin >= end {
            return;
        }
        for block in begin / REDUCTION_BLOCK..end.div_ceil(REDUCTION_BLOCK) {
            if !self.listed[block] {
                self.listed[block] = true;
                self.dirty.push(block);
            }
        }
    }

    pub(super) fn totals(&mut self, mut reduce: impl FnMut(usize, &mut [f64; N])) -> [f64; N] {
        for block in self.dirty.drain(..) {
            self.listed[block] = false;
            let mut sum = [0.0; N];
            reduce(block, &mut sum);
            let mut node = self.leaves + block;
            if sum.map(f64::to_bits) == self.nodes[node].map(f64::to_bits) {
                continue;
            }
            self.nodes[node] = sum;
            while node > 1 {
                node /= 2;
                let sum =
                    std::array::from_fn(|c| self.nodes[node * 2][c] + self.nodes[node * 2 + 1][c]);
                if sum.map(f64::to_bits) == self.nodes[node].map(f64::to_bits) {
                    break;
                }
                self.nodes[node] = sum;
            }
        }
        self.nodes[1]
    }
}
