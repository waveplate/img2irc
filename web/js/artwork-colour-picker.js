import { colourHex, visiblePalette } from "./artwork-colours.js";
import { effectControl } from "./effect-catalog.js";

export class ArtworkColourPicker {
  #root;
  #prefix;
  #artwork;
  #picked;

  constructor(root, prefix, artwork, picked) {
    this.#root = root;
    this.#prefix = prefix;
    this.#artwork = artwork;
    this.#picked = picked;
  }

  #element(id) { return effectControl(this.#root, `${this.#prefix}-${id}`); }

  close() { this.#element("picker").hidden = true; }

  start() {
    this.#element("cancel").addEventListener("click", () => this.close());
    this.#root.addEventListener("effectreset", () => this.close());
    this.#element("canvas").addEventListener("click", (event) => {
      const canvas = event.currentTarget;
      const rect = canvas.getBoundingClientRect();
      if (!rect.width || !rect.height) return;
      const x = Math.max(0, Math.min(canvas.width - 1, Math.floor((event.clientX - rect.left) * canvas.width / rect.width)));
      const y = Math.max(0, Math.min(canvas.height - 1, Math.floor((event.clientY - rect.top) * canvas.height / rect.height)));
      const pixel = canvas.getContext("2d").getImageData(x, y, 1, 1).data;
      if (!pixel[3]) {
        this.#element("picker-title").textContent = "That pixel is transparent; choose a visible colour";
        return;
      }
      this.#choose(colourHex([...pixel.slice(0, 3)]));
    });
  }

  #choose(colour) {
    if (this.#picked(colour) !== false) this.close();
  }

  open(title) {
    const result = this.#artwork();
    const canvas = this.#element("canvas");
    this.#element("picker").hidden = false;
    canvas.hidden = true;
    const swatches = this.#element("picker-swatches");
    swatches?.replaceChildren();
    if (!result?.previewRgba?.length || !result.previewWidth || !result.previewHeight) {
      this.#element("picker-title").textContent = "Render the art first to sample its colours.";
      return;
    }
    canvas.width = result.previewWidth;
    canvas.height = result.previewHeight;
    canvas.getContext("2d", { willReadFrequently: true }).putImageData(
      new ImageData(new Uint8ClampedArray(result.previewRgba), canvas.width, canvas.height), 0, 0,
    );
    canvas.hidden = false;
    this.#element("picker-title").textContent = title;
    if (swatches) {
      swatches.replaceChildren(...visiblePalette(result).map((colour) => {
        const button = document.createElement("button");
        button.type = "button";
        button.textContent = colour;
        button.title = `Choose ${colour}`;
        button.style.borderLeft = `1.25rem solid ${colour}`;
        button.addEventListener("click", () => this.#choose(colour));
        return button;
      }));
    }
  }
}
