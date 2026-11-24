import test from "node:test";
import assert from "node:assert/strict";
import { resizeDitherRaster } from "../web/js/dither-preview.js";

const background = 0xff000000;
const foreground = 0xffffffff;
const scaledText = 0xff777777;

function fixture(character, period) {
  const cellWidth = 9;
  const cellHeight = 15;
  const columns = 2;
  const rows = 2;
  const source = { width: columns * cellWidth, height: rows * cellHeight };
  const pixels = new Uint32Array(source.width * source.height);
  for (let y = 0; y < source.height; y++) {
    for (let x = 0; x < source.width; x++) {
      pixels[y * source.width + x] = (x % cellWidth + y % cellHeight) % period === 0
        ? foreground : background;
    }
  }
  const cells = Array.from({ length: rows }, () => Array.from({ length: columns }, () => ({ character })));
  const result = { cellWidth, cellHeight, columns, rows, cells,
    previewRgba: new Uint8Array(pixels.buffer), previewStipples: [[character, period]] };
  return { source, result };
}

function mockCanvas() {
  const ctx = {
    drawImage() {},
    getImageData(_x, _y, w, h) {
      const pixels = new Uint32Array(w * h).fill(scaledText);
      return { data: new Uint8ClampedArray(pixels.buffer) };
    },
    putImageData(raster) { this.pixels = new Uint32Array(raster.data.buffer); },
  };
  return { ctx, getContext() { return ctx; } };
}

test("fractional Dither scaling keeps the stipple continuous across odd cell boundaries", () => {
  for (const [character, period] of [["▒", 2], ["░", 4]]) {
    const { source, result } = fixture(character, period);
    for (const [width, height] of [[16, 27], [19, 32]]) {
      const canvas = mockCanvas();
      resizeDitherRaster(canvas, source, result, width, height);
      for (let y = 0; y < height; y++) {
        for (let x = 0; x < width; x++) {
          assert.equal(canvas.ctx.pixels[y * width + x], (x + y) % period === 0 ? foreground : background);
        }
      }
    }
  }
});

test("styled text keeps the filtered raster and whole-pixel enlargement skips resampling the stipple", () => {
  const { source, result } = fixture("▒", 2);
  result.previewTextCells = [1];
  const canvas = mockCanvas();
  resizeDitherRaster(canvas, source, result, 16, 27);
  assert.equal(canvas.ctx.pixels[7], background);
  assert.equal(canvas.ctx.pixels[8], scaledText);

  const enlarged = mockCanvas();
  resizeDitherRaster(enlarged, source, result, source.width * 2, source.height * 2);
  assert.equal(enlarged.ctx.imageSmoothingEnabled, false);
  assert.equal(enlarged.ctx.pixels, undefined);
});
