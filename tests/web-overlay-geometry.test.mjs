import test from "node:test";
import assert from "node:assert/strict";
import { resizeEdges, resizeBox } from "../web/js/overlay-geometry.js";

test("the entire interior moves; the generous outside ring selects any edge or corner", () => {
  const box = { x: 100, y: 200, width: 80, height: 40 };
  for (const x of [100, 101, 140, 179]) {
    for (const y of [200, 201, 220, 239]) assert.equal(resizeEdges(box, x, y), undefined);
  }
  for (const [x, y, edges] of [
    [90, 220, [true, false, false, false]], [190, 220, [false, true, false, false]],
    [140, 190, [false, false, true, false]], [140, 250, [false, false, false, true]],
    [90, 190, [true, false, true, false]], [190, 190, [false, true, true, false]],
    [90, 250, [true, false, false, true]], [190, 250, [false, true, false, true]],
  ]) {
    const hit = resizeEdges(box, x, y);
    assert.deepEqual([hit.left, hit.right, hit.top, hit.bottom], edges);
    assert.deepEqual(resizeBox(box, hit, 0, 0), box);
  }
  assert.equal(resizeEdges(box, 85, 220), undefined);
});

test("left and top resizing anchor the opposite sides and clamp at one cell", () => {
  const box = { x: 10, y: 20, width: 8, height: 4 };
  const edges = { left: true, top: true };
  assert.deepEqual(resizeBox(box, edges, -2, -1), { x: 8, y: 19, width: 10, height: 5 });
  assert.deepEqual(resizeBox(box, edges, 50, 50), { x: 17, y: 23, width: 1, height: 1 });
});
