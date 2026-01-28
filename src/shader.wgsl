struct Globals {
    img_width: u32,
    img_height: u32,
    block_width: u32,
    block_height: u32,
    num_glyphs: u32,
    mode: u32, // 0=ANSI, 1=IRC, 2=ANSI24
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
// WGSL doesn't support u8 storage buffers directly. We read as u32 and unpack.
@group(0) @binding(3) var<storage, read> glyph_bitmaps: array<u32>; 
@group(0) @binding(4) var<storage, read_write> output: array<OutputCell>;

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

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let bx = global_id.x;
    let by = global_id.y;
    
    let block_cols = (globals.img_width + globals.block_width - 1u) / globals.block_width;
    let block_rows = (globals.img_height + globals.block_height - 1u) / globals.block_height;

    if (bx >= block_cols || by >= block_rows) {
        return;
    }

    // Extract block pixels
    let start_x = bx * globals.block_width;
    let start_y = by * globals.block_height;
    
    var block_sum = vec3<f32>(0.0);
    var block_sq_sum = vec3<f32>(0.0);
    var valid_pixels = 0u;

    // Iterate glyphs
    var best_error = 3.40282347e+38; // Max f32
    var best_glyph = 0u;
    var best_fg = vec3<f32>(0.0);
    var best_bg = vec3<f32>(0.0);
    var best_inv = 0u;

    // Pre-calculate block stats
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

                let bit_idx = y * info.width + x;
                // Use helper to read byte
                let bit = get_glyph_pixel(info.offset, bit_idx);
                
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

        var avg_ones = vec3<f32>(0.0);
        if (count_ones > 0u) { avg_ones = sum_ones / f32(count_ones); }
        
        var avg_zeros = vec3<f32>(0.0);
        if (count_zeros > 0u) { avg_zeros = sum_zeros / f32(count_zeros); }

        let err_ones = (sum_ones_sq) - (2.0 * avg_ones * sum_ones) + (f32(count_ones) * avg_ones * avg_ones);
        let err_zeros = (sum_zeros_sq) - (2.0 * avg_zeros * sum_zeros) + (f32(count_zeros) * avg_zeros * avg_zeros);
        let total_err_norm = (err_ones.r + err_ones.g + err_ones.b) + (err_zeros.r + err_zeros.g + err_zeros.b);

        if (total_err_norm < best_error) {
            best_error = total_err_norm;
            best_glyph = i;
            best_fg = avg_ones;
            best_bg = avg_zeros;
            best_inv = 0u;
        }

        // Inverted case
        if (total_err_norm < best_error - 0.1) { 
             best_error = total_err_norm;
             best_glyph = i;
             best_fg = avg_zeros;
             best_bg = avg_ones;
             best_inv = 1u;
        }
    }

    let out_idx = by * block_cols + bx;
    output[out_idx].glyph_index = best_glyph;
    output[out_idx].fg = pack_rgb(best_fg);
    output[out_idx].bg = pack_rgb(best_bg);
    output[out_idx].flags = best_inv;
}