export function resizeEdges({ x, y, width, height }, px, py, margin = 14) {
  if ((px >= x && px < x + width && py >= y && py < y + height)
    || px < x - margin || px >= x + width + margin || py < y - margin || py >= y + height + margin) return undefined;
  return { left: px < x, right: px >= x + width, top: py < y, bottom: py >= y + height };
}

export function resizeBox({ x, y, width, height }, edges, dx, dy) {
  const left = edges.left ? Math.min(x + dx, x + width - 1) : x;
  const top = edges.top ? Math.min(y + dy, y + height - 1) : y;
  const right = edges.right ? Math.max(x + width + dx, left + 1) : x + width;
  const bottom = edges.bottom ? Math.max(y + height + dy, top + 1) : y + height;
  return { x: left, y: top, width: right - left, height: bottom - top };
}
