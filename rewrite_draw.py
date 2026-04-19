import sys

def main():
    with open('src/draw.rs', 'r') as f:
        content = f.read()

    start_str = "    let block_glyph_costs: Vec<Vec<u64>> = if args.fix_disc {"
    end_str = "                    .unwrap_or_else(|| vec![(cur_gi, cur_inv, best_internal)]);"

    start_idx = content.find(start_str)
    end_idx = content.find(end_str)

    if start_idx == -1 or end_idx == -1:
        print("Could not find start or end string.")
        return
    
    end_idx += len(end_str)

    replacement = """    let candidate_pool_size = args.fix_disc_pool_size.max(1) as usize;
    let shift_radius = args.fix_disc_shift_radius as isize;

    let (block_glyph_costs, block_glyph_pools): (Vec<Vec<u64>>, Vec<Vec<(usize, bool, u64, u32, i8, i8)>>) = if args.fix_disc {
        grid_data
            .par_iter()
            .enumerate()
            .map(|(curr_idx, data)| {
                let x = curr_idx % grid_w;
                let y = curr_idx / grid_w;
                let blk = &image.block[y][x];

                if let Some(flag) = &abort_flag {
                    if flag.load(std::sync::atomic::Ordering::Relaxed) {
                        return (Vec::new(), Vec::new());
                    }
                }
                let fg_p = PrePixel {
                    r: data.fg[0] as u64,
                    g: data.fg[1] as u64,
                    b: data.fg[2] as u64,
                    sq: (data.fg[0] as u64).pow(2)
                        + (data.fg[1] as u64).pow(2)
                        + (data.fg[2] as u64).pow(2),
                };
                let bg_p = PrePixel {
                    r: data.bg[0] as u64,
                    g: data.bg[1] as u64,
                    b: data.bg[2] as u64,
                    sq: (data.bg[0] as u64).pow(2)
                        + (data.bg[1] as u64).pow(2)
                        + (data.bg[2] as u64).pow(2),
                };

                let mut base_cost = 0u64;
                let mut diff = vec![0i64; bp];

                for i in 0..bp {
                    let lx = i % gw;
                    let ly = i / gw;
                    let rgb = unpack_rgb(blk.pixels[ly][lx].orig);
                    let p_r = rgb[0] as u64;
                    let p_g = rgb[1] as u64;
                    let p_b = rgb[2] as u64;
                    let p_sq = p_r * p_r + p_g * p_g + p_b * p_b;

                    let c_fg = p_sq + fg_p.sq - 2 * (p_r * fg_p.r + p_g * fg_p.g + p_b * fg_p.b);
                    let c_bg = p_sq + bg_p.sq - 2 * (p_r * bg_p.r + p_g * bg_p.g + p_b * bg_p.b);
                    let bg_wins = if c_bg <= c_fg { 0i64 } else { 1i64 };
                    base_cost += bg_wins as u64;
                    let fg_wins = if c_fg <= c_bg { 0i64 } else { 1i64 };
                    diff[i] = fg_wins - bg_wins;
                }

                let total_diff: i64 = diff.iter().sum();
                let mut costs = Vec::with_capacity(glyphs.len() * 2);

                for fglyph in glyphs.iter() {
                    let mut dot = 0i64;
                    for &i in &fglyph.ones {
                        dot += unsafe { *diff.get_unchecked(i) };
                    }

                    let c_total_f = (base_cost as i64 + dot).max(0) as u64;
                    let c_total_t = (base_cost as i64 + total_diff - dot).max(0) as u64;

                    costs.push(c_total_f);
                    costs.push(c_total_t);
                }

                let pool = best_candidate_pool_pre(&costs, candidate_pool_size);
                
                let current_reference = &quantized_references[curr_idx];
                let pre_pool = pool.into_iter().map(|(gi, inv, internal)| {
                    let (ref_cost, sx, sy) = best_shifted_reference_match_with_radius(
                        &current_reference.mask,
                        &glyphs[gi],
                        inv,
                        gw,
                        gh,
                        shift_radius,
                    );
                    (gi, inv, internal, ref_cost, sx, sy)
                }).collect();

                (costs, pre_pool)
            })
            .unzip()
    } else {
        (Vec::new(), Vec::new())
    };

    let initial_time = process_start_time.elapsed();
    let mut fix_disc_time = None;

    if args.fix_disc {
        let raster = rendered_raster
            .as_mut()
            .expect("rendered raster should exist when fix_disc is enabled");
        let t_fix = std::time::Instant::now();
        let reference_tolerance_ratio = args.fix_disc_ref_tolerance_ratio.max(0.0);
        let reference_tolerance_pixels = args.fix_disc_ref_tolerance_pixels;
        let mut change_manifest = String::from(
            "pass,cell_x,cell_y,before_ref_cost,after_ref_cost,before_continuity,after_continuity,before_shift_x,before_shift_y,after_shift_x,after_shift_y,before_char,before_inv,after_char,after_inv,before_raw,after_raw,before_marked,after_marked\\n",
        );

        for pass in 0..args.fix_disc_passes as usize {
            if let Some(flag) = &abort_flag {
                if flag.load(std::sync::atomic::Ordering::Relaxed) {
                    return "Cancelled".into();
                }
            }

            let mut changes = 0u32;
            let forward = pass % 2 == 0;
            let indices: Vec<usize> = if forward {
                (0..grid_h * grid_w).collect()
            } else {
                (0..grid_h * grid_w).rev().collect()
            };

            for &curr_idx in &indices {
                let cx = curr_idx % grid_w;
                let cy = curr_idx / grid_w;
                let (cur_gi, cur_inv) = grid_state[curr_idx];
                let data = &grid_data[curr_idx];

                if data.fg == data.bg {
                    continue;
                }
                if has_flat_uniform_neighbor_context(curr_idx, &grid_data, grid_w, grid_h) {
                    continue;
                }

                let current_choice = (cur_gi, cur_inv);
                let current_continuity = continuity_hint_score(
                    curr_idx,
                    current_choice,
                    &grid_data,
                    &grid_state,
                    glyphs,
                    grid_w,
                    grid_h,
                    gw,
                    gh,
                );
                let current_reference = quantized_references
                    .get(curr_idx)
                    .expect("quantized references should exist when fix_disc is enabled");
                let (current_ref_cost, current_dx, current_dy) =
                    best_shifted_reference_match_with_radius(
                        &current_reference.mask,
                        &glyphs[cur_gi],
                        cur_inv,
                        gw,
                        gh,
                        shift_radius,
                    );

                let mut best_ref_cost = current_ref_cost;
                let mut best_internal = block_glyph_costs
                    .get(curr_idx)
                    .and_then(|v| v.get(cur_gi * 2 + cur_inv as usize).copied())
                    .unwrap_or(u64::MAX);
                let mut best_continuity = current_continuity;
                let mut best_choice = current_choice;
                let mut best_shift = (current_dx, current_dy);

                let mut pool = block_glyph_pools.get(curr_idx).cloned().unwrap_or_default();
                if !pool.iter().any(|&(gi, inv, _, _, _, _)| (gi, inv) == current_choice) {
                    pool.push((cur_gi, cur_inv, best_internal, current_ref_cost as u32, current_dx as i8, current_dy as i8));
                }"""

    new_content = content[:start_idx] + replacement + content[end_idx:]
    with open('src/draw.rs', 'w') as f:
        f.write(new_content)

if __name__ == "__main__":
    main()
