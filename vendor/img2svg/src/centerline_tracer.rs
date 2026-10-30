//! Centerline tracing via morphological skeletonization (line art mode).

use crate::vectorizer::Point;

/// Extract open centerline polylines from a binary mask (true = foreground).
pub fn trace_centerlines(mask: &[bool], width: usize, height: usize) -> Vec<Vec<Point>> {
    let skel = skeletonize(mask, width, height);
    let mut used = vec![false; skel.len()];
    let mut paths = Vec::new();

    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            if !skel[idx] || used[idx] {
                continue;
            }
            let mut path = vec![Point {
                x: x as f64 + 0.5,
                y: y as f64 + 0.5,
            }];
            used[idx] = true;
            let mut cx = x;
            let mut cy = y;
            for _ in 0..(width * height) {
                let mut next = None;
                for (dx, dy) in [
                    (-1, -1),
                    (0, -1),
                    (1, -1),
                    (-1, 0),
                    (1, 0),
                    (-1, 1),
                    (0, 1),
                    (1, 1),
                ] {
                    let nx = cx as i32 + dx;
                    let ny = cy as i32 + dy;
                    if nx < 0 || ny < 0 || nx >= width as i32 || ny >= height as i32 {
                        continue;
                    }
                    let ni = ny as usize * width + nx as usize;
                    if skel[ni] && !used[ni] {
                        next = Some((nx as usize, ny as usize));
                        break;
                    }
                }
                let Some((nx, ny)) = next else { break };
                used[ny * width + nx] = true;
                path.push(Point {
                    x: nx as f64 + 0.5,
                    y: ny as f64 + 0.5,
                });
                cx = nx;
                cy = ny;
            }
            if path.len() >= 2 {
                paths.push(path);
            }
        }
    }

    paths
}

fn skeletonize(mask: &[bool], width: usize, height: usize) -> Vec<bool> {
    let mut img = mask.to_vec();
    let mut changed = true;
    while changed {
        changed = false;
        let mut to_remove = Vec::new();
        for y in 1..height - 1 {
            for x in 1..width - 1 {
                let idx = y * width + x;
                if !img[idx] {
                    continue;
                }
                let p2 = img[(y - 1) * width + x] as u8;
                let p3 = img[(y - 1) * width + x + 1] as u8;
                let p4 = img[y * width + x + 1] as u8;
                let p5 = img[(y + 1) * width + x + 1] as u8;
                let p6 = img[(y + 1) * width + x] as u8;
                let p7 = img[(y + 1) * width + x - 1] as u8;
                let p8 = img[y * width + x - 1] as u8;
                let p9 = img[(y - 1) * width + x - 1] as u8;
                let b = p2 + p3 + p4 + p5 + p6 + p7 + p8 + p9;
                if b < 2 || b > 6 {
                    continue;
                }
                let a = transitions(p2, p3, p4, p5, p6, p7, p8, p9);
                if a != 1 {
                    continue;
                }
                if p2 * p4 * p6 == 0 && p4 * p6 * p8 == 0 {
                    to_remove.push(idx);
                }
            }
        }
        for idx in to_remove {
            if img[idx] {
                img[idx] = false;
                changed = true;
            }
        }
    }
    img
}

fn transitions(p2: u8, p3: u8, p4: u8, p5: u8, p6: u8, p7: u8, p8: u8, p9: u8) -> u32 {
    let seq = [p2, p3, p4, p5, p6, p7, p8, p9, p2];
    let mut count = 0;
    for w in seq.windows(2) {
        if w[0] == 0 && w[1] == 1 {
            count += 1;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centerline_horizontal_line() {
        let w = 20;
        let h = 10;
        let mut mask = vec![false; w * h];
        for x in 2..18 {
            mask[5 * w + x] = true;
        }
        let paths = trace_centerlines(&mask, w, h);
        assert!(!paths.is_empty());
        assert!(paths[0].len() >= 2);
    }
}
