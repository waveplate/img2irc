import { quantizeRgb } from "./output-palettes.js";

export class ColourReplacementPicker {
  #root;
  #result;
  #renderMode;
  #changed;
  #target = "from";

  constructor(root, result, renderMode, changed) {
    this.#root = root;
    this.#result = result;
    this.#renderMode = renderMode;
    this.#changed = changed;
  }

  #element(id) { return this.#root.querySelector(`#${id}`); }

  close() {
    this.#element("replace-picker").hidden = true;
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
    this.#element("reset-adjustments").addEventListener("click", syncValidation);
    syncValidation();
    this.#element("replace-cancel").addEventListener("click", () => this.close());
    this.#element("replace-pick").addEventListener("click", () => { void this.#open("from"); });
    this.#element("replace-pick-to").addEventListener("click", () => { void this.#open("to"); });
    for (const id of ["replace-from", "replace-to"]) {
      this.#element(id).addEventListener("change", () => {
        this.#normalizeControl(id);
        this.#changed();
      });
    }
    this.#element("render").addEventListener("change", () => {
      this.#normalizeControl("replace-from");
      this.#normalizeControl("replace-to");
    });
    this.#element("replace-canvas").addEventListener("click", (event) => {
      const canvas = event.currentTarget;
      const rect = canvas.getBoundingClientRect();
      if (!rect.width || !rect.height) return;
      const x = Math.max(0, Math.min(canvas.width - 1, Math.floor((event.clientX - rect.left) * canvas.width / rect.width)));
      const y = Math.max(0, Math.min(canvas.height - 1, Math.floor((event.clientY - rect.top) * canvas.height / rect.height)));
      const pixel = canvas.getContext("2d").getImageData(x, y, 1, 1).data;
      if (pixel[3] === 0) {
        this.#element("replace-picker-title").textContent = "That pixel is transparent; choose a visible colour";
        return;
      }
      this.#element(`replace-${this.#target}`).value = `#${[...pixel.slice(0, 3)].map((c) => c.toString(16).padStart(2, "0")).join("")}`;
      this.#element("replace-enabled").checked = true;
      syncValidation();
      this.close();
      this.#changed();
    });
  }

  #normalizeControl(id) {
    const control = this.#element(id);
    if (!/^#[0-9a-f]{6}$/i.test(control.value)) return;
    const rgb = [1, 3, 5].map((start) => Number.parseInt(control.value.slice(start, start + 2), 16));
    const visible = quantizeRgb(rgb, this.#renderMode());
    control.value = `#${visible.map((channel) => channel.toString(16).padStart(2, "0")).join("")}`;
  }

  #open(target) {
    const result = this.#result();
    this.#target = target;
    const title = this.#element("replace-picker-title");
    const picker = this.#element("replace-picker");
    const canvas = this.#element("replace-canvas");
    canvas.hidden = true;
    const mode = this.#renderMode();
    picker.hidden = false;
    if (result?.previewRgba?.length && result.previewWidth && result.previewHeight) {
      // Keep the backing pixels intact. CSS fits the inline sampling image;
      // sampling never sees interpolated colours introduced by a resize.
      canvas.width = result.previewWidth;
      canvas.height = result.previewHeight;
      const context = canvas.getContext("2d", { willReadFrequently: true });
      context.putImageData(new ImageData(new Uint8ClampedArray(result.previewRgba), canvas.width, canvas.height), 0, 0);
      canvas.hidden = false;
      const palette = mode === "irc" ? "IRC" : mode === "ansi" ? "ANSI 256" : "24-bit";
      title.textContent = `Click a visible ${palette} colour for ${target === "from" ? "From" : "To"}`;
    } else {
      title.textContent = "Render the art first to sample its colours.";
    }
  }
}
