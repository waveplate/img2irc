import test from "node:test";
import assert from "node:assert/strict";
import { initializeSliderToggles } from "../web/js/slider-toggle.js";

class Input extends EventTarget {
  constructor(id, value, min, max, off) {
    super();
    Object.assign(this, {
      id, value, defaultValue: value, min, max, step: "1", off,
      validity: { valid: true }, disabled: false, readOnly: false,
    });
  }
  hasAttribute() { return this.off !== undefined; }
  getAttribute() { return this.off; }
  space(options = {}) {
    const event = new Event("keydown", { cancelable: true });
    Object.assign(event, { key: " ", ...options });
    this.dispatchEvent(event);
    return event;
  }
}

test("Space restores warp and negative values independently, without repeat bounce", () => {
  const warp = new Input("superpixel-strength", "75", "0", "100");
  const brightness = new Input("brightness", "-42", "-255", "255");
  initializeSliderToggles({ querySelectorAll: () => [warp, brightness] });
  let changes = 0;
  warp.addEventListener("input", () => changes++);
  assert.equal(warp.space().defaultPrevented, true);
  brightness.space();
  assert.equal(warp.value, "0");
  assert.equal(brightness.value, "0");
  warp.space({ repeat: true });
  assert.equal(warp.value, "0");
  warp.space();
  brightness.space();
  assert.equal(warp.value, "75");
  assert.equal(brightness.value, "-42");
  assert.equal(changes, 2);
});

test("manual edits, neutral scale, and automatic height round trip", () => {
  const warp = new Input("superpixel-strength", "100", "0", "100");
  const scale = new Input("scale-x", "1.5", "0.5", "2");
  const height = new Input("height", "60", "1", "300", "");
  const count = new Input("smooth-candidates", "5", "1", "256");
  initializeSliderToggles({ querySelectorAll: () => [warp, scale, height, count] });
  warp.value = "65";
  warp.dispatchEvent(new Event("input"));
  warp.value = "0";
  warp.dispatchEvent(new Event("input"));
  warp.space();
  assert.equal(warp.value, "65");
  scale.space();
  height.space();
  count.space();
  assert.equal(scale.value, "1");
  assert.equal(height.value, "");
  assert.equal(count.value, "1");
  scale.space();
  height.space();
  count.space();
  assert.equal(scale.value, "1.5");
  assert.equal(height.value, "60");
  assert.equal(count.value, "5");
});

test("disabled fields and modifier shortcuts are untouched", () => {
  const warp = new Input("superpixel-strength", "80", "0", "100");
  initializeSliderToggles({ querySelectorAll: () => [warp] });
  assert.equal(warp.space({ ctrlKey: true }).defaultPrevented, false);
  warp.disabled = true;
  assert.equal(warp.space().defaultPrevented, false);
  assert.equal(warp.value, "80");
});
