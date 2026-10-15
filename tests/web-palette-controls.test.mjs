import test from "node:test";
import assert from "node:assert/strict";
import { parsePalette, resetAdjustments } from "../web/js/render-options.js";
import { visiblePalette, initializePaletteControls } from "../web/js/palette-controls.js";
import { ColourReplacementPicker } from "../web/js/colour-replacement.js";

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
  replaceChildren(...children) { this.children = children; }
  click() { this.dispatchEvent(new Event("click")); }
}

const form = () => {
  const elements = new Map();
  const querySelector = (selector) => {
    if (!elements.has(selector)) elements.set(selector, new Element());
    return elements.get(selector);
  };
  return { querySelector };
};

test("inline palette controls freeze, remove, reset, and validate chosen colours", () => {
  const previous = globalThis.document;
  globalThis.document = { createElement: () => new Element() };
  try {
    const root = form();
    const el = (id) => root.querySelector(`#${id}`);
    const result = { previewRgba: new Uint8Array([255, 0, 0, 255, 0, 0, 0, 255]) };
    el("reset-adjustments").addEventListener("click", () => resetAdjustments(root));
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
    el("reset-adjustments").click();
    assert.equal(el("quantize-palette").value, "");
    assert.equal(el("quantize-colours").disabled, false);
  } finally {
    globalThis.document = previous;
  }
});

test("replacement opens an inline eyedropper only when requested and samples exact RGB", () => {
  const previous = globalThis.ImageData;
  globalThis.ImageData = class {};
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
  } finally {
    globalThis.ImageData = previous;
  }
});
