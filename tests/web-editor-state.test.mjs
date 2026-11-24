import test from "node:test";
import assert from "node:assert/strict";

import { EditorState } from "../web/js/editor-state.js";

test("OCR overlay edits remain dirty until replacement or clearing", () => {
  const state = new EditorState();
  const overlays = [{ kind: "ocr", text: "detected" }];

  state.replaceOcrOverlays(overlays);
  assert.equal(state.ocrOverlaysDirty, false);

  state.markOcrOverlaysEdited();
  overlays[0].text = "corrected";
  assert.equal(state.ocrOverlaysDirty, true);
  assert.equal(state.ocrOverlays[0].text, "corrected");

  state.setOcrOverlaysEnabled(false);
  assert.deepEqual(state.overlays, []);
  assert.equal(state.ocrOverlays[0].text, "corrected");
  assert.equal(state.ocrOverlaysDirty, true);
  state.setOcrOverlaysEnabled(true);
  assert.equal(state.overlays[0].text, "corrected");

  state.replaceOcrOverlays([{ kind: "ocr", text: "regenerated" }]);
  assert.equal(state.ocrOverlaysDirty, false);
  assert.equal(state.ocrOverlays[0].text, "regenerated");

  state.markOcrOverlaysEdited();
  state.clearOcrOverlays();
  assert.equal(state.ocrOverlaysDirty, false);
  assert.deepEqual(state.ocrOverlays, []);
});

test("overlay stacking spans OCR and manual layers, preserves identity, and survives session serialization", () => {
  const state = new EditorState();
  const ocr = { kind: "ocr", text: "OCR" };
  const rectangle = { kind: "manual", text: "rectangle" };
  const text = { kind: "manual", text: "text" };
  state.replaceOcrOverlays([ocr]);
  state.addManualOverlay(rectangle);
  state.addManualOverlay(text);
  assert.deepEqual(state.overlays, [ocr, rectangle, text]);
  assert.equal(state.reorderOverlay(rectangle, "back"), true);
  assert.deepEqual(state.overlays, [rectangle, ocr, text]);
  assert.equal(state.ocrOverlaysDirty, true);
  assert.equal(state.reorderOverlay(text, "lower"), true);
  assert.deepEqual(state.overlays, [rectangle, text, ocr]);
  assert.equal(state.reorderOverlay(rectangle, "front"), true);
  assert.deepEqual(state.overlays, [text, ocr, rectangle]);
  assert.equal(state.reorderOverlay(rectangle, "raise"), false);
  const restored = new EditorState();
  restored.manualOverlays = JSON.parse(JSON.stringify(state.manualOverlays));
  restored.ocrOverlays = JSON.parse(JSON.stringify(state.ocrOverlays));
  assert.deepEqual(restored.overlays.map((overlay) => overlay.text), ["text", "OCR", "rectangle"]);
});

test("hidden OCR keeps its stack position and new overlays are added at the front", () => {
  const state = new EditorState();
  const ocr = { kind: "ocr", text: "OCR" };
  const first = { kind: "manual", text: "first" };
  const next = { kind: "manual", text: "next" };
  state.replaceOcrOverlays([ocr]);
  state.addManualOverlay(first);
  state.reorderOverlay(first, "back");
  state.setOcrOverlaysEnabled(false);
  state.addManualOverlay(next);
  assert.deepEqual(state.overlays, [first, next]);
  state.setOcrOverlaysEnabled(true);
  assert.deepEqual(state.overlays, [first, ocr, next]);
});
