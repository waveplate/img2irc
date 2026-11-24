#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ResizeEdges {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
}

/// The whole interior moves the box. Only the generous outside ring resizes it.
pub(super) fn resize_edges([x, y, w, h]: [i32; 4], [px, py]: [i32; 2], margin: i32) -> Option<ResizeEdges> {
    if (px >= x && px < x + w && py >= y && py < y + h)
        || px < x - margin || px >= x + w + margin || py < y - margin || py >= y + h + margin {
        return None;
    }
    Some(ResizeEdges { left: px < x, right: px >= x + w, top: py < y, bottom: py >= y + h })
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ResizeDrag {
    pub rect: [i32; 4],
    pub start: [i32; 2],
    pub edges: ResizeEdges,
}

impl ResizeDrag {
    pub fn rect_at(self, [px, py]: [i32; 2]) -> [i32; 4] {
        let [x, y, w, h] = self.rect;
        let dx = px - self.start[0];
        let dy = py - self.start[1];
        let left = if self.edges.left { (x + dx).min(x + w - 1) } else { x };
        let top = if self.edges.top { (y + dy).min(y + h - 1) } else { y };
        let right = if self.edges.right { (x + w + dx).max(left + 1) } else { x + w };
        let bottom = if self.edges.bottom { (y + h + dy).max(top + 1) } else { y + h };
        [left, top, right - left, bottom - top]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_interior_cell_moves_and_all_outside_edges_and_corners_resize() {
        let rect = [10, 20, 6, 4];
        for y in 20..24 { for x in 10..16 { assert_eq!(resize_edges(rect, [x, y], 2), None); } }
        for (point, expected) in [
            ([8, 21], [true, false, false, false]), ([17, 21], [false, true, false, false]),
            ([12, 18], [false, false, true, false]), ([12, 25], [false, false, false, true]),
            ([8, 18], [true, false, true, false]), ([17, 18], [false, true, true, false]),
            ([8, 25], [true, false, false, true]), ([17, 25], [false, true, false, true]),
        ] {
            let edges = resize_edges(rect, point, 2).unwrap();
            assert_eq!([edges.left, edges.right, edges.top, edges.bottom], expected);
            assert_eq!(ResizeDrag { rect, start: point, edges }.rect_at(point), rect);
        }
        assert_eq!(resize_edges(rect, [7, 20], 2), None);
    }

    #[test]
    fn dragging_left_and_top_keeps_the_opposite_edges_fixed_and_never_inverts() {
        let drag = ResizeDrag { rect: [10, 20, 6, 4], start: [9, 19], edges: ResizeEdges { left: true, top: true, ..ResizeEdges::default() } };
        assert_eq!(drag.rect_at([6, 17]), [7, 18, 9, 6]);
        assert_eq!(drag.rect_at([30, 40]), [15, 23, 1, 1]);
    }
}
