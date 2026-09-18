import sys

with open('src/draw.rs', 'r') as f:
    lines = f.readlines()

# Verify initial lines to be absolutely certain before writing
if 'fn seam_color_mismatch' not in lines[1458]:
    print('Error: seam_color_mismatch mismatch')
    sys.exit(1)

if '#[inline(always)]' not in lines[1979]:
    print('Error: allowed_local_discontinuity_score mismatch')
    sys.exit(1)

if 'fn affected_discontinuity_cell_indices' not in lines[2200]:
    print('Error: affected_discontinuity_cell_indices mismatch')
    sys.exit(1)

# Perform deletions and replacements from the bottom up to keep indexes stable
# 3. Delete dead Range 2 functions (lines 2201 to 2235, 0-indexed: 2200:2235)
del lines[2200:2235]

# 2. Replace dead functions & old cell_discontinuity_score (lines 1980 to 2185, 0-indexed: 1979:2185)
replacement_code = """fn get_cell_shape_bg_colors(
    cell_idx: usize,
    grid_data: &[BlockData],
    raster: &[[u8; 3]],
    grid_w: usize,
    gw: usize,
    gh: usize,
) -> ([u8; 3], [u8; 3]) {
    if !grid_data.is_empty() && cell_idx < grid_data.len() {
        let data = &grid_data[cell_idx];
        if data.inv {
            (data.bg, data.fg)
        } else {
            (data.fg, data.bg)
        }
    } else {
        let cx = cell_idx % grid_w;
        let cy = cell_idx / grid_w;
        let width = grid_w * gw;
        let mut colors = Vec::new();
        for dy in 0..gh {
            let y = cy * gh + dy;
            for dx in 0..gw {
                let p = raster[y * width + cx * gw + dx];
                if !colors.contains(&p) {
                    colors.push(p);
                }
            }
        }
        if colors.is_empty() {
            ([0, 0, 0], [0, 0, 0])
        } else if colors.len() == 1 {
            (colors[0], colors[0])
        } else {
            let mut counts = vec![0u32; colors.len()];
            for dy in 0..gh {
                let y = cy * gh + dy;
                for dx in 0..gw {
                    let p = raster[y * width + cx * gw + dx];
                    if let Some(pos) = colors.iter().position(|&c| c == p) {
                        counts[pos] += 1;
                    }
                }
            }
            let mut indices: Vec<usize> = (0..colors.len()).collect();
            indices.sort_by_key(|&i| std::cmp::Reverse(counts[i]));
            if indices.len() >= 2 {
                (colors[indices[1]], colors[indices[0]])
            } else {
                (colors[indices[0]], colors[indices[0]])
            }
        }
    }
}

fn seam_shape_mismatch(
    c_left: &[[u8; 3]],
    c_right: &[[u8; 3]],
    left_shape: [u8; 3],
    left_bg: [u8; 3],
    right_shape: [u8; 3],
    right_bg: [u8; 3],
) -> f32 {
    let n = c_left.len();
    if n == 0 {
        return 0.0;
    }

    let get_val = |p: [u8; 3], shape: [u8; 3], bg: [u8; 3]| -> f32 {
        if shape == bg {
            0.0
        } else {
            let dist_shape = normalized_rgb_distance(p, shape);
            let dist_bg = normalized_rgb_distance(p, bg);
            if dist_shape + dist_bg < 1e-5 {
                0.0
            } else {
                dist_bg / (dist_shape + dist_bg)
            }
        }
    };

    let mut total_mismatch = 0.0f32;
    for y in 0..n {
        let v_left_y = get_val(c_left[y], left_shape, left_bg);
        
        let mut best_dist_left = 1.0f32;
        let y_start = y.saturating_sub(1);
        let y_end = (y + 1).min(n - 1);
        for dy in y_start..=y_end {
            let v_right_dy = get_val(c_right[dy], right_shape, right_bg);
            let dist = (v_left_y - v_right_dy).abs();
            if dist < best_dist_left {
                best_dist_left = dist;
            }
        }

        let v_right_y = get_val(c_right[y], right_shape, right_bg);
        let mut best_dist_right = 1.0f32;
        for dy in y_start..=y_end {
            let v_left_dy = get_val(c_left[dy], left_shape, left_bg);
            let dist = (v_right_y - v_left_dy).abs();
            if dist < best_dist_right {
                best_dist_right = dist;
            }
        }

        total_mismatch += best_dist_left.max(best_dist_right);
    }
    total_mismatch / n as f32
}

fn cell_discontinuity_score(
    curr_idx: usize,
    raster: &[[u8; 3]],
    grid_data: &[BlockData],
    grid_w: usize,
    grid_h: usize,
    gw: usize,
    gh: usize,
) -> f32 {
    let cx = curr_idx % grid_w;
    let cy = curr_idx / grid_w;
    let width = grid_w * gw;
    let height = grid_h * gh;

    let mut max_mismatch = 0.0f32;

    // Left seam
    if cx > 0 {
        let seam_x = cx * gw;
        let mut c_left = Vec::with_capacity(gh);
        let mut c_right = Vec::with_capacity(gh);
        for dy in 0..gh {
            let y = cy * gh + dy;
            c_left.push(raster[y * width + seam_x - 1]);
            c_right.push(raster[y * width + seam_x]);
        }
        let left_idx = curr_idx - 1;
        let (left_shape, left_bg) = get_cell_shape_bg_colors(left_idx, grid_data, raster, grid_w, gw, gh);
        let (right_shape, right_bg) = get_cell_shape_bg_colors(curr_idx, grid_data, raster, grid_w, gw, gh);
        let mismatch = seam_shape_mismatch(&c_left, &c_right, left_shape, left_bg, right_shape, right_bg);
        if mismatch > max_mismatch {
            max_mismatch = mismatch;
        }
    }

    // Right seam
    if cx + 1 < grid_w {
        let seam_x = (cx + 1) * gw;
        let mut c_left = Vec::with_capacity(gh);
        let mut c_right = Vec::with_capacity(gh);
        for dy in 0..gh {
            let y = cy * gh + dy;
            c_left.push(raster[y * width + seam_x - 1]);
            c_right.push(raster[y * width + seam_x]);
        }
        let right_idx = curr_idx + 1;
        let (left_shape, left_bg) = get_cell_shape_bg_colors(curr_idx, grid_data, raster, grid_w, gw, gh);
        let (right_shape, right_bg) = get_cell_shape_bg_colors(right_idx, grid_data, raster, grid_w, gw, gh);
        let mismatch = seam_shape_mismatch(&c_left, &c_right, left_shape, left_bg, right_shape, right_bg);
        if mismatch > max_mismatch {
            max_mismatch = mismatch;
        }
    }

    // Top seam
    if cy > 0 {
        let seam_y = cy * gh;
        let mut c_top = Vec::with_capacity(gw);
        let mut c_bottom = Vec::with_capacity(gw);
        for dx in 0..gw {
            let x = cx * gw + dx;
            c_top.push(raster[(seam_y - 1) * width + x]);
            c_bottom.push(raster[seam_y * width + x]);
        }
        let top_idx = curr_idx - grid_w;
        let (top_shape, top_bg) = get_cell_shape_bg_colors(top_idx, grid_data, raster, grid_w, gw, gh);
        let (bottom_shape, bottom_bg) = get_cell_shape_bg_colors(curr_idx, grid_data, raster, grid_w, gw, gh);
        let mismatch = seam_shape_mismatch(&c_top, &c_bottom, top_shape, top_bg, bottom_shape, bottom_bg);
        if mismatch > max_mismatch {
            max_mismatch = mismatch;
        }
    }

    // Bottom seam
    if cy + 1 < grid_h {
        let seam_y = (cy + 1) * gh;
        let mut c_top = Vec::with_capacity(gw);
        let mut c_bottom = Vec::with_capacity(gw);
        for dx in 0..gw {
            let x = cx * gw + dx;
            c_top.push(raster[(seam_y - 1) * width + x]);
            c_bottom.push(raster[seam_y * width + x]);
        }
        let bottom_idx = curr_idx + grid_w;
        let (top_shape, top_bg) = get_cell_shape_bg_colors(curr_idx, grid_data, raster, grid_w, gw, gh);
        let (bottom_shape, bottom_bg) = get_cell_shape_bg_colors(bottom_idx, grid_data, raster, grid_w, gw, gh);
        let mismatch = seam_shape_mismatch(&c_top, &c_bottom, top_shape, top_bg, bottom_shape, bottom_bg);
        if mismatch > max_mismatch {
            max_mismatch = mismatch;
        }
    }

    max_mismatch * 100.0
}
"""

lines[1979:2185] = [replacement_code + '\n']

# 1. Delete seam_color_mismatch (lines 1459 to 1487, 0-indexed: 1458:1487)
del lines[1458:1487]

with open('src/draw.rs', 'w') as f:
    f.writelines(lines)

print('Successfully applied modifications!')
