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
        if (palette_size == 99u && i < 16u) {
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
            if (palette_size == 99u && i < 16u) {
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

    for (var i: u32 = 0u; i < bp; i++) {
        let p = blocks[base_pixel_idx + i];
        let r = p.r;
        let g = p.g;
        let b = p.b;
        if (metadata.render_mode == 2u) {
            block_sum_r += r;
            block_sum_g += g;
            block_sum_b += b;
        }
        let rgb_u32 = (r << 16u) | (g << 8u) | b;
        let idx = nearest_strict_partitioned_hex_colour(rgb_u32, metadata.tolerance);
        indices[i] = idx;
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
    var scale_best_fgi: u32 = 0u;
    var scale_best_bgi: u32 = 0u;
    
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
                scale_best_fgi = fgi;
                scale_best_bgi = bgi;
                
                if (metadata.render_mode != 2u) {
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
    }
    
    if (metadata.render_mode == 2u) {
        let glyph = glyphs[scale_best_glyph];
        let ones_start = glyph.ones_starts;
        let count_ones = glyph.ones_len;
        
        // Find a pixel matching scale_best_fgi under the foreground group
        var found_fg = false;
        for (var j: u32 = 0u; j < bp; j++) {
            var is_one = false;
            for (var k: u32 = 0u; k < count_ones; k++) {
                if (glyph_ones[ones_start + k] == j) {
                    is_one = true;
                    break;
                }
            }
            let is_target = (scale_best_inv == 0u && is_one) || (scale_best_inv == 1u && !is_one);
            if (is_target) {
                if (indices[j] == scale_best_fgi) {
                    let px = blocks[base_pixel_idx + j];
                    scale_best_fg_r = px.r;
                    scale_best_fg_g = px.g;
                    scale_best_fg_b = px.b;
                    found_fg = true;
                    break;
                }
            }
        }
        if (!found_fg) {
            let safe_fgi = min(scale_best_fgi, palette_size - 1u);
            scale_best_fg_r = (palette[safe_fgi] >> 16u) & 0xFFu;
            scale_best_fg_g = (palette[safe_fgi] >> 8u) & 0xFFu;
            scale_best_fg_b = palette[safe_fgi] & 0xFFu;
        }
        
        // Find a pixel matching scale_best_bgi under the background group
        var found_bg = false;
        for (var j: u32 = 0u; j < bp; j++) {
            var is_one = false;
            for (var k: u32 = 0u; k < count_ones; k++) {
                if (glyph_ones[ones_start + k] == j) {
                    is_one = true;
                    break;
                }
            }
            let is_target = (scale_best_inv == 0u && !is_one) || (scale_best_inv == 1u && is_one);
            if (is_target) {
                if (indices[j] == scale_best_bgi) {
                    let px = blocks[base_pixel_idx + j];
                    scale_best_bg_r = px.r;
                    scale_best_bg_g = px.g;
                    scale_best_bg_b = px.b;
                    found_bg = true;
                    break;
                }
            }
        }
        if (!found_bg) {
            let safe_bgi = min(scale_best_bgi, palette_size - 1u);
            scale_best_bg_r = (palette[safe_bgi] >> 16u) & 0xFFu;
            scale_best_bg_g = (palette[safe_bgi] >> 8u) & 0xFFu;
            scale_best_bg_b = palette[safe_bgi] & 0xFFu;
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
