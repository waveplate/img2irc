import test from "node:test";
import assert from "node:assert/strict";

import { quantizeImageData, quantizeRgb } from "../web/js/output-palettes.js";

test("browser eyedropper quantizes IRC and ANSI but not 24-bit RGB", () => {
  assert.deepEqual(quantizeRgb([250, 8, 8], "ansi"), [255, 0, 0]);
  assert.deepEqual(quantizeRgb([2, 3, 4], "irc"), [0, 0, 0]);
  assert.deepEqual(quantizeRgb([2, 3, 4], "ansi24"), [2, 3, 4]);
});

test("image quantization preserves alpha and transparent RGB", () => {
  const image = {
    data: new Uint8ClampedArray([250, 8, 8, 127, 1, 2, 3, 0]),
  };
  quantizeImageData(image, "ansi");
  assert.deepEqual([...image.data], [255, 0, 0, 127, 1, 2, 3, 0]);
});
