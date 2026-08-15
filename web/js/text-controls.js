function rgbToHex(rgb, fallback) {
  if (!Array.isArray(rgb) || rgb.length !== 3) return fallback;
  return `#${rgb.map((channel) => Number(channel).toString(16).padStart(2, "0")).join("")}`;
}

function hexToRgb(value) {
  const match = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(value);
  return match ? match.slice(1).map((channel) => Number.parseInt(channel, 16)) : undefined;
}

function defaultOverlay() {
  return {
    kind: "manual",
    text: "Text",
    x: 0,
    y: 0,
    width: 4,
    height: 1,
    foreground: [255, 255, 255],
    background: undefined,
    wrap: false,
    autoGrow: true,
    bold: false,
    italic: false,
    underline: false,
  };
}

export class TextControls {
  #root;
  #state;
  #changed;
  #selected;

  constructor(root, state, changed) {
    this.#root = root;
    this.#state = state;
    this.#changed = changed;
  }

  start() {
    this.#element("overlay-add").addEventListener("click", () => {
      const overlay = defaultOverlay();
      this.#state.manualOverlays.push(overlay);
      this.#selected = overlay;
      this.#render();
      this.#changed();
    });
    this.#element("overlay-delete").addEventListener("click", () => {
      if (!this.#selected) return;
      for (const collection of [this.#state.ocrOverlays, this.#state.manualOverlays]) {
        const index = collection.indexOf(this.#selected);
        if (index >= 0) collection.splice(index, 1);
      }
      this.#selected = undefined;
      this.#render();
      this.#changed();
    });
    this.#element("overlay-list").addEventListener("change", (event) => {
      const [kind, rawIndex] = event.target.value.split(":");
      const collection = kind === "ocr" ? this.#state.ocrOverlays : this.#state.manualOverlays;
      this.#selected = collection[Number(rawIndex)];
      this.#renderEditor();
    });

    for (const id of ["overlay-text", "overlay-x", "overlay-y", "overlay-width", "overlay-height"]) {
      this.#element(id).addEventListener("input", () => this.#updateSelected());
    }
    for (const id of [
      "overlay-wrap", "overlay-auto-grow", "overlay-bold", "overlay-italic", "overlay-underline",
      "overlay-foreground-enabled", "overlay-background-enabled", "overlay-foreground", "overlay-background",
    ]) {
      this.#element(id).addEventListener("input", () => this.#updateSelected());
    }
    this.#render();
  }

  setOcrOverlays(overlays) {
    const previous = this.#selected;
    this.#state.ocrOverlays = overlays;
    if (previous?.kind === "ocr") this.#selected = overlays[0];
    this.#render();
  }

  clearOcrOverlays() {
    if (this.#selected?.kind === "ocr") this.#selected = undefined;
    this.#state.ocrOverlays = [];
    this.#render();
  }

  #element(id) {
    return this.#root.querySelector(`#${id}`);
  }

  #entries() {
    return [
      ...this.#state.ocrOverlays.map((overlay, index) => ({ overlay, value: `ocr:${index}` })),
      ...this.#state.manualOverlays.map((overlay, index) => ({ overlay, value: `manual:${index}` })),
    ];
  }

  #render() {
    const list = this.#element("overlay-list");
    const entries = this.#entries();
    list.replaceChildren(...entries.map(({ overlay, value }, index) => {
      const option = document.createElement("option");
      option.value = value;
      const text = overlay.text.replaceAll("\n", " ↵ ") || "(empty)";
      option.textContent = `${overlay.kind === "ocr" ? "OCR" : "Text"} ${index + 1}: ${text}`;
      option.selected = overlay === this.#selected;
      return option;
    }));
    if (this.#selected && !entries.some(({ overlay }) => overlay === this.#selected)) {
      this.#selected = undefined;
    }
    this.#renderEditor();
  }

  #renderEditor() {
    const editor = this.#element("overlay-editor");
    editor.hidden = !this.#selected;
    this.#element("overlay-empty").hidden = Boolean(this.#selected);
    if (!this.#selected) return;
    const overlay = this.#selected;
    this.#element("overlay-text").value = overlay.text;
    this.#element("overlay-x").value = overlay.x;
    this.#element("overlay-y").value = overlay.y;
    this.#element("overlay-width").value = overlay.width;
    this.#element("overlay-height").value = overlay.height;
    this.#element("overlay-wrap").checked = overlay.wrap;
    this.#element("overlay-auto-grow").checked = overlay.autoGrow;
    this.#element("overlay-bold").checked = overlay.bold;
    this.#element("overlay-italic").checked = overlay.italic;
    this.#element("overlay-underline").checked = overlay.underline;
    this.#element("overlay-foreground-enabled").checked = Boolean(overlay.foreground);
    this.#element("overlay-background-enabled").checked = Boolean(overlay.background);
    this.#element("overlay-foreground").value = rgbToHex(overlay.foreground, "#ffffff");
    this.#element("overlay-background").value = rgbToHex(overlay.background, "#000000");
    this.#syncDisabledFields();
  }

  #syncDisabledFields() {
    this.#element("overlay-width").disabled = this.#element("overlay-auto-grow").checked;
    this.#element("overlay-height").disabled = this.#element("overlay-auto-grow").checked;
    this.#element("overlay-foreground").disabled = !this.#element("overlay-foreground-enabled").checked;
    this.#element("overlay-background").disabled = !this.#element("overlay-background-enabled").checked;
  }

  #updateSelected() {
    if (!this.#selected) return;
    const overlay = this.#selected;
    overlay.text = this.#element("overlay-text").value;
    overlay.x = Number(this.#element("overlay-x").value);
    overlay.y = Number(this.#element("overlay-y").value);
    overlay.width = Number(this.#element("overlay-width").value);
    overlay.height = Number(this.#element("overlay-height").value);
    overlay.wrap = this.#element("overlay-wrap").checked;
    overlay.autoGrow = this.#element("overlay-auto-grow").checked;
    overlay.bold = this.#element("overlay-bold").checked;
    overlay.italic = this.#element("overlay-italic").checked;
    overlay.underline = this.#element("overlay-underline").checked;
    overlay.foreground = this.#element("overlay-foreground-enabled").checked
      ? hexToRgb(this.#element("overlay-foreground").value)
      : undefined;
    overlay.background = this.#element("overlay-background-enabled").checked
      ? hexToRgb(this.#element("overlay-background").value)
      : undefined;
    this.#syncDisabledFields();
    const option = this.#element("overlay-list").selectedOptions[0];
    if (option) {
      const text = overlay.text.replaceAll("\n", " ↵ ") || "(empty)";
      option.textContent = `${overlay.kind === "ocr" ? "OCR" : "Text"}: ${text}`;
    }
    this.#changed();
  }
}
