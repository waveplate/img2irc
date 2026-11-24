//! Factor a cell's seam error into fixed context and cached glyph-mask bits.
//! Integer arithmetic preserves the original RGB/source-gradient comparison.
use super::*;

pub(super) struct SeamCosts {
    base: i64,
    delta: [i64; CW * CH],
    boundary: u128,
    comparisons: usize,
}

impl SeamCosts {
    pub(super) fn new(
        cell: usize,
        data: &BlockData,
        source: &[[u8; 3]],
        output: &[[u8; 3]],
        grid_w: usize,
        grid_h: usize,
    ) -> Self {
        let mut result = Self {
            base: 0,
            delta: [0; CW * CH],
            boundary: 0,
            comparisons: 0,
        };
        let x0 = cell % grid_w * CW;
        let y0 = cell / grid_w * CH;
        let width = grid_w * CW;
        let mut compare = |x: usize, y: usize, other: usize| {
            let pixel = (y0 + y) * width + x0 + x;
            let error = |colour: [u8; 3]| -> i64 {
                (0..3)
                    .map(|c| {
                        let difference = colour[c] as i64
                            - output[other][c] as i64
                            - (source[pixel][c] as i64 - source[other][c] as i64);
                        difference * difference
                    })
                    .sum()
            };
            let clear = error(data.bg);
            let bit = y * CW + x;
            result.base += clear;
            result.delta[bit] += error(data.fg) - clear;
            result.boundary |= 1 << bit;
            result.comparisons += 1;
        };
        for y in 0..CH {
            if x0 > 0 {
                compare(0, y, (y0 + y) * width + x0 - 1);
            }
            if x0 + CW < width {
                compare(CW - 1, y, (y0 + y) * width + x0 + CW);
            }
        }
        for x in 0..CW {
            if y0 > 0 {
                compare(x, 0, (y0 - 1) * width + x0 + x);
            }
            if y0 + CH < grid_h * CH {
                compare(x, CH - 1, (y0 + CH) * width + x0 + x);
            }
        }
        result
    }

    pub(super) fn error(&self, mask: u128) -> f64 {
        let mut selected = mask & self.boundary;
        let mut error = self.base;
        while selected != 0 {
            error += self.delta[selected.trailing_zeros() as usize];
            selected &= selected - 1;
        }
        error as f64 / self.comparisons.max(1) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_seams_match_pixel_comparisons_at_edges_corners_and_inversions() {
        for (grid_w, grid_h) in [(1, 1), (1, 3), (3, 1), (3, 3)] {
            let pixels = grid_w * CW * grid_h * CH;
            let source: Vec<_> = (0..pixels)
                .map(|p| {
                    [
                        (p * 31 % 256) as u8,
                        (p * 17 % 256) as u8,
                        (p * 3 % 256) as u8,
                    ]
                })
                .collect();
            let output: Vec<_> = (0..pixels)
                .map(|p| {
                    [
                        (p * 7 % 256) as u8,
                        (p * 13 % 256) as u8,
                        (p * 19 % 256) as u8,
                    ]
                })
                .collect();
            let data = BlockData {
                fg: [255, 32, 190],
                bg: [5, 240, 0],
                fg_col: None,
                bg_col: None,
                alt_fg_col: None,
                alt_bg_col: None,
            };
            for cell in 0..grid_w * grid_h {
                let cached = SeamCosts::new(cell, &data, &source, &output, grid_w, grid_h);
                for mask in [
                    0,
                    u128::MAX,
                    0x35d53fc813ab13ef,
                    u128::from_le_bytes([0x81; CH]),
                ] {
                    for mask in [mask, !mask] {
                        let exact = super::super::seam_error(
                            cell, mask, &data, &source, &output, grid_w, grid_h,
                        );
                        assert_eq!(cached.error(mask).to_bits(), exact.to_bits());
                    }
                }
            }
        }
    }
}
