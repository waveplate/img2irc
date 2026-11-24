import test from "node:test";
import assert from "node:assert/strict";
import { parsePalette } from "../web/js/render-options.js";
import { visiblePalette, initializePaletteControls } from "../web/js/palette-controls.js";
import { ColourReplacementPicker } from "../web/js/colour-replacement.js";
import { sampleRenderedArtwork } from "../web/js/artwork-colours.js";

test("explicit palettes accept hex colours, remove duplicates, and reject invalid input", () => {
  assert.deepEqual(parsePalette("#ff0000, 00ff00; #FF0000"), [[255, 0, 0], [0, 255, 0]]);
  assert.deepEqual(parsePalette(""), []);
  assert.throws(() => parsePalette("#00000x"), /Invalid colour/);
  assert.throws(() => parsePalette("#12345"), /Invalid colour/);
  assert.throws(() => parsePalette(Array.from({ length: 257 }, (_, i) => i.toString(16).padStart(6, "0")).join(" ")), /at most 256/);
});

test("the current palette counts visible pixels and ignores transparent RGB", () => {
  const result = { previewRgba: new Uint8Array([
    255, 0, 0, 255, 255, 0, 0, 255, 0, 0, 0, 255, 3, 4, 5, 0,
  ]) };
  assert.deepEqual(visiblePalette(result), ["#ff0000", "#000000"]);
  assert.deepEqual(visiblePalette(undefined), []);
});

test("restored cell grids supply exact palette colours without antialiased shades or transparent backgrounds", () => {
  const result = { cells: [[
    { character: "A", foreground: [255, 0, 0], background: [0, 0, 0] },
    { character: " ", foreground: [0, 255, 0], background: [0, 0, 0] },
    { character: "B", foreground: [255, 0, 0], background: null },
    { character: " ", foreground: [0, 0, 255], background: [0, 0, 0], inverted: true },
  ]], previewRgba: [128, 0, 0, 255] };
  assert.deepEqual(visiblePalette(result), ["#000000", "#ff0000", "#0000ff"]);
});

test("sampling committed and restored results uses the canvas after RGBA is released in either preview mode", () => {
  for (const initialMode of ["text", "bitmap"]) {
    const result = { content: "art", cells: [[{}]], previewWidth: 500, previewHeight: 500 };
    let mode = initialMode;
    let rasterized = false;
    const preview = {
      getMode: () => mode,
      setMode: (next) => { mode = next; if (next === "bitmap") rasterized = true; },
    };
    const canvas = { width: 2, height: 1, getContext: () => ({ getImageData: (x, y, width, height) => {
      assert.equal(rasterized, true);
      assert.deepEqual([x, y, width, height], [0, 0, 2, 1]);
      return { data: new Uint8ClampedArray([250, 8, 8, 255, 0, 0, 0, 255]) };
    } }) };
    assert.equal(sampleRenderedArtwork(undefined, preview, canvas, "ansi"), undefined);
    assert.equal(rasterized, false);
    const sampled = sampleRenderedArtwork(result, preview, canvas, "ansi");
    assert.deepEqual([...sampled.previewRgba], [255, 0, 0, 255, 0, 0, 0, 255]);
    assert.deepEqual([sampled.previewWidth, sampled.previewHeight], [2, 1]);
    assert.equal(mode, initialMode);
    assert.equal(result.previewRgba, undefined);
  }
});

class Element extends EventTarget {
  value = "";
  checked = false;
  disabled = false;
  hidden = true;
  children = [];
  style = {};
  nextElementSibling = { value: "" };
  validityMessage = "";
  removeAttribute(name) { delete this[name]; }
  setCustomValidity(message) { this.validityMessage = message; }
  reportValidity() { return !this.validityMessage; }
  replaceChildren(...children) { this.children = children; }
  click() { this.dispatchEvent(new Event("click")); }
}

const form = () => {
  const elements = new Map();
  const root = new EventTarget();
  root.querySelector = (selector) => {
    const id = selector.startsWith("#") ? selector.slice(1) : selector.match(/data-effect-control="([^"]+)"/)?.[1];
    if (!elements.has(id)) elements.set(id, new Element());
    return elements.get(id);
  };
  return root;
};

test("inline palette controls freeze, remove, reset, and validate chosen colours", () => {
  const previous = globalThis.document;
  globalThis.document = { createElement: () => new Element() };
  try {
    const root = form();
    const el = (id) => root.querySelector(`#${id}`);
    const result = { previewRgba: new Uint8Array([255, 0, 0, 255, 0, 0, 0, 255]) };
    initializePaletteControls(root, () => result, () => {});
    el("quantize-current").click();
    assert.equal(el("quantize-enabled").checked, true);
    assert.equal(el("quantize-swatches").children.length, 2);
    assert.equal(el("quantize-colours").disabled, true);
    el("quantize-swatches").children[0].click();
    assert.equal(el("quantize-palette").value, "#ff0000");
    el("quantize-auto").click();
    assert.equal(el("quantize-palette").value, "");
    assert.equal(el("quantize-threshold").disabled, false);
    el("quantize-palette").value = "invalid";
    el("quantize-palette").dispatchEvent(new Event("input"));
    assert.match(el("quantize-palette").validityMessage, /Invalid colour/);
    el("quantize-enabled").checked = false;
    el("quantize-enabled").dispatchEvent(new Event("change"));
    assert.equal(el("quantize-palette").validityMessage, "");
    el("quantize-current").click();
    // The pipeline controller resets values before notifying each effect instance.
    el("quantize-palette").value = "";
    root.dispatchEvent(new Event("effectreset"));
    assert.equal(el("quantize-palette").value, "");
    assert.equal(el("quantize-colours").disabled, false);
  } finally {
    globalThis.document = previous;
  }
});

test("replacement opens an inline eyedropper only when requested and samples exact RGB", () => {
  const previous = globalThis.ImageData;
  const previousDocument = globalThis.document;
  globalThis.ImageData = class {};
  globalThis.document = { createElement: () => new Element() };
  try {
    const root = form();
    const el = (id) => root.querySelector(`#${id}`);
    el("replace-from").value = "#ffffff";
    el("replace-to").value = "#000000";
    const canvas = el("replace-canvas");
    canvas.getBoundingClientRect = () => ({ left: 0, top: 0, width: 2, height: 1 });
    canvas.getContext = () => ({ putImageData() {}, getImageData: () => ({ data: new Uint8Array([255, 0, 0, 255]) }) });
    const result = { previewWidth: 2, previewHeight: 1, previewRgba: new Uint8Array([255, 0, 0, 255, 0, 0, 0, 255]) };
    let changes = 0;
    const picker = new ColourReplacementPicker(root, () => result, () => "ansi24", () => changes++);
    picker.start();
    el("replace-enabled").checked = true;
    el("replace-enabled").dispatchEvent(new Event("change"));
    assert.equal(el("replace-picker").hidden, true);
    el("replace-pick").click();
    assert.equal(el("replace-picker").hidden, false);
    assert.equal(canvas.hidden, false);
    const click = new Event("click");
    Object.assign(click, { clientX: 0.5, clientY: 0.5 });
    canvas.dispatchEvent(click);
    assert.equal(el("replace-from").value, "#ff0000");
    assert.equal(el("replace-picker").hidden, true);
    assert.equal(changes, 1);
    el("replace-pick-to").click();
    assert.match(el("replace-picker-title").textContent, /To/);
    el("replace-picker-swatches").children[0].click();
    assert.equal(el("replace-to").value, "#000000");
    assert.equal(el("replace-picker").hidden, true);
    assert.equal(changes, 2);
  } finally {
    globalThis.ImageData = previous;
    globalThis.document = previousDocument;
  }
});

test("explicit palettes add custom colours, current-art swatches, and sampled pixels without duplicates", () => {
  const previousImageData = globalThis.ImageData;
  const previousDocument = globalThis.document;
  globalThis.ImageData = class {};
  globalThis.document = { createElement: () => new Element() };
  try {
    const root = form();
    const el = (id) => root.querySelector(`#${id}`);
    let result;
    let changes = 0;
    const canvas = el("quantize-canvas");
    canvas.getBoundingClientRect = () => ({ left: 10, top: 20, width: 4, height: 2 });
    let pixel = [255, 0, 0, 0];
    canvas.getContext = () => ({ putImageData() {}, getImageData: (x, y) => {
      assert.deepEqual([x, y], [1, 0]);
      return { data: new Uint8Array(pixel) };
    } });
    initializePaletteControls(root, () => result, () => changes++);
    el("quantize-colour").value = "#123456";
    el("quantize-add").click();
    assert.equal(el("quantize-palette").value, "#123456");
    assert.equal(el("quantize-enabled").checked, true);
    el("quantize-current").click();
    assert.equal(el("quantize-palette").value, "#123456");
    assert.match(el("quantize-picker-title").textContent, /Render the art first/);
    result = { previewWidth: 2, previewHeight: 1, previewRgba: new Uint8Array([255, 0, 0, 255, 0, 0, 0, 255]) };
    el("quantize-pick").click();
    assert.equal(canvas.hidden, false);
    assert.equal(el("quantize-picker-swatches").children.length, 2);
    el("quantize-picker-swatches").children[1].click();
    assert.equal(el("quantize-palette").value, "#123456 #ff0000");
    assert.equal(el("quantize-picker").hidden, true);
    el("quantize-pick").click();
    const click = new Event("click");
    Object.assign(click, { clientX: 13, clientY: 21 });
    canvas.dispatchEvent(click);
    assert.match(el("quantize-picker-title").textContent, /transparent/);
    assert.equal(el("quantize-picker").hidden, false);
    assert.equal(changes, 2);
    pixel = [0, 255, 0, 255];
    canvas.dispatchEvent(click);
    assert.equal(el("quantize-palette").value, "#123456 #ff0000 #00ff00");
    assert.equal(el("quantize-picker").hidden, true);
    el("quantize-add").click();
    assert.equal(el("quantize-palette").value, "#123456 #ff0000 #00ff00");
    el("quantize-palette").value = Array.from({ length: 256 }, (_, i) => `#0000${i.toString(16).padStart(2, "0")}`).join(" ");
    const fullPalette = el("quantize-palette").value;
    el("quantize-add").click();
    assert.equal(el("quantize-palette").value, fullPalette);
    assert.match(el("quantize-palette").validityMessage, /at most 256/);
    el("quantize-auto").click();
    assert.equal(el("quantize-palette").validityMessage, "");
    assert.equal(el("quantize-colours").disabled, false);
  } finally {
    globalThis.ImageData = previousImageData;
    globalThis.document = previousDocument;
  }
});
