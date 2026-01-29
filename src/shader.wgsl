struct Globals {
    img_width: u32,
    img_height: u32,
    block_width: u32,
    block_height: u32,
    num_glyphs: u32,
    mode: u32, // 0=ANSI, 1=IRC, 2=ANSI24
    flags: u32, // bit 0 = fg_only, bit 1 = has_fixed_bg
    fixed_bg: u32, // packed rgb
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

fn find_closest_palette(c: vec3<f32>) -> vec3<f32> {
    if (globals.mode == 2u) { return c; } // Truecolor
    
    var best_d = 1e38;
    var best_c = vec3<f32>(0.0);
    
    let limit = select(256u, 99u, globals.mode == 1u); // IRC99 has 99 colors
    
    for (var i = 0u; i < limit; i = i + 1u) {
        let pc = unpack_rgb(palette[i]);
        let d = dot(c - pc, c - pc);
        if (d < best_d) {
            best_d = d;
            best_c = pc;
        }
    }
    return best_c;
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

    var best_error = 1e38;
    var best_glyph = 0u;
    var best_fg = vec3<f32>(0.0);
    var best_bg = vec3<f32>(0.0);
    var best_inv = 0u;

    let fg_only = (globals.flags & 1u) != 0u;
    let has_fixed_bg = (globals.flags & 2u) != 0u;
    let fixed_bg_rgb = unpack_rgb(globals.fixed_bg);

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

        // Normal (Non-inverted)
        var fg = vec3<f32>(0.0);
        if (count_ones > 0u) { fg = find_closest_palette(sum_ones / f32(count_ones)); }
        
        var bg = vec3<f32>(0.0);
        if (fg_only) {
            if (has_fixed_bg) {
                bg = find_closest_palette(fixed_bg_rgb);
            } else {
                bg = vec3<f32>(0.0); // Assume black background if none specified
            }
        } else {
            if (count_zeros > 0u) { bg = find_closest_palette(sum_zeros / f32(count_zeros)); }
        }

        let err_ones = (sum_ones_sq) - (2.0 * fg * sum_ones) + (f32(count_ones) * fg * fg);
        let err_zeros = (sum_zeros_sq) - (2.0 * bg * sum_zeros) + (f32(count_zeros) * bg * bg);
        var total_err_norm = dot(err_ones + err_zeros, vec3<f32>(1.0));

        // Stable Tie-breaker: prefer glyphs with more coverage (only affects ties or near-ties)
        let coverage = f32(count_ones) / f32(valid_pixels);
        total_err_norm = total_err_norm - (coverage * 0.001);

        if (total_err_norm < best_error) {
            best_error = total_err_norm;
            best_glyph = i;
            best_fg = fg;
            best_bg = bg;
            best_inv = 0u;
        }

        // Inverted (avg_zeros becomes FG, avg_ones becomes BG)
        if (!fg_only) {
            var fg_inv = bg;
            var bg_inv = fg;
            var total_err_inv = dot(err_ones + err_zeros, vec3<f32>(1.0)); // same error base
            
            // Inverted Tie-breaker: use inverted coverage
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
    }

    let out_idx = by * block_cols + bx;
    output[out_idx].glyph_index = best_glyph;
    output[out_idx].fg = pack_rgb(best_fg);
    output[out_idx].bg = pack_rgb(best_bg);
    output[out_idx].flags = best_inv;
}
