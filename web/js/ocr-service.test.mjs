import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync } from "node:fs";
import { eraseRegions, estimateBackground, groupOcrItems, groupOcrBlocks, ocrFitWidth, ocrItemsToOverlays, scaleOcrItems } from "./ocr-service.js";

const bounds = (x0, y0, x1, y1) => ({ x0, y0, x1, y1, width: x1 - x0, height: y1 - y0 });
const put = (raw, width, x, y, pixel) => raw.set(pixel, (y * width + x) * 4);

test("auto width fits the tightest box and produces complete overlays", async () => {
  const prepared = {
    result: { image: { width: 1000, height: 300 } },
    items: [
      { text: "Twenty characters!!!", bounds: bounds(100, 20, 200, 40) },
      { text: "Short", bounds: bounds(600, 100, 800, 120) },
    ],
  };
  assert.deepEqual(ocrFitWidth(prepared, 40), {
    width: 200, required: 200, coverage: 100, targetCoverage: 100,
  });
  assert.equal(ocrFitWidth(prepared, 300).width, 300);
  const mapped = await ocrItemsToOverlays(prepared, 200, 30, { enabled: false }, async (requests) => ({
    arts: requests.map((request) => ({
      source: "plain", text: request.text, width: [...request.text].length, height: 1,
    })),
    fonts: {},
  }));
  assert.equal(mapped.overlays.length, 2);
  assert.equal(mapped.overlays[0].renderedText, prepared.items[0].text);
});

test("auto width handles empty results, fractional ratios, and limits", () => {
  assert.equal(ocrFitWidth({ items: [] }), undefined);
  const prepared = {
    result: { image: { width: 101, height: 20 } },
    items: [{ text: "Hello", bounds: bounds(0, 0, 3, 10) }],
  };
  assert.deepEqual(ocrFitWidth(prepared, 1), {
    width: 169, required: 169, coverage: 100, targetCoverage: 100,
  });
  assert.deepEqual(ocrFitWidth(prepared, 1, 100), {
    width: 100, required: 169, coverage: 0, targetCoverage: 100,
  });
});

test("auto width can target a percentage of completely fitted text", () => {
  const prepared = {
    result: { image: { width: 100, height: 40 } },
    items: [
      { text: "A".repeat(90), bounds: bounds(0, 0, 90, 10) },
      { text: "B".repeat(10), bounds: bounds(0, 20, 1, 30) },
    ],
  };
  assert.deepEqual(ocrFitWidth(prepared, 1, 4096, 90), {
    width: 100, required: 1000, coverage: 90, targetCoverage: 90,
  });
  assert.equal(ocrFitWidth(prepared, 1, 4096, 91).width, 1000);
});

test("locked OCR regions scale to the currently transformed image", () => {
  const items = [{ text: "LOCKED", bounds: bounds(10, 20, 40, 50) }];
  const scaled = scaleOcrItems(items, { width: 100, height: 100 }, 200, 50);
  assert.deepEqual(scaled[0].bounds, bounds(20, 10, 80, 25));
  assert.deepEqual(items[0].bounds, bounds(10, 20, 40, 50));
});

test("background follows the inner panel with dense dark or light text", () => {
  for (const [surface, ink] of [
    [[245, 240, 230, 255], [0, 0, 0, 255]],
    [[15, 20, 25, 255], [255, 255, 255, 255]],
  ]) {
    const raw = new Uint8ClampedArray(Array(60 * 20).fill([0, 0, 0, 255]).flat());
    const box = bounds(10, 5, 50, 15);
    for (let y = 5; y < 15; y += 1) {
      for (let x = 10; x < 50; x += 1) {
        put(raw, 60, x, y, y > 5 && y < 14 && x > 10 && x < 49 ? ink : surface);
      }
    }
    assert.deepEqual(estimateBackground(raw, 60, 20, box), surface);
    const imageData = { data: raw.slice() };
    const items = eraseRegions(imageData, 60, 20, [{ text: "TEXT", bounds: box }]);
    assert.deepEqual([...imageData.data.slice((10 * 60 + 30) * 4, (10 * 60 + 30) * 4 + 4)], surface);
    assert.deepEqual([...imageData.data.slice((10 * 60 + 9) * 4, (10 * 60 + 9) * 4 + 4)], [0, 0, 0, 255]);
    assert.deepEqual(items[0].foreground, ink.slice(0, 3));
  }
});

test("edge-to-edge boxes use their own pixels and skip fully transparent regions", () => {
  const surface = [240, 230, 210, 255];
  const raw = new Uint8ClampedArray(Array(200).fill(surface).flat());
  put(raw, 20, 0, 0, [0, 0, 0, 255]);
  assert.deepEqual(estimateBackground(raw, 20, 10, bounds(0, 0, 20, 10)), surface);
  assert.equal(estimateBackground(new Uint8ClampedArray(800), 20, 10, bounds(0, 0, 20, 10)), undefined);
});

test("nearby background shades beat a uniform black text cluster", () => {
  const raw = new Uint8ClampedArray(60 * 20 * 4);
  for (let y = 0; y < 20; y += 1) {
    for (let x = 0; x < 60; x += 1) {
      const gray = y > 5 && y < 15 && x % 3 === 0 ? 0 : 190 + x % 40;
      put(raw, 60, x, y, [gray, gray, gray, 255]);
    }
  }
  const bg = estimateBackground(raw, 60, 20, bounds(0, 0, 60, 20));
  assert.ok(bg[0] >= 190 && bg[0] <= 229);
});


test("FIGlet fill uses allowed expansion while respecting neighbors and canvas bounds", async () => {
  const prepared = {
    result: { image: { width: 100, height: 100 } },
    items: [
      { text: "A", bounds: bounds(10, 10, 30, 30) },
      { text: "B", bounds: bounds(35, 18, 55, 26) },
      { text: "C", bounds: bounds(20, 35, 40, 55) },
      { text: "D", bounds: bounds(90, 90, 100, 100) },
    ],
  };
  let requests;
  const render = async (input) => { requests = input; return { arts: [] }; };
  const settings = { enabled: true, minHeight: 1, maxWidthRatio: 1, maxHeightRatio: 1, fillAvailable: true };
  await ocrItemsToOverlays(prepared, 100, 100, settings, render);
  assert.equal(requests[0].fillAvailable, true);
  assert.equal(requests[0].baseWidth, 20);
  assert.equal(requests[0].baseHeight, 20);
  assert.equal(requests[0].maxWidth, 25);
  assert.equal(requests[0].maxHeight, 25);
  assert.equal(requests[3].maxWidth, 10);
  assert.equal(requests[3].maxHeight, 10);
  await ocrItemsToOverlays(prepared, 100, 100, { ...settings, fillAvailable: false }, render);
  assert.equal(requests[0].fillAvailable, false);
});

test("OCR line grouping matches the native geometry cases", () => {
  const cases = JSON.parse(readFileSync(new URL("../../tests/fixtures/ocr-lines.json", import.meta.url)));
  for (const { name, boxes, groups, blocks } of cases) {
    const items = boxes.map((box, index) => ({ text: String(index), bounds: bounds(...box) }));
    assert.deepEqual(groupOcrItems(items).map((item) => item.text.split(" ").map(Number)), groups, name);
    if (blocks) {
      assert.deepEqual(groupOcrBlocks(items).map((item) => item.text.split("\n").map((line) => line.split(" ").map(Number))), blocks, name);
    }
  }
});

test("SIGNAL ONE is fitted once as a complete title in either detector order", async () => {
  const words = [
    { text: "SIGNAL", score: 0.9986, foreground: [254, 254, 254], bounds: bounds(145, 2069, 1088, 2409) },
    { text: "ONE", score: 0.9823, foreground: [250, 250, 250], bounds: bounds(1188, 2077, 1768, 2396) },
  ];
  for (const items of [words, [...words].reverse()]) {
    const prepared = { result: { image: { width: 1920, height: 2468 } }, items };
    const mapped = await ocrItemsToOverlays(prepared, 80, 50, { enabled: true, minHeight: 2 }, async (requests) => {
      assert.equal(requests.length, 1);
      assert.equal(requests[0].text, "SIGNAL ONE");
      return { arts: [{ text: "SIGNAL ONE", width: 10, height: 1, source: "plain" }] };
    });
    assert.equal(mapped.overlays.length, 1);
    assert.equal(mapped.overlays[0].text, "SIGNAL ONE");
    assert.deepEqual(mapped.overlays[0].foreground, [254, 254, 254]);
    assert.equal(mapped.overlays[0].confidence, 0.9823);
  }
});

test("auto width includes word separators and grouped overlay text retains them", async () => {
  const prepared = {
    result: { image: { width: 100, height: 30 } },
    items: [
      { text: "ONE", bounds: bounds(10, 5, 25, 15) },
      { text: "TWO", bounds: bounds(26, 5, 41, 15) },
      { text: "THREE", bounds: bounds(42, 5, 67, 15) },
    ],
  };
  const { width } = ocrFitWidth(prepared, 1);
  assert.equal(width, 23);
  const mapped = await ocrItemsToOverlays(prepared, width, 10, { enabled: false }, async (requests) => {
    assert.equal(requests.length, 1);
    assert.ok(requests[0].maxWidth >= 13);
    return { arts: [{ text: requests[0].text, width: 13, height: 1, source: "plain" }] };
  });
  assert.equal(mapped.overlays[0].renderedText, "ONE TWO THREE");
});

test("merging layout boxes does not erase pixels in the gap", () => {
  const raw = new Uint8ClampedArray(Array(100 * 20).fill([255, 255, 255, 255]).flat());
  put(raw, 100, 40, 10, [200, 0, 0, 255]);
  const items = [
    { text: "ONE", bounds: bounds(10, 5, 38, 15) },
    { text: "TWO", bounds: bounds(43, 5, 70, 15) },
  ];
  const analyzed = eraseRegions({ data: raw }, 100, 20, items);
  assert.equal(groupOcrItems(analyzed).length, 1);
  assert.deepEqual([...raw.slice((10 * 100 + 40) * 4, (10 * 100 + 40) * 4 + 4)], [200, 0, 0, 255]);
});

test("related lines become one multiline overlay with a shared font request", async () => {
  const prepared = {
    result: { image: { width: 100, height: 100 } },
    items: [
      { text: "FIRST LINE", bounds: bounds(10, 10, 80, 20) },
      { text: "SECOND LINE", bounds: bounds(10, 20, 80, 30) },
    ],
  };
  const mapped = await ocrItemsToOverlays(prepared, 100, 10, { enabled: true, minHeight: 2 }, async (requests) => {
    assert.equal(requests.length, 1);
    assert.equal(requests[0].text, "FIRST LINE\nSECOND LINE");
    return { arts: [{ text: "AA\nBB", width: 2, height: 2, source: "test-font" }] };
  });
  assert.equal(mapped.overlays.length, 1);
  assert.equal(mapped.overlays[0].text, "FIRST LINE\nSECOND LINE");
  assert.equal(mapped.overlays[0].figletFont, "test-font");
});

test("multiline overlays preserve missing source rows as blank text rows", async () => {
  const prepared = {
    result: { image: { width: 100, height: 100 } },
    items: [
      { text: "ALPHA", bounds: bounds(10, 5, 60, 15) },
      { text: "BETA", bounds: bounds(10, 25, 55, 35) },
      { text: "and", bounds: bounds(10, 50, 28, 60) },
    ],
  };
  const mapped = await ocrItemsToOverlays(
    prepared,
    50,
    10,
    { enabled: false, minHeight: 1 },
    async (requests) => {
      assert.equal(requests.length, 1);
      assert.equal(requests[0].text, "ALPHA\n\nBETA\n\n\nand");
      assert.equal(requests[0].baseHeight, 6);
      return {
        arts: [{
          source: "plain",
          text: requests[0].text,
          width: 5,
          height: 6,
        }],
      };
    },
  );
  assert.equal(mapped.overlays.length, 1);
  assert.equal(mapped.overlays[0].text, "ALPHA\n\nBETA\n\n\nand");
  assert.equal(mapped.overlays[0].height, 6);
});
