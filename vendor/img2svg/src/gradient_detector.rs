//! Gradient detection for smoother SVG gradient rendering.
//!
//! Detects linear and radial gradients within image regions so they can be
//! rendered as native SVG `<linearGradient>` / `<radialGradient>` instead of
//! discrete color bands.

use crate::image_processor::ImageData;
use rgb::RGBA8;

/// Detected gradient type and parameters.
#[derive(Debug, Clone, PartialEq)]
pub enum DetectedGradient {
    Linear(LinearGradient),
    Radial(RadialGradient),
}

/// Linear gradient parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct LinearGradient {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
    pub start_color: (u8, u8, u8, u8),
    pub end_color: (u8, u8, u8, u8),
    /// Mean squared error of the fit (lower = better)
    pub error: f64,
}

/// Radial gradient parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct RadialGradient {
    pub cx: f64,
    pub cy: f64,
    pub r: f64,
    pub start_color: (u8, u8, u8, u8),
    pub end_color: (u8, u8, u8, u8),
    pub error: f64,
}

/// Detect a gradient in the largest connected component of `mask`. When the
/// mask spans multiple disjoint regions (e.g. the same quantized color appears
/// in both halves of a split gradient image), fitting over the union
/// conflates the gradients and produces a meaningless fit. Restricting to the
/// largest connected component gives a clean single-region fit.
pub fn detect_gradient(
    image_data: &ImageData,
    mask: &[bool],
    error_threshold: f64,
) -> Option<DetectedGradient> {
    let w = image_data.width as usize;
    let h = image_data.height as usize;
    let component = largest_connected_component(mask, w, h);
    let m = if component.iter().any(|&b| b) {
        &component[..]
    } else {
        mask
    };
    let linear = detect_linear_gradient(image_data, m);
    let radial = detect_radial_gradient(image_data, m);

    match (linear, radial) {
        (Some(l), Some(r)) => {
            if l.error < r.error && l.error < error_threshold {
                Some(DetectedGradient::Linear(l))
            } else if r.error < error_threshold {
                Some(DetectedGradient::Radial(r))
            } else {
                None
            }
        }
        (Some(l), None) => {
            if l.error < error_threshold {
                Some(DetectedGradient::Linear(l))
            } else {
                None
            }
        }
        (None, Some(r)) => {
            if r.error < error_threshold {
                Some(DetectedGradient::Radial(r))
            } else {
                None
            }
        }
        (None, None) => None,
    }
}

/// 4-connected flood fill labeling. Returns the mask of the largest connected
/// component (true = part of the largest component). Empty when the input mask
/// is empty.
fn largest_connected_component(mask: &[bool], w: usize, h: usize) -> Vec<bool> {
    if mask.is_empty() || w == 0 || h == 0 {
        return Vec::new();
    }
    let mut labels = vec![0u32; w * h];
    let mut best_label = 0u32;
    let mut best_size = 0usize;
    let mut next_label = 1u32;
    let mut stack: Vec<usize> = Vec::new();

    for start in 0..w * h {
        if !mask[start] || labels[start] != 0 {
            continue;
        }
        let label = next_label;
        next_label += 1;
        let mut size = 0usize;
        stack.clear();
        stack.push(start);
        labels[start] = label;
        while let Some(i) = stack.pop() {
            size += 1;
            let x = i % w;
            let y = i / w;
            if x > 0 {
                let n = i - 1;
                if mask[n] && labels[n] == 0 {
                    labels[n] = label;
                    stack.push(n);
                }
            }
            if x + 1 < w {
                let n = i + 1;
                if mask[n] && labels[n] == 0 {
                    labels[n] = label;
                    stack.push(n);
                }
            }
            if y > 0 {
                let n = i - w;
                if mask[n] && labels[n] == 0 {
                    labels[n] = label;
                    stack.push(n);
                }
            }
            if y + 1 < h {
                let n = i + w;
                if mask[n] && labels[n] == 0 {
                    labels[n] = label;
                    stack.push(n);
                }
            }
        }
        if size > best_size {
            best_size = size;
            best_label = label;
        }
    }

    if best_label == 0 {
        return vec![false; w * h];
    }
    mask.iter()
        .zip(labels.iter())
        .map(|(&m, &l)| m && l == best_label)
        .collect()
}

/// Label all 4-connected components in `mask`. Returns a vector of component
/// masks, one per connected component, in arbitrary order. Each mask has the
/// same length as `mask` (w*h). Empty input → empty output.
pub fn connected_components(mask: &[bool], w: usize, h: usize) -> Vec<Vec<bool>> {
    if mask.is_empty() || w == 0 || h == 0 {
        return Vec::new();
    }
    let mut labels = vec![0u32; w * h];
    let mut next_label = 1u32;
    let mut stack: Vec<usize> = Vec::new();
    let mut components: Vec<Vec<bool>> = Vec::new();

    for start in 0..w * h {
        if !mask[start] || labels[start] != 0 {
            continue;
        }
        let label = next_label;
        next_label += 1;
        let idx = components.len();
        components.push(vec![false; w * h]);
        stack.clear();
        stack.push(start);
        labels[start] = label;
        components[idx][start] = true;
        while let Some(i) = stack.pop() {
            let x = i % w;
            let y = i / w;
            if x > 0 {
                let n = i - 1;
                if mask[n] && labels[n] == 0 {
                    labels[n] = label;
                    components[idx][n] = true;
                    stack.push(n);
                }
            }
            if x + 1 < w {
                let n = i + 1;
                if mask[n] && labels[n] == 0 {
                    labels[n] = label;
                    components[idx][n] = true;
                    stack.push(n);
                }
            }
            if y > 0 {
                let n = i - w;
                if mask[n] && labels[n] == 0 {
                    labels[n] = label;
                    components[idx][n] = true;
                    stack.push(n);
                }
            }
            if y + 1 < h {
                let n = i + w;
                if mask[n] && labels[n] == 0 {
                    labels[n] = label;
                    components[idx][n] = true;
                    stack.push(n);
                }
            }
        }
    }
    components
}

/// Detect a linear gradient by finding the principal direction of color
/// variation and fitting start/end colors along that axis.
fn color_variance(colors: &[RGBA8]) -> f64 {
    if colors.is_empty() {
        return 0.0;
    }
    let mean_r = colors.iter().map(|c| c.r as f64).sum::<f64>() / colors.len() as f64;
    let mean_g = colors.iter().map(|c| c.g as f64).sum::<f64>() / colors.len() as f64;
    let mean_b = colors.iter().map(|c| c.b as f64).sum::<f64>() / colors.len() as f64;
    let var_r = colors.iter().map(|c| (c.r as f64 - mean_r).powi(2)).sum::<f64>() / colors.len() as f64;
    let var_g = colors.iter().map(|c| (c.g as f64 - mean_g).powi(2)).sum::<f64>() / colors.len() as f64;
    let var_b = colors.iter().map(|c| (c.b as f64 - mean_b).powi(2)).sum::<f64>() / colors.len() as f64;
    (var_r + var_g + var_b) / 3.0
}

pub fn detect_linear_gradient(image_data: &ImageData, mask: &[bool]) -> Option<LinearGradient> {
    let w = image_data.width as usize;
    let h = image_data.height as usize;

    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut colors = Vec::new();

    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if i < mask.len() && mask[i] {
                xs.push(x as f64);
                ys.push(y as f64);
                colors.push(image_data.pixels[i]);
            }
        }
    }

    if xs.len() < 4 {
        return None;
    }

    // Reject near-uniform color regions
    if color_variance(&colors) < 25.0 {
        return None;
    }

    let cx = mean(&xs);
    let cy = mean(&ys);

    // --- Candidate directions ---
    // Try axis-aligned + 45° diagonals. PCA is unreliable for thin strips
    // (spatial extent dominates even with color weighting), so we rely on
    // direct fit-error comparison across fixed directions. This correctly
    // picks horizontal for a horizontal gradient in a tall thin strip, etc.
    let s = std::f64::consts::FRAC_1_SQRT_2;
    let candidates: [(f64, f64); 4] = [
        (1.0, 0.0),                       // horizontal
        (0.0, 1.0),                       // vertical
        (s, s),                           // 45° diagonal
        (s, -s),                          // 135° diagonal
    ];

    let mut best: Option<LinearGradient> = None;
    for &(dx, dy) in &candidates {
        if let Some(g) = fit_linear_in_direction(&xs, &ys, &colors, cx, cy, dx, dy) {
            if best.as_ref().map_or(true, |b| g.error < b.error) {
                best = Some(g);
            }
        }
    }

    best
}

/// Fit a linear gradient along a fixed direction `(dx, dy)` and return the
/// gradient if the fit is non-degenerate.
fn fit_linear_in_direction(
    xs: &[f64],
    ys: &[f64],
    colors: &[RGBA8],
    cx: f64,
    cy: f64,
    dx: f64,
    dy: f64,
) -> Option<LinearGradient> {
    // Project all points onto the direction.
    let mut projections: Vec<f64> = xs
        .iter()
        .zip(ys.iter())
        .map(|(x, y)| (x - cx) * dx + (y - cy) * dy)
        .collect();

    let t_min = projections.iter().copied().fold(f64::INFINITY, f64::min);
    let t_max = projections.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let t_range = t_max - t_min;

    if t_range < 1.0 {
        return None;
    }

    for p in &mut projections {
        *p = (*p - t_min) / t_range;
    }

    let (start_r, end_r, err_r) = fit_line(&projections, colors, |c| c.r as f64);
    let (start_g, end_g, err_g) = fit_line(&projections, colors, |c| c.g as f64);
    let (start_b, end_b, err_b) = fit_line(&projections, colors, |c| c.b as f64);
    let (start_a, end_a, err_a) = fit_line(&projections, colors, |c| c.a as f64);

    let error = (err_r + err_g + err_b + err_a) / 4.0;

    let sc = (
        start_r.round().clamp(0.0, 255.0) as u8,
        start_g.round().clamp(0.0, 255.0) as u8,
        start_b.round().clamp(0.0, 255.0) as u8,
        start_a.round().clamp(0.0, 255.0) as u8,
    );
    let ec = (
        end_r.round().clamp(0.0, 255.0) as u8,
        end_g.round().clamp(0.0, 255.0) as u8,
        end_b.round().clamp(0.0, 255.0) as u8,
        end_a.round().clamp(0.0, 255.0) as u8,
    );

    // Reject degenerate gradients: start ≈ end color (no visible gradient).
    let color_dist = ((sc.0 as f64 - ec.0 as f64).powi(2)
        + (sc.1 as f64 - ec.1 as f64).powi(2)
        + (sc.2 as f64 - ec.2 as f64).powi(2)
        + (sc.3 as f64 - ec.3 as f64).powi(2))
    .sqrt();
    if color_dist < 10.0 {
        return None;
    }

    Some(LinearGradient {
        x1: cx + t_min * dx,
        y1: cy + t_min * dy,
        x2: cx + t_max * dx,
        y2: cy + t_max * dy,
        start_color: sc,
        end_color: ec,
        error,
    })
}

/// Detect a radial gradient by fitting color as a function of distance from
/// the region's geometric center.
pub fn detect_radial_gradient(image_data: &ImageData, mask: &[bool]) -> Option<RadialGradient> {
    let w = image_data.width as usize;
    let h = image_data.height as usize;

    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut colors = Vec::new();

    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if i < mask.len() && mask[i] {
                xs.push(x as f64);
                ys.push(y as f64);
                colors.push(image_data.pixels[i]);
            }
        }
    }

    if xs.len() < 4 {
        return None;
    }

    // Reject near-uniform color regions
    if color_variance(&colors) < 25.0 {
        return None;
    }

    let cx = mean(&xs);
    let cy = mean(&ys);

    // Compute distances from center
    let mut distances: Vec<f64> = xs
        .iter()
        .zip(ys.iter())
        .map(|(x, y)| ((x - cx).powi(2) + (y - cy).powi(2)).sqrt())
        .collect();

    let d_max = distances.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if d_max < 1.0 {
        return None;
    }

    // Normalize to [0, 1]
    for d in &mut distances {
        *d /= d_max;
    }

    // Fit colors per channel
    let (start_r, end_r, err_r) = fit_line(&distances, &colors, |c| c.r as f64);
    let (start_g, end_g, err_g) = fit_line(&distances, &colors, |c| c.g as f64);
    let (start_b, end_b, err_b) = fit_line(&distances, &colors, |c| c.b as f64);
    let (start_a, end_a, err_a) = fit_line(&distances, &colors, |c| c.a as f64);

    let error = (err_r + err_g + err_b + err_a) / 4.0;

    let sc = (
        start_r.round().clamp(0.0, 255.0) as u8,
        start_g.round().clamp(0.0, 255.0) as u8,
        start_b.round().clamp(0.0, 255.0) as u8,
        start_a.round().clamp(0.0, 255.0) as u8,
    );
    let ec = (
        end_r.round().clamp(0.0, 255.0) as u8,
        end_g.round().clamp(0.0, 255.0) as u8,
        end_b.round().clamp(0.0, 255.0) as u8,
        end_a.round().clamp(0.0, 255.0) as u8,
    );

    // Reject degenerate radial gradients: start ≈ end color.
    let color_dist = ((sc.0 as f64 - ec.0 as f64).powi(2)
        + (sc.1 as f64 - ec.1 as f64).powi(2)
        + (sc.2 as f64 - ec.2 as f64).powi(2)
        + (sc.3 as f64 - ec.3 as f64).powi(2))
    .sqrt();
    if color_dist < 10.0 {
        return None;
    }

    Some(RadialGradient {
        cx,
        cy,
        r: d_max,
        start_color: sc,
        end_color: ec,
        error,
    })
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

/// Fit `y = start + t * (end - start)` (i.e. `y = a + b*t`) and return
/// `(start, end, mean_squared_error)`.
fn fit_line<F>(t: &[f64], y: &[RGBA8], extractor: F) -> (f64, f64, f64)
where
    F: Fn(&RGBA8) -> f64,
{
    let n = t.len() as f64;
    let mean_t = mean(t);
    let mean_y: f64 = y.iter().map(&extractor).sum::<f64>() / n;

    let mut num = 0.0;
    let mut den = 0.0;
    for i in 0..t.len() {
        let yi = extractor(&y[i]);
        let dt = t[i] - mean_t;
        num += dt * (yi - mean_y);
        den += dt * dt;
    }

    let b = if den.abs() > 1e-9 { num / den } else { 0.0 };
    let a = mean_y - b * mean_t;

    // start at t=0, end at t=1
    let start = a;
    let end_val = a + b;

    let mut sse = 0.0;
    for i in 0..t.len() {
        let predicted = a + b * t[i];
        let diff = extractor(&y[i]) - predicted;
        sse += diff * diff;
    }
    let mse = sse / t.len().max(1) as f64;

    (start, end_val, mse)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_horizontal_gradient_image(width: u32, height: u32) -> ImageData {
        let mut pixels = Vec::new();
        for _y in 0..height {
            for x in 0..width {
                let t = x as f64 / (width as f64 - 1.0);
                let r = (t * 255.0) as u8;
                let g = ((1.0 - t) * 255.0) as u8;
                let b = 128u8;
                pixels.push(RGBA8::new(r, g, b, 255));
            }
        }
        ImageData { width, height, pixels }
    }

    fn create_radial_gradient_image(width: u32, height: u32) -> ImageData {
        let cx = width as f64 / 2.0;
        let cy = height as f64 / 2.0;
        let max_dist = ((cx * cx + cy * cy).sqrt()).max(1.0);
        let mut pixels = Vec::new();
        for y in 0..height {
            for x in 0..width {
                let dist =
                    (((x as f64 - cx).powi(2) + (y as f64 - cy).powi(2)).sqrt() / max_dist).min(1.0);
                let r = (dist * 255.0) as u8;
                let g = ((1.0 - dist) * 255.0) as u8;
                let b = 128u8;
                pixels.push(RGBA8::new(r, g, b, 255));
            }
        }
        ImageData { width, height, pixels }
    }

    fn full_mask(width: u32, height: u32) -> Vec<bool> {
        vec![true; (width * height) as usize]
    }

    #[test]
    fn test_detect_linear_gradient_horizontal() {
        let img = create_horizontal_gradient_image(20, 10);
        let mask = full_mask(20, 10);
        let grad = detect_linear_gradient(&img, &mask);
        assert!(grad.is_some(), "Should detect a linear gradient");
        let g = grad.unwrap();
        assert!(g.error < 50.0, "Error should be low for synthetic gradient: {}", g.error);
        // Principal direction eigenvector sign is arbitrary, so start/end may be swapped.
        // One end should be red-ish (high R) and the other green-ish (high G).
        let (end_a, end_b) = (g.start_color, g.end_color);
        let red_at_one_end = (end_a.0 > 200 && end_b.1 > 200) || (end_b.0 > 200 && end_a.1 > 200);
        assert!(red_at_one_end, "One end should be red-ish and the other green-ish");
    }

    #[test]
    fn test_detect_radial_gradient() {
        let img = create_radial_gradient_image(20, 20);
        let mask = full_mask(20, 20);
        let grad = detect_radial_gradient(&img, &mask);
        assert!(grad.is_some(), "Should detect a radial gradient");
        let g = grad.unwrap();
        assert!(g.error < 100.0, "Error should be reasonable for synthetic gradient: {}", g.error);
        // Radial: center vs edge colors may be swapped depending on fit direction
        let center_dark = g.start_color.0 < 50 || g.end_color.0 < 50;
        assert!(center_dark, "Center should be dark at one end");
    }

    #[test]
    fn test_detect_gradient_prefers_better_fit() {
        let img = create_horizontal_gradient_image(20, 10);
        let mask = full_mask(20, 10);
        let grad = detect_gradient(&img, &mask, 50.0);
        assert!(grad.is_some());
        // Horizontal gradient → linear should fit better than radial
        assert!(
            matches!(grad, Some(DetectedGradient::Linear(_))),
            "Should prefer linear for horizontal gradient"
        );
    }

    #[test]
    fn test_detect_gradient_rejects_uniform_color() {
        let pixels = vec![RGBA8::new(128, 128, 128, 255); 100];
        let img = ImageData {
            width: 10,
            height: 10,
            pixels,
        };
        let mask = full_mask(10, 10);
        let grad = detect_gradient(&img, &mask, 50.0);
        assert!(grad.is_none(), "Uniform color should not be detected as gradient");
    }

    #[test]
    fn test_detect_gradient_small_region() {
        let pixels = vec![RGBA8::new(0, 0, 0, 255); 3];
        let img = ImageData {
            width: 3,
            height: 1,
            pixels,
        };
        let mask = vec![true, true, true];
        let grad = detect_linear_gradient(&img, &mask);
        assert!(grad.is_none(), "Region too small for reliable detection");
    }

    #[test]
    fn test_detect_linear_gradient_vertical() {
        // Vertical gradient: color varies along Y, not X.
        let mut pixels = Vec::new();
        for y in 0..10u32 {
            for _x in 0..20u32 {
                let t = y as f64 / 9.0;
                let r = (t * 255.0) as u8;
                let g = ((1.0 - t) * 255.0) as u8;
                pixels.push(RGBA8::new(r, g, 128, 255));
            }
        }
        let img = ImageData { width: 20, height: 10, pixels };
        let mask = full_mask(20, 10);
        let grad = detect_linear_gradient(&img, &mask);
        assert!(grad.is_some(), "Should detect vertical gradient");
        let g = grad.unwrap();
        // Direction should be vertical: x1 ≈ x2, y1 ≠ y2
        assert!((g.x1 - g.x2).abs() < 1.0, "Vertical gradient should have x1≈x2, got x1={} x2={}", g.x1, g.x2);
        assert!((g.y1 - g.y2).abs() > 5.0, "Vertical gradient should have y1≠y2, got y1={} y2={}", g.y1, g.y2);
    }

    #[test]
    fn test_detect_linear_gradient_thin_vertical_strip() {
        // Horizontal gradient in a tall thin vertical strip — the case that
        // broke PCA-based detection. The strip is 4px wide, 40px tall, with
        // color varying left→right.
        let mut pixels = Vec::new();
        for _y in 0..40u32 {
            for x in 0..4u32 {
                let t = x as f64 / 3.0;
                let r = (t * 255.0) as u8;
                let g = ((1.0 - t) * 255.0) as u8;
                pixels.push(RGBA8::new(r, g, 128, 255));
            }
        }
        let img = ImageData { width: 4, height: 40, pixels };
        let mask = full_mask(4, 40);
        let grad = detect_linear_gradient(&img, &mask);
        assert!(grad.is_some(), "Should detect gradient in thin strip");
        let g = grad.unwrap();
        // Direction should be horizontal (x1 ≠ x2), NOT vertical.
        assert!((g.x1 - g.x2).abs() > 1.0, "Thin strip gradient should be horizontal (x1≠x2), got x1={} x2={}", g.x1, g.x2);
    }

    #[test]
    fn test_detect_gradient_rejects_degenerate() {
        // Near-uniform color with tiny variation — should not produce a
        // degenerate gradient (start ≈ end color).
        let pixels: Vec<RGBA8> = (0..200).map(|i| {
            let v = 128 + (i % 3) as u8;
            RGBA8::new(v, v, v, 255)
        }).collect();
        let img = ImageData { width: 20, height: 10, pixels };
        let mask = full_mask(20, 10);
        let grad = detect_gradient(&img, &mask, 50.0);
        assert!(grad.is_none(), "Near-uniform region should not produce a gradient");
    }

    #[test]
    fn test_connected_components_separates_disjoint() {
        // Two disjoint blocks in a 10x10 mask.
        let mut mask = vec![false; 100];
        // Top-left 3x3 block
        for y in 0..3 {
            for x in 0..3 {
                mask[y * 10 + x] = true;
            }
        }
        // Bottom-right 3x3 block
        for y in 7..10 {
            for x in 7..10 {
                mask[y * 10 + x] = true;
            }
        }
        let comps = connected_components(&mask, 10, 10);
        assert_eq!(comps.len(), 2, "Should find 2 connected components");
        // Each component should have 9 pixels
        for comp in &comps {
            assert_eq!(comp.iter().filter(|&&b| b).count(), 9);
        }
    }

    #[test]
    fn test_connected_components_single_block() {
        let mut mask = vec![false; 100];
        for y in 0..10 {
            for x in 0..10 {
                mask[y * 10 + x] = true;
            }
        }
        let comps = connected_components(&mask, 10, 10);
        assert_eq!(comps.len(), 1);
        assert_eq!(comps[0].iter().filter(|&&b| b).count(), 100);
    }

    #[test]
    fn test_detect_gradient_per_component() {
        // Two disjoint regions with different gradient directions: top half
        // horizontal, bottom half vertical, separated by a 2px gap row.
        let mut pixels = Vec::new();
        let mut mask = vec![false; 20 * 20];
        for y in 0..20u32 {
            for x in 0..20u32 {
                if y < 9 {
                    // Horizontal gradient in top half
                    let t = x as f64 / 19.0;
                    let r = (t * 255.0) as u8;
                    let g = ((1.0 - t) * 255.0) as u8;
                    pixels.push(RGBA8::new(r, g, 128, 255));
                    mask[(y * 20 + x) as usize] = true;
                } else if y > 11 {
                    // Vertical gradient in bottom half
                    let t = (y - 12) as f64 / 7.0;
                    let r = (t * 255.0) as u8;
                    let b = ((1.0 - t) * 255.0) as u8;
                    pixels.push(RGBA8::new(r, 128, b, 255));
                    mask[(y * 20 + x) as usize] = true;
                } else {
                    // Gap row (not in mask)
                    pixels.push(RGBA8::new(0, 0, 0, 255));
                }
            }
        }
        let img = ImageData { width: 20, height: 20, pixels };
        let grad = detect_gradient(&img, &mask, 100.0);
        assert!(grad.is_some(), "Should detect a gradient in the largest component");
        // The largest component is one half; the gradient should be axis-aligned.
        if let Some(DetectedGradient::Linear(g)) = &grad {
            let dx = (g.x2 - g.x1).abs();
            let dy = (g.y2 - g.y1).abs();
            let is_axis_aligned = (dx > 5.0 && dy < 1.0) || (dy > 5.0 && dx < 1.0);
            assert!(is_axis_aligned, "Gradient should be axis-aligned, got dx={} dy={}", dx, dy);
        }
    }
}
