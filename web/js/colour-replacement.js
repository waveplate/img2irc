import { quantizeImageData, quantizeRgb } from "./output-palettes.js";

export class ColourReplacementPicker {
  #root;
  #sourceUrl;
  #renderMode;
  #changed;
  #revision = 0;
  #target = "from";

  constructor(root, sourceUrl, renderMode, changed) {
    this.#root = root;
    this.#sourceUrl = sourceUrl;
    this.#renderMode = renderMode;
    this.#changed = changed;
  }

  #element(id) { return this.#root.querySelector(`#${id}`); }

  close() {
    this.#revision += 1;
    this.#element("replace-dialog").close();
  }

  start() {
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
        this.#element("replace-dialog-title").textContent = "That pixel is transparent; choose a visible colour";
        return;
      }
      this.#element(`replace-${this.#target}`).value = `#${[...pixel.slice(0, 3)].map((c) => c.toString(16).padStart(2, "0")).join("")}`;
      this.#element("replace-enabled").checked = true;
      this.close();
      this.#changed();
    });
  }

  #normalizeControl(id) {
    const control = this.#element(id);
    const rgb = [1, 3, 5].map((start) => Number.parseInt(control.value.slice(start, start + 2), 16));
    const visible = quantizeRgb(rgb, this.#renderMode());
    control.value = `#${visible.map((channel) => channel.toString(16).padStart(2, "0")).join("")}`;
  }

  async #open(target) {
    const url = this.#sourceUrl();
    if (!url) return;
    this.#target = target;
    const revision = ++this.#revision;
    const title = this.#element("replace-dialog-title");
    const dialog = this.#element("replace-dialog");
    const canvas = this.#element("replace-canvas");
    canvas.hidden = true;
    const mode = this.#renderMode();
    title.textContent = "Loading visible colours…";
    dialog.showModal();
    try {
      const img = new Image();
      img.src = url;
      await img.decode();
      if (revision !== this.#revision || !dialog.open) return;
      const maxWidth = Math.max(1, Math.min(1000, window.innerWidth - 80));
      const maxHeight = Math.max(1, Math.min(700, window.innerHeight * 0.65));
      const scale = Math.min(1, maxWidth / img.naturalWidth, maxHeight / img.naturalHeight);
      canvas.width = Math.max(1, Math.round(img.naturalWidth * scale));
      canvas.height = Math.max(1, Math.round(img.naturalHeight * scale));
      const context = canvas.getContext("2d", { willReadFrequently: true });
      context.drawImage(img, 0, 0, canvas.width, canvas.height);
      const pixels = context.getImageData(0, 0, canvas.width, canvas.height);
      context.putImageData(quantizeImageData(pixels, mode), 0, 0);
      canvas.hidden = false;
      const palette = mode === "irc" ? "IRC" : mode === "ansi" ? "ANSI 256" : "24-bit";
      title.textContent = `Click a visible ${palette} colour for ${target === "from" ? "From" : "To"}`;
    } catch {
      if (revision === this.#revision) title.textContent = `Could not load the image. Use the ${target === "from" ? "From" : "To"} colour control instead.`;
    }
  }
}
