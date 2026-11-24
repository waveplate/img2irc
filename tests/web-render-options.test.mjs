import test from "node:test";
import assert from "node:assert/strict";
import { collectRenderOptions } from "../web/js/render-options.js";

function controls(values) {
  return {
    querySelector(selector) {
      const field = values[selector.slice(1)];
      return { value: field ?? "", checked: field === true, children: [] };
    },
    querySelectorAll(selector) {
      assert.equal(selector, "#glyph-groups input:checked");
      return [{ value: "dither" }];
    },
  };
}

test("Dither keeps large numeric settings and uses the selected glyphs and smoothing", () => {
  const values = {
    render: "irc",
    "render-mode": "dither",
    "dither-detail": "12",
    "dither-palette-candidates": "64",
    smooth: true,
    "smooth-shapes": true,
    "glyph-pair-search": true,
  };
  const glyphCatalog = {
    characters(groups) {
      assert.deepEqual(groups, ["dither"]);
      return "░🮜";
    },
  };
  const options = collectRenderOptions(controls(values), glyphCatalog);
  assert.equal(options.renderMode, "dither");
  assert.equal(options.ditherDetail, 12);
  assert.equal(options.ditherPaletteCandidates, 64);
  assert.equal(options.glyphs, "░🮜");
  assert.equal(options.smooth, true);
  assert.equal(options.smoothShapes, true);
  assert.equal(options.glyphSearch, false);
  assert.equal(options.includePreview, true);
  values["dither-palette-candidates"] = "0";
  assert.equal(collectRenderOptions(controls(values), glyphCatalog).ditherPaletteCandidates, 0);
});

test("Dither rejects incompatible settings and fractional coverage detail", () => {
  const options = {
    render: "ansi24",
    "render-mode": "dither",
    "dither-detail": "2.5",
    "dither-palette-candidates": "4",
  };
  const glyphs = { characters: () => "▒" };
  assert.throws(() => collectRenderOptions(controls(options), glyphs), /IRC99 or ANSI 256/);
  options.render = "ansi";
  assert.throws(() => collectRenderOptions(controls(options), glyphs), /Dither Detail/);
});

test("Glyph and Braille omit inactive Dither fields from the WASM request", () => {
  const glyphs = { characters: () => " ▀" };
  for (const mode of ["glyph", "braille"]) {
    const options = collectRenderOptions(controls({
      render: "ansi24",
      "render-mode": mode,
      "dither-detail": "",
      "dither-palette-candidates": "",
    }), glyphs);
    assert.equal(options.renderMode, mode);
    assert.equal(options.includePreview, false);
    const request = structuredClone(options);
    assert.equal(Object.hasOwn(request, "ditherDetail"), false);
    assert.equal(Object.hasOwn(request, "ditherPaletteCandidates"), false);
  }
});
