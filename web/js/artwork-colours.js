import { displayStyle } from "../vendor/irc2html.js";
import { quantizeImageData } from "./output-palettes.js";

export const colourHex = (rgb) => `#${rgb.map((c) => c.toString(16).padStart(2, "0")).join("")}`;

export function visiblePalette(result) {
  const counts = new Map();
  const count = (rgb, weight = 1) => {
    const key = colourHex(rgb);
    counts.set(key, (counts.get(key) ?? 0) + weight);
  };
  if (result?.cells?.length) {
    // Use the actual art colours, excluding shades introduced by glyph antialiasing.
    for (const row of result.cells) {
      for (const cell of row) {
        const style = displayStyle(cell, [255, 255, 255], [0, 0, 0]);
        if (cell.background != null || cell.inverted) count(style.background);
        if (!style.hidden && cell.character?.trim()) count(style.foreground);
      }
    }
  } else {
    const pixels = result?.previewRgba ?? [];
    for (let i = 0; i < pixels.length; i += 4) {
      if (pixels[i + 3]) count([pixels[i], pixels[i + 1], pixels[i + 2]], pixels[i + 3]);
    }
  }
  return [...counts].sort(([a, na], [b, nb]) => nb - na || a.localeCompare(b)).slice(0, 256).map(([colour]) => colour);
}

export function sampleRenderedArtwork(result, preview, canvas, renderMode) {
  if (!result) return undefined;
  // Show the bitmap while reading the canvas so picking also works in text
  // mode and after restoring a saved session.
  const mode = preview.getMode();
  try {
    preview.setMode("bitmap");
    if (!canvas.width || !canvas.height) return undefined;
    const pixels = canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height);
    quantizeImageData(pixels, renderMode);
    return { ...result, previewWidth: canvas.width, previewHeight: canvas.height, previewRgba: pixels.data };
  } finally {
    if (mode !== "bitmap") preview.setMode(mode);
  }
}
