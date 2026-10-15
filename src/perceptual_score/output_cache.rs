//! Cache per-pixel output features, updating their exact finite support when
//! glyphs change. Global reductions retain their original raster order.

use super::*;

#[derive(Default)]
pub(crate) struct OutputCache {
    width: usize,
    height: usize,
    step: usize,
    pub(super) fine: Vec<V3>,
    pub(super) scaled: Vec<V3>,
    variation: Vec<[f32; 2]>,
    pub(super) fragility: Vec<f32>,
    variation_blocks: Vec<u16>,
    fragile_blocks: Vec<u16>,
}

impl OutputCache {
    pub(super) fn new(output: &[[u8; 3]], width: usize, height: usize, step: usize) -> Self {
        let mut cache = Self {
            width,
            height,
            step,
            fine: vec![[0.0; 3]; width * height],
            scaled: vec![[0.0; 3]; width.div_ceil(step) * height.div_ceil(step)],
            variation: vec![[0.0; 2]; width * height],
            fragility: vec![0.0; width * height],
            variation_blocks: vec![0; (width * height).div_ceil(REDUCTION_BLOCK)],
            fragile_blocks: vec![0; (width * height).div_ceil(REDUCTION_BLOCK)],
        };
        cache.update(output, [0, 0, width, height]);
        cache
    }

    /// Reuse worker allocations while copying a pair's immutable base state.
    pub(crate) fn copy_from(&mut self, base: &Self) {
        self.width = base.width;
        self.height = base.height;
        self.step = base.step;
        self.fine.clone_from(&base.fine);
        self.scaled.clone_from(&base.scaled);
        self.variation.clone_from(&base.variation);
        self.fragility.clone_from(&base.fragility);
        self.variation_blocks.clone_from(&base.variation_blocks);
        self.fragile_blocks.clone_from(&base.fragile_blocks);
    }

    /// Exclusive rectangle in normalized preview pixels. The one-pixel halo
    /// covers both sides of variation terms and every fragility neighbour.
    pub(crate) fn update(&mut self, output: &[[u8; 3]], rect: [usize; 4]) {
        assert_eq!(output.len(), self.fine.len());
        let [x0, y0, x1, y1] = rect;
        assert!(x0 <= x1 && x1 <= self.width && y0 <= y1 && y1 <= self.height);
        if x0 == x1 || y0 == y1 {
            return;
        }
        for y in y0..y1 {
            for x in x0..x1 {
                let p = y * self.width + x;
                self.fine[p] = color(output[p]);
            }
        }
        for y in y0.saturating_sub(1)..(y1 + 1).min(self.height) {
            for x in x0.saturating_sub(1)..(x1 + 1).min(self.width) {
                let p = y * self.width + x;
                let variation = [
                    if x > 0 {
                        norm(difference(self.fine[p], self.fine[p - 1]))
                    } else {
                        0.0
                    },
                    if y > 0 {
                        norm(difference(self.fine[p], self.fine[p - self.width]))
                    } else {
                        0.0
                    },
                ];
                let fragile = fragility_at(&self.fine, self.width, self.height, x, y);
                let block = p / REDUCTION_BLOCK;
                let old_variation = self.variation[p].iter().any(|&v| v != 0.0);
                let new_variation = variation.iter().any(|&v| v != 0.0);
                self.variation_blocks[block] = self.variation_blocks[block]
                    - u16::from(old_variation)
                    + u16::from(new_variation);
                self.fragile_blocks[block] = self.fragile_blocks[block]
                    - u16::from(self.fragility[p] != 0.0)
                    + u16::from(fragile != 0.0);
                self.variation[p] = variation;
                self.fragility[p] = fragile;
            }
        }
        let scaled_width = self.width.div_ceil(self.step);
        for y in y0 / self.step..y1.div_ceil(self.step) {
            for x in x0 / self.step..x1.div_ceil(self.step) {
                self.scaled[y * scaled_width + x] = if self.step == 1 {
                    self.fine[y * self.width + x]
                } else {
                    // Same f32 additions and division as half_size, including
                    // partial 2x2 samples at an odd-sized image boundary.
                    let mut sum = [0.0; 3];
                    let mut count = 0.0;
                    for yy in y * 2..(y * 2 + 2).min(self.height) {
                        for xx in x * 2..(x * 2 + 2).min(self.width) {
                            for c in 0..3 {
                                sum[c] += self.fine[yy * self.width + xx][c];
                            }
                            count += 1.0;
                        }
                    }
                    sum.map(|v| v / count)
                };
            }
        }
    }

    pub(super) fn reduce_fine_block(&self, allowance: &[f32], block: usize, sums: &mut [f64; 2]) {
        let begin = block * REDUCTION_BLOCK;
        let end = (begin + REDUCTION_BLOCK).min(self.fine.len());
        if self.variation_blocks[block] != 0 {
            for &[left, up] in &self.variation[begin..end] {
                sums[0] += left as f64;
                sums[0] += up as f64;
            }
        }
        if self.fragile_blocks[block] != 0 {
            for (&a, &b) in self.fragility[begin..end].iter().zip(&allowance[begin..end]) {
                sums[1] += (a - b).max(0.0) as f64;
            }
        }
    }

    pub(super) fn variation(&self) -> f64 {
        let mut sum = 0.0;
        for (pixels, &count) in self
            .variation
            .chunks(REDUCTION_BLOCK)
            .zip(&self.variation_blocks)
        {
            if count == 0 {
                continue;
            }
            for &[left, up] in pixels {
                // Keep left then up, in the original raster order.
                sum += left as f64;
                sum += up as f64;
            }
        }
        sum
    }

    pub(super) fn thin_errors(&self, allowance: &[f32]) -> f64 {
        let mut sum = 0.0;
        for ((pixels, allowance), &count) in self
            .fragility
            .chunks(REDUCTION_BLOCK)
            .zip(allowance.chunks(REDUCTION_BLOCK))
            .zip(&self.fragile_blocks)
        {
            if count == 0 {
                continue;
            }
            for (&a, &b) in pixels.iter().zip(allowance) {
                sum += (a - b).max(0.0) as f64;
            }
        }
        sum
    }
}
