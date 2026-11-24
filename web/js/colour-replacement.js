import { quantizeRgb } from "./output-palettes.js";
import { effectControl } from "./effect-catalog.js";
import { ArtworkColourPicker } from "./artwork-colour-picker.js";

export class ColourReplacementPicker {
  #root;
  #picker;
  #renderMode;
  #changed;
  #target = "from";

  constructor(root, result, renderMode, changed) {
    this.#root = root;
    this.#renderMode = renderMode;
    this.#changed = changed;
    this.#picker = new ArtworkColourPicker(root, "replace", result, (colour) => {
      this.#element(`replace-${this.#target}`).value = colour;
      this.#normalizeControl(`replace-${this.#target}`);
      this.#element("replace-enabled").checked = true;
      this.#element("replace-enabled").dispatchEvent(new Event("change"));
      this.#changed();
    });
  }

  #element(id) { return effectControl(this.#root, id); }

  close() {
    this.#picker.close();
  }

  start() {
    const syncValidation = () => {
      const enabled = this.#element("replace-enabled").checked;
      for (const id of ["replace-from", "replace-to"]) {
        const input = this.#element(id);
        if (enabled) input.pattern = "#[0-9a-fA-F]{6}";
        else input.removeAttribute("pattern");
        input.required = enabled;
      }
    };
    this.#element("replace-enabled").addEventListener("change", syncValidation);
    this.#root.addEventListener("effectreset", () => {
      this.close();
      syncValidation();
    });
    syncValidation();
    this.#picker.start();
    this.#element("replace-pick").addEventListener("click", () => this.#open("from"));
    this.#element("replace-pick-to").addEventListener("click", () => this.#open("to"));
    for (const id of ["replace-from", "replace-to"]) {
      this.#element(id).addEventListener("change", () => {
        this.#normalizeControl(id);
        this.#changed();
      });
    }
  }

  normalize() {
    this.#normalizeControl("replace-from");
    this.#normalizeControl("replace-to");
  }

  #normalizeControl(id) {
    const control = this.#element(id);
    if (!/^#[0-9a-f]{6}$/i.test(control.value)) return;
    const rgb = [1, 3, 5].map((start) => Number.parseInt(control.value.slice(start, start + 2), 16));
    const visible = quantizeRgb(rgb, this.#renderMode());
    control.value = `#${visible.map((channel) => channel.toString(16).padStart(2, "0")).join("")}`;
  }

  #open(target) {
    this.#target = target;
    const mode = this.#renderMode();
    const palette = mode === "irc" ? "IRC" : mode === "ansi" ? "ANSI 256" : "24-bit";
    this.#picker.open(`Click a palette swatch or a visible ${palette} colour for ${target === "from" ? "From" : "To"}`);
  }
}
