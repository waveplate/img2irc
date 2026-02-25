
use crate::draw::FastGlyph;

pub fn calculate_local_wiggle(
    grid_state: &[(usize, bool)],
    curr_idx: usize,
    candidate_gi: usize,
    candidate_inv: bool,
    gw: usize,
    gh: usize,
    grid_w: usize,
    grid_h: usize,
    glyphs: &[FastGlyph]
) -> f32 {
    let cx = curr_idx % grid_w;
    let cy = curr_idx / grid_w;
    let width = 3 * gw;
    let height = 3 * gh;
    let mut raw = vec![0.0f32; width * height];
    
    // Fill 3x3 Block
    for dy in 0..3 {
        for dx in 0..3 {
            let ny = cy as isize + dy as isize - 1;
            let nx = cx as isize + dx as isize - 1;
            if ny >= 0 && ny < grid_h as isize && nx >= 0 && nx < grid_w as isize {
                let n_idx = (ny as usize) * grid_w + (nx as usize);
                let (gi, inv) = if n_idx == curr_idx { (candidate_gi, candidate_inv) } else { grid_state[n_idx] };
                let g = &glyphs[gi];
                // Offset in raw buffer
                let y_base = dy * gh;
                let x_base = dx * gw;
                for y in 0..gh {
                    for x in 0..gw {
                        let val = if (g.bitmap[y*gw+x] ^ (inv as u8)) == 1 { 255.0 } else { 0.0 };
                        raw[(y_base + y)*width + (x_base + x)] = val;
                    }
                }
            }
        }
    }
    
    let mut smooth = raw.clone();
    let mut temp = vec![0.0f32; width * height];
    
    // 2 passes of smoothing for speed (vs 8) - use 2 passes to approximate 4-pass
    // Actually, calculate_smoothness uses 8 passes. 2 passes might be too little.
    // Let's use 4 passes.
    for _ in 0..4 {
         for y in 1..height-1 { for x in 1..width-1 {
            let i = y*width+x;
            temp[i] = (smooth[i-width-1] + 2.0*smooth[i-width] + smooth[i-width+1] + 2.0*smooth[i-1] + 4.0*smooth[i] + 2.0*smooth[i+1] + smooth[i+width-1] + 2.0*smooth[i+width] + smooth[i+width+1]) / 16.0;
        }}
        smooth.copy_from_slice(&temp);
    }
    
    let mut wiggle = 0.0;
    let mut total_mag = 0.0;
    
    // Correct angle diff logic helper (consts::PI)
    fn angle_diff(a: f32, b: f32) -> f32 {
        let mut d = a - b;
        while d > std::f32::consts::PI { d -= 2.0 * std::f32::consts::PI; }
        while d < -std::f32::consts::PI { d += 2.0 * std::f32::consts::PI; }
        d
    }

    // Calculate gradients only for the CENTER block (gh..2gh, gw..2gw)
    // To calculate wiggle (change in gradient direction), we need gradients of neighbors too?
    // Wiggle = mag * (d_angle/dx^2 + d_angle/dy^2)
    // This requires angles at (y,x) and (y,x+1) etc.
    // So we need gradients for Center block AND immediately adjacent pixels to compute derivatives.
    // Range: gh..2gh, gw..2gw.
    // Loop over center block pixels.
    // For each pixel, compute gradient at i, i+1, i+width.
    
    // We compute gradients on the fly
    let get_grad = |s: &[f32], i: usize, w: usize| -> (f32, f32) {
        let gx = -s[i-w-1] + 1.0*s[i-w+1] -2.0*s[i-1] + 2.0*s[i+1] -1.0*s[i+w-1] + 1.0*s[i+w+1];
        let gy = -s[i-w-1] - 2.0*s[i-w] - 1.0*s[i-w+1] + 1.0*s[i+w-1] + 2.0*s[i+w] + 1.0*s[i+w+1];
        (gx, gy)
    };

    for y in gh..2*gh {
        for x in gw..2*gw {
            let i = y*width+x;
            // Gradient at current pixel
            let (gx, gy) = get_grad(&smooth, i, width);
            let m = (gx*gx + gy*gy).sqrt();
            
            if m > 0.001 {
                total_mag += m;
                let angle = gy.atan2(gx);
                
                // Gradient at Right pixel (x+1)
                let (gx_r, gy_r) = get_grad(&smooth, i+1, width);
                let angle_r = gy_r.atan2(gx_r);
                
                // Gradient at Bottom pixel (y+1)
                let (gx_b, gy_b) = get_grad(&smooth, i+width, width);
                let angle_b = gy_b.atan2(gx_b);

                let d_ang_x = angle_diff(angle_r, angle);
                let d_ang_y = angle_diff(angle_b, angle);
                wiggle += m * (d_ang_x*d_ang_x + d_ang_y*d_ang_y);
            }
        }
    }
    let scale_factor = (gw + gh) as f32 / 2.0;
    if total_mag > 0.0 { (wiggle * 100.0 / total_mag) * scale_factor } else { 0.0 }
}
