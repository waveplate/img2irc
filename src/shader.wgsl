struct BlockPixel {
    r: u32,
    g: u32,
    b: u32,
    sq: u32,
}

// 4-byte aligned due to u32 instead of u8.
// The Rust side must map its `u8` bitmap arrays to `u32` buffer entries.
struct Glyph {
    ones_starts: u32,
    ones_len: u32,
}

struct Metadata {
    bp: u32,              // block pixel count = gw * gh
    num_glyphs: u32,
    render_mode: u32,     // 0 = Ansi, 1 = Irc/Preview, 2 = Ansi24
    tolerance: u32,       // Grayscale tolerance
}

@group(0) @binding(0) var<storage, read> blocks: array<BlockPixel>;
@group(0) @binding(1) var<storage, read> glyphs: array<Glyph>;
@group(0) @binding(2) var<storage, read> glyph_ones: array<u32>;
@group(0) @binding(3) var<storage, read> palette: array<u32>;
@group(0) @binding(4) var<uniform> metadata: Metadata;

// Output: best glyph index, best fg (r,g,b), best bg (r,g,b). Packed as u32s.
struct Result {
    best_cost: u32,
    best_glyph: u32,
    best_inv: u32,       // 0 = false, 1 = true
    fg_r: u32,
    fg_g: u32,
    fg_b: u32,
    bg_r: u32,
    bg_g: u32,
    bg_b: u32,
}

@group(0) @binding(5) var<storage, read_write> results: array<Result>;

fn is_near_grayscale(col: u32, tolerance: u32) -> bool {
    let r = (col >> 16u) & 0xFFu;
    let g = (col >> 8u) & 0xFFu;
    let b = col & 0xFFu;
    
    let min_val = min(r, min(g, b));
    let max_val = max(r, max(g, b));
    
    return max_val - min_val <= tolerance;
}

fn nearest_strict_partitioned_hex_colour(target_col: u32, tol: u32) -> u32 {
    let target_is_chromatic = !is_near_grayscale(target_col, tol);
    var best_i: u32 = 0u;
    var best_d: u32 = 0xFFFFFFFFu;
    
    let pr = (target_col >> 16u) & 0xFFu;
    let pg = (target_col >> 8u) & 0xFFu;
    let pb = target_col & 0xFFu;
    
    // palette size length limit depends on the Render argument; passed bound is 256 for ANSI or 99 for IRC.
    let palette_size = arrayLength(&palette);
    
    for (var i: u32 = 0u; i < palette_size; i++) {
        if (palette_size == 99u && i >= 2u && i <= 15u) {
            continue;
        }
        
        let p = palette[i];
        let p_is_chromatic = !is_near_grayscale(p, 0u);
        
        if (target_is_chromatic == p_is_chromatic) {
            let r = (p >> 16u) & 0xFFu;
            let g = (p >> 8u) & 0xFFu;
            let b = p & 0xFFu;
            
            // let dr = pr - r / dg...
            var diff = 0u;
            if (pr > r) { diff += (pr - r) * (pr - r); } else { diff += (r - pr) * (r - pr); }
            if (pg > g) { diff += (pg - g) * (pg - g); } else { diff += (g - pg) * (g - pg); }
            if (pb > b) { diff += (pb - b) * (pb - b); } else { diff += (b - pb) * (b - pb); }
            
            if (diff < best_d) {
                best_d = diff;
                best_i = i;
                if (diff == 0u) {
                    break;
                }
            }
        }
    }
    
    if (best_d == 0xFFFFFFFFu) {
        // Fallback: nearest_hex_colour_fast
        best_d = 0xFFFFFFFFu;
        for (var i: u32 = 0u; i < palette_size; i++) {
            if (palette_size == 99u && i >= 2u && i <= 15u) {
                continue;
            }
            let p = palette[i];
            let r = (p >> 16u) & 0xFFu;
            let g = (p >> 8u) & 0xFFu;
            let b = p & 0xFFu;
            var diff = 0u;
            if (pr > r) { diff += (pr - r) * (pr - r); } else { diff += (r - pr) * (r - pr); }
            if (pg > g) { diff += (pg - g) * (pg - g); } else { diff += (g - pg) * (g - pg); }
            if (pb > b) { diff += (pb - b) * (pb - b); } else { diff += (b - pb) * (b - pb); }
            
            if (diff < best_d) {
                best_d = diff;
                best_i = i;
                if (diff == 0u) {
                    break;
                }
            }
        }
    }
    
    return best_i;
}

// 1024 max bp (supports font sizes up to ~45x22)
var<private> indices: array<u32, 1024>; 

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let block_idx = global_id.x;
    let bp = metadata.bp;
    let total_blocks = arrayLength(&blocks) / bp;
    if (block_idx >= total_blocks) {
        return;
    }
    
    let base_pixel_idx = block_idx * bp;
    
    var block_sum_r: u32 = 0u;
    var block_sum_g: u32 = 0u;
    var block_sum_b: u32 = 0u;
    var block_sum_sq: u32 = 0u;

    if (metadata.render_mode != 2u) {
        for (var i: u32 = 0u; i < bp; i++) {
            let p = blocks[base_pixel_idx + i];
            let r = p.r;
            let g = p.g;
            let b = p.b;
            let rgb_u32 = (r << 16u) | (g << 8u) | b;
            let idx = nearest_strict_partitioned_hex_colour(rgb_u32, metadata.tolerance);
            indices[i] = idx;
        }
    } else {
        for (var i: u32 = 0u; i < bp; i++) {
            let p = blocks[base_pixel_idx + i];
            block_sum_r += p.r;
            block_sum_g += p.g;
            block_sum_b += p.b;
            block_sum_sq += p.sq;
        }
    }
    
    var scale_best_cost: u32 = 0xFFFFFFFFu;
    var scale_best_glyph: u32 = 0u;
    var scale_best_inv: u32 = 0u;
    var scale_best_fg_r: u32 = 0u;
    var scale_best_fg_g: u32 = 0u;
    var scale_best_fg_b: u32 = 0u;
    var scale_best_bg_r: u32 = 0u;
    var scale_best_bg_g: u32 = 0u;
    var scale_best_bg_b: u32 = 0u;
    
    if (metadata.render_mode != 2u) {
        // Indexed Mode
        let palette_size = arrayLength(&palette);
        
        var fg_cnt: array<u32, 256>;
        var bg_cnt: array<u32, 256>;
        
        var base_cnt: array<u32, 256>;
        for (var i: u32 = 0u; i < 256u; i++) {
            base_cnt[i] = 0u;
        }
        for (var i: u32 = 0u; i < bp; i++) {
            base_cnt[indices[i]]++;
        }

        for (var gi: u32 = 0u; gi < metadata.num_glyphs; gi++) {
            let glyph = glyphs[gi];
            let ones_start = glyph.ones_starts;
            let ones_len = glyph.ones_len;

            for (var inv_iter: u32 = 0u; inv_iter < 2u; inv_iter++) {
                
                for (var i: u32 = 0u; i < 256u; i++) {
                    if (base_cnt[i] > 0u) {
                        if (inv_iter == 0u) {
                            fg_cnt[i] = 0u;
                            bg_cnt[i] = base_cnt[i];
                        } else {
                            fg_cnt[i] = base_cnt[i];
                            bg_cnt[i] = 0u;
                        }
                    } else {
                        fg_cnt[i] = 0u;
                        bg_cnt[i] = 0u;
                    }
                }
                
                var fg_len: u32;
                var bg_len: u32;

                if (inv_iter == 0u) {
                    fg_len = ones_len;
                    bg_len = bp - ones_len;
                    for (var j: u32 = 0u; j < ones_len; j++) {
                        let p = glyph_ones[ones_start + j];
                        let idx = indices[p];
                        bg_cnt[idx]--;
                        fg_cnt[idx]++;
                    }
                } else {
                    fg_len = bp - ones_len;
                    bg_len = ones_len;
                    for (var j: u32 = 0u; j < ones_len; j++) {
                        let p = glyph_ones[ones_start + j];
                        let idx = indices[p];
                        fg_cnt[idx]--;
                        bg_cnt[idx]++;
                    }
                }
                
                // Find modes
                var fgi_raw: u32 = 0u;
                var fx: u32 = 0u;
                var bgi_raw: u32 = 0u;
                var bx: u32 = 0u;
                
                for (var i: u32 = 0u; i < 256u; i++) {
                    let fc = fg_cnt[i];
                    if (fc >= fx) {
                        fx = fc;
                        fgi_raw = i;
                    }
                    let bc = bg_cnt[i];
                    if (bc >= bx) {
                        bx = bc;
                        bgi_raw = i;
                    }
                }
                
                var fgi: u32 = fgi_raw;
                if (fg_len == 0u) { fgi = bgi_raw; }
                var bgi: u32 = bgi_raw;
                if (bg_len == 0u) { bgi = fgi_raw; }
                
                let cost = (fg_len - fx) + (bg_len - bx);
                
                if (cost < scale_best_cost) {
                    scale_best_cost = cost;
                    scale_best_glyph = gi;
                    scale_best_inv = inv_iter;
                    
                    let safe_fgi = min(fgi, palette_size - 1u);
                    scale_best_fg_r = (palette[safe_fgi] >> 16u) & 0xFFu;
                    scale_best_fg_g = (palette[safe_fgi] >> 8u) & 0xFFu;
                    scale_best_fg_b = palette[safe_fgi] & 0xFFu;
                    
                    let safe_bgi = min(bgi, palette_size - 1u);
                    scale_best_bg_r = (palette[safe_bgi] >> 16u) & 0xFFu;
                    scale_best_bg_g = (palette[safe_bgi] >> 8u) & 0xFFu;
                    scale_best_bg_b = palette[safe_bgi] & 0xFFu;
                }
            }
        }
        
    } else {
        // TrueColor Mode
        let term_sq = (block_sum_r * block_sum_r / bp) + (block_sum_g * block_sum_g / bp) + (block_sum_b * block_sum_b / bp);
        let uniform_cost = (block_sum_sq + bp * term_sq); // simplified, not matching CPU exactly yet, but actually CPU `uniform_cost` is a bit complex.
        
        for (var gi: u32 = 0u; gi < metadata.num_glyphs; gi++) {
            let glyph = glyphs[gi];
            var sum_ones_r: u32 = 0u;
            var sum_ones_g: u32 = 0u;
            var sum_ones_b: u32 = 0u;
            var sum_ones_sq: u32 = 0u;
            let count_ones = glyph.ones_len;
            
            for (var j: u32 = 0u; j < count_ones; j++) {
                let px_idx = glyph_ones[glyph.ones_starts + j];
                let px = blocks[base_pixel_idx + px_idx];
                sum_ones_r += px.r;
                sum_ones_g += px.g;
                sum_ones_b += px.b;
                sum_ones_sq += px.sq;
            }
            
            let count_zeros = bp - count_ones;
            let sum_zeros_r = block_sum_r - sum_ones_r;
            let sum_zeros_g = block_sum_g - sum_ones_g;
            let sum_zeros_b = block_sum_b - sum_ones_b;
            let sum_zeros_sq = block_sum_sq - sum_ones_sq;
            
            var avg_ones_r: u32 = 0u;
            var avg_ones_g: u32 = 0u;
            var avg_ones_b: u32 = 0u;
            if (count_ones > 0u) {
                avg_ones_r = sum_ones_r / count_ones;
                avg_ones_g = sum_ones_g / count_ones;
                avg_ones_b = sum_ones_b / count_ones;
            }
            
            var avg_zeros_r: u32 = 0u;
            var avg_zeros_g: u32 = 0u;
            var avg_zeros_b: u32 = 0u;
            if (count_zeros > 0u) {
                avg_zeros_r = sum_zeros_r / count_zeros;
                avg_zeros_g = sum_zeros_g / count_zeros;
                avg_zeros_b = sum_zeros_b / count_zeros;
            }
            
            let t1_1 = sum_ones_sq;
            let t2_1 = 2u * (avg_ones_r * sum_ones_r + avg_ones_g * sum_ones_g + avg_ones_b * sum_ones_b);
            let t3_1 = count_ones * (avg_ones_r * avg_ones_r + avg_ones_g * avg_ones_g + avg_ones_b * avg_ones_b);
            var c1: u32 = 0u;
            if (t1_1 + t3_1 > t2_1) { c1 = t1_1 + t3_1 - t2_1; }
            
            let t1_0 = sum_zeros_sq;
            let t2_0 = 2u * (avg_zeros_r * sum_zeros_r + avg_zeros_g * sum_zeros_g + avg_zeros_b * sum_zeros_b);
            let t3_0 = count_zeros * (avg_zeros_r * avg_zeros_r + avg_zeros_g * avg_zeros_g + avg_zeros_b * avg_zeros_b);
            var c0: u32 = 0u;
            if (t1_0 + t3_0 > t2_0) { c0 = t1_0 + t3_0 - t2_0; }
            
            let cost_normal = c1 + c0;
            if (cost_normal < scale_best_cost) {
                scale_best_cost = cost_normal;
                scale_best_glyph = gi;
                scale_best_inv = 0u;
                scale_best_fg_r = avg_ones_r;
                scale_best_fg_g = avg_ones_g;
                scale_best_fg_b = avg_ones_b;
                scale_best_bg_r = avg_zeros_r;
                scale_best_bg_g = avg_zeros_g;
                scale_best_bg_b = avg_zeros_b;
            }
        }
        
        let term_linear = (block_sum_r / bp) * block_sum_r + (block_sum_g / bp) * block_sum_g + (block_sum_b / bp) * block_sum_b;
        var act_uniform = 0u;
        let uniform_term = block_sum_sq + bp * term_sq;
        if (uniform_term > 2u * term_linear) { act_uniform = uniform_term - 2u * term_linear; }
        var gain = 0u;
        if (act_uniform > scale_best_cost) {
            gain = act_uniform - scale_best_cost;
        }
        let noise_gain_threshold = 800u;
        if (gain < noise_gain_threshold) { // CPU `args.smooth` check, GPU will always smooth when in auto? Yes!
            let r = block_sum_r / bp;
            let g = block_sum_g / bp;
            let b = block_sum_b / bp;
            scale_best_fg_r = r;
            scale_best_fg_g = g;
            scale_best_fg_b = b;
            scale_best_bg_r = r;
            scale_best_bg_g = g;
            scale_best_bg_b = b;
        }
    }
    
    results[block_idx].best_cost = scale_best_cost;
    results[block_idx].best_glyph = scale_best_glyph;
    results[block_idx].best_inv = scale_best_inv;
    results[block_idx].fg_r = scale_best_fg_r;
    results[block_idx].fg_g = scale_best_fg_g;
    results[block_idx].fg_b = scale_best_fg_b;
    results[block_idx].bg_r = scale_best_bg_r;
    results[block_idx].bg_g = scale_best_bg_g;
    results[block_idx].bg_b = scale_best_bg_b;
}
