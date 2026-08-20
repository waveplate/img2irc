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

function stat(label, value, prominent = false) {
  const item = document.createElement("span");
  item.className = prominent ? "render-stat render-stat-prominent" : "render-stat";
  const heading = document.createElement("strong");
  heading.textContent = `${label}:`;
  item.append(heading, ` ${value}`);
  return item;
}

export class RenderStatus {
  #container;
  #showDetails = false;
  #lastResult;
  #lastElapsedMs;

  constructor(container) {
    this.#container = container;
  }

  update(result, elapsedMs) {
    this.#lastResult = result;
    this.#lastElapsedMs = elapsedMs;
    this.#render();
  }

  #render() {
    const result = this.#lastResult;
    const elapsedMs = this.#lastElapsedMs;
    if (!result) return;

    const primaryItems = [
      stat("Grid", `${result.columns} × ${result.rows}`),
      stat("Raster", `${result.previewWidth} × ${result.previewHeight} px`),
      stat("Cell", `${result.cellWidth} × ${result.cellHeight} px`),
      stat("Longest", `line ${longestLineNumber(result.content)} · ${result.longestLineBytes} bytes`, true),
      stat("Contour", result.score.toFixed(3), true),
    ];

    const toggleBtn = document.createElement("button");
    toggleBtn.type = "button";
    toggleBtn.className = "render-stats-toggle";
    toggleBtn.textContent = this.#showDetails ? "Hide stats ▴" : "More stats ▾";
    toggleBtn.addEventListener("click", () => {
      this.#showDetails = !this.#showDetails;
      this.#render();
    });

    const items = [...primaryItems, toggleBtn];

    if (this.#showDetails) {
      const detailsContainer = document.createElement("div");
      detailsContainer.className = "render-stats-details";

      const detailStats = [
        stat("Advance", `${result.cellAdvance.toFixed(1)} px`),
        stat("Line height", `${result.lineHeight.toFixed(1)} px`),
        stat("Time", `${Math.round(elapsedMs)} ms`),
      ];

      const timings = result.timings;
      if (timings) {
        detailStats.push(
          stat("Prepare", timings.canvasCacheHit ? "cached" : `${timings.prepareCanvasMs.toFixed(1)} ms`),
          stat("Match", timings.resultCacheHit ? "cached" : `${timings.glyphMatchMs.toFixed(1)} ms`),
          stat("Smooth prep", `${timings.smoothingPrepareMs.toFixed(1)} ms`),
          stat("Smooth", `${timings.smoothingSearchMs.toFixed(1)} ms`),
          stat("Shapes", `${timings.shapeRefineMs.toFixed(1)} ms`),
          stat("Score", `${timings.finalScoreMs.toFixed(1)} ms`),
          stat("Encode", `${timings.encodeMs.toFixed(1)} ms`),
          stat("Text", `${timings.overlayMs.toFixed(1)} ms`),
          stat("Preview", `${timings.previewMs.toFixed(1)} ms`),
          stat("UI/transfer", `${Math.max(0, elapsedMs - (result.workerRenderMs ?? elapsedMs)).toFixed(1)} ms`),
          stat(
            "Contour cache",
            `${timings.contourCacheHits}/${timings.contourCacheHits + timings.contourCacheMisses}`,
          ),
        );
      }
      detailsContainer.append(...detailStats);
      items.push(detailsContainer);
    }

    this.#container.replaceChildren(...items);
  }
}
