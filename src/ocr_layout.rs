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
                    && gap <= 1.5 * h)
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

#[cfg(test)]
mod tests {
    use super::*;

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
