import { OutputPreview as TextPreview, displayStyle, rgbToString } from "../vendor/irc2html.js";

function paintRaster(canvas, result, fontFamily) {
  canvas.width = result.previewWidth;
  canvas.height = result.previewHeight;
  const rgba = result.previewRgba;
  const pixels = new Uint8ClampedArray(rgba.buffer, rgba.byteOffset, rgba.byteLength);
  const ctx = canvas.getContext("2d");
  ctx.putImageData(new ImageData(pixels, canvas.width, canvas.height), 0, 0);
  if (!result.previewTextCells?.length) return;

  // Text overlays can contain styled glyphs and use the browser's fallback fonts.
  const { cellWidth, cellHeight, columns, fontSize } = result;
  ctx.font = `400 ${fontSize}px ${fontFamily}`;
  const font = ctx.measureText("M");
  const ascent = font.fontBoundingBoxAscent ?? fontSize;
  const descent = font.fontBoundingBoxDescent ?? 0;
  const scaleX = cellWidth / (font.width || cellWidth);
  const scaleY = cellHeight / (ascent + descent || cellHeight);
  ctx.textBaseline = "alphabetic";
  ctx.fontKerning = "none";
  for (const index of result.previewTextCells) {
    const x = index % columns;
    const y = Math.floor(index / columns);
    const cell = result.cells[y][x];
    const style = displayStyle(cell);
    const left = x * cellWidth;
    const top = y * cellHeight;
    ctx.fillStyle = rgbToString(style.background);
    ctx.fillRect(left, top, cellWidth, cellHeight);
    ctx.save();
    ctx.beginPath();
    ctx.rect(left, top, cellWidth, cellHeight);
    ctx.clip();
    ctx.fillStyle = rgbToString(style.foreground);
    if (style.underline) {
      const line = Math.min(cellHeight - 1, Math.ceil((ascent + 1) * scaleY));
      ctx.fillRect(left, top + line, cellWidth, 1);
    }
    ctx.setTransform(scaleX, 0, 0, scaleY, left, top);
    ctx.font = `${style.italic ? "italic " : ""}${style.bold ? 700 : 400} ${fontSize}px ${fontFamily}`;
    ctx.fillText(cell.character, 0, ascent);
    ctx.restore();
  }
}

export class OutputPreview extends TextPreview {
  #canvas;
  #text;
  #raster;
  #rasterResult;
  #mode;
  #fontSize = 16;

  constructor(stage, canvas, text, placeholder) {
    super(stage, canvas, text, placeholder);
    this.#canvas = canvas;
    this.#text = text;
    this.#mode = super.getMode();
    this.#raster = document.createElement("canvas");
    this.#raster.className = "output-dither-canvas";
    this.#raster.hidden = true;
    this.#raster.setAttribute("role", "img");
    this.#raster.setAttribute("aria-label", "Rendered Dither artwork");
    text.before(this.#raster);
  }

  get canvas() { return this.#rasterResult ? this.#raster : this.#canvas; }

  getMode() { return this.#mode; }

  setMode(mode) {
    this.#mode = mode === "bitmap" ? "bitmap" : "text";
    if (!this.#rasterResult) super.setMode(this.#mode);
    this.#updateRasterVisibility();
  }

  setOutputFontSize(value) {
    const size = Number(value);
    if (size > 0 && Number.isFinite(size)) this.#fontSize = size;
    super.setOutputFontSize(value);
  }

  draw(result, options) {
    if (result?.previewRgba?.length) {
      if (super.getMode() !== "text") super.setMode("text");
      // Keep the raster and selectable grid aligned at the bitmap's cell size.
      // Fractional font advances can resample a one-pixel stipple into bands.
      super.draw({ ...result, cellAdvance: result.cellWidth, lineHeight: result.cellHeight }, options);
      if (this.#rasterResult !== result) paintRaster(this.#raster, result, this.#text.style.fontFamily);
      this.#rasterResult = result;
    } else {
      this.#clearRaster();
      super.draw(result, options);
      if (super.getMode() !== this.#mode) super.setMode(this.#mode);
    }
    this.#updateRasterVisibility();
  }

  clear(message) {
    this.#clearRaster();
    this.#updateRasterVisibility();
    super.clear(message);
    super.setMode(this.#mode);
  }

  toPngBlob(result = this.#rasterResult) {
    if (!result?.previewRgba?.length) return super.toPngBlob(result);
    const source = result === this.#rasterResult ? this.#raster : document.createElement("canvas");
    if (source !== this.#raster) paintRaster(source, result, this.#text.style.fontFamily);
    const scale = this.#fontSize / result.fontSize * Math.max(1, globalThis.devicePixelRatio || 1);
    const canvas = document.createElement("canvas");
    canvas.width = Math.max(1, Math.round(source.width * scale));
    canvas.height = Math.max(1, Math.round(source.height * scale));
    const ctx = canvas.getContext("2d");
    ctx.imageSmoothingEnabled = false;
    ctx.drawImage(source, 0, 0, canvas.width, canvas.height);
    return new Promise((resolve, reject) => canvas.toBlob(
      (blob) => blob ? resolve(blob) : reject(new Error("Could not create PNG")), "image/png",
    ));
  }

  #clearRaster() {
    if (!this.#rasterResult) return;
    this.#rasterResult = undefined;
    this.#raster.width = this.#raster.height = 0;
  }

  #updateRasterVisibility() {
    const raster = Boolean(this.#rasterResult);
    const selectable = this.#mode === "text";
    this.#raster.hidden = !raster;
    this.#raster.setAttribute("aria-hidden", String(selectable));
    this.#text.classList.toggle("output-text-over-raster", raster);
    this.#text.classList.toggle("output-text-bitmap-mode", raster && !selectable);
  }
}
