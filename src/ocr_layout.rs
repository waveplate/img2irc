//! Group horizontal OCR word boxes before fitting a complete text line.
//! Keep these thresholds in sync with groupOcrItems in web/js/ocr-service.js.

pub fn group_boxes(boxes: &[[f64; 4]]) -> Vec<Vec<usize>> {
    fn compatible(a: [f64; 4], b: [f64; 4]) -> bool {
        let ah = a[3] - a[1];
        let bh = b[3] - b[1];
        let height = ah.min(bh);
        height > 0.0
            && height >= ah.max(bh) * 0.75
            && ((a[1] + a[3]) - (b[1] + b[3])).abs() <= height * 0.5
    }

    let mut order: Vec<usize> = (0..boxes.len()).collect();
    order.sort_by(|&a, &b| {
        boxes[a][0]
            .total_cmp(&boxes[b][0])
            .then_with(|| boxes[a][1].total_cmp(&boxes[b][1]))
            .then_with(|| a.cmp(&b))
    });
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for index in order {
        let b = boxes[index];
        let candidate = groups
            .iter()
            .enumerate()
            .filter_map(|(group_index, group)| {
                let a = boxes[*group.last()?];
                let gap = b[0] - a[2];
                let height = (a[3] - a[1]).min(b[3] - b[1]);
                // Comparing against the first word prevents a chain of slightly
                // offset boxes from drifting into another line.
                (compatible(a, b)
                    && compatible(boxes[group[0]], b)
                    && b[2] > a[2]
                    // Models pad word boxes differently. Permit boundary
                    // overlap, but never consume most of a short word's box.
                    && gap >= -(0.35 * height).min(0.25 * (a[2] - a[0]).min(b[2] - b[0]))
                    && gap <= 0.75 * height)
                    .then_some((group_index, gap.max(0.0) / height))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((group_index, _)) = candidate {
            groups[group_index].push(index);
        } else {
            groups.push(vec![index]);
        }
    }
    groups
}

/// Stack nearby, similarly sized and aligned lines into an editable block.
pub fn group_blocks(boxes: &[[f64; 4]]) -> Vec<Vec<Vec<usize>>> {
    fn bounds(line: &[usize], boxes: &[[f64; 4]]) -> [f64; 4] {
        line.iter().fold(
            [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ],
            |a, &i| {
                [
                    a[0].min(boxes[i][0]),
                    a[1].min(boxes[i][1]),
                    a[2].max(boxes[i][2]),
                    a[3].max(boxes[i][3]),
                ]
            },
        )
    }
    fn line_height(line: &[usize], boxes: &[[f64; 4]]) -> f64 {
        let mut heights = line
            .iter()
            .map(|&index| boxes[index][3] - boxes[index][1])
            .collect::<Vec<_>>();
        heights.sort_by(f64::total_cmp);
        heights.get(heights.len() / 2).copied().unwrap_or(0.0)
    }
    fn aligned(a: [f64; 4], ah: f64, b: [f64; 4], bh: f64) -> bool {
        let h = ah.min(bh);
        let overlap = a[2].min(b[2]) - a[0].max(b[0]);
        h > 0.0
            && h >= ah.max(bh) * 0.6
            && overlap >= (a[2] - a[0]).min(b[2] - b[0]) * 0.5
            && ((a[0] - b[0]).abs() <= h * 0.25
                || (a[2] - b[2]).abs() <= h * 0.25
                || ((a[0] + a[2]) - (b[0] + b[2])).abs() <= h * 0.5)
    }
    let mut lines = group_boxes(boxes);
    lines.sort_by(|a, b| {
        let a = bounds(a, boxes);
        let b = bounds(b, boxes);
        a[1].total_cmp(&b[1]).then_with(|| a[0].total_cmp(&b[0]))
    });
    let mut blocks: Vec<Vec<Vec<usize>>> = Vec::new();
    for line in lines {
        let b = bounds(&line, boxes);
        let bh = line_height(&line, boxes);
        let candidate = blocks
            .iter()
            .enumerate()
            .filter_map(|(index, block)| {
                let previous = block.last()?;
                let a = bounds(previous, boxes);
                let ah = line_height(previous, boxes);
                let first = &block[0];
                let gap = b[1] - a[3];
                let mut block_heights = block
                    .iter()
                    .map(|member| line_height(member, boxes))
                    .collect::<Vec<_>>();
                block_heights.sort_by(f64::total_cmp);
                let h = block_heights[block_heights.len() / 2];
                (aligned(a, ah, b, bh)
                    && aligned(bounds(first, boxes), line_height(first, boxes), b, bh)
                    && gap >= -0.2 * h
                    && gap <= h
                    // Compare every pitch to the first. Pairwise comparison
                    // alone allows a paragraph break to grow gradually.
                    && (block.len() < 2 || {
                        let first_box = bounds(first, boxes);
                        let second = bounds(&block[1], boxes);
                        let pitch = (second[1] + second[3] - first_box[1] - first_box[3]) * 0.5;
                        let next = (b[1] + b[3] - a[1] - a[3]) * 0.5;
                        (next - pitch).abs() <= h * 0.3
                    }))
                .then_some((index, gap.max(0.0) / h.max(f64::EPSILON)))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((index, _)) = candidate {
            blocks[index].push(line);
        } else {
            blocks.push(vec![line]);
        }
    }
    blocks
}

/// Place already trimmed line art in output rows. The first and last ink rows
/// span the source union; blank font margins do not count toward that height.
/// Absolute positions avoid accumulating rounding error over many lines.
pub fn space_line_art<T: Clone>(lines: &[Vec<Vec<T>>], bounds: &[[f64; 2]]) -> Option<Vec<Vec<T>>> {
    if lines.len() < 2
        || lines.len() != bounds.len()
        || lines.iter().any(Vec::is_empty)
        || bounds.iter().flatten().any(|v| !v.is_finite())
    {
        return None;
    }
    let origin = bounds[0][0];
    let span = (bounds.last()?[1] - origin).round().max(1.0) as usize;
    let required: usize = lines.iter().map(Vec::len).sum();
    if required > span || span > 65536 {
        return None;
    }
    let mut rows = Vec::with_capacity(span);
    let mut remaining = required;
    for (index, line) in lines.iter().enumerate() {
        let desired = if index == 0 {
            0
        } else if index + 1 == lines.len() {
            span - line.len()
        } else {
            ((bounds[index][0] + bounds[index][1]) * 0.5 - origin - line.len() as f64 * 0.5)
                .round()
                .max(0.0) as usize
        };
        let start = desired.clamp(rows.len(), span - remaining);
        if start.abs_diff(desired) > 1 {
            return None;
        }
        rows.resize_with(start, Vec::new);
        rows.extend_from_slice(line);
        remaining -= line.len();
    }
    Some(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_rows_span_source_bounds_without_cumulative_rounding() {
        let bounds = [[0.0, 2.0], [3.6, 5.6], [7.2, 9.2]];
        for line in [vec![vec!['X']], vec![vec!['X'], vec!['X']]] {
            let rows = space_line_art(&vec![line.clone(); 3], &bounds).unwrap();
            assert_eq!(rows.len(), 9);
            assert!(!rows[0].is_empty());
            assert!(!rows[8].is_empty());
            assert_eq!(
                rows.iter().filter(|row| !row.is_empty()).count(),
                line.len() * 3
            );
        }
        assert!(space_line_art(&vec![vec![vec!['X']; 4]; 3], &bounds).is_none());
    }

    #[test]
    fn shared_ocr_line_cases() {
        let cases: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/ocr-lines.json")).unwrap();
        for case in cases.as_array().unwrap() {
            let boxes: Vec<[f64; 4]> = serde_json::from_value(case["boxes"].clone()).unwrap();
            let expected: Vec<Vec<usize>> = serde_json::from_value(case["groups"].clone()).unwrap();
            assert_eq!(group_boxes(&boxes), expected, "{}", case["name"]);
            if let Some(blocks) = case.get("blocks") {
                let expected: Vec<Vec<Vec<usize>>> =
                    serde_json::from_value(blocks.clone()).unwrap();
                assert_eq!(group_blocks(&boxes), expected, "{}", case["name"]);
            }
        }
    }
}
