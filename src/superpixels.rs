//! Experimental edge attraction to a glyph's 2×3 lattice.
//!
//! Fit one continuous inverse displacement mesh, then resample once. Constraints
//! act along edge normals, leaving tangential motion free. All coordinates in
//! the solver are measured in half-cell widths / third-cell heights, so fonts
//! with odd raster dimensions still use exact geometric halves and thirds.
use photon_rs::PhotonImage;

const SAMPLES: usize = 4;
/// Full alignment is the maximum supported strength.
pub const MAX_STRENGTH: u32 = 100;
// A point can be half an interval away from its nearest intersection in both
// axes. Cover that entire square, including features halfway between lines.
const MAX_MOVE: f32 = std::f32::consts::FRAC_1_SQRT_2;
const MIN_JACOBIAN: f32 = 0.30;

#[derive(Clone, Copy, Default, Debug)]
struct Vec2 {
    x: f32,
    y: f32,
}

impl Vec2 {
    fn dot(self, b: Self) -> f32 {
        self.x * b.x + self.y * b.y
    }
    fn length(self) -> f32 {
        self.dot(self).sqrt()
    }
    fn scaled(self, s: f32) -> Self {
        Self {
            x: self.x * s,
            y: self.y * s,
        }
    }
    fn plus(self, b: Self) -> Self {
        Self {
            x: self.x + b.x,
            y: self.y + b.y,
        }
    }
    fn minus(self, b: Self) -> Self {
        self.plus(b.scaled(-1.0))
    }
}

#[derive(Clone, Copy, Default)]
struct Edge {
    normal: Vec2,
    magnitude: f32,
}

#[derive(Clone, Copy, Default)]
struct Anchor {
    normal: Vec2,
    offset: f32,
    weight: f32,
}

#[derive(Clone, Debug, Default)]
pub struct WarpStats {
    pub anchors: usize,
    /// Largest source lookup offset in half-width / third-height units.
    pub max_displacement: f32,
    pub min_jacobian: f32,
    /// Smallest fraction of requested movement retained at any node for safety.
    pub safety_scale: f32,
}

pub struct WarpResult {
    pub image: PhotonImage,
    pub stats: WarpStats,
    mesh: Vec<Vec2>,
    nx: usize,
    ny: usize,
    step: Vec2,
}

fn sample_scalar(values: &[Edge], w: usize, h: usize, x: f32, y: f32) -> f32 {
    let x = x.clamp(0.0, (w - 1) as f32);
    let y = y.clamp(0.0, (h - 1) as f32);
    let (ix, iy) = (x as usize, y as usize);
    let (jx, jy) = ((ix + 1).min(w - 1), (iy + 1).min(h - 1));
    let (fx, fy) = (x - ix as f32, y - iy as f32);
    (values[iy * w + ix].magnitude * (1.0 - fx) + values[iy * w + jx].magnitude * fx) * (1.0 - fy)
        + (values[jy * w + ix].magnitude * (1.0 - fx) + values[jy * w + jx].magnitude * fx) * fy
}

fn edges(image: &PhotonImage, w: usize, h: usize) -> Vec<Edge> {
    let rgba = image::RgbaImage::from_raw(
        image.get_width(),
        image.get_height(),
        image.get_raw_pixels(),
    )
    .unwrap();
    let analysis = image::imageops::resize(
        &rgba,
        w as u32,
        h as u32,
        image::imageops::FilterType::Triangle,
    );
    // Premultiplied colour plus alpha finds isoluminant colour boundaries and
    // transparency boundaries without detecting hidden RGB in transparent pixels.
    let mut values: Vec<[f32; 4]> = analysis
        .pixels()
        .map(|p| {
            let a = p[3] as f32 / 255.0;
            [
                p[0] as f32 / 255.0 * a,
                p[1] as f32 / 255.0 * a,
                p[2] as f32 / 255.0 * a,
                a,
            ]
        })
        .collect();
    // Separable binomial blur: six taps instead of nine per pixel, reusing the
    // input allocation for the vertical pass.
    let mut horizontal = vec![[0.0; 4]; w * h];
    for y in 0..h {
        for x in 0..w {
            let left = y * w + x.saturating_sub(1);
            let right = y * w + (x + 1).min(w - 1);
            for c in 0..4 {
                horizontal[y * w + x][c] =
                    (values[left][c] + 2.0 * values[y * w + x][c] + values[right][c]) * 0.25;
            }
        }
    }
    for y in 0..h {
        for x in 0..w {
            let above = y.saturating_sub(1) * w + x;
            let below = (y + 1).min(h - 1) * w + x;
            for c in 0..4 {
                values[y * w + x][c] =
                    (horizontal[above][c] + 2.0 * horizontal[y * w + x][c] + horizontal[below][c])
                        * 0.25;
            }
        }
    }
    drop(horizontal);
    let blurred = values;
    let mut result = vec![Edge::default(); w * h];
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let mut xx = 0.0;
            let mut xy = 0.0;
            let mut yy = 0.0;
            for c in 0..4 {
                let dx = (blurred[y * w + x + 1][c] - blurred[y * w + x - 1][c]) * 0.5;
                let dy = (blurred[(y + 1) * w + x][c] - blurred[(y - 1) * w + x][c]) * 0.5;
                xx += dx * dx;
                xy += dx * dy;
                yy += dy * dy;
            }
            let difference = ((xx - yy).powi(2) + 4.0 * xy * xy).sqrt();
            let coherence = difference / (xx + yy + 1e-8);
            // Reject ambiguous junctions/texture rather than choosing an arbitrary normal.
            if coherence < 0.65 {
                continue;
            }
            let angle = 0.5 * (2.0 * xy).atan2(xx - yy);
            result[y * w + x] = Edge {
                normal: Vec2 {
                    x: angle.cos(),
                    y: angle.sin(),
                },
                magnitude: ((xx + yy + difference) * 0.5).sqrt(),
            };
        }
    }
    result
}

fn anchors(image: &PhotonImage, nx: usize, ny: usize) -> Vec<Anchor> {
    let (w, h) = ((nx - 1) * SAMPLES, (ny - 1) * SAMPLES);
    let features = edges(image, w, h);
    let peak = features.iter().map(|e| e.magnitude).fold(0.0f32, f32::max);
    let threshold = (peak * 0.08).max(0.025);
    let mut result = vec![Anchor::default(); nx * ny];
    let mut costs = vec![f32::INFINITY; nx * ny];
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let edge = features[y * w + x];
            if edge.magnitude < threshold {
                continue;
            }
            let n = edge.normal;
            let before = sample_scalar(&features, w, h, x as f32 - n.x, y as f32 - n.y);
            let after = sample_scalar(&features, w, h, x as f32 + n.x, y as f32 + n.y);
            if edge.magnitude < before || edge.magnitude < after {
                continue;
            }
            // A quadratic fit locates the edge between pixel centres, including
            // flat two-pixel ridges (a common case for an already aligned edge).
            let denominator = before - 2.0 * edge.magnitude + after;
            let offset = if denominator < -1e-7 {
                (0.5 * (before - after) / denominator).clamp(-0.5, 0.5)
            } else {
                0.0
            };
            let p = Vec2 {
                x: (x as f32 + 0.5 + n.x * offset) / SAMPLES as f32,
                y: (y as f32 + 0.5 + n.y * offset) / SAMPLES as f32,
            };
            // Associate each ridge sample with its nearest lattice intersection.
            let gx = p.x.round().clamp(0.0, (nx - 1) as f32) as usize;
            let gy = p.y.round().clamp(0.0, (ny - 1) as f32) as usize;
            let delta = p.minus(Vec2 {
                x: gx as f32,
                y: gy as f32,
            });
            let normal_offset = delta.dot(n);
            let tangent_offset = delta.dot(Vec2 { x: -n.y, y: n.x });
            // Nearest-node assignment already bounds the capture region. A
            // second cutoff used to leave gaps near half-interval boundaries,
            // missing precisely the features that need the largest correction.
            let cost = normal_offset.powi(2) + tangent_offset.powi(2) * 0.3;
            let index = gy * nx + gx;
            if cost < costs[index] {
                costs[index] = cost;
                result[index] = Anchor {
                    normal: n,
                    offset: normal_offset,
                    weight: 24.0 * (edge.magnitude / (threshold * 3.0)).min(1.0),
                };
            }
        }
    }
    result
}

fn solve(anchors: &[Anchor], nx: usize, ny: usize) -> Vec<Vec2> {
    let mut current = vec![Vec2::default(); nx * ny];
    let mut next = current.clone();
    // Screened membrane energy: smooth displacement, weak attraction to the
    // identity map, and strong normal-only edge constraints. Fixed work O(cells).
    for _ in 0..48 {
        for y in 0..ny {
            for x in 0..nx {
                let i = y * nx + x;
                let a = anchors[i];
                let mut sum = Vec2::default();
                let mut neighbours = 0.0;
                for (dx, dy) in [(-1isize, 0isize), (1, 0), (0, -1), (0, 1)] {
                    let (px, py) = (x as isize + dx, y as isize + dy);
                    if px >= 0 && px < nx as isize && py >= 0 && py < ny as isize {
                        sum = sum.plus(current[py as usize * nx + px as usize]);
                        neighbours += 1.0;
                    }
                }
                let lambda = 0.35;
                let diagonal = neighbours * lambda + 0.025;
                let aa = diagonal + a.weight * a.normal.x * a.normal.x;
                let bb = a.weight * a.normal.x * a.normal.y;
                let cc = diagonal + a.weight * a.normal.y * a.normal.y;
                let rhs = sum
                    .scaled(lambda)
                    .plus(a.normal.scaled(a.weight * a.offset));
                let determinant = aa * cc - bb * bb;
                let mut u = Vec2 {
                    x: (cc * rhs.x - bb * rhs.y) / determinant,
                    y: (aa * rhs.y - bb * rhs.x) / determinant,
                };
                // The boundary can slide along itself but cannot leave the image.
                if x == 0 || x == nx - 1 {
                    u.x = 0.0;
                }
                if y == 0 || y == ny - 1 {
                    u.y = 0.0;
                }
                let length = u.length();
                if length > MAX_MOVE {
                    u = u.scaled(MAX_MOVE / length);
                }
                next[i] = u;
            }
        }
        std::mem::swap(&mut next, &mut current);
    }
    current
}

fn cell_jacobian(mesh: &[Vec2], nx: usize, x: usize, y: usize) -> f32 {
    let a = mesh[y * nx + x];
    let b = mesh[y * nx + x + 1];
    let c = mesh[(y + 1) * nx + x];
    let d = mesh[(y + 1) * nx + x + 1];
    // The determinant of a bilinear map is affine within each cell;
    // checking all four corners bounds it everywhere, not just at nodes.
    let mut minimum = 1.0f32;
    for dx in [b.minus(a), d.minus(c)] {
        for dy in [c.minus(a), d.minus(b)] {
            minimum = minimum.min((1.0 + dx.x) * (1.0 + dy.y) - dx.y * dy.x);
        }
    }
    minimum
}

fn min_jacobian(mesh: &[Vec2], nx: usize, ny: usize) -> f32 {
    let mut minimum = 1.0f32;
    for y in 0..ny - 1 {
        for x in 0..nx - 1 {
            minimum = minimum.min(cell_jacobian(mesh, nx, x, y));
        }
    }
    minimum
}

fn protect_mesh(mesh: &mut [Vec2], nx: usize, ny: usize, min_area: f32) -> f32 {
    let mut scales = vec![1.0f32; mesh.len()];
    let mut crowded = vec![false; mesh.len()];
    // Only attenuate nodes touching unsafe cells. Previously one conflicting
    // pair of features weakened the warp everywhere, even on distant contours.
    // Recheck neighbours after each batch: they share the modified nodes.
    for _ in 0..32 {
        crowded.fill(false);
        let mut safe = true;
        for y in 0..ny - 1 {
            for x in 0..nx - 1 {
                if cell_jacobian(mesh, nx, x, y) < min_area {
                    safe = false;
                    let i = y * nx + x;
                    for node in [i, i + 1, i + nx, i + nx + 1] {
                        crowded[node] = true;
                    }
                }
            }
        }
        if safe {
            return scales.into_iter().fold(1.0, f32::min);
        }
        for i in 0..mesh.len() {
            if crowded[i] {
                mesh[i] = mesh[i].scaled(0.8);
                scales[i] *= 0.8;
            }
        }
    }
    // Bounded local work, with a conservative fallback for pathological meshes.
    // Scaling toward identity guarantees that the no-fold condition is reached.
    let mut scale = scales.into_iter().fold(1.0, f32::min);
    while min_jacobian(mesh, nx, ny) < min_area {
        for u in mesh.iter_mut() {
            *u = u.scaled(0.8);
        }
        scale *= 0.8;
    }
    scale
}

impl WarpResult {
    fn displacement(&self, x: f32, y: f32) -> Vec2 {
        if self.mesh.is_empty() {
            return Vec2::default();
        }
        let x = (x / self.step.x).clamp(0.0, (self.nx - 1) as f32);
        let y = (y / self.step.y).clamp(0.0, (self.ny - 1) as f32);
        let (ix, iy) = ((x as usize).min(self.nx - 2), (y as usize).min(self.ny - 2));
        let (fx, fy) = (x - ix as f32, y - iy as f32);
        self.mesh[iy * self.nx + ix]
            .scaled((1.0 - fx) * (1.0 - fy))
            .plus(self.mesh[iy * self.nx + ix + 1].scaled(fx * (1.0 - fy)))
            .plus(self.mesh[(iy + 1) * self.nx + ix].scaled((1.0 - fx) * fy))
            .plus(self.mesh[(iy + 1) * self.nx + ix + 1].scaled(fx * fy))
    }

    /// Export the two passes and source locations sampled by the lattice nodes.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn save_debug(&self, source: &PhotonImage, directory: &str) -> Result<(), String> {
        let directory = std::path::Path::new(directory);
        std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
        let rgba = |p: &PhotonImage| {
            image::RgbaImage::from_raw(p.get_width(), p.get_height(), p.get_raw_pixels()).unwrap()
        };
        rgba(source)
            .save(directory.join("input.png"))
            .map_err(|e| e.to_string())?;
        let mut overlay = rgba(&self.image);
        overlay
            .save(directory.join("warped.png"))
            .map_err(|e| e.to_string())?;
        for y in 0..self.ny {
            for x in 0..self.nx {
                let target = (
                    (x as f32 * self.step.x - 0.5).max(0.0),
                    (y as f32 * self.step.y - 0.5).max(0.0),
                );
                let u = self.mesh[y * self.nx + x];
                let source = (target.0 + u.x * self.step.x, target.1 + u.y * self.step.y);
                imageproc::drawing::draw_line_segment_mut(
                    &mut overlay,
                    target,
                    source,
                    image::Rgba([255, 80, 60, 255]),
                );
                imageproc::drawing::draw_cross_mut(
                    &mut overlay,
                    image::Rgba([40, 210, 240, 255]),
                    target.0.round() as i32,
                    target.1.round() as i32,
                );
            }
        }
        overlay
            .save(directory.join("lattice.png"))
            .map_err(|e| e.to_string())?;
        std::fs::write(directory.join("stats.txt"), format!("{:#?}\n", self.stats))
            .map_err(|e| e.to_string())
    }
}

/// `image` must already have its final size: this is a geometry pass immediately
/// before glyph matching, after crop/scale/resize and before palette conversion.
pub fn warp(image: PhotonImage, cell: (usize, usize), strength: u32) -> WarpResult {
    let (w, h) = (image.get_width() as usize, image.get_height() as usize);
    let (gw, gh) = cell;
    let mut result = WarpResult {
        image,
        stats: WarpStats {
            min_jacobian: 1.0,
            safety_scale: 1.0,
            ..Default::default()
        },
        mesh: Vec::new(),
        nx: 0,
        ny: 0,
        step: Vec2::default(),
    };
    if strength == 0 || gw == 0 || gh == 0 || w < gw || h < gh {
        return result;
    }
    let (nx, ny) = (w / gw * 2 + 1, h / gh * 3 + 1);
    // Bound analysis storage for unusually large requested grids. The normal
    // rendering path still works unchanged if this experimental pass is skipped.
    if (nx - 1)
        .saturating_mul(ny - 1)
        .saturating_mul(SAMPLES * SAMPLES)
        > 4_000_000
    {
        log::warn!("Superpixel warp skipped: analysis would exceed four million pixels");
        return result;
    }
    let constraints = anchors(&result.image, nx, ny);
    result.stats.anchors = constraints.iter().filter(|a| a.weight > 0.0).count();
    if result.stats.anchors == 0 {
        return result;
    }
    result.mesh = solve(&constraints, nx, ny);
    result.nx = nx;
    result.ny = ny;
    result.step = Vec2 {
        x: gw as f32 / 2.0,
        y: gh as f32 / 3.0,
    };
    let amount = strength.min(MAX_STRENGTH) as f32 / 100.0;
    for u in &mut result.mesh {
        *u = u.scaled(amount);
    }
    result.stats.safety_scale = protect_mesh(&mut result.mesh, nx, ny, MIN_JACOBIAN);
    result.stats.min_jacobian = min_jacobian(&result.mesh, nx, ny);
    result.stats.max_displacement = result.mesh.iter().map(|u| u.length()).fold(0.0, f32::max);
    if result.stats.max_displacement < 1e-5 {
        return result;
    }
    let source = result.image.get_raw_pixels();
    let mut pixels = vec![0u8; source.len()];
    for y in 0..h {
        for x in 0..w {
            let u = result.displacement(x as f32 + 0.5, y as f32 + 0.5);
            let sx = (x as f32 + u.x * result.step.x).clamp(0.0, (w - 1) as f32);
            let sy = (y as f32 + u.y * result.step.y).clamp(0.0, (h - 1) as f32);
            let (ix, iy) = (sx as usize, sy as usize);
            let (fx, fy) = (sx - ix as f32, sy - iy as f32);
            let mut sum = [0.0; 4];
            for (px, py, weight) in [
                (ix, iy, (1.0 - fx) * (1.0 - fy)),
                ((ix + 1).min(w - 1), iy, fx * (1.0 - fy)),
                (ix, (iy + 1).min(h - 1), (1.0 - fx) * fy),
                ((ix + 1).min(w - 1), (iy + 1).min(h - 1), fx * fy),
            ] {
                let p = &source[(py * w + px) * 4..][..4];
                let alpha = p[3] as f32 / 255.0;
                for c in 0..3 {
                    sum[c] += p[c] as f32 * alpha * weight;
                }
                sum[3] += alpha * weight;
            }
            let p = &mut pixels[(y * w + x) * 4..][..4];
            if sum[3] > 1e-6 {
                for c in 0..3 {
                    p[c] = (sum[c] / sum[3]).round().clamp(0.0, 255.0) as u8;
                }
            }
            p[3] = (sum[3] * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    result.image = PhotonImage::new(pixels, w as u32, h as u32);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vertical_edge(width: u32, height: u32, edge: u32) -> PhotonImage {
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        for y in 0..height as usize {
            for x in 0..width as usize {
                let p = &mut pixels[(y * width as usize + x) * 4..][..4];
                let value = if x >= edge as usize { 255 } else { 0 };
                p[..3].fill(value);
                p[3] = 255;
            }
        }
        PhotonImage::new(pixels, width, height)
    }

    #[test]
    fn disabled_warp_is_bit_exact_and_has_no_mesh() {
        let input = vertical_edge(32, 16, 5);
        let original = input.get_raw_pixels();
        let result = warp(input, (8, 8), 0);
        assert_eq!(result.image.get_raw_pixels(), original);
        assert_eq!(result.stats.anchors, 0);
        assert_eq!(result.stats.max_displacement, 0.0);
    }

    #[test]
    fn warp_preserves_dimensions_and_keeps_mesh_valid() {
        let result = warp(vertical_edge(32, 16, 5), (8, 8), 100);
        assert_eq!(
            (result.image.get_width(), result.image.get_height()),
            (32, 16)
        );
        assert!(result.stats.min_jacobian >= MIN_JACOBIAN - 1e-5);
        assert!(result.stats.max_displacement <= MAX_MOVE + 1e-5);
        assert!(result.stats.safety_scale <= 1.0);
    }

    #[test]
    fn a_boundary_near_a_lattice_line_moves_toward_that_line() {
        let result = warp(vertical_edge(32, 16, 5), (8, 8), 100);
        assert!(
            result.stats.anchors > 0,
            "the synthetic edge should produce an anchor"
        );
        // The boundary starts one source pixel to the right of the first
        // half-cell line. A positive inverse lookup at that line means the
        // resampler is pulling it toward x=4.
        let line = result.displacement(4.5, 8.0);
        assert!(
            line.x > 0.0,
            "expected a positive source lookup at the lattice line"
        );
    }

    #[test]
    fn a_boundary_halfway_between_lines_is_captured_at_full_strength() {
        // Half-cell lines are four pixels apart. The old 0.45 cutoff missed
        // this edge entirely; the nearest line is two pixels away at x=8.
        let result = warp(vertical_edge(32, 24, 6), (8, 12), 100);
        let line = result.displacement(8.0, 12.0);
        assert!(result.stats.anchors > 0);
        assert!(line.x < -0.45, "expected nearly two pixels of alignment");
        assert!(result.stats.min_jacobian >= MIN_JACOBIAN - 1e-5);

        let gentle = warp(vertical_edge(32, 24, 6), (8, 12), 50);
        assert!(line.x.abs() > gentle.displacement(8.0, 12.0).x.abs() * 1.9);
    }

    #[test]
    fn protecting_a_crowded_region_preserves_distant_alignment() {
        let (nx, ny) = (12, 5);
        let mut mesh = vec![Vec2::default(); nx * ny];
        // Two adjacent nodes pull past one another. A separate edge has a
        // safe displacement that must not be weakened by this conflict.
        mesh[2 * nx + 2].x = 0.6;
        mesh[2 * nx + 3].x = -0.6;
        mesh[2 * nx + 9].x = 0.4;
        assert!(min_jacobian(&mesh, nx, ny) < MIN_JACOBIAN);

        let scale = protect_mesh(&mut mesh, nx, ny, MIN_JACOBIAN);
        assert!(scale < 1.0);
        assert!(min_jacobian(&mesh, nx, ny) >= MIN_JACOBIAN - 1e-5);
        assert_eq!(mesh[2 * nx + 9].x, 0.4);
    }

    #[test]
    fn obsolete_overdrive_values_are_clamped_to_full_alignment() {
        let normal = warp(vertical_edge(64, 24, 17), (8, 12), 100);
        let clamped = warp(vertical_edge(64, 24, 17), (8, 12), 500);
        assert_eq!(clamped.image.get_raw_pixels(), normal.image.get_raw_pixels());
        assert!(clamped.stats.min_jacobian >= MIN_JACOBIAN - 1e-5);
    }
}
