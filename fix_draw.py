import sys

def main():
    with open('src/draw.rs', 'r') as f:
        content = f.read()

    start_str = "                    for (sx, sy) in &scales {"
    end_str = "                    let best_fg_rgb = best_overall_fg;"

    start_idx = content.find(start_str)
    end_idx = content.find(end_str)

    if start_idx == -1 or end_idx == -1:
        print("Could not find start or end string.")
        return

    replacement = """                    for (sx, sy) in &scales {
                        if *sx == 1.0 && *sy == 1.0 {
                            working_pixels_buffer.clear();
                            working_pixels_buffer.extend_from_slice(&original_pixels);
                        } else {
                            get_transformed_block_pixels_into(
                                &original_pixels,
                                gw,
                                gh,
                                *sx,
                                *sy,
                                &mut working_pixels_buffer,
                            );
                        }
                        let pixels = &working_pixels_buffer;

                        let mut block_sum_r = 0u64;
                        let mut block_sum_g = 0u64;
                        let mut block_sum_b = 0u64;
                        let mut block_sum_sq = 0u64;

                        let palette: &[u32] = match render {
                            Render::Ansi | Render::Ansi24 => &ANSI256,
                            _ => &IRC99,
                        };
                        let mut indices = Vec::with_capacity(bp);

                        for p in pixels {
                            block_sum_r += p.r;
                            block_sum_g += p.g;
                            block_sum_b += p.b;
                            block_sum_sq += p.sq;
                            
                            let rgb_u32 = ((p.r as u32) << 16) | ((p.g as u32) << 8) | p.b as u32;
                            let idx = nearest_strict_partitioned_hex_colour(
                                rgb_u32,
                                palette,
                                args.grayscale_tolerance,
                            );
                            indices.push(idx);
                        }

                        let mut shape = Vec::with_capacity(bp);
                        let mut seen = Vec::new();
                        for &idx in &indices {
                            if let Some(pos) = seen.iter().position(|&c| c == idx) {
                                shape.push(pos as u8);
                            } else {
                                shape.push(seen.len() as u8);
                                seen.push(idx);
                            }
                        }

                        use std::hash::{Hash, Hasher};
                        let mut hasher = std::collections::hash_map::DefaultHasher::new();
                        shape.hash(&mut hasher);
                        let shape_hash = hasher.finish();

                        let cached = shape_cache.lock().unwrap().get(&shape_hash).copied();

                        let (scale_best_glyph, scale_best_inv) = if let Some(c) = cached {
                            c
                        } else {
                            let mut best_cost = u64::MAX;
                            let mut best_glyph = 0;
                            let mut best_inv = false;

                            if render != Render::Ansi24 {
                                for (gi, fglyph) in glyphs.iter().enumerate() {
                                    let invert_opts: &[bool] = &[false, true];
                                    for &inv in invert_opts {
                                        let mut fg_cnt = [0u16; 256];
                                        let mut bg_cnt = [0u16; 256];
                                        let mut fg_len = 0;
                                        let mut bg_len = 0;

                                        let (fg_iter, bg_iter) = if inv {
                                            (&fglyph.zeros, &fglyph.ones)
                                        } else {
                                            (&fglyph.ones, &fglyph.zeros)
                                        };

                                        for &i in fg_iter {
                                            fg_cnt[indices[i] as usize] += 1;
                                            fg_len += 1;
                                        }
                                        for &i in bg_iter {
                                            bg_cnt[indices[i] as usize] += 1;
                                            bg_len += 1;
                                        }
                                        let (_fgi_raw, fx) = fg_cnt
                                            .iter()
                                            .enumerate()
                                            .max_by_key(|&(_, c)| c)
                                            .unwrap_or((0, &0));
                                        let (_bgi_raw, bx) = bg_cnt
                                            .iter()
                                            .enumerate()
                                            .max_by_key(|&(_, c)| c)
                                            .unwrap_or((0, &0));

                                        let cost = ((fg_len as u64) - (*fx as u64))
                                            + ((bg_len as u64) - (*bx as u64));

                                        if cost < best_cost {
                                            best_cost = cost;
                                            best_glyph = gi;
                                            best_inv = inv;
                                        }
                                    }
                                }
                            } else {
                                for (gi, fglyph) in glyphs.iter().enumerate() {
                                    let mut sum_ones_r = 0u64;
                                    let mut sum_ones_g = 0u64;
                                    let mut sum_ones_b = 0u64;
                                    let mut sum_ones_sq = 0u64;
                                    let count_ones = fglyph.ones.len() as u64;

                                    for &i in &fglyph.ones {
                                        let px = unsafe { pixels.get_unchecked(i) };
                                        sum_ones_r += px.r;
                                        sum_ones_g += px.g;
                                        sum_ones_b += px.b;
                                        sum_ones_sq += px.sq;
                                    }
                                    let count_zeros = bp as u64 - count_ones;
                                    let sum_zeros_r = block_sum_r - sum_ones_r;
                                    let sum_zeros_g = block_sum_g - sum_ones_g;
                                    let sum_zeros_b = block_sum_b - sum_ones_b;
                                    let sum_zeros_sq = block_sum_sq - sum_ones_sq;

                                    let avg_ones = if count_ones > 0 {
                                        [
                                            (sum_ones_r / count_ones) as u8,
                                            (sum_ones_g / count_ones) as u8,
                                            (sum_ones_b / count_ones) as u8,
                                        ]
                                    } else {
                                        [0, 0, 0]
                                    };
                                    let avg_zeros = if count_zeros > 0 {
                                        [
                                            (sum_zeros_r / count_zeros) as u8,
                                            (sum_zeros_g / count_zeros) as u8,
                                            (sum_zeros_b / count_zeros) as u8,
                                        ]
                                    } else {
                                        [0, 0, 0]
                                    };

                                    let t1_1 = sum_ones_sq;
                                    let t2_1 = 2
                                        * (avg_ones[0] as u64 * sum_ones_r
                                            + avg_ones[1] as u64 * sum_ones_g
                                            + avg_ones[2] as u64 * sum_ones_b);
                                    let t3_1 = count_ones
                                        * (avg_ones[0] as u64 * avg_ones[0] as u64
                                            + avg_ones[1] as u64 * avg_ones[1] as u64
                                            + avg_ones[2] as u64 * avg_ones[2] as u64);
                                    let c1 = (t1_1 + t3_1).saturating_sub(t2_1);

                                    let t1_0 = sum_zeros_sq;
                                    let t2_0 = 2
                                        * (avg_zeros[0] as u64 * sum_zeros_r
                                            + avg_zeros[1] as u64 * sum_zeros_g
                                            + avg_zeros[2] as u64 * sum_zeros_b);
                                    let t3_0 = count_zeros
                                        * (avg_zeros[0] as u64 * avg_zeros[0] as u64
                                            + avg_zeros[1] as u64 * avg_zeros[1] as u64
                                            + avg_zeros[2] as u64 * avg_zeros[2] as u64);
                                    let c0 = (t1_0 + t3_0).saturating_sub(t2_0);

                                    let cost_normal = c1 + c0;
                                    if cost_normal < best_cost {
                                        best_cost = cost_normal;
                                        best_glyph = gi;
                                        best_inv = false;
                                    }
                                }
                            }

                            shape_cache.lock().unwrap().insert(shape_hash, (best_glyph, best_inv));
                            (best_glyph, best_inv)
                        };

                        let mut scale_best_fg = [0u8; 3];
                        let mut scale_best_bg = [0u8; 3];

                        if render != Render::Ansi24 {
                            let fglyph = &glyphs[scale_best_glyph];
                            let (fg_iter, bg_iter) = if scale_best_inv {
                                (&fglyph.zeros, &fglyph.ones)
                            } else {
                                (&fglyph.ones, &fglyph.zeros)
                            };

                            let mut fg_cnt = [0u16; 256];
                            let mut bg_cnt = [0u16; 256];
                            let mut fg_len = 0;
                            let mut bg_len = 0;

                            for &i in fg_iter {
                                fg_cnt[indices[i] as usize] += 1;
                                fg_len += 1;
                            }
                            for &i in bg_iter {
                                bg_cnt[indices[i] as usize] += 1;
                                bg_len += 1;
                            }
                            let (fgi_raw, _) = fg_cnt
                                .iter()
                                .enumerate()
                                .max_by_key(|&(_, c)| c)
                                .unwrap_or((0, &0));
                            let (bgi_raw, _) = bg_cnt
                                .iter()
                                .enumerate()
                                .max_by_key(|&(_, c)| c)
                                .unwrap_or((0, &0));

                            let fgi = if fg_len == 0 { bgi_raw } else { fgi_raw };
                            let bgi = if bg_len == 0 { fgi_raw } else { bgi_raw };

                            scale_best_fg = unpack_rgb(palette[fgi.min(palette.len() - 1)]);
                            scale_best_bg = unpack_rgb(palette[bgi.min(palette.len() - 1)]);
                        } else {
                            let fglyph = &glyphs[scale_best_glyph];
                            let count_ones = fglyph.ones.len() as u64;
                            let mut sum_ones_r = 0u64;
                            let mut sum_ones_g = 0u64;
                            let mut sum_ones_b = 0u64;

                            for &i in &fglyph.ones {
                                let px = unsafe { pixels.get_unchecked(i) };
                                sum_ones_r += px.r;
                                sum_ones_g += px.g;
                                sum_ones_b += px.b;
                            }
                            let count_zeros = bp as u64 - count_ones;
                            let sum_zeros_r = block_sum_r - sum_ones_r;
                            let sum_zeros_g = block_sum_g - sum_ones_g;
                            let sum_zeros_b = block_sum_b - sum_ones_b;

                            scale_best_fg = if count_ones > 0 {
                                [
                                    (sum_ones_r / count_ones) as u8,
                                    (sum_ones_g / count_ones) as u8,
                                    (sum_ones_b / count_ones) as u8,
                                ]
                            } else {
                                [0, 0, 0]
                            };
                            scale_best_bg = if count_zeros > 0 {
                                [
                                    (sum_zeros_r / count_zeros) as u8,
                                    (sum_zeros_g / count_zeros) as u8,
                                    (sum_zeros_b / count_zeros) as u8,
                                ]
                            } else {
                                [0, 0, 0]
                            };
                        }

                        best_overall_glyph = scale_best_glyph;
                        best_overall_inv = scale_best_inv;
                        best_overall_fg = scale_best_fg;
                        best_overall_bg = scale_best_bg;
                    }
"""

    new_content = content[:start_idx] + replacement + content[end_idx:]
    with open('src/draw.rs', 'w') as f:
        f.write(new_content)

if __name__ == "__main__":
    main()
