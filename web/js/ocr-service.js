const SDK_URL = new URL("../vendor/paddleocr/index.js", import.meta.url);
const ORT_WASM_PATHS = {
  wasm: new URL("../vendor/ort/ort-wasm-simd-threaded.wasm", import.meta.url).href,
  mjs: new URL("../vendor/ort/ort-wasm-simd-threaded.js", import.meta.url).href,
};
const DETECTION_MODEL_URL = new URL(
  "../assets/ocr/PP-OCRv6_tiny_det_onnx_infer.tar",
  import.meta.url,
).href;
const RECOGNITION_MODEL_URL = new URL(
  "../assets/ocr/PP-OCRv6_tiny_rec_onnx_infer.tar",
  import.meta.url,
).href;

function median(values) {
  if (values.length === 0) return 0;
  const ordered = [...values].sort((left, right) => left - right);
  return ordered[Math.floor(ordered.length / 2)];
}

function boundsFor(poly, width, height) {
  if (!Array.isArray(poly) || poly.length < 4 || width < 1 || height < 1) return undefined;
  const points = poly.filter((point) => Array.isArray(point)
    && point.length >= 2 && Number.isFinite(point[0]) && Number.isFinite(point[1]));
  if (points.length < 4) return undefined;
  const xs = points.map((point) => point[0]);
  const ys = points.map((point) => point[1]);
  const x0 = Math.max(0, Math.min(width - 1, Math.floor(Math.min(...xs))));
  const y0 = Math.max(0, Math.min(height - 1, Math.floor(Math.min(...ys))));
  const x1 = Math.max(x0 + 1, Math.min(width, Math.ceil(Math.max(...xs))));
  const y1 = Math.max(y0 + 1, Math.min(height, Math.ceil(Math.max(...ys))));
  return { x0, y0, x1, y1, width: x1 - x0, height: y1 - y0 };
}

function printableAsciiRatio(text) {
  const characters = [...text];
  if (characters.length === 0) return 0;
  return characters.filter((character) => {
    const code = character.codePointAt(0);
    return code >= 32 && code <= 126;
  }).length / characters.length;
}

function filterItems(result, settings) {
  const image = result.image;
  let items = result.items
    .map((item) => ({
      ...item,
      text: item.text.trim(),
      bounds: boundsFor(item.poly, image.width, image.height),
    }))
    .filter((item) => item.bounds
      && item.text
      && item.score >= settings.minConfidence
      && printableAsciiRatio(item.text) >= settings.minAsciiRatio);

  if (settings.maxTextHeightRatio > 0 && items.length >= 2) {
    const normalHeight = median(items.map((item) => item.bounds.height));
    const maximum = normalHeight * settings.maxTextHeightRatio;
    items = items.filter((item) => item.bounds.height <= maximum);
  }
  return items;
}

function rgbaAt(raw, imageWidth, x, y) {
  const index = (y * imageWidth + x) * 4;
  return [raw[index], raw[index + 1], raw[index + 2], raw[index + 3]];
}

function colorDistanceSquared(left, right) {
  const red = left[0] - right[0];
  const green = left[1] - right[1];
  const blue = left[2] - right[2];
  return red * red + green * green + blue * blue;
}

function quantizedKey(pixel) {
  return `${pixel[0] >> 4},${pixel[1] >> 4},${pixel[2] >> 4}`;
}

function dominantColor(pixels) {
  const bins = new Map();
  for (const pixel of pixels) {
    const key = quantizedKey(pixel);
    const bin = bins.get(key) ?? { count: 0, sums: [0, 0, 0, 0] };
    bin.count += 1;
    for (let channel = 0; channel < 4; channel += 1) bin.sums[channel] += pixel[channel];
    bins.set(key, bin);
  }
  let winner;
  for (const bin of bins.values()) {
    if (!winner || bin.count > winner.count) winner = bin;
  }
  if (!winner) return [0, 0, 0, 255];
  return winner.sums.map((sum) => Math.round(sum / winner.count));
}

function regionAnalysis(raw, imageWidth, imageHeight, bounds) {
  const pad = Math.max(1, Math.min(3, Math.ceil(bounds.height * 0.08)));
  const erase = {
    x0: Math.max(0, bounds.x0 - pad),
    y0: Math.max(0, bounds.y0 - pad),
    x1: Math.min(imageWidth, bounds.x1 + pad),
    y1: Math.min(imageHeight, bounds.y1 + pad),
  };
  const ring = Math.max(3, Math.min(8, Math.floor(bounds.height / 4)));
  const sample = {
    x0: Math.max(0, erase.x0 - ring),
    y0: Math.max(0, erase.y0 - ring),
    x1: Math.min(imageWidth, erase.x1 + ring),
    y1: Math.min(imageHeight, erase.y1 + ring),
  };
  const backgroundPixels = [];
  for (let y = sample.y0; y < sample.y1; y += 1) {
    for (let x = sample.x0; x < sample.x1; x += 1) {
      const inside = x >= erase.x0 && x < erase.x1 && y >= erase.y0 && y < erase.y1;
      if (!inside) backgroundPixels.push(rgbaAt(raw, imageWidth, x, y));
    }
  }
  const background = dominantColor(backgroundPixels);
  const ringDistances = backgroundPixels.map((pixel) => colorDistanceSquared(pixel, background));
  ringDistances.sort((left, right) => left - right);
  const percentile = ringDistances.length
    ? ringDistances[Math.min(ringDistances.length - 1, Math.floor(ringDistances.length * 0.85))]
    : 18 * 18;
  const threshold = Math.max(18 * 18, Math.min(80 * 80, percentile * 4));
  const foregroundPixels = [];
  const maskWidth = erase.x1 - erase.x0;
  const maskHeight = erase.y1 - erase.y0;
  const mask = new Uint8Array(maskWidth * maskHeight);
  for (let y = erase.y0; y < erase.y1; y += 1) {
    for (let x = erase.x0; x < erase.x1; x += 1) {
      const pixel = rgbaAt(raw, imageWidth, x, y);
      if (colorDistanceSquared(pixel, background) < threshold) continue;
      mask[(y - erase.y0) * maskWidth + x - erase.x0] = 1;
      if (x >= bounds.x0 && x < bounds.x1 && y >= bounds.y0 && y < bounds.y1) {
        foregroundPixels.push(pixel);
      }
    }
  }

  const dilated = mask.slice();
  for (let y = 0; y < maskHeight; y += 1) {
    for (let x = 0; x < maskWidth; x += 1) {
      if (!mask[y * maskWidth + x]) continue;
      for (let adjacentY = Math.max(0, y - 1); adjacentY <= Math.min(maskHeight - 1, y + 1); adjacentY += 1) {
        for (let adjacentX = Math.max(0, x - 1); adjacentX <= Math.min(maskWidth - 1, x + 1); adjacentX += 1) {
          dilated[adjacentY * maskWidth + adjacentX] = 1;
        }
      }
    }
  }
  return {
    erase,
    mask: dilated,
    maskWidth,
    background,
    foreground: dominantColor(foregroundPixels).slice(0, 3),
  };
}

function eraseRegions(imageData, imageWidth, imageHeight, items) {
  const analyses = items.map((item) => ({
    item,
    analysis: regionAnalysis(imageData.data, imageWidth, imageHeight, item.bounds),
  }));
  for (const { analysis } of analyses) {
    const { erase, mask, maskWidth, background } = analysis;
    for (let y = erase.y0; y < erase.y1; y += 1) {
      for (let x = erase.x0; x < erase.x1; x += 1) {
        if (!mask[(y - erase.y0) * maskWidth + x - erase.x0]) continue;
        const index = (y * imageWidth + x) * 4;
        imageData.data.set(background, index);
      }
    }
  }
  return analyses.map(({ item, analysis }) => ({
    ...item,
    foreground: analysis.foreground,
  }));
}

async function canvasBlob(canvas) {
  if (typeof canvas.convertToBlob === "function") return canvas.convertToBlob({ type: "image/png" });
  return new Promise((resolve, reject) => canvas.toBlob(
    (blob) => blob ? resolve(blob) : reject(new Error("Could not encode the OCR background")),
    "image/png",
  ));
}

async function prepareBackground(blob, result, settings) {
  const items = filterItems(result, settings);
  if (items.length === 0) return { items, processedBytes: undefined };
  const bitmap = await createImageBitmap(blob);
  try {
    const canvas = typeof OffscreenCanvas === "function"
      ? new OffscreenCanvas(bitmap.width, bitmap.height)
      : Object.assign(document.createElement("canvas"), { width: bitmap.width, height: bitmap.height });
    const context = canvas.getContext("2d", { willReadFrequently: true });
    if (!context) throw new Error("A 2D canvas is required for OCR text removal");
    context.drawImage(bitmap, 0, 0);
    const imageData = context.getImageData(0, 0, bitmap.width, bitmap.height);
    const preparedItems = eraseRegions(imageData, bitmap.width, bitmap.height, items);
    context.putImageData(imageData, 0, 0);
    const processed = await canvasBlob(canvas);
    return { items: preparedItems, processedBytes: await processed.arrayBuffer() };
  } finally {
    bitmap.close();
  }
}

export async function ocrItemsToOverlays(
  prepared,
  columns,
  rows,
  figletSettings,
  renderFiglet,
) {
  const { result, items } = prepared;
  const imageWidth = Math.max(1, result.image.width);
  const imageHeight = Math.max(1, result.image.height);
  const scaled = items.map((item) => ({
    ...item,
    left: item.bounds.x0 * columns / imageWidth,
    right: item.bounds.x1 * columns / imageWidth,
    top: item.bounds.y0 * rows / imageHeight,
    bottom: item.bounds.y1 * rows / imageHeight,
  }));
  const occupied = new Set();
  const ordered = [...scaled].sort((left, right) => right.score - left.score);
  const heights = ordered.map((item) => item.bounds.height).sort((left, right) => left - right);
  const normalHeight = heights.length >= 3 ? median(heights) : 0;
  const requests = ordered.map((item) => {
    const nextLeft = scaled
      .filter((other) => other !== item
        && other.left >= item.right
        && Math.min(other.bottom, item.bottom) > Math.max(other.top, item.top))
      .reduce((limit, other) => Math.min(limit, other.left), columns);
    const ownWidth = Math.max(1, Math.floor(item.right - item.left + 1e-9));
    const maxWidth = Math.max(ownWidth, Math.floor(nextLeft - item.left + 1e-9), 1);
    const maxHeight = Math.max(1, Math.floor(item.bottom - item.top + 1e-9));
    const meetsAbsoluteHeight = maxHeight >= figletSettings.minHeight;
    const meetsRelativeHeight = figletSettings.minHeightRatio <= 0
      || normalHeight <= 0
      || item.bounds.height >= normalHeight * figletSettings.minHeightRatio;
    return {
      text: item.text,
      maxWidth,
      maxHeight,
      useFiglet: figletSettings.enabled && meetsAbsoluteHeight && meetsRelativeHeight,
    };
  });
  const figlet = await renderFiglet(requests);
  const overlays = [];
  for (let index = 0; index < ordered.length; index += 1) {
    const item = ordered[index];
    const art = figlet.arts[index];
    if (!art || art.width > columns || art.height > rows) continue;
    const x = Math.max(0, Math.min(columns - art.width, Math.round(item.left)));
    const centeredY = item.top + (item.bottom - item.top - art.height) * 0.5;
    const y = Math.max(0, Math.min(rows - art.height, Math.round(centeredY)));
    const cells = Array.from({ length: art.width * art.height }, (_, cellIndex) => {
      const cellX = x + cellIndex % art.width;
      const cellY = y + Math.floor(cellIndex / art.width);
      return `${cellX},${cellY}`;
    });
    if (cells.some((cell) => occupied.has(cell))) continue;
    cells.forEach((cell) => occupied.add(cell));
    overlays.push({
      kind: "ocr",
      text: art.text,
      x,
      y,
      width: art.width,
      height: art.height,
      foreground: item.foreground,
      background: undefined,
      wrap: false,
      autoGrow: false,
      bold: false,
      italic: false,
      underline: false,
      confidence: item.score,
      figletFont: art.source,
    });
  }
  return {
    overlays: overlays.sort((left, right) => left.y - right.y || left.x - right.x),
    figlet,
  };
}

export class OcrService {
  #ocrPromise;
  #cache = new WeakMap();

  async prepare(blob, settings, status = () => {}) {
    let entries = this.#cache.get(blob);
    if (!entries) {
      entries = new Map();
      this.#cache.set(blob, entries);
    }
    const key = JSON.stringify(settings);
    if (entries.has(key)) return { ...await entries.get(key), cacheHit: true, key };
    const promise = this.#prepare(blob, settings, status);
    entries.set(key, promise);
    try {
      return { ...await promise, cacheHit: false, key };
    } catch (error) {
      entries.delete(key);
      throw error;
    }
  }

  async #prepare(blob, settings, status) {
    status("Loading browser OCR…");
    const ocr = await this.#instance();
    status("Detecting text…");
    const started = performance.now();
    const [result] = await ocr.predict(blob, {
      textDetLimitSideLen: settings.maxSideLength,
      textDetLimitType: "max",
      textDetMaxSideLimit: settings.maxSideLength,
      textDetThresh: settings.boxThreshold,
      textDetBoxThresh: settings.boxScoreThreshold,
      textDetUnclipRatio: settings.unclipRatio,
      textRecScoreThresh: settings.minConfidence,
    });
    status("Removing detected text…");
    const background = await prepareBackground(blob, result, settings);
    return {
      result,
      ...background,
      elapsedMs: performance.now() - started,
    };
  }

  async #instance() {
    if (!this.#ocrPromise) {
      this.#ocrPromise = (async () => {
        const { PaddleOCR } = await import(SDK_URL.href);
        return PaddleOCR.create({
          worker: true,
          textDetectionModelName: "PP-OCRv6_tiny_det",
          textDetectionModelAsset: { url: DETECTION_MODEL_URL },
          textRecognitionModelName: "PP-OCRv6_tiny_rec",
          textRecognitionModelAsset: { url: RECOGNITION_MODEL_URL },
          textDetectionBatchSize: 1,
          textRecognitionBatchSize: 6,
          ortOptions: {
            backend: "wasm",
            wasmPaths: ORT_WASM_PATHS,
            numThreads: 1,
            simd: true,
          },
        });
      })().catch((error) => {
        this.#ocrPromise = undefined;
        throw error;
      });
    }
    return this.#ocrPromise;
  }

  async dispose() {
    if (!this.#ocrPromise) return;
    const ocr = await this.#ocrPromise.catch(() => undefined);
    await ocr?.dispose();
    this.#ocrPromise = undefined;
  }
}
