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
