const IRC99 = [
  0xffffff, 0x000000, 0x00007f, 0x009300, 0xff0000, 0x7f0000, 0x9c009c, 0xfc7f00,
  0xffff00, 0x00fc00, 0x009393, 0x00ffff, 0x0000fc, 0xff00ff, 0x555555, 0xaaaaaa,
  0x470000, 0x472100, 0x474700, 0x324700, 0x004700, 0x00472c, 0x004747, 0x002747,
  0x000047, 0x2e0047, 0x470047, 0x47002a, 0x740000, 0x743a00, 0x747400, 0x517400,
  0x007400, 0x007449, 0x007474, 0x004074, 0x000074, 0x4b0074, 0x740074, 0x740045,
  0xb50000, 0xb56300, 0xb5b500, 0x7db500, 0x00b500, 0x00b571, 0x00b5b5, 0x0063b5,
  0x0000b5, 0x7500b5, 0xb500b5, 0xb5006b, 0xff0000, 0xff8c00, 0xffff00, 0xb2ff00,
  0x00ff00, 0x00ffa0, 0x00ffff, 0x008cff, 0x0000ff, 0xa500ff, 0xff00ff, 0xff0098,
  0xff5959, 0xffb459, 0xffff71, 0xcfff60, 0x6fff6f, 0x65ffc9, 0x6dffff, 0x59b4ff,
  0x5959ff, 0xc459ff, 0xff66ff, 0xff59bc, 0xff9c9c, 0xffd39c, 0xffff9c, 0xe2ff9c,
  0x9cff9c, 0x9cffdb, 0x9cffff, 0x9cd3ff, 0x9c9cff, 0xdc9cff, 0xff9cff, 0xff94d3,
  0x000000, 0x131313, 0x282828, 0x363636, 0x4d4d4d, 0x656565, 0x818181, 0x9f9f9f,
  0xbcbcbc, 0xe2e2e2, 0xffffff,
];
const IRC_EXTENDED = IRC99.slice(16);

const ANSI_BASE = [
  0x000000, 0x800000, 0x008000, 0x808000, 0x000080, 0x800080, 0x008080, 0xc0c0c0,
  0x808080, 0xff0000, 0x00ff00, 0xffff00, 0x0000ff, 0xff00ff, 0x00ffff, 0xffffff,
];
const ANSI_LEVELS = [0, 95, 135, 175, 215, 255];
const ANSI_GRAYS = Array.from({ length: 24 }, (_, index) => 8 + index * 10);
const ANSI256 = [
  ...ANSI_BASE,
  ...ANSI_LEVELS.flatMap((red) => ANSI_LEVELS.flatMap((green) => (
    ANSI_LEVELS.map((blue) => (red << 16) | (green << 8) | blue)
  ))),
  ...ANSI_GRAYS.map((value) => {
    return (value << 16) | (value << 8) | value;
  }),
];

function components(colour) {
  return [(colour >> 16) & 0xff, (colour >> 8) & 0xff, colour & 0xff];
}

function distanceSquared(left, right) {
  return left.reduce((sum, channel, index) => sum + (channel - right[index]) ** 2, 0);
}

function nearestLevel(value, levels) {
  return levels.reduce((best, level) => (
    Math.abs(level - value) < Math.abs(best - value) ? level : best
  ), levels[0]);
}

function quantizeAnsi(rgb) {
  // The 6×6×6 cube is separable, and the grayscale ramp is uniform, so only
  // their nearest candidates plus the 16 base colours need comparison.
  let best = components(ANSI_BASE[0]);
  let bestDistance = distanceSquared(best, rgb);
  for (const colour of ANSI_BASE.slice(1)) {
    const candidate = components(colour);
    const distance = distanceSquared(candidate, rgb);
    if (distance < bestDistance) {
      best = candidate;
      bestDistance = distance;
    }
  }
  const cube = rgb.map((channel) => nearestLevel(channel, ANSI_LEVELS));
  const cubeDistance = distanceSquared(cube, rgb);
  if (cubeDistance < bestDistance) {
    best = cube;
    bestDistance = cubeDistance;
  }
  const mean = rgb.reduce((sum, channel) => sum + channel, 0) / 3;
  const gray = nearestLevel(mean, ANSI_GRAYS);
  const grayscale = [gray, gray, gray];
  if (distanceSquared(grayscale, rgb) < bestDistance) best = grayscale;
  return best;
}

export function paletteForRender(render) {
  if (render === "irc") return IRC_EXTENDED;
  if (render === "ansi") return ANSI256;
  return undefined;
}

export function quantizeRgb(rgb, render) {
  if (render === "ansi") return quantizeAnsi(rgb);
  const palette = paletteForRender(render);
  if (!palette) return [...rgb];
  let best = palette[0];
  let bestDistance = Number.POSITIVE_INFINITY;
  for (const colour of palette) {
    const candidate = components(colour);
    const distance = distanceSquared(candidate, rgb);
    if (distance < bestDistance) {
      best = colour;
      bestDistance = distance;
      if (distance === 0) break;
    }
  }
  return components(best);
}

export function quantizeImageData(imageData, render) {
  const palette = paletteForRender(render);
  if (!palette) return imageData;
  const cache = new Map();
  for (let offset = 0; offset < imageData.data.length; offset += 4) {
    if (imageData.data[offset + 3] === 0) continue;
    const packed = imageData.data[offset] << 16
      | imageData.data[offset + 1] << 8
      | imageData.data[offset + 2];
    let visible = cache.get(packed);
    if (!visible) {
      visible = quantizeRgb([
        imageData.data[offset], imageData.data[offset + 1], imageData.data[offset + 2],
      ], render);
      cache.set(packed, visible);
    }
    imageData.data.set(visible, offset);
  }
  return imageData;
}
