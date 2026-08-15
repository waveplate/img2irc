const encoder = new TextEncoder();

function longestLineNumber(content) {
  let bestIndex = 0;
  let bestLength = -1;
  content.split("\n").forEach((line, index) => {
    const length = encoder.encode(line).length;
    if (length > bestLength) {
      bestIndex = index;
      bestLength = length;
    }
  });
  return bestIndex + 1;
}

function stat(label, value) {
  const item = document.createElement("span");
  item.className = "render-stat";
  const heading = document.createElement("strong");
  heading.textContent = `${label}:`;
  item.append(heading, ` ${value}`);
  return item;
}

export class RenderStatus {
  #container;

  constructor(container) {
    this.#container = container;
  }

  update(result, elapsedMs) {
    const items = [
      stat("Grid", `${result.columns} × ${result.rows}`),
      stat("Raster", `${result.previewWidth} × ${result.previewHeight} px`),
      stat("Cell", `${result.cellWidth} × ${result.cellHeight} px`),
      stat("Advance", `${result.cellAdvance.toFixed(1)} px`),
      stat("Line height", `${result.lineHeight.toFixed(1)} px`),
      stat("Longest", `line ${longestLineNumber(result.content)} · ${result.longestLineBytes} bytes`),
      stat("Contour", result.score.toFixed(3)),
      stat("Time", `${Math.round(elapsedMs)} ms`),
    ];
    const timings = result.timings;
    if (timings) {
      items.push(
        stat("Prepare", timings.canvasCacheHit ? "cached" : `${timings.prepareCanvasMs.toFixed(1)} ms`),
        stat("Match", `${timings.glyphMatchMs.toFixed(1)} ms`),
        stat("Smooth prep", `${timings.smoothingPrepareMs.toFixed(1)} ms`),
        stat("Smooth", `${timings.smoothingSearchMs.toFixed(1)} ms`),
        stat("Shapes", `${timings.shapeRefineMs.toFixed(1)} ms`),
        stat("Score", `${timings.finalScoreMs.toFixed(1)} ms`),
        stat("Encode", `${timings.encodeMs.toFixed(1)} ms`),
        stat("Preview", `${timings.previewMs.toFixed(1)} ms`),
        stat("UI/transfer", `${Math.max(0, elapsedMs - (result.workerRenderMs ?? elapsedMs)).toFixed(1)} ms`),
        stat(
          "Contour cache",
          `${timings.contourCacheHits}/${timings.contourCacheHits + timings.contourCacheMisses}`,
        ),
      );
    }
    this.#container.replaceChildren(...items);
  }
}
