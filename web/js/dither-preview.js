function stippleSamples(start, end, sourceSize, period) {
  const samples = new Uint32Array(end - start);
  const scale = sourceSize / samples.length;
  for (let i = 0; i < samples.length; i++) {
    const phase = (start + i) % period;
    const source = (i + 0.5) * scale - 0.5;
    const tile = Math.round((source - phase) / period);
    const lastTile = Math.floor((sourceSize - 1 - phase) / period);
    samples[i] = phase + period * Math.max(0, Math.min(lastTile, tile));
  }
  return samples;
}

export function resizeDitherRaster(canvas, source, result, width, height) {
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext("2d");
  const wholePixels = width % source.width === 0 && height % source.height === 0;
  ctx.imageSmoothingEnabled = !wholePixels;
  ctx.imageSmoothingQuality = "high";
  ctx.drawImage(source, 0, 0, width, height);
  if (wholePixels || !result.previewStipples?.length || !result.cells?.length) return;

  const stipples = new Map(result.previewStipples);
  const textCells = new Set(result.previewTextCells);
  const { cellWidth, cellHeight, columns, rows, previewRgba } = result;
  const original = new Uint32Array(previewRgba.buffer, previewRgba.byteOffset, previewRgba.byteLength / 4);
  const raster = ctx.getImageData(0, 0, width, height);
  const scaled = new Uint32Array(raster.data.buffer);

  // A stippled region has a fixed phase on the device-pixel grid. Find the
  // closest sample in the source cell with the corresponding tile phase.
  for (let row = 0; row < rows; row++) {
    const top = Math.round(row * height / rows);
    const bottom = Math.round((row + 1) * height / rows);
    const sourceRow = row * cellHeight * source.width;
    for (let col = 0; col < columns; col++) {
      const period = stipples.get(result.cells[row][col].character);
      if (!period || period > Math.min(cellWidth, cellHeight) || textCells.has(row * columns + col)) continue;
      const left = Math.round(col * width / columns);
      const right = Math.round((col + 1) * width / columns);
      const sampleX = stippleSamples(left, right, cellWidth, period);
      const sampleY = stippleSamples(top, bottom, cellHeight, period);
      const origin = sourceRow + col * cellWidth;
      for (let y = 0; y < sampleY.length; y++) {
        const sourceStart = origin + sampleY[y] * source.width;
        const targetStart = (top + y) * width + left;
        for (let x = 0; x < sampleX.length; x++) {
          scaled[targetStart + x] = original[sourceStart + sampleX[x]];
        }
      }
    }
  }
  ctx.putImageData(raster, 0, 0);
}
