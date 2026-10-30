//! Visvalingam-Whyatt path simplification with corner preservation.
//!
//! Better shape preservation than RDP alone — iteratively removes the point
//! contributing the least area while protecting detected corners.
//! Ported from vec project's PathSimplifier.

use crate::vectorizer::Point;
use std::collections::HashSet;

/// Detect corners using angle deviation with Harris-like corner measure.
pub fn detect_corners(points: &[Point], corner_threshold_deg: f64) -> Vec<usize> {
    if points.len() < 3 {
        return Vec::new();
    }

    let threshold_rad = corner_threshold_deg.to_radians();
    let mut corner_scores = vec![0.0f64; points.len()];

    for i in 1..(points.len() - 1) {
        let prev = &points[i - 1];
        let curr = &points[i];
        let next = &points[i + 1];

        let angle = calculate_angle(prev, curr, next);
        let deviation = (std::f64::consts::PI - angle).abs();

        // Harris-like corner measure at multiple scales
        let mut harris_score = 0.0f64;
        for &window_size in &[3usize, 5, 7] {
            if let Some(score) = harris_corner_measure(points, i, window_size) {
                harris_score = harris_score.max(score);
            }
        }

        corner_scores[i] = deviation * (1.0 + harris_score);
    }

    // Non-maximum suppression
    let mut corners = Vec::new();
    for i in 1..(points.len() - 1) {
        let score = corner_scores[i];
        if score > threshold_rad {
            let is_local_max = (i == 1 || score >= corner_scores[i - 1])
                && (i == points.len() - 2 || score >= corner_scores[i + 1]);
            if is_local_max {
                corners.push(i);
            }
        }
    }

    corners
}

fn calculate_angle(p1: &Point, p2: &Point, p3: &Point) -> f64 {
    let v1x = p1.x - p2.x;
    let v1y = p1.y - p2.y;
    let v2x = p3.x - p2.x;
    let v2y = p3.y - p2.y;

    let dot = v1x * v2x + v1y * v2y;
    let len1 = (v1x * v1x + v1y * v1y).sqrt();
    let len2 = (v2x * v2x + v2y * v2y).sqrt();

    if len1 == 0.0 || len2 == 0.0 {
        return std::f64::consts::PI;
    }

    let cos_angle = (dot / (len1 * len2)).clamp(-1.0, 1.0);
    cos_angle.acos()
}

fn harris_corner_measure(points: &[Point], idx: usize, window_size: usize) -> Option<f64> {
    let half_window = window_size / 2;
    let start_idx = idx.saturating_sub(half_window);
    let end_idx = (idx + half_window + 1).min(points.len());

    if end_idx - start_idx < 3 {
        return None;
    }

    let mut ixx = 0.0f64;
    let mut iyy = 0.0f64;
    let mut ixy = 0.0f64;

    for i in start_idx..end_idx - 1 {
        let dx = points[i + 1].x - points[i].x;
        let dy = points[i + 1].y - points[i].y;
        ixx += dx * dx;
        iyy += dy * dy;
        ixy += dx * dy;
    }

    let det = ixx * iyy - ixy * ixy;
    let trace = ixx + iyy;
    let k = 0.04;
    let response = det - k * trace * trace;

    if response > 0.0 {
        Some(response / (ixx + iyy + 1e-10))
    } else {
        Some(0.0)
    }
}

/// Visvalingam-Whyatt simplification: iteratively removes the point that
/// contributes the least area (triangle formed with its neighbors).
/// Preserves specified corner indices by giving them infinite area.
pub fn visvalingam_whyatt(points: &[Point], min_area: f64, corner_indices: &[usize]) -> Vec<Point> {
    let n = points.len();
    if n <= 3 {
        return points.to_vec();
    }

    let corner_set: HashSet<usize> = corner_indices.iter().copied().collect();

    // Compute initial triangle areas
    let mut areas: Vec<f64> = vec![f64::MAX; n];
    for i in 1..n - 1 {
        if corner_set.contains(&i) {
            areas[i] = f64::MAX;
        } else {
            areas[i] = triangle_area(&points[i - 1], &points[i], &points[i + 1]);
        }
    }

    let mut alive: Vec<bool> = vec![true; n];
    let mut alive_count = n;

    loop {
        let mut min_idx = None;
        let mut min_val = f64::MAX;
        for i in 1..n - 1 {
            if alive[i] && areas[i] < min_val {
                min_val = areas[i];
                min_idx = Some(i);
            }
        }

        match min_idx {
            Some(idx) if min_val < min_area && alive_count > 3 => {
                alive[idx] = false;
                alive_count -= 1;

                let prev = find_prev_alive(&alive, idx);
                let next = find_next_alive(&alive, idx);

                if let (Some(p), Some(nx)) = (prev, next) {
                    if let Some(pp) = find_prev_alive(&alive, p)
                        && !corner_set.contains(&p)
                    {
                        areas[p] =
                            triangle_area(&points[pp], &points[p], &points[nx]).max(min_val);
                    }
                    if let Some(nn) = find_next_alive(&alive, nx)
                        && !corner_set.contains(&nx)
                    {
                        areas[nx] =
                            triangle_area(&points[p], &points[nx], &points[nn]).max(min_val);
                    }
                }
            }
            _ => break,
        }
    }

    points
        .iter()
        .enumerate()
        .filter(|(i, _)| alive[*i])
        .map(|(_, p)| p.clone())
        .collect()
}

/// Gaussian-weighted smoothing that preserves corners.
pub fn smooth_with_corners(
    points: &[Point],
    window_size: usize,
    corner_threshold_deg: f64,
) -> Vec<Point> {
    if points.len() <= window_size {
        return points.to_vec();
    }

    let n = points.len();
    let half = window_size / 2;

    // Precompute Gaussian weights
    let sigma = half as f64 * 0.6;
    let mut weights = Vec::with_capacity(window_size);
    for i in 0..=half * 2 {
        let d = i as f64 - half as f64;
        weights.push((-d * d / (2.0 * sigma * sigma)).exp());
    }

    // Detect corners to preserve them
    let corners = detect_corners(points, corner_threshold_deg);
    let corner_set: HashSet<usize> = corners.into_iter().collect();

    let mut smoothed = Vec::with_capacity(n);

    for i in 0..n {
        if i == 0 || i == n - 1 || corner_set.contains(&i) {
            smoothed.push(points[i].clone());
            continue;
        }

        let start = i.saturating_sub(half);
        let end = (i + half + 1).min(n);

        let mut sum_x = 0.0;
        let mut sum_y = 0.0;
        let mut sum_w = 0.0;

        for j in start..end {
            let wi = (j as isize - i as isize + half as isize) as usize;
            let w = if wi < weights.len() { weights[wi] } else { 0.0 };
            sum_x += points[j].x * w;
            sum_y += points[j].y * w;
            sum_w += w;
        }

        if sum_w > 0.0 {
            smoothed.push(Point {
                x: sum_x / sum_w,
                y: sum_y / sum_w,
            });
        } else {
            smoothed.push(points[i].clone());
        }
    }

    smoothed
}

#[inline]
fn triangle_area(p1: &Point, p2: &Point, p3: &Point) -> f64 {
    ((p1.x * (p2.y - p3.y) + p2.x * (p3.y - p1.y) + p3.x * (p1.y - p2.y)) / 2.0).abs()
}

fn find_prev_alive(alive: &[bool], idx: usize) -> Option<usize> {
    (0..idx).rev().find(|&i| alive[i])
}

fn find_next_alive(alive: &[bool], idx: usize) -> Option<usize> {
    (idx + 1..alive.len()).find(|&i| alive[i])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_corners_right_angle() {
        let points = vec![
            Point { x: 0.0, y: 0.0 },
            Point { x: 10.0, y: 0.0 },
            Point { x: 10.0, y: 10.0 },
            Point { x: 0.0, y: 10.0 },
        ];
        let corners = detect_corners(&points, 60.0);
        assert!(!corners.is_empty());
    }

    #[test]
    fn test_detect_corners_straight_line() {
        let points: Vec<Point> = (0..10)
            .map(|i| Point {
                x: i as f64,
                y: 0.0,
            })
            .collect();
        let corners = detect_corners(&points, 60.0);
        assert!(corners.is_empty());
    }

    #[test]
    fn test_visvalingam_preserves_corners() {
        let points = vec![
            Point { x: 0.0, y: 0.0 },
            Point { x: 5.0, y: 0.0 },
            Point { x: 10.0, y: 0.0 }, // corner
            Point { x: 10.0, y: 5.0 },
            Point { x: 10.0, y: 10.0 }, // corner
            Point { x: 5.0, y: 10.0 },
            Point { x: 0.0, y: 10.0 },
        ];
        let corners = detect_corners(&points, 60.0);
        let simplified = visvalingam_whyatt(&points, 25.0, &corners);
        assert!(simplified.len() >= 3);
        assert_eq!(simplified[0].x, 0.0);
        assert_eq!(simplified.last().unwrap().y, 10.0);
    }

    #[test]
    fn test_visvalingam_collinear_reduction() {
        let points: Vec<Point> = (0..20)
            .map(|i| Point {
                x: i as f64,
                y: 0.0,
            })
            .collect();
        let simplified = visvalingam_whyatt(&points, 1.0, &[]);
        assert!(simplified.len() < points.len());
    }

    #[test]
    fn test_visvalingam_small_input() {
        let points = vec![
            Point { x: 0.0, y: 0.0 },
            Point { x: 1.0, y: 1.0 },
            Point { x: 2.0, y: 0.0 },
        ];
        let simplified = visvalingam_whyatt(&points, 100.0, &[]);
        assert_eq!(simplified.len(), 3); // Can't reduce below 3
    }

    #[test]
    fn test_smooth_with_corners_preserves_endpoints() {
        let points = vec![
            Point { x: 0.0, y: 0.0 },
            Point { x: 1.0, y: 5.0 },
            Point { x: 2.0, y: 0.0 },
            Point { x: 3.0, y: 5.0 },
            Point { x: 4.0, y: 0.0 },
        ];
        let smoothed = smooth_with_corners(&points, 3, 60.0);
        assert_eq!(smoothed.len(), points.len());
        assert_eq!(smoothed[0].x, 0.0);
        assert_eq!(smoothed[0].y, 0.0);
        assert_eq!(smoothed[4].x, 4.0);
        assert_eq!(smoothed[4].y, 0.0);
    }

    #[test]
    fn test_triangle_area_right_triangle() {
        let area = triangle_area(
            &Point { x: 0.0, y: 0.0 },
            &Point { x: 4.0, y: 0.0 },
            &Point { x: 0.0, y: 3.0 },
        );
        assert!((area - 6.0).abs() < 0.01);
    }
}
