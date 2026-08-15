// Terminal glyph size is a display property, independent of the raster size
// used while matching glyphs in WASM.
const DEFAULT_DISPLAY_FONT_SIZE = 16;

function rgb(color, fallback = [0, 0, 0]) {
  const [red, green, blue] = color ?? fallback;
  return `rgb(${red} ${green} ${blue})`;
}

function displayStyle(cell) {
  const foreground = cell.inverted ? (cell.background ?? [0, 0, 0]) : cell.foreground;
  const background = cell.inverted ? cell.foreground : (cell.background ?? [0, 0, 0]);
  return {
    foreground,
    background,
    bold: cell.bold,
    italic: cell.italic,
    underline: cell.underline,
  };
}

function sameStyle(left, right) {
  return left.bold === right.bold
    && left.italic === right.italic
    && left.underline === right.underline
    && left.foreground.every((value, index) => value === right.foreground[index])
    && left.background.every((value, index) => value === right.background[index]);
}

export class OutputPreview {
  #canvas;
  #text;
  #placeholder;
  #mode = "bitmap";
  #displayFontSize = DEFAULT_DISPLAY_FONT_SIZE;
  #fontFamily = "monospace";
  #bitmapMetrics;
  #textMetrics;
  #hasText = false;
  #measurementContext = document.createElement("canvas").getContext("2d");

  constructor(canvas, text, placeholder) {
    this.#canvas = canvas;
    this.#text = text;
    this.#placeholder = placeholder;
  }

  setMode(mode) {
    this.#mode = mode === "text" ? "text" : "bitmap";
    this.#updateVisibility();
  }

  setFontFamily(family) {
    this.#fontFamily = family || "monospace";
    this.#text.style.fontFamily = `"${this.#fontFamily.replaceAll('"', '\\"')}", monospace`;
    if (this.#textMetrics) this.#applyTextDisplayMetrics(this.#textMetrics);
  }

  setOutputFontSize(value) {
    const next = Number(value);
    if (!Number.isFinite(next) || next <= 0) return;
    this.#displayFontSize = next;
    if (this.#bitmapMetrics) this.#applyBitmapDisplaySize(this.#bitmapMetrics);
    if (this.#textMetrics) this.#applyTextDisplayMetrics(this.#textMetrics);
  }

  draw(result) {
    this.#drawBitmap(result);
    if (result.cells) {
      this.#textMetrics = {
        columns: result.columns,
        rows: result.rows,
        renderFontSize: result.fontSize,
        cellAdvance: result.cellAdvance,
        lineHeight: result.lineHeight,
      };
      this.#applyTextDisplayMetrics(this.#textMetrics);
      this.#drawText(result.cells);
    }
    else {
      this.#text.replaceChildren();
      this.#textMetrics = undefined;
      this.#hasText = false;
    }
    this.#updateVisibility();
  }

  hasText() {
    return this.#hasText;
  }

  #drawBitmap(result) {
    if (!result.previewRgba?.length || !result.previewWidth || !result.previewHeight) {
      this.#canvas.width = 0;
      this.#canvas.height = 0;
      this.#bitmapMetrics = undefined;
      return;
    }

    const expectedBytes = result.previewWidth * result.previewHeight * 4;
    if (result.previewRgba.byteLength !== expectedBytes) {
      this.#canvas.width = 0;
      this.#canvas.height = 0;
      this.#bitmapMetrics = undefined;
      return;
    }

    this.#canvas.width = result.previewWidth;
    this.#canvas.height = result.previewHeight;
    const context = this.#canvas.getContext("2d", { alpha: false });
    context.imageSmoothingEnabled = false;
    const pixels = new Uint8ClampedArray(
      result.previewRgba.buffer,
      result.previewRgba.byteOffset,
      result.previewRgba.byteLength,
    );
    context.putImageData(new ImageData(pixels, result.previewWidth, result.previewHeight), 0, 0);

    this.#bitmapMetrics = {
      previewWidth: result.previewWidth,
      previewHeight: result.previewHeight,
      columns: result.columns,
      rows: result.rows,
      renderFontSize: result.fontSize,
      cellAdvance: result.cellAdvance,
      lineHeight: result.lineHeight,
    };
    this.#applyBitmapDisplaySize(this.#bitmapMetrics);
  }

  #drawText(rows) {
    const documentFragment = document.createDocumentFragment();
    for (const cells of rows) {
      const row = document.createElement("div");
      row.className = "output-text-row";
      let run;
      let runStyle;
      for (const cell of cells) {
        const style = displayStyle(cell);
        if (!run || !sameStyle(style, runStyle)) {
          run = document.createElement("span");
          run.className = "output-text-run";
          run.style.color = rgb(style.foreground, [255, 255, 255]);
          run.style.backgroundColor = rgb(style.background);
          run.style.fontWeight = style.bold ? "700" : "400";
          run.style.fontStyle = style.italic ? "italic" : "normal";
          run.style.textDecoration = style.underline ? "underline" : "none";
          row.append(run);
          runStyle = style;
        }
        run.append(document.createTextNode(cell.character));
      }
      documentFragment.append(row);
    }
    this.#text.replaceChildren(documentFragment);
    this.#hasText = rows.length > 0;
  }

  #applyBitmapDisplaySize(result) {
    const scale = result.renderFontSize > 0 ? this.#displayFontSize / result.renderFontSize : 1;
    const width = result.columns * result.cellAdvance * scale;
    const height = result.rows * result.lineHeight * scale;
    this.#canvas.style.width = `${width}px`;
    this.#canvas.style.height = `${height}px`;
  }

  #applyTextDisplayMetrics(result) {
    const scale = result.renderFontSize > 0 ? this.#displayFontSize / result.renderFontSize : 1;
    const cellAdvance = result.cellAdvance * scale;
    const lineHeight = result.lineHeight * scale;
    let naturalAdvance = cellAdvance;
    if (this.#measurementContext) {
      this.#measurementContext.font = `400 ${this.#displayFontSize}px "${this.#fontFamily.replaceAll('"', '\\"')}"`;
      naturalAdvance = this.#measurementContext.measureText("M").width;
    }
    this.#text.style.fontSize = `${this.#displayFontSize}px`;
    this.#text.style.letterSpacing = `${cellAdvance - naturalAdvance}px`;
    this.#text.style.lineHeight = `${lineHeight}px`;
    this.#text.style.width = `${result.columns * cellAdvance}px`;
    this.#text.style.height = `${result.rows * lineHeight}px`;
    this.#text.style.setProperty("--output-line-height", `${lineHeight}px`);
  }

  #updateVisibility() {
    const showText = this.#mode === "text" && this.#hasText;
    const showBitmap = this.#mode === "bitmap" && Boolean(this.#bitmapMetrics);
    this.#text.hidden = !showText;
    this.#canvas.hidden = !showBitmap;
    this.#placeholder.textContent = this.#mode === "text"
      ? "Render once to create the browser-text preview."
      : "Rendered output will appear here.";
    this.#placeholder.hidden = showText || showBitmap;
  }

  clear(message = "Rendered output will appear here.") {
    this.#bitmapMetrics = undefined;
    this.#textMetrics = undefined;
    this.#hasText = false;
    this.#canvas.width = 0;
    this.#canvas.height = 0;
    this.#text.replaceChildren();
    this.#canvas.hidden = true;
    this.#text.hidden = true;
    this.#placeholder.textContent = message;
    this.#placeholder.hidden = false;
  }

  // Future overlay tools should convert pointer coordinates through this
  // method so selection and drag logic stay independent of preview mode.
  clientPointToCell(clientX, clientY, columns, rows) {
    const target = this.#mode === "text" ? this.#text : this.#canvas;
    const bounds = target.getBoundingClientRect();
    return {
      x: Math.max(0, Math.min(columns - 1, Math.floor((clientX - bounds.left) * columns / bounds.width))),
      y: Math.max(0, Math.min(rows - 1, Math.floor((clientY - bounds.top) * rows / bounds.height))),
    };
  }
}
