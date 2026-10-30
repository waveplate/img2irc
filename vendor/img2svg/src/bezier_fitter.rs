//! Least-squares cubic Bézier fitting with Newton-Raphson reparameterization.
//!
//! Produces smooth curves from boundary points, much better than line segments.
//! Ported from vec project's BezierFitter.

use crate::vectorizer::Point;
use std::io::{Result as IoResult, Write};

/// A cubic Bézier curve segment.
#[derive(Debug, Clone)]
pub struct BezierCurve {
    pub start: Point,
    pub control1: Point,
    pub control2: Point,
    pub end: Point,
}

/// Fit cubic Bézier curves to a sequence of points.
pub struct BezierFitter {
    tolerance: f64,
    max_iterations: usize,
}

impl BezierFitter {
    pub fn new(tolerance: f64) -> Self {
        Self {
            tolerance,
            max_iterations: 12,
        }
    }

    /// Fit a path (sequence of points) into a series of cubic Bézier curves.
    /// If `closed`, a closing segment is added if endpoints don't match.
    pub fn fit_path(&self, points: &[Point], closed: bool) -> Vec<BezierCurve> {
        if points.len() < 2 {
            return Vec::new();
        }
        if points.len() == 2 {
            return vec![self.linear_to_cubic(&points[0], &points[1])];
        }

        // Detect sharp corners (angle < 120°) and split the path there.
        // This prevents the fitter from curving through what should be sharp edges.
        let corner_indices = self.detect_sharp_corners(points);

        let mut curves = Vec::new();

        if corner_indices.is_empty() {
            // No sharp corners — fit as one piece
            self.fit_segment(points, &mut curves);
        } else {
            // Split at corners and fit each segment independently
            let mut splits: Vec<usize> = Vec::new();
            splits.push(0);
            for &ci in &corner_indices {
                if ci > 0 && ci < points.len() - 1 {
                    splits.push(ci);
                }
            }
            splits.push(points.len() - 1);
            splits.dedup();

            for i in 0..splits.len() - 1 {
                let start = splits[i];
                let end = splits[i + 1];
                if end <= start {
                    continue;
                }
                let segment = &points[start..=end];
                if segment.len() >= 2 {
                    self.fit_segment(segment, &mut curves);
                }
            }
        }

        // Enforce G1 continuity between adjacent curves at smooth junctions.
        // Sharp-corner junctions are preserved internally by the function, so
        // this is safe to run even when the path contains real corners.
        if curves.len() > 1 {
            self.enforce_g1_continuity(&mut curves);
        }

        // Close the path if needed
        if closed && !curves.is_empty() {
            let last_end = &curves.last().unwrap().end;
            let first_start = &curves[0].start;
            let dx = last_end.x - first_start.x;
            let dy = last_end.y - first_start.y;
            if (dx * dx + dy * dy).sqrt() > 0.5 {
                curves.push(self.linear_to_cubic(last_end, first_start));
            }
        }

        // Clamp control points to prevent overshoot
        if !curves.is_empty() && points.len() >= 2 {
            let mut min_x = f64::INFINITY;
            let mut min_y = f64::INFINITY;
            let mut max_x = f64::NEG_INFINITY;
            let mut max_y = f64::NEG_INFINITY;
            for p in points {
                min_x = min_x.min(p.x);
                min_y = min_y.min(p.y);
                max_x = max_x.max(p.x);
                max_y = max_y.max(p.y);
            }
            let margin = ((max_x - min_x).max(max_y - min_y) * 0.15).max(2.0);
            let lo_x = min_x - margin;
            let lo_y = min_y - margin;
            let hi_x = max_x + margin;
            let hi_y = max_y + margin;

            for curve in &mut curves {
                curve.control1.x = curve.control1.x.clamp(lo_x, hi_x);
                curve.control1.y = curve.control1.y.clamp(lo_y, hi_y);
                curve.control2.x = curve.control2.x.clamp(lo_x, hi_x);
                curve.control2.y = curve.control2.y.clamp(lo_y, hi_y);
            }
        }

        curves
    }

    /// Detect sharp corners (turn angle > 60°) in a point sequence.
    /// The angle measures the turn between consecutive edge vectors:
    /// 0° = straight, 90° = right angle, 180° = U-turn.
    /// Returns indices of corner points.
    fn detect_sharp_corners(&self, points: &[Point]) -> Vec<usize> {
        let n = points.len();
        if n < 3 {
            return Vec::new();
        }
        let threshold_rad = 30.0f64.to_radians(); // 30° turn = sharp corner (catches 45° chamfers from marching squares)
        let mut corners = Vec::new();

        for i in 1..n - 1 {
            let v1x = points[i].x - points[i - 1].x;
            let v1y = points[i].y - points[i - 1].y;
            let v2x = points[i + 1].x - points[i].x;
            let v2y = points[i + 1].y - points[i].y;
            let len1 = (v1x * v1x + v1y * v1y).sqrt();
            let len2 = (v2x * v2x + v2y * v2y).sqrt();
            if len1 < 1e-6 || len2 < 1e-6 {
                continue;
            }
            let cos_angle = ((v1x * v2x + v1y * v2y) / (len1 * len2)).clamp(-1.0, 1.0);
            let turn_angle = cos_angle.acos(); // 0=straight, π=U-turn
            if turn_angle > threshold_rad {
                corners.push(i);
            }
        }

        corners
    }

    /// Recursively fit cubic Bézier to a segment of points.
    fn fit_segment(&self, points: &[Point], curves: &mut Vec<BezierCurve>) {
        if points.len() < 2 {
            return;
        }
        if points.len() == 2 {
            curves.push(self.linear_to_cubic(&points[0], &points[1]));
            return;
        }

        // Check if points are nearly collinear — use linear Bézier
        if self.is_nearly_linear(points) {
            curves.push(self.linear_to_cubic(&points[0], &points[points.len() - 1]));
            return;
        }

        if points.len() == 3 {
            curves.push(self.fit_three_points(points));
            return;
        }

        const MAX_POINTS_PER_SEGMENT: usize = 40;
        if points.len() > MAX_POINTS_PER_SEGMENT {
            // Split at the point of maximum curvature (corner) instead of midpoint
            let split = self.find_best_split(points);
            self.fit_segment(&points[..=split], curves);
            self.fit_segment(&points[split..], curves);
            return;
        }

        // Initial chord-length parameterization
        let mut t_values = self.chord_length_parameterize(points);

        // Iterative fit with Newton-Raphson reparameterization
        let mut best_curve = self.least_squares_fit(points, &t_values);
        let (mut best_err, mut best_idx) = self.max_fitting_error(&best_curve, points);

        if best_err <= self.tolerance {
            curves.push(best_curve);
            return;
        }

        for _ in 0..self.max_iterations {
            let new_t = self.newton_raphson_reparameterize(&best_curve, points, &t_values);
            t_values = new_t;

            let new_curve = self.least_squares_fit(points, &t_values);
            let (new_err, new_idx) = self.max_fitting_error(&new_curve, points);

            if new_err < best_err {
                best_curve = new_curve;
                best_err = new_err;
                best_idx = new_idx;

                if best_err <= self.tolerance {
                    curves.push(best_curve);
                    return;
                }
            } else {
                break;
            }
        }

        if points.len() <= 3 {
            curves.push(best_curve);
        } else {
            let split = best_idx.max(2).min(points.len() - 2);
            self.fit_segment(&points[..=split], curves);
            self.fit_segment(&points[split..], curves);
        }
    }

    /// Check if a sequence of points is nearly collinear (max deviation < threshold).
    /// For long segments, uses a relative threshold (1% of segment length) to avoid
    /// fitting curves to what are essentially straight lines with tiny deviations.
    fn is_nearly_linear(&self, points: &[Point]) -> bool {
        if points.len() < 3 {
            return true;
        }
        let start = &points[0];
        let end = &points[points.len() - 1];
        let dx = end.x - start.x;
        let dy = end.y - start.y;
        let line_len = (dx * dx + dy * dy).sqrt();
        if line_len < 1e-6 {
            return true;
        }
        // Use the larger of: fixed tolerance, or 1% of segment length.
        // This prevents long near-vertical/horizontal lines from being curved
        // due to tiny pixel-level deviations after simplification.
        let threshold = (self.tolerance * 0.5).max(line_len * 0.01);
        for p in &points[1..points.len() - 1] {
            let dist = ((p.y - start.y) * dx - (p.x - start.x) * dy).abs() / line_len;
            if dist > threshold {
                return false;
            }
        }
        true
    }

    /// Find the best split point for a long segment — the point of maximum angle change.
    fn find_best_split(&self, points: &[Point]) -> usize {
        let n = points.len();
        let mut best_idx = n / 2;
        let mut best_angle_change = 0.0f64;

        for i in 2..n - 2 {
            let v1x = points[i].x - points[i - 2].x;
            let v1y = points[i].y - points[i - 2].y;
            let v2x = points[i + 2].x - points[i].x;
            let v2y = points[i + 2].y - points[i].y;
            let len1 = (v1x * v1x + v1y * v1y).sqrt();
            let len2 = (v2x * v2x + v2y * v2y).sqrt();
            if len1 > 0.0 && len2 > 0.0 {
                let cross = (v1x * v2y - v1y * v2x).abs() / (len1 * len2);
                if cross > best_angle_change {
                    best_angle_change = cross;
                    best_idx = i;
                }
            }
        }

        best_idx.max(2).min(n - 2)
    }

    /// Newton-Raphson reparameterization: find better t values by minimizing |B(t) - P|^2.
    fn newton_raphson_reparameterize(
        &self,
        curve: &BezierCurve,
        points: &[Point],
        t_values: &[f64],
    ) -> Vec<f64> {
        let mut new_t = t_values.to_vec();
        for i in 1..points.len() - 1 {
            let t = t_values[i];
            let p = &points[i];

            let bt = self.evaluate(curve, t);
            let bt_prime = self.evaluate_derivative(curve, t);
            let bt_double_prime = self.evaluate_second_derivative(curve, t);

            let dx = bt.x - p.x;
            let dy = bt.y - p.y;
            let numerator = dx * bt_prime.x + dy * bt_prime.y;
            let denominator = bt_prime.x * bt_prime.x
                + bt_prime.y * bt_prime.y
                + dx * bt_double_prime.x
                + dy * bt_double_prime.y;

            if denominator.abs() > 1e-12 {
                new_t[i] = (t - numerator / denominator).clamp(0.0, 1.0);
            }
        }
        // Ensure monotonicity
        for i in 1..new_t.len() {
            if new_t[i] <= new_t[i - 1] {
                new_t[i] = new_t[i - 1] + 1e-10;
            }
        }
        new_t[0] = 0.0;
        *new_t.last_mut().unwrap() = 1.0;
        new_t
    }

    /// Least-squares cubic Bézier fit with given parameterization.
    fn least_squares_fit(&self, points: &[Point], t_values: &[f64]) -> BezierCurve {
        let n = points.len();
        let start = points[0].clone();
        let end = points[n - 1].clone();

        let mut a11 = 0.0;
        let mut a12 = 0.0;
        let mut a22 = 0.0;
        let mut bx1 = 0.0;
        let mut by1 = 0.0;
        let mut bx2 = 0.0;
        let mut by2 = 0.0;

        for i in 0..n {
            let t = t_values[i];
            let mt = 1.0 - t;
            let b1 = 3.0 * mt * mt * t;
            let b2 = 3.0 * mt * t * t;
            let b0 = mt * mt * mt;
            let b3 = t * t * t;

            a11 += b1 * b1;
            a12 += b1 * b2;
            a22 += b2 * b2;

            let rx = points[i].x - b0 * start.x - b3 * end.x;
            let ry = points[i].y - b0 * start.y - b3 * end.y;

            bx1 += b1 * rx;
            by1 += b1 * ry;
            bx2 += b2 * rx;
            by2 += b2 * ry;
        }

        let det = a11 * a22 - a12 * a12;

        let (control1, control2) = if det.abs() < 1e-12 {
            let dx = end.x - start.x;
            let dy = end.y - start.y;
            (
                Point {
                    x: start.x + dx / 3.0,
                    y: start.y + dy / 3.0,
                },
                Point {
                    x: start.x + 2.0 * dx / 3.0,
                    y: start.y + 2.0 * dy / 3.0,
                },
            )
        } else {
            let inv_det = 1.0 / det;
            (
                Point {
                    x: (a22 * bx1 - a12 * bx2) * inv_det,
                    y: (a22 * by1 - a12 * by2) * inv_det,
                },
                Point {
                    x: (a11 * bx2 - a12 * bx1) * inv_det,
                    y: (a11 * by2 - a12 * by1) * inv_det,
                },
            )
        };

        BezierCurve {
            start,
            control1,
            control2,
            end,
        }
    }

    fn chord_length_parameterize(&self, points: &[Point]) -> Vec<f64> {
        // Centripetal parameterization (α = 0.5): accumulate the square root of
        // the segment length rather than the length itself. Compared with
        // uniform chord-length parameterization this gives parameter values
        // that are more proportional to the curve's actual arc length near
        // sharp turns, which materially reduces Bézier overshoot and
        // self-intersection at corners (Lee 1989). The downstream Newton-Raphson
        // reparameterization refines these further.
        let n = points.len();
        let mut t = vec![0.0; n];
        for i in 1..n {
            let dx = points[i].x - points[i - 1].x;
            let dy = points[i].y - points[i - 1].y;
            let seg = (dx * dx + dy * dy).sqrt();
            // sqrt(seg) → centripetal; guard against zero-length segments.
            t[i] = t[i - 1] + seg.sqrt();
        }
        let total = t[n - 1];
        if total > 0.0 {
            for ti in t.iter_mut() {
                *ti /= total;
            }
        }
        t[n - 1] = 1.0;
        t
    }

    fn fit_three_points(&self, points: &[Point]) -> BezierCurve {
        let p0 = &points[0];
        let p1 = &points[1];
        let p2 = &points[2];
        BezierCurve {
            start: p0.clone(),
            control1: Point {
                x: p0.x + 2.0 / 3.0 * (p1.x - p0.x),
                y: p0.y + 2.0 / 3.0 * (p1.y - p0.y),
            },
            control2: Point {
                x: p2.x + 2.0 / 3.0 * (p1.x - p2.x),
                y: p2.y + 2.0 / 3.0 * (p1.y - p2.y),
            },
            end: p2.clone(),
        }
    }

    fn linear_to_cubic(&self, start: &Point, end: &Point) -> BezierCurve {
        let dx = end.x - start.x;
        let dy = end.y - start.y;
        BezierCurve {
            start: start.clone(),
            control1: Point {
                x: start.x + dx / 3.0,
                y: start.y + dy / 3.0,
            },
            control2: Point {
                x: start.x + 2.0 * dx / 3.0,
                y: start.y + 2.0 * dy / 3.0,
            },
            end: end.clone(),
        }
    }

    fn max_fitting_error(&self, curve: &BezierCurve, points: &[Point]) -> (f64, usize) {
        let t_values = self.chord_length_parameterize(points);
        let mut max_err = 0.0;
        let mut max_idx = 0;
        for i in 1..points.len() - 1 {
            let curve_pt = self.evaluate(curve, t_values[i]);
            let dx = points[i].x - curve_pt.x;
            let dy = points[i].y - curve_pt.y;
            let err = (dx * dx + dy * dy).sqrt();
            if err > max_err {
                max_err = err;
                max_idx = i;
            }
        }
        (max_err, max_idx)
    }

    fn evaluate(&self, curve: &BezierCurve, t: f64) -> Point {
        let t2 = t * t;
        let t3 = t2 * t;
        let mt = 1.0 - t;
        let mt2 = mt * mt;
        let mt3 = mt2 * mt;

        Point {
            x: mt3 * curve.start.x
                + 3.0 * mt2 * t * curve.control1.x
                + 3.0 * mt * t2 * curve.control2.x
                + t3 * curve.end.x,
            y: mt3 * curve.start.y
                + 3.0 * mt2 * t * curve.control1.y
                + 3.0 * mt * t2 * curve.control2.y
                + t3 * curve.end.y,
        }
    }

    fn evaluate_derivative(&self, curve: &BezierCurve, t: f64) -> Point {
        let mt = 1.0 - t;
        let a_x = curve.control1.x - curve.start.x;
        let a_y = curve.control1.y - curve.start.y;
        let b_x = curve.control2.x - curve.control1.x;
        let b_y = curve.control2.y - curve.control1.y;
        let c_x = curve.end.x - curve.control2.x;
        let c_y = curve.end.y - curve.control2.y;

        Point {
            x: 3.0 * mt * mt * a_x + 6.0 * mt * t * b_x + 3.0 * t * t * c_x,
            y: 3.0 * mt * mt * a_y + 6.0 * mt * t * b_y + 3.0 * t * t * c_y,
        }
    }

    fn evaluate_second_derivative(&self, curve: &BezierCurve, t: f64) -> Point {
        let mt = 1.0 - t;
        let a_x = curve.control2.x - 2.0 * curve.control1.x + curve.start.x;
        let a_y = curve.control2.y - 2.0 * curve.control1.y + curve.start.y;
        let b_x = curve.end.x - 2.0 * curve.control2.x + curve.control1.x;
        let b_y = curve.end.y - 2.0 * curve.control2.y + curve.control1.y;

        Point {
            x: 6.0 * mt * a_x + 6.0 * t * b_x,
            y: 6.0 * mt * a_y + 6.0 * t * b_y,
        }
    }

    /// Enforce G1 continuity between adjacent curves, but skip junctions that
    /// are genuine sharp corners. At a sharp corner the incoming and outgoing
    /// tangents are meant to differ, so aligning them would round off the
    /// corner. A junction counts as a corner when the turn between the
    /// incoming tangent (end→control2 of curve i) and the outgoing tangent
    /// (start→control1 of curve i+1) exceeds ~30° — the same threshold used by
    /// `detect_sharp_corners`. This lets smooth stretches that were split
    /// (e.g. by the recursive error split) still join tangent-continuously
    /// even when the overall path contains one or more real corners.
    fn enforce_g1_continuity(&self, curves: &mut [BezierCurve]) {
        let corner_turn = 30.0f64.to_radians();
        for i in 0..curves.len().saturating_sub(1) {
            let current_control2 = curves[i].control2.clone();
            let current_end = curves[i].end.clone();
            let next = &mut curves[i + 1];

            let t1x = current_end.x - current_control2.x;
            let t1y = current_end.y - current_control2.y;
            let t2x = next.control1.x - next.start.x;
            let t2y = next.control1.y - next.start.y;

            let len1 = (t1x * t1x + t1y * t1y).sqrt();
            let len2 = (t2x * t2x + t2y * t2y).sqrt();

            if len1 > 1e-10 && len2 > 1e-10 {
                // Skip G1 alignment at sharp corners (preserve the kink).
                let cos_turn = ((t1x * t2x + t1y * t2y) / (len1 * len2)).clamp(-1.0, 1.0);
                let turn = cos_turn.acos();
                if turn > corner_turn {
                    continue;
                }
                let scale = len2 / len1;
                next.control1.x = next.start.x + t1x * scale;
                next.control1.y = next.start.y + t1y * scale;
            }
        }
    }
}

/// Default number of decimal places used when writing SVG path coordinates.
pub const DEFAULT_COORDINATE_PRECISION: u32 = 2;

/// Clamp a requested coordinate precision to the supported 1..=6 range.
pub fn clamp_precision(p: u32) -> u32 {
    p.clamp(1, 6)
}

/// Write a formatted number with the requested decimal precision directly to
/// a writer (avoids returning a String). Whole numbers collapse to integers
/// and trailing zeros are trimmed.
pub fn write_num_to<W: Write>(writer: &mut W, v: f64, precision: u32) -> IoResult<()> {
    if (v - v.round()).abs() < 1e-4 {
        write!(writer, "{}", v.round() as i64)
    } else {
        let s = format!("{:.*}", precision as usize, v);
        let trimmed = s.trim_end_matches('0').trim_end_matches('.');
        write!(writer, "{}", trimmed)
    }
}

/// Stream Bézier curves as SVG path data directly to a writer.
pub fn bezier_to_svg_path_to<W: Write>(
    writer: &mut W,
    curves: &[BezierCurve],
    closed: bool,
) -> IoResult<()> {
    bezier_to_svg_path_to_prec(writer, curves, closed, DEFAULT_COORDINATE_PRECISION)
}

/// Stream Bézier curves as SVG path data directly to a writer, with
/// user-controlled coordinate precision (1-6 decimal places, clamped).
pub fn bezier_to_svg_path_to_prec<W: Write>(
    writer: &mut W,
    curves: &[BezierCurve],
    closed: bool,
    precision: u32,
) -> IoResult<()> {
    let precision = clamp_precision(precision);
    if curves.is_empty() {
        return Ok(());
    }

    write!(writer, "M",)?;
    write_num_to(writer, curves[0].start.x, precision)?;
    write!(writer, ",",)?;
    write_num_to(writer, curves[0].start.y, precision)?;

    let mut i = 0;
    while i < curves.len() {
        let curve = &curves[i];
        if is_linear_curve(curve) {
            let start = &curves[i].start;
            let mut end = &curve.end;
            let mut j = i + 1;
            while j < curves.len() {
                let next = &curves[j];
                if !is_linear_curve(next) {
                    break;
                }
                let candidate_end = &next.end;
                let dx = candidate_end.x - start.x;
                let dy = candidate_end.y - start.y;
                let line_len = (dx * dx + dy * dy).sqrt();
                if line_len < 0.5 {
                    end = candidate_end;
                    j += 1;
                    continue;
                }
                let dist = ((end.y - start.y) * dx - (end.x - start.x) * dy).abs() / line_len;
                if dist < 1.5 {
                    end = candidate_end;
                    j += 1;
                } else {
                    break;
                }
            }
            write!(writer, "L",)?;
            write_num_to(writer, end.x, precision)?;
            write!(writer, ",",)?;
            write_num_to(writer, end.y, precision)?;
            i = j;
        } else {
            write!(writer, "C",)?;
            write_num_to(writer, curve.control1.x, precision)?;
            write!(writer, ",",)?;
            write_num_to(writer, curve.control1.y, precision)?;
            write!(writer, " ",)?;
            write_num_to(writer, curve.control2.x, precision)?;
            write!(writer, ",",)?;
            write_num_to(writer, curve.control2.y, precision)?;
            write!(writer, " ",)?;
            write_num_to(writer, curve.end.x, precision)?;
            write!(writer, ",",)?;
            write_num_to(writer, curve.end.y, precision)?;
            i += 1;
        }
    }

    if closed {
        write!(writer, "Z",)?;
    }

    Ok(())
}

/// Check if a cubic Bézier is effectively a straight line
/// (control points lie close to the start-end line).
pub fn is_linear_curve(curve: &BezierCurve) -> bool {
    let dx = curve.end.x - curve.start.x;
    let dy = curve.end.y - curve.start.y;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 0.5 {
        return true;
    }
    let d1 = ((curve.control1.y - curve.start.y) * dx - (curve.control1.x - curve.start.x) * dy)
        .abs()
        / len;
    let d2 = ((curve.control2.y - curve.start.y) * dx - (curve.control2.x - curve.start.x) * dy)
        .abs()
        / len;
    d1 < 1.0 && d2 < 1.0
}

/// Potrace-style corner optimization: collinearize nearly-linear cubics at sharp joins.
pub fn optimize_bezier_corners(curves: &mut [BezierCurve]) {
    for curve in curves.iter_mut() {
        if is_linear_curve(curve) {
            let t1 = 1.0 / 3.0;
            let t2 = 2.0 / 3.0;
            curve.control1 = Point {
                x: curve.start.x + t1 * (curve.end.x - curve.start.x),
                y: curve.start.y + t1 * (curve.end.y - curve.start.y),
            };
            curve.control2 = Point {
                x: curve.start.x + t2 * (curve.end.x - curve.start.x),
                y: curve.start.y + t2 * (curve.end.y - curve.start.y),
            };
        }
    }
}

fn point_dist(a: &Point, b: &Point) -> f64 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
}

fn merge_linear_curves(a: &BezierCurve, b: &BezierCurve) -> BezierCurve {
    let t1 = 1.0 / 3.0;
    let t2 = 2.0 / 3.0;
    BezierCurve {
        start: a.start.clone(),
        control1: Point {
            x: a.start.x + t1 * (b.end.x - a.start.x),
            y: a.start.y + t1 * (b.end.y - a.start.y),
        },
        control2: Point {
            x: a.start.x + t2 * (b.end.x - a.start.x),
            y: a.start.y + t2 * (b.end.y - a.start.y),
        },
        end: b.end.clone(),
    }
}

/// Default merge tolerance (px) for Potrace `-O` style spline optimization.
pub const DEFAULT_OPTIMIZE_TOLERANCE: f64 = 1.5;

/// Potrace `-O` style spline optimization: collinearize then merge consecutive nearly-linear segments.
#[allow(dead_code)] // Re-exported at crate root; clippy cannot see the chain.
pub fn optimize_bezier_curves(curves: &mut Vec<BezierCurve>) {
    optimize_bezier_curves_with_tolerance(curves, DEFAULT_OPTIMIZE_TOLERANCE);
}

/// Potrace `-O` style spline optimization with a user-controlled merge
/// tolerance (max perpendicular distance in px for merging nearly-linear
/// segments). Higher values merge more aggressively and shrink path counts.
pub fn optimize_bezier_curves_with_tolerance(curves: &mut Vec<BezierCurve>, tolerance: f64) {
    if curves.len() < 2 {
        optimize_bezier_corners(curves);
        return;
    }

    optimize_bezier_corners(curves);

    let mut merged = Vec::with_capacity(curves.len());
    let mut i = 0;
    while i < curves.len() {
        let mut current = curves[i].clone();
        i += 1;
        while i < curves.len() {
            let next = &curves[i];
            if point_dist(&current.end, &next.start) > 1.0 {
                break;
            }
            if !(is_linear_curve(&current) && is_linear_curve(next)) {
                break;
            }
            let start = &current.start;
            let candidate_end = &next.end;
            let dx = candidate_end.x - start.x;
            let dy = candidate_end.y - start.y;
            let line_len = (dx * dx + dy * dy).sqrt();
            if line_len < 0.5 {
                current = merge_linear_curves(&current, next);
                i += 1;
                continue;
            }
            let mid = &current.end;
            let dist = ((mid.y - start.y) * dx - (mid.x - start.x) * dy).abs() / line_len;
            if dist < tolerance {
                current = merge_linear_curves(&current, next);
                i += 1;
            } else {
                break;
            }
        }
        merged.push(current);
    }
    *curves = merged;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_linear_to_cubic() {
        let fitter = BezierFitter::new(2.0);
        let start = Point { x: 0.0, y: 0.0 };
        let end = Point { x: 10.0, y: 10.0 };
        let curve = fitter.linear_to_cubic(&start, &end);
        assert_eq!(curve.start.x, 0.0);
        assert_eq!(curve.end.x, 10.0);
    }

    #[test]
    fn test_fit_path_two_points() {
        let fitter = BezierFitter::new(2.0);
        let points = vec![Point { x: 0.0, y: 0.0 }, Point { x: 10.0, y: 10.0 }];
        let curves = fitter.fit_path(&points, false);
        assert_eq!(curves.len(), 1);
    }

    #[test]
    fn test_fit_path_semicircle() {
        let fitter = BezierFitter::new(2.0);
        let points: Vec<Point> = (0..=20)
            .map(|i| {
                let t = i as f64 / 20.0 * std::f64::consts::PI;
                Point {
                    x: t.cos() * 50.0 + 50.0,
                    y: t.sin() * 50.0,
                }
            })
            .collect();
        let curves = fitter.fit_path(&points, false);
        assert!(!curves.is_empty());
        assert!(curves.len() <= 10);
    }

    #[test]
    fn test_fit_path_closed() {
        let fitter = BezierFitter::new(2.0);
        let points = vec![
            Point { x: 0.0, y: 0.0 },
            Point { x: 10.0, y: 0.0 },
            Point { x: 10.0, y: 10.0 },
            Point { x: 0.0, y: 10.0 },
        ];
        let curves = fitter.fit_path(&points, true);
        assert!(!curves.is_empty());
    }

    #[test]
    fn test_g1_preserves_sharp_corner() {
        // An L-shape with a 90° corner: the junction at the corner must keep
        // distinct tangents (not be G1-aligned), otherwise the corner rounds off.
        let fitter = BezierFitter::new(2.0);
        let mut points = Vec::new();
        for x in 0..=10 {
            points.push(Point { x: x as f64, y: 0.0 });
        }
        for y in 1..=10 {
            points.push(Point { x: 10.0, y: y as f64 });
        }
        let curves = fitter.fit_path(&points, false);
        assert!(curves.len() >= 2);
        // Find the junction closest to the corner (10, 0) and verify the
        // incoming and outgoing tangents are not parallel (corner preserved).
        let mut found_corner_junction = false;
        for i in 0..curves.len().saturating_sub(1) {
            let j = i + 1;
            if (curves[i].end.x - 10.0).abs() < 1.5 && (curves[i].end.y - 0.0).abs() < 1.5 {
                let t1x = curves[i].end.x - curves[i].control2.x;
                let t1y = curves[i].end.y - curves[i].control2.y;
                let t2x = curves[j].control1.x - curves[j].start.x;
                let t2y = curves[j].control1.y - curves[j].start.y;
                let cross = t1x * t2y - t1y * t2x;
                assert!(cross.abs() > 1e-3, "corner was G1-smoothed away");
                found_corner_junction = true;
                break;
            }
        }
        assert!(found_corner_junction, "expected a junction near the L corner");
    }

    #[test]
    fn test_centripetal_parameterization_monotonic_and_bounded() {
        let fitter = BezierFitter::new(2.0);
        let points = vec![
            Point { x: 0.0, y: 0.0 },
            Point { x: 10.0, y: 0.0 },
            Point { x: 10.0, y: 10.0 },
            Point { x: 0.0, y: 10.0 },
        ];
        let t = fitter.chord_length_parameterize(&points);
        assert_eq!(t[0], 0.0);
        assert!((t[t.len() - 1] - 1.0).abs() < 1e-9);
        for i in 1..t.len() {
            assert!(t[i] > t[i - 1], "parameterization must be strictly increasing");
        }
    }

    #[test]
    fn test_newton_raphson_monotonic() {
        let fitter = BezierFitter::new(1.0);
        let curve = BezierCurve {
            start: Point { x: 0.0, y: 0.0 },
            control1: Point { x: 5.0, y: 10.0 },
            control2: Point { x: 10.0, y: 10.0 },
            end: Point { x: 15.0, y: 0.0 },
        };
        let points: Vec<Point> = (0..=10)
            .map(|i| fitter.evaluate(&curve, i as f64 / 10.0))
            .collect();
        let t_initial = fitter.chord_length_parameterize(&points);
        let t_refined = fitter.newton_raphson_reparameterize(&curve, &points, &t_initial);
        for i in 1..t_refined.len() {
            assert!(t_refined[i] >= t_refined[i - 1]);
        }
    }

    #[test]
    fn test_control_point_clamping() {
        let fitter = BezierFitter::new(2.0);
        // Points in a small region — control points should be clamped
        let points = vec![
            Point { x: 0.0, y: 0.0 },
            Point { x: 1.0, y: 5.0 },
            Point { x: 2.0, y: 0.0 },
            Point { x: 3.0, y: 5.0 },
            Point { x: 4.0, y: 0.0 },
        ];
        let curves = fitter.fit_path(&points, false);
        for curve in &curves {
            // Control points should be within reasonable bounds
            assert!(curve.control1.x >= -2.0 && curve.control1.x <= 6.0);
            assert!(curve.control1.y >= -2.0 && curve.control1.y <= 7.0);
        }
    }

    #[test]
    fn test_bezier_to_svg_path_to_starts_with_m() {
        let fitter = BezierFitter::new(2.0);
        let points = vec![
            Point { x: 0.0, y: 0.0 },
            Point { x: 5.0, y: 10.0 },
            Point { x: 10.0, y: 10.0 },
            Point { x: 15.0, y: 0.0 },
        ];
        let curves = fitter.fit_path(&points, true);

        let mut buf = Vec::new();
        bezier_to_svg_path_to(&mut buf, &curves, true).unwrap();
        let svg = String::from_utf8(buf).unwrap();

        assert!(svg.starts_with('M'));
        assert!(svg.ends_with('Z'));
    }

    #[test]
    fn test_bezier_to_svg_path_to_empty_returns_empty() {
        let mut buf = Vec::new();
        bezier_to_svg_path_to(&mut buf, &[], true).unwrap();
        assert!(buf.is_empty());
    }

    #[test]
    fn test_write_num_to_compact_format() {
        let cases = [
            (0.0, "0"),
            (1.0, "1"),
            (1.5, "1.5"),
            (2.25, "2.25"),
            (10.0, "10"),
            (10.10, "10.1"),
            (10.01, "10.01"),
        ];
        for (v, expected) in cases {
            let mut buf = Vec::new();
            write_num_to(&mut buf, v, 2).unwrap();
            let actual = String::from_utf8(buf).unwrap();
            assert_eq!(actual, expected, "write_num_to mismatch for {}", v);
        }
    }

    #[test]
    fn test_write_num_to_precision() {
        // Precision 1: rounds to one decimal, trims trailing zero.
        let mut buf = Vec::new();
        write_num_to(&mut buf, 1.2345, 1).unwrap();
        assert_eq!(String::from_utf8(buf).unwrap(), "1.2");

        // Precision 3: keeps three decimals.
        let mut buf = Vec::new();
        write_num_to(&mut buf, 1.2346, 3).unwrap();
        assert_eq!(String::from_utf8(buf).unwrap(), "1.235");

        // Precision 1 on a whole number collapses to integer.
        let mut buf = Vec::new();
        write_num_to(&mut buf, 7.0, 1).unwrap();
        assert_eq!(String::from_utf8(buf).unwrap(), "7");

        // Values above 6 are clamped.
        let mut buf = Vec::new();
        write_num_to(&mut buf, 1.2345678, 99).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.starts_with("1.23456"), "unexpected: {}", s);
    }

    #[test]
    fn test_bezier_to_svg_path_to_prec() {
        let curves = vec![BezierCurve {
            start: Point { x: 0.0, y: 0.0 },
            control1: Point { x: 5.0, y: 0.5 },
            control2: Point { x: 0.5, y: 6.0 },
            end: Point { x: 5.6789, y: 6.7890 },
        }];

        let mut buf = Vec::new();
        bezier_to_svg_path_to_prec(&mut buf, &curves, true, 1).unwrap();
        let p1 = String::from_utf8(buf).unwrap();
        assert_eq!(p1, "M0,0C5,0.5 0.5,6 5.7,6.8Z");

        let mut buf = Vec::new();
        bezier_to_svg_path_to_prec(&mut buf, &curves, true, 3).unwrap();
        let p3 = String::from_utf8(buf).unwrap();
        assert_eq!(p3, "M0,0C5,0.5 0.5,6 5.679,6.789Z");

        // Default precision matches the historical 2-decimal output.
        let mut buf = Vec::new();
        bezier_to_svg_path_to(&mut buf, &curves, true).unwrap();
        let default = String::from_utf8(buf).unwrap();
        assert_eq!(default, "M0,0C5,0.5 0.5,6 5.68,6.79Z");
    }

    #[test]
    fn test_clamp_precision() {
        assert_eq!(clamp_precision(0), 1);
        assert_eq!(clamp_precision(1), 1);
        assert_eq!(clamp_precision(2), 2);
        assert_eq!(clamp_precision(6), 6);
        assert_eq!(clamp_precision(100), 6);
    }

    /// A chain of collinear segments with one segment slightly off the line
    /// (perpendicular offset `off`), plus a sharp 90° corner at the end.
    fn almost_collinear_chain(off: f64) -> Vec<BezierCurve> {
        let lin = |x0: f64, y0: f64, x1: f64, y1: f64| BezierCurve {
            start: Point { x: x0, y: y0 },
            control1: Point {
                x: x0 + (x1 - x0) / 3.0,
                y: y0 + (y1 - y0) / 3.0,
            },
            control2: Point {
                x: x0 + 2.0 * (x1 - x0) / 3.0,
                y: y0 + 2.0 * (y1 - y0) / 3.0,
            },
            end: Point { x: x1, y: y1 },
        };
        vec![
            lin(0.0, 0.0, 10.0, 0.0),
            lin(10.0, 0.0, 20.0, off), // slightly off the y=0 line
            lin(20.0, off, 20.0, 10.0), // 90° corner — must never merge
        ]
    }

    #[test]
    fn test_optimize_tolerance_controls_merging() {
        // Offset 1.0 → midpoint distance ≈ 0.25: below 1.5 merges, above does not.
        let mut strict = almost_collinear_chain(1.0);
        optimize_bezier_curves_with_tolerance(&mut strict, 0.1);
        assert_eq!(strict.len(), 3, "tolerance 0.1 must not merge offset 1.0");

        let mut loose = almost_collinear_chain(1.0);
        optimize_bezier_curves_with_tolerance(&mut loose, 1.5);
        assert!(loose.len() < 3, "tolerance 1.5 should merge offset 1.0");

        // Offset 5.0 → midpoint distance ≈ 1.21: tolerance 1.0 must not merge it.
        let mut far = almost_collinear_chain(5.0);
        optimize_bezier_curves_with_tolerance(&mut far, 1.0);
        assert_eq!(far.len(), 3, "offset beyond tolerance must not merge");

        // Default wrapper matches the historical behavior.
        let mut default = almost_collinear_chain(1.0);
        optimize_bezier_curves(&mut default);
        assert_eq!(default.len(), loose.len(), "wrapper must match default tolerance");
    }

    #[test]
    fn optimize_curves_merges_collinear_segments() {
        let t1 = 1.0 / 3.0;
        let t2 = 2.0 / 3.0;
        let seg = |x0: f64, y0: f64, x1: f64, y1: f64| BezierCurve {
            start: Point { x: x0, y: y0 },
            control1: Point {
                x: x0 + t1 * (x1 - x0),
                y: y0 + t1 * (y1 - y0),
            },
            control2: Point {
                x: x0 + t2 * (x1 - x0),
                y: y0 + t2 * (y1 - y0),
            },
            end: Point { x: x1, y: y1 },
        };
        let mut curves = vec![seg(0.0, 0.0, 5.0, 0.0), seg(5.0, 0.0, 10.0, 0.0), seg(10.0, 0.0, 15.0, 0.0)];
        optimize_bezier_curves(&mut curves);
        assert_eq!(curves.len(), 1);
        assert_eq!(curves[0].start.x, 0.0);
        assert_eq!(curves[0].end.x, 15.0);
    }

    #[test]
    fn optimize_curves_preserves_corners() {
        let fitter = BezierFitter::new(2.0);
        let points = vec![
            Point { x: 0.0, y: 0.0 },
            Point { x: 10.0, y: 0.0 },
            Point { x: 10.0, y: 10.0 },
        ];
        let mut curves = fitter.fit_path(&points, false);
        let before = curves.len();
        optimize_bezier_curves(&mut curves);
        assert_eq!(curves.len(), before, "right-angle path should keep segment count");
    }
}
