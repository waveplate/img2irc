struct Globals {
    img_width: u32,
    img_height: u32,
    block_width: u32,
    block_height: u32,
    num_glyphs: u32,
    mode: u32, // 0=ANSI, 1=IRC, 2=ANSI24
    flags: u32, // bit 0 = fg_only
}

struct GlyphInfo {
    width: u32,
    height: u32,
    offset: u32,
    padding: u32,
}

struct OutputCell {
    glyph_index: u32,
    fg: u32,
    bg: u32,
    flags: u32,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var<storage, read> pixels: array<u32>; // Packed RGB
@group(0) @binding(2) var<storage, read> glyph_infos: array<GlyphInfo>;
@group(0) @binding(3) var<storage, read> glyph_bitmaps: array<u32>; 
@group(0) @binding(4) var<storage, read_write> output: array<OutputCell>;
@group(0) @binding(5) var<storage, read> palette: array<u32>;

fn unpack_rgb(c: u32) -> vec3<f32> {
    return vec3<f32>(
        f32((c >> 16u) & 0xFFu),
        f32((c >> 8u) & 0xFFu),
        f32((c >> 0u) & 0xFFu)
    );
}

fn pack_rgb(c: vec3<f32>) -> u32 {
    let r = u32(clamp(c.r, 0.0, 255.0));
    let g = u32(clamp(c.g, 0.0, 255.0));
    let b = u32(clamp(c.b, 0.0, 255.0));
    return (r << 16u) | (g << 8u) | b;
}

fn get_glyph_pixel(offset: u32, idx: u32) -> u32 {
    let total_idx = offset + idx;
    let word_idx = total_idx / 4u;
    let byte_idx = total_idx % 4u;
    let word = glyph_bitmaps[word_idx];
    let shift = byte_idx * 8u;
    return (word >> shift) & 0xFFu;
}

fn find_closest_palette_index(c: vec3<f32>) -> u32 {
    var best_d = 1e38;
    var best_i = 0u;
    
    let limit = select(256u, 99u, globals.mode == 1u); // IRC99 has 99 colors
    
    for (var i = 0u; i < limit; i = i + 1u) {
        let pc = unpack_rgb(palette[i]);
        let d = dot(c - pc, c - pc);
        if (d < best_d) {
            best_d = d;
            best_i = i;
        }
    }
    return best_i;
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let bx = global_id.x;
    let by = global_id.y;
    
    let block_cols = (globals.img_width + globals.block_width - 1u) / globals.block_width;
    let block_rows = (globals.img_height + globals.block_height - 1u) / globals.block_height;

    if (bx >= block_cols || by >= block_rows) {
        return;
    }

    let start_x = bx * globals.block_width;
    let start_y = by * globals.block_height;
    let fg_only = (globals.flags & 1u) != 0u;

    var best_error = 1e38; // Used for TrueColor
    var best_score = -1000000; // Used for Indexed (higher is better)
    var best_glyph = 0u;
    var best_fg = vec3<f32>(0.0);
    var best_bg = vec3<f32>(0.0);
    var best_inv = 0u;

    if (globals.mode == 2u) {
        // --- TrueColor Logic (Averaging) ---
        var block_sum = vec3<f32>(0.0);
        var block_sq_sum = vec3<f32>(0.0);
        var valid_pixels = 0u;

        for (var y = 0u; y < globals.block_height; y = y + 1u) {
            for (var x = 0u; x < globals.block_width; x = x + 1u) {
                let px = start_x + x;
                let py = start_y + y;
                if (px < globals.img_width && py < globals.img_height) {
                    let p = unpack_rgb(pixels[py * globals.img_width + px]);
                    block_sum = block_sum + p;
                    block_sq_sum = block_sq_sum + (p * p);
                    valid_pixels = valid_pixels + 1u;
                }
            }
        }

        for (var i = 0u; i < globals.num_glyphs; i = i + 1u) {
            let info = glyph_infos[i];
            
            var sum_ones = vec3<f32>(0.0);
            var sum_ones_sq = vec3<f32>(0.0);
            var count_ones = 0u;
            
            for (var y = 0u; y < globals.block_height; y = y + 1u) {
                for (var x = 0u; x < globals.block_width; x = x + 1u) {
                    if (x >= info.width || y >= info.height) { continue; }
                    let px = start_x + x;
                    let py = start_y + y;
                    if (px >= globals.img_width || py >= globals.img_height) { continue; }

                    let bit = get_glyph_pixel(info.offset, y * info.width + x);
                    if (bit > 0u) {
                        let p = unpack_rgb(pixels[py * globals.img_width + px]);
                        sum_ones = sum_ones + p;
                        sum_ones_sq = sum_ones_sq + (p * p);
                        count_ones = count_ones + 1u;
                    }
                }
            }

            let count_zeros = valid_pixels - count_ones;
            let sum_zeros = block_sum - sum_ones;
            let sum_zeros_sq = block_sq_sum - sum_ones_sq;

            var fg = vec3<f32>(0.0);
            if (count_ones > 0u) { fg = sum_ones / f32(count_ones); }
            
            var bg = vec3<f32>(0.0);
            if (!fg_only && count_zeros > 0u) { bg = sum_zeros / f32(count_zeros); }

            let err_ones = (sum_ones_sq) - (2.0 * fg * sum_ones) + (f32(count_ones) * fg * fg);
            let err_zeros = (sum_zeros_sq) - (2.0 * bg * sum_zeros) + (f32(count_zeros) * bg * bg);
            
            // Heuristic: If fg_only, we penalize background pixels (count_zeros)
            // Instead of error, we treat 'bg' error as fixed penalty
            var total_err_norm = 0.0;
            if (fg_only) {
                total_err_norm = dot(err_ones, vec3<f32>(1.0)) + (f32(count_zeros) * 1000.0);
            } else {
                total_err_norm = dot(err_ones + err_zeros, vec3<f32>(1.0));
            }

            let coverage = f32(count_ones) / f32(valid_pixels);
            total_err_norm = total_err_norm - (coverage * 0.001);

            if (total_err_norm < best_error) {
                best_error = total_err_norm;
                best_glyph = i;
                best_fg = fg;
                best_bg = bg;
                best_inv = 0u;
            }

            // Inverted Logic (Only if !fg_only or if we want to support it with penalty)
            // If fg_only, we can still invert, but BG becomes FG (Color), and FG becomes BG (Black/Transparent).
            // This is useful if the glyph shape is inverted.
            
            // Inverted: Visual 1s (Orig 0s) -> FG. Visual 0s (Orig 1s) -> BG.
            var fg_inv = vec3<f32>(0.0);
            if (count_zeros > 0u) { fg_inv = sum_zeros / f32(count_zeros); }
            
            var bg_inv = vec3<f32>(0.0);
            if (!fg_only && count_ones > 0u) { bg_inv = sum_ones / f32(count_ones); }

            let err_ones_inv = (sum_zeros_sq) - (2.0 * fg_inv * sum_zeros) + (f32(count_zeros) * fg_inv * fg_inv);
            let err_zeros_inv = (sum_ones_sq) - (2.0 * bg_inv * sum_ones) + (f32(count_ones) * bg_inv * bg_inv);

            var total_err_inv = 0.0;
            if (fg_only) {
                 total_err_inv = dot(err_ones_inv, vec3<f32>(1.0)) + (f32(count_ones) * 1000.0);
            } else {
                 total_err_inv = dot(err_ones_inv + err_zeros_inv, vec3<f32>(1.0));
            }
            
            // Coverage for inverted: We care about Visual 1s (Orig 0s)
            let inv_coverage = f32(count_zeros) / f32(valid_pixels);
            total_err_inv = total_err_inv - (inv_coverage * 0.001);

            if (total_err_inv < best_error) { 
                 best_error = total_err_inv;
                 best_glyph = i;
                 best_fg = fg_inv;
                 best_bg = bg_inv;
                 best_inv = 1u;
            }
        }
        
        // Denoising: Force Refined Flat if gain is small (Matches CPU Threshold of 800)
        let uniform_avg = block_sum / f32(valid_pixels);
        let u_t1 = block_sq_sum;
        let u_t2 = 2.0 * uniform_avg * block_sum;
        let u_t3 = f32(valid_pixels) * uniform_avg * uniform_avg;
        let uniform_err_vec = u_t1 - u_t2 + u_t3;
        let uniform_err = dot(uniform_err_vec, vec3<f32>(1.0));
        
        if ((uniform_err - best_error) < 800.0) {
             let refined = vec3<f32>(
                 floor(uniform_avg.r + 0.5),
                 floor(uniform_avg.g + 0.5),
                 floor(uniform_avg.b + 0.5)
             );
             best_fg = refined;
             best_bg = refined;
        }

    } else {
        // --- Indexed Logic (Histogram / Dominant Color) ---
        // Maximum palette size is 256 (ANSI256). IRC is 99.
        var hist = array<u32, 256>(); 
        var block_indices = array<u32, 1024>(); // Max 32x32 block support
        var valid_pixels = 0u;

        // 1. Quantize & Build Histogram
        for (var y = 0u; y < globals.block_height; y = y + 1u) {
            for (var x = 0u; x < globals.block_width; x = x + 1u) {
                let px = start_x + x;
                let py = start_y + y;
                if (px < globals.img_width && py < globals.img_height) {
                    let p = unpack_rgb(pixels[py * globals.img_width + px]);
                    let idx = find_closest_palette_index(p);
                    if (valid_pixels < 1024u) { block_indices[valid_pixels] = idx; }
                    hist[idx] = hist[idx] + 1u;
                    valid_pixels = valid_pixels + 1u;
                }
            }
        }

        // 2. Find Top 16 Candidates
        var candidates = array<u32, 16>(0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u);
        var counts = array<u32, 16>(0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u, 0u);
        
        for (var i = 0u; i < 256u; i = i + 1u) {
            let c = hist[i];
            if (c > counts[15]) {
                // Insert sorted
                var k = 0u;
                loop {
                    if (k >= 16u) { break; }
                    if (c > counts[k]) {
                        // Shift down
                        for (var j = 15u; j > k; j = j - 1u) {
                            counts[j] = counts[j - 1u];
                            candidates[j] = candidates[j - 1u];
                        }
                        counts[k] = c;
                        candidates[k] = i;
                        break;
                    }
                    k = k + 1u;
                }
            }
        }

        // 3. Iterate Glyphs and Match Candidates
        for (var i = 0u; i < globals.num_glyphs; i = i + 1u) {
            let info = glyph_infos[i];
            
            // Loop Inverted status: 0=Normal, 1=Inverted
            for (var inv = 0u; inv < 2u; inv = inv + 1u) {
                
                // For each candidate as FG
                for (var c_fg_idx = 0u; c_fg_idx < 16u; c_fg_idx = c_fg_idx + 1u) {
                    if (counts[c_fg_idx] == 0u) { continue; }
                    let fg_idx = candidates[c_fg_idx];
                    
                    var fg_matches = 0u;
                    var fg_total_bits = 0u;
                    
                    // Count matches for this FG candidate
                    var p_idx = 0u;
                    for (var y = 0u; y < globals.block_height; y = y + 1u) {
                        for (var x = 0u; x < globals.block_width; x = x + 1u) {
                            if (x >= info.width || y >= info.height) { continue; }
                            let px = start_x + x;
                            let py = start_y + y;
                            if (px >= globals.img_width || py >= globals.img_height) { continue; }
                            
                            let bit = get_glyph_pixel(info.offset, y * info.width + x);
                            // Visual Bit: If inv=1, bit 0 -> 1, bit 1 -> 0.
                            var vis_bit = 0u;
                            if (bit > 0u) { vis_bit = 1u; }
                            if (inv == 1u) { vis_bit = 1u - vis_bit; }
                            
                            if (vis_bit == 1u) {
                                fg_total_bits = fg_total_bits + 1u;
                                if (block_indices[p_idx] == fg_idx) { fg_matches = fg_matches + 1u; }
                            }
                            
                            p_idx = p_idx + 1u;
                        }
                    }
                    
                    var score = 0;
                    var best_local_bg = 0u;
                    
                    if (fg_only) {
                        // Score = (2 * Matches) - TotalBits. Prefers dense matches.
                        score = (2 * i32(fg_matches)) - i32(fg_total_bits);
                        best_local_bg = 0u; // Black
                    } else {
                        // Find best BG candidate
                        var max_bg_matches = 0u;
                        var best_bg_c = 0u;
                        
                        for (var c_bg_idx = 0u; c_bg_idx < 16u; c_bg_idx = c_bg_idx + 1u) {
                             if (counts[c_bg_idx] == 0u) { continue; }
                             let bg_idx = candidates[c_bg_idx];
                             
                             var bg_matches = 0u;
                             
                             var p2_idx = 0u;
                             for (var y = 0u; y < globals.block_height; y = y + 1u) {
                                for (var x = 0u; x < globals.block_width; x = x + 1u) {
                                    if (x >= info.width || y >= info.height) { continue; }
                                    let px = start_x + x;
                                    let py = start_y + y;
                                    if (px >= globals.img_width || py >= globals.img_height) { continue; }
                                    
                                    let bit = get_glyph_pixel(info.offset, y * info.width + x);
                                    var vis_bit = 0u;
                                    if (bit > 0u) { vis_bit = 1u; }
                                    if (inv == 1u) { vis_bit = 1u - vis_bit; }
                                    
                                    if (vis_bit == 0u) {
                                        if (block_indices[p2_idx] == bg_idx) { bg_matches = bg_matches + 1u; }
                                    }
                                    p2_idx = p2_idx + 1u;
                                }
                             }
                             
                             if (bg_matches > max_bg_matches) {
                                 max_bg_matches = bg_matches;
                                 best_bg_c = bg_idx;
                             }
                        }
                        
                        // Score = FG_Matches + BG_Matches
                        score = i32(fg_matches + max_bg_matches);
                        best_local_bg = best_bg_c;
                    }
                    
                    if (score > best_score) {
                        best_score = score;
                        best_glyph = i;
                        best_inv = inv;
                        
                        let fg_col = unpack_rgb(palette[fg_idx]);
                        let bg_col = unpack_rgb(palette[best_local_bg]);
                        
                        best_fg = fg_col;
                        best_bg = bg_col;
                    }
                }
            }
        }
    }

    let out_idx = by * block_cols + bx;
    output[out_idx].glyph_index = best_glyph;
    output[out_idx].fg = pack_rgb(best_fg);
    output[out_idx].bg = pack_rgb(best_bg);
    output[out_idx].flags = best_inv;
}
