import { resizeEdges, resizeBox } from "./overlay-geometry.js";

function rgbToHex(rgb, fallback) {
  if (!Array.isArray(rgb) || rgb.length !== 3) return fallback;
  return `#${rgb.map((channel) => Number(channel).toString(16).padStart(2, "0")).join("")}`;
}

function hexToRgb(value) {
  const match = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(value);
  return match ? match.slice(1).map((channel) => Number.parseInt(channel, 16)) : undefined;
}

function textDimensions(text) {
  const lines = text.split("\n");
  return {
    width: Math.max(1, ...lines.map((line) => [...line.replace(/\r$/, "")].length)),
    height: Math.max(1, lines.length),
  };
}

function defaultOverlay(overrides = {}) {
  return {
    kind: "manual",
    text: "Text",
    x: 0,
    y: 0,
    width: 4,
    height: 1,
    foreground: [255, 255, 255],
    background: undefined,
    wrap: true,
    autoGrow: true,
    figlet: false,
    renderedText: undefined,
    figletFont: undefined,
    figletFontChoice: undefined,
    bold: false,
    italic: false,
    underline: false,
    ...overrides,
  };
}

export function renderableOverlays(overlays) {
  return overlays.map((overlay) => ({
    text: overlay.renderedText ?? overlay.text,
    x: overlay.x,
    y: overlay.y,
    width: overlay.width,
    height: overlay.height,
    foreground: overlay.foreground,
    background: overlay.background,
    transparentSpaces: Boolean(overlay.figletFont && overlay.figletFont !== "plain"),
    wrap: overlay.figlet ? false : overlay.wrap,
    autoGrow: overlay.figlet ? false : overlay.autoGrow,
    bold: overlay.bold,
    italic: overlay.italic,
    underline: overlay.underline,
  }));
}

export class TextControls {
  #root;
  #state;
  #changed;
  #preview;
  #layer;
  #selected;
  #columns = 0;
  #rows = 0;
  #interactive = false;
  #figletFonts = [];
  #gesture;

  constructor(root, state, changed, preview) {
    this.#root = root;
    this.#state = state;
    this.#changed = changed;
    this.#preview = preview;
    this.#layer = root.querySelector("#overlay-tools");
  }

  start() {
    this.#element("overlay-add").addEventListener("click", () => {
      const overlay = defaultOverlay();
      this.#state.addManualOverlay(overlay);
      this.#selected = overlay;
      this.#render();
      this.#focusText();
      this.#changed();
    });
    this.#element("overlay-delete").addEventListener("click", () => this.#deleteSelected());
    for (const action of ["back", "lower", "raise", "front"]) {
      this.#element(`overlay-${action}`).addEventListener("click", () => this.#reorderSelected(action));
    }
    this.#root.addEventListener("keydown", (event) => {
      if (!this.#interactive) return;
      let action;
      if ((event.ctrlKey || event.metaKey) && ["PageUp", "PageDown"].includes(event.key)) {
        action = event.key === "PageUp" ? (event.shiftKey ? "front" : "raise") : (event.shiftKey ? "back" : "lower");
      } else if (event.shiftKey && !event.ctrlKey && !event.metaKey && !event.altKey
        && !event.target.closest("input, textarea, [contenteditable='true']")) {
        action = { ArrowUp: "lower", ArrowDown: "raise", Home: "back", End: "front" }[event.key];
      }
      if (!action) return;
      event.preventDefault();
      this.#reorderSelected(action);
    });
    this.#element("overlay-list").addEventListener("change", (event) => {
      this.#selected = this.#overlayForKey(event.target.value);
      this.#renderEditor();
      this.#renderLayer();
    });

    const updateSelected = (event) => {
      event.stopPropagation();
      this.#updateSelected();
    };
    for (const id of ["overlay-text", "overlay-x", "overlay-y", "overlay-width", "overlay-height"]) {
      this.#element(id).addEventListener("input", updateSelected);
    }
    for (const id of [
      "overlay-wrap", "overlay-auto-grow", "overlay-figlet", "overlay-figlet-font",
      "overlay-bold", "overlay-italic", "overlay-underline", "overlay-foreground-enabled",
      "overlay-background-enabled", "overlay-foreground", "overlay-background",
    ]) {
      this.#element(id).addEventListener("input", updateSelected);
    }

    this.#layer.addEventListener("pointerdown", (event) => this.#beginPointerGesture(event));
    this.#layer.addEventListener("pointermove", (event) => this.#continuePointerGesture(event));
    this.#layer.addEventListener("pointerup", (event) => this.#finishPointerGesture(event));
    this.#layer.addEventListener("pointercancel", (event) => this.#finishPointerGesture(event));
    this.#layer.addEventListener("keydown", (event) => this.#handleLayerKey(event));
    this.#render();
  }

  setFigletFonts(fonts) {
    this.#figletFonts = Array.isArray(fonts) ? fonts : [];
    const select = this.#element("overlay-figlet-font");
    const heights = [...new Set(this.#figletFonts.map((font) => font.height))]
      .sort((left, right) => left - right);
    select.replaceChildren(...heights.map((height) => {
      const group = document.createElement("optgroup");
      group.label = `${height} rows`;
      group.append(...this.#figletFonts
        .filter((font) => font.height === height)
        .map((font) => {
          const option = document.createElement("option");
          option.value = font.name;
          option.textContent = font.name;
          return option;
        }));
      return group;
    }));
    select.disabled = this.#figletFonts.length === 0;
    if (this.#figletFonts.length === 0) {
      this.#renderEditor();
      return;
    }

    let changed = false;
    const names = new Set(this.#figletFonts.map((font) => font.name));
    for (const overlay of this.#state.manualOverlays) {
      if (overlay.figlet && !names.has(overlay.figletFontChoice)) {
        overlay.figletFontChoice = this.#figletFonts[0]?.name;
        this.#invalidateArt(overlay);
        changed = true;
      }
    }
    this.#renderEditor();
    if (changed) this.#changed();
  }

  setInteractive(interactive) {
    this.#interactive = Boolean(interactive);
    this.#renderLayer();
  }

  setGrid(columns, rows) {
    this.#columns = Number(columns) || 0;
    this.#rows = Number(rows) || 0;
    this.#renderLayer();
  }

  setOcrOverlays(overlays) {
    const previous = this.#selected;
    this.#state.replaceOcrOverlays(overlays);
    if (previous?.kind === "ocr") this.#selected = overlays[0];
    this.#render();
  }

  clearOcrOverlays() {
    if (this.#selected?.kind === "ocr") this.#selected = undefined;
    this.#state.clearOcrOverlays();
    this.#render();
  }

  refresh() {
    this.#render();
  }

  selection() { return this.#keyForOverlay(this.#selected); }

  restoreSelection(key) {
    this.#selected = key ? this.#overlayForKey(key) : undefined;
    this.#render();
  }

  #element(id) {
    return this.#root.querySelector(`#${id}`);
  }

  #entries() {
    return this.#state.overlays.map((overlay) => {
      const ocr = this.#state.ocrOverlays.indexOf(overlay);
      return { overlay, key: ocr >= 0 ? `ocr:${ocr}` : `manual:${this.#state.manualOverlays.indexOf(overlay)}` };
    });
  }

  #overlayForKey(key) {
    const [kind, rawIndex] = String(key).split(":");
    const collection = kind === "ocr" ? this.#state.ocrOverlays : this.#state.manualOverlays;
    return collection[Number(rawIndex)];
  }

  #keyForOverlay(overlay) {
    return this.#entries().find((entry) => entry.overlay === overlay)?.key;
  }

  #notifyOverlayChanged(overlay) {
    if (overlay?.kind === "ocr") this.#state.markOcrOverlaysEdited();
    this.#changed();
  }

  #deleteSelected() {
    if (!this.#selected) return;
    const deleted = this.#selected;
    for (const collection of [this.#state.ocrOverlays, this.#state.manualOverlays]) {
      const index = collection.indexOf(this.#selected);
      if (index >= 0) collection.splice(index, 1);
    }
    this.#selected = undefined;
    this.#render();
    this.#notifyOverlayChanged(deleted);
  }

  #reorderSelected(action) {
    if (!this.#state.reorderOverlay(this.#selected, action)) return;
    this.#render();
    this.#changed();
  }

  #entryLabel(overlay, index) {
    const text = overlay.text.replaceAll("\n", " ↵ ") || "(empty)";
    const kind = overlay.kind === "ocr" ? "OCR" : "Text";
    const figlet = overlay.figlet ? " · FIGlet" : "";
    return `${kind} ${index + 1}${figlet}: ${text}`;
  }

  #render() {
    const list = this.#element("overlay-list");
    const entries = this.#entries();
    list.replaceChildren(...entries.map(({ overlay, key }, index) => {
      const option = document.createElement("option");
      option.value = key;
      option.textContent = this.#entryLabel(overlay, index);
      option.selected = overlay === this.#selected;
      return option;
    }));
    if (this.#selected && !entries.some(({ overlay }) => overlay === this.#selected)) {
      this.#selected = undefined;
    }
    this.#renderEditor();
    this.#renderLayer();
  }

  #renderEditor() {
    const editor = this.#element("overlay-editor");
    editor.hidden = !this.#selected;
    this.#element("overlay-empty").hidden = Boolean(this.#selected);
    this.#element("overlay-delete").disabled = !this.#selected;
    const index = this.#state.overlays.indexOf(this.#selected);
    for (const action of ["back", "lower", "raise", "front"]) {
      this.#element(`overlay-${action}`).disabled = index < 0 || ((action === "back" || action === "lower")
        ? index === 0 : index === this.#state.overlays.length - 1);
    }
    if (!this.#selected) return;
    const overlay = this.#selected;
    this.#element("overlay-text").value = overlay.text;
    this.#element("overlay-x").value = overlay.x;
    this.#element("overlay-y").value = overlay.y;
    this.#element("overlay-width").value = overlay.width;
    this.#element("overlay-height").value = overlay.height;
    this.#element("overlay-wrap").checked = overlay.wrap;
    this.#element("overlay-auto-grow").checked = overlay.autoGrow;
    this.#element("overlay-figlet").checked = Boolean(overlay.figlet);
    const figletFontControl = this.#element("overlay-figlet-font-control");
    const choosesFont = Boolean(overlay.figlet);
    figletFontControl.hidden = !choosesFont;
    const figletFont = this.#element("overlay-figlet-font");
    if (choosesFont) {
      figletFont.value = overlay.figletFontChoice
        ?? (this.#figletFonts.some((f) => f.name === overlay.figletFont) ? overlay.figletFont : this.#figletFonts[0]?.name)
        ?? "";
    }
    this.#element("overlay-bold").checked = overlay.bold;
    this.#element("overlay-italic").checked = overlay.italic;
    this.#element("overlay-underline").checked = overlay.underline;
    this.#element("overlay-foreground-enabled").checked = Boolean(overlay.foreground);
    this.#element("overlay-background-enabled").checked = Boolean(overlay.background);
    this.#element("overlay-foreground").value = rgbToHex(overlay.foreground, "#ffffff");
    this.#element("overlay-background").value = rgbToHex(overlay.background, "#000000");
    const figletStatus = this.#element("overlay-figlet-status");
    figletStatus.textContent = overlay.figlet
      ? overlay.figletFont && overlay.figletFont !== "plain"
        ? `Rendered with ${overlay.figletFont}.`
        : !overlay.figletFont
          ? "Render to apply the selected FIGlet font."
          : overlay.figletFontChoice
          ? overlay.autoGrow
            ? `${overlay.figletFontChoice} could not render this text; plain text is used.`
            : `${overlay.figletFontChoice} does not currently fit this box; enable Fit to text or resize the box. Plain text is used.`
          : "No hosted FIGlet font currently fits this box; plain text is used."
      : "";
    this.#syncDisabledFields();
  }

  #syncDisabledFields() {
    const figlet = this.#element("overlay-figlet").checked;
    this.#element("overlay-width").disabled = this.#element("overlay-auto-grow").checked && !(figlet && this.#element("overlay-wrap").checked);
    this.#element("overlay-height").disabled = this.#element("overlay-auto-grow").checked;
    this.#element("overlay-figlet-font").disabled = !figlet || this.#figletFonts.length === 0;
    this.#element("overlay-foreground").disabled = !this.#element("overlay-foreground-enabled").checked;
    this.#element("overlay-background").disabled = !this.#element("overlay-background-enabled").checked;
  }

  #invalidateArt(overlay) {
    overlay.renderedText = undefined;
    overlay.figletFont = undefined;
  }

  #updateSelected() {
    if (!this.#selected) return;
    const overlay = this.#selected;
    const previousArtKey = JSON.stringify([
      overlay.text, overlay.width, overlay.height, Boolean(overlay.figlet),
      overlay.figletFontChoice, overlay.autoGrow, overlay.wrap,
    ]);
    const wasFiglet = Boolean(overlay.figlet);
    overlay.text = this.#element("overlay-text").value;
    overlay.x = Number(this.#element("overlay-x").value);
    overlay.y = Number(this.#element("overlay-y").value);
    overlay.width = Number(this.#element("overlay-width").value);
    overlay.height = Number(this.#element("overlay-height").value);
    overlay.wrap = this.#element("overlay-wrap").checked;
    overlay.autoGrow = this.#element("overlay-auto-grow").checked;
    overlay.figlet = this.#element("overlay-figlet").checked;
    if (overlay.figlet) {
      overlay.figletFontChoice = this.#element("overlay-figlet-font").value
        || overlay.figletFontChoice
        || (this.#figletFonts.some((f) => f.name === overlay.figletFont) ? overlay.figletFont : this.#figletFonts[0]?.name);
    }
    if (overlay.figlet && !wasFiglet) {
      const availableWidth = Math.max(1, this.#columns - Math.max(0, overlay.x));
      const availableHeight = Math.max(1, this.#rows - Math.max(0, overlay.y));
      overlay.width = Math.max(overlay.width, Math.min(24, availableWidth));
      overlay.height = Math.max(overlay.height, Math.min(6, availableHeight));
    }
    overlay.bold = this.#element("overlay-bold").checked;
    overlay.italic = this.#element("overlay-italic").checked;
    overlay.underline = this.#element("overlay-underline").checked;
    overlay.foreground = this.#element("overlay-foreground-enabled").checked
      ? hexToRgb(this.#element("overlay-foreground").value)
      : undefined;
    overlay.background = this.#element("overlay-background-enabled").checked
      ? hexToRgb(this.#element("overlay-background").value)
      : undefined;
    const nextArtKey = JSON.stringify([
      overlay.text, overlay.width, overlay.height, Boolean(overlay.figlet),
      overlay.figletFontChoice, overlay.autoGrow, overlay.wrap,
    ]);
    if (previousArtKey !== nextArtKey) this.#invalidateArt(overlay);
    this.#render();
    this.#notifyOverlayChanged(overlay);
  }

  #overlayDimensions(overlay) {
    if (overlay.autoGrow) {
      const dimensions = textDimensions(overlay.figlet ? overlay.renderedText ?? overlay.text : overlay.text);
      return overlay.figlet && overlay.wrap ? { ...dimensions, width: Math.max(1, Number(overlay.width) || 1) } : dimensions;
    }
    return {
      width: Math.max(1, Number(overlay.width) || 1),
      height: Math.max(1, Number(overlay.height) || 1),
    };
  }

  #positionBox(box, geometry) {
    const dimensions = this.#overlayDimensions(geometry);
    box.style.left = `${geometry.x * 100 / this.#columns}%`;
    box.style.top = `${geometry.y * 100 / this.#rows}%`;
    box.style.width = `${dimensions.width * 100 / this.#columns}%`;
    box.style.height = `${dimensions.height * 100 / this.#rows}%`;
  }

  #renderLayer(draft) {
    const focused = this.#layer.contains(document.activeElement)
      ? document.activeElement.closest(".overlay-box")?.dataset.overlayKey : undefined;
    const entries = this.#entries();
    this.#layer.hidden = !this.#interactive || this.#columns < 1 || this.#rows < 1;
    if (this.#layer.hidden) {
      this.#layer.replaceChildren();
      return;
    }
    const boxes = entries.map(({ overlay, key }, index) => {
      const box = document.createElement("div");
      box.className = "overlay-box";
      box.dataset.overlayKey = key;
      box.dataset.kind = overlay.kind;
      box.role = "button";
      box.tabIndex = 0;
      box.setAttribute("aria-label", `${this.#entryLabel(overlay, index)}; drag to move`);
      box.setAttribute("aria-selected", String(overlay === this.#selected));
      this.#positionBox(box, overlay);
      for (const edge of ["n", "ne", "e", "se", "s", "sw", "w", "nw"]) {
        const handle = document.createElement("span");
        handle.className = "overlay-resize-handle";
        handle.dataset.edge = edge;
        handle.setAttribute("aria-hidden", "true");
        box.append(handle);
      }
      return box;
    });
    if (draft) {
      const box = document.createElement("div");
      box.className = "overlay-draft";
      this.#positionBox(box, draft);
      boxes.push(box);
    }
    this.#layer.replaceChildren(...boxes);
    if (focused) this.#layer.querySelector(`[data-overlay-key="${focused}"]`)?.focus({ preventScroll: true });
  }

  #beginPointerGesture(event) {
    if (!this.#interactive || event.button !== 0) return;
    const cell = this.#pointerCell(event);
    if (!cell) return;
    event.preventDefault();
    const boxes = [...this.#layer.querySelectorAll(".overlay-box")].reverse();
    const selectedKey = this.#keyForOverlay(this.#selected);
    const selectedBox = boxes.find((box) => box.dataset.overlayKey === selectedKey);
    const selectedEdges = selectedBox && resizeEdges(selectedBox.getBoundingClientRect(), event.clientX, event.clientY);
    const inside = boxes.find((box) => {
      const rect = box.getBoundingClientRect();
      return event.clientX >= rect.left && event.clientX < rect.right && event.clientY >= rect.top && event.clientY < rect.bottom;
    });
    const box = selectedEdges ? selectedBox : inside ?? boxes.find((box) => resizeEdges(box.getBoundingClientRect(), event.clientX, event.clientY));
    const edges = selectedEdges ?? (box && !inside ? resizeEdges(box.getBoundingClientRect(), event.clientX, event.clientY) : undefined);
    const overlay = box ? this.#overlayForKey(box.dataset.overlayKey) : undefined;
    if (overlay) {
      this.#selected = overlay;
      const dimensions = this.#overlayDimensions(overlay);
      this.#gesture = {
        mode: edges ? "resize" : "move",
        edges,
        pointerId: event.pointerId,
        start: cell,
        overlay,
        element: box,
        original: { x: overlay.x, y: overlay.y, ...dimensions },
        changed: false,
      };
      this.#renderEditor();
      this.#element("overlay-list").value = box.dataset.overlayKey;
      this.#layer.querySelectorAll(".overlay-box").forEach((candidate) => {
        candidate.setAttribute("aria-selected", String(candidate === box));
      });
    } else {
      if (cell.x < 0 || cell.y < 0 || cell.x >= this.#columns || cell.y >= this.#rows) return;
      const draft = document.createElement("div");
      draft.className = "overlay-draft";
      this.#positionBox(draft, { x: cell.x, y: cell.y, width: 1, height: 1 });
      this.#layer.append(draft);
      this.#gesture = {
        mode: "create",
        pointerId: event.pointerId,
        start: cell,
        current: cell,
        element: draft,
        changed: false,
      };
    }
    this.#layer.setPointerCapture(event.pointerId);
  }

  #continuePointerGesture(event) {
    const gesture = this.#gesture;
    if (!gesture || event.pointerId !== gesture.pointerId) return;
    const cell = this.#pointerCell(event);
    if (!cell) return;
    event.preventDefault();
    if (gesture.mode === "create") {
      gesture.current = cell;
      const x = Math.min(gesture.start.x, cell.x);
      const y = Math.min(gesture.start.y, cell.y);
      this.#positionBox(gesture.element, {
        x,
        y,
        width: Math.abs(cell.x - gesture.start.x) + 1,
        height: Math.abs(cell.y - gesture.start.y) + 1,
      });
      return;
    }

    const deltaX = cell.x - gesture.start.x;
    const deltaY = cell.y - gesture.start.y;
    if (gesture.mode === "move") {
      gesture.overlay.x = Math.max(0, Math.min(
        this.#columns - gesture.original.width,
        gesture.original.x + deltaX,
      ));
      gesture.overlay.y = Math.max(0, Math.min(
        this.#rows - gesture.original.height,
        gesture.original.y + deltaY,
      ));
    } else {
      gesture.overlay.autoGrow = false;
      Object.assign(gesture.overlay, resizeBox(gesture.original, gesture.edges, deltaX, deltaY));
      this.#invalidateArt(gesture.overlay);
    }
    gesture.changed = true;
    this.#positionBox(gesture.element, gesture.overlay);
  }

  #pointerCell(event) {
    const rect = this.#layer.getBoundingClientRect();
    if (!rect.width || !rect.height || !this.#columns || !this.#rows) return undefined;
    return { x: Math.floor((event.clientX - rect.left) * this.#columns / rect.width),
      y: Math.floor((event.clientY - rect.top) * this.#rows / rect.height) };
  }

  #finishPointerGesture(event) {
    const gesture = this.#gesture;
    if (!gesture || event.pointerId !== gesture.pointerId) return;
    if (this.#layer.hasPointerCapture(event.pointerId)) this.#layer.releasePointerCapture(event.pointerId);
    this.#gesture = undefined;
    if (gesture.mode === "create") {
      const current = gesture.current ?? gesture.start;
      const x = Math.min(gesture.start.x, current.x);
      const y = Math.min(gesture.start.y, current.y);
      const overlay = defaultOverlay({
        text: "",
        x,
        y,
        width: Math.abs(current.x - gesture.start.x) + 1,
        height: Math.abs(current.y - gesture.start.y) + 1,
        autoGrow: false,
      });
      this.#state.addManualOverlay(overlay);
      this.#selected = overlay;
      this.#render();
      this.#focusText();
      this.#changed();
    } else {
      this.#render();
      if (gesture.changed) this.#notifyOverlayChanged(gesture.overlay);
    }
  }

  #handleLayerKey(event) {
    const box = event.target.closest(".overlay-box");
    const overlay = box ? this.#overlayForKey(box.dataset.overlayKey) : undefined;
    if (!overlay) return;
    this.#selected = overlay;
    if (event.shiftKey && ["ArrowUp", "ArrowDown", "Home", "End"].includes(event.key)) return;
    if (event.key === "Delete" || event.key === "Backspace") {
      event.preventDefault();
      this.#deleteSelected();
      return;
    }
    const directions = {
      ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1],
    };
    const direction = directions[event.key];
    if (!direction) {
      if (event.key === "Enter") this.#focusText();
      return;
    }
    event.preventDefault();
    const dimensions = this.#overlayDimensions(overlay);
    overlay.x = Math.max(0, Math.min(this.#columns - dimensions.width, overlay.x + direction[0]));
    overlay.y = Math.max(0, Math.min(this.#rows - dimensions.height, overlay.y + direction[1]));
    this.#render();
    this.#notifyOverlayChanged(overlay);
  }

  #focusText() {
    queueMicrotask(() => {
      const input = this.#element("overlay-text");
      input.focus();
      input.select();
    });
  }
}
