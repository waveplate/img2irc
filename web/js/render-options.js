const value = (root, id) => root.querySelector(`#${id}`).value;
const checked = (root, id) => root.querySelector(`#${id}`).checked;
const number = (root, id) => Number(value(root, id));

const hexRgb = (hex) => [1, 3, 5].map((start) => parseInt(hex.slice(start, start + 2), 16));

function optionalPositiveInteger(root, id) {
  const input = value(root, id).trim();
  const parsed = Number(input);
  return input === "" || parsed === 0 ? undefined : parsed;
}

function lines(root, id) {
  return value(root, id).split(/[\s,]+/).map((item) => item.trim()).filter(Boolean);
}

function validateRanges(ranges) {
  const invalid = ranges.find((range) => !/^[0-9a-f]{1,6}-[0-9a-f]{1,6}$/i.test(range));
  if (invalid) throw new Error(`Invalid Unicode range "${invalid}"; use hexadecimal START-END`);
}

export function collectRenderOptions(root, glyphCatalog, overlays = []) {
  const includeRanges = lines(root, "include-ranges");
  const excludeRanges = lines(root, "exclude-ranges");
  validateRanges([...includeRanges, ...excludeRanges]);

  const braille = checked(root, "braille");
  const groups = [...root.querySelectorAll("#glyph-groups input:checked")]
    .map((input) => input.value);
  const include = value(root, "include");
  if (!braille && groups.length === 0 && include.length === 0 && includeRanges.length === 0) {
    throw new Error("Select at least one glyph group or provide custom included glyphs");
  }

  return {
    width: optionalPositiveInteger(root, "width"),
    height: optionalPositiveInteger(root, "height"),
    fontSize: number(root, "font-size"),
    render: value(root, "render"),
    braille,
    glyphs: glyphCatalog.characters(braille ? ["braille"] : groups),
    include: include || undefined,
    includeRanges,
    exclude: value(root, "exclude"),
    excludeRanges,
    filter: value(root, "filter"),
    scaleX: number(root, "scale-x"),
    scaleY: number(root, "scale-y"),
    rotate: number(root, "rotate"),
    flipHorizontal: checked(root, "flip-horizontal"),
    flipVertical: checked(root, "flip-vertical"),
    grayscaleTolerance: number(root, "grayscale-tolerance"),
    colourSpace: value(root, "colour-space"),
    brightness: number(root, "brightness"),
    contrast: number(root, "contrast"),
    lumaContrast: number(root, "luma-contrast"),
    medianBlur: number(root, "median-blur"),
    lineThickness: number(root, "line-thickness"),
    lightLines: checked(root, "light-lines"),
    replaceFrom: checked(root, "replace-enabled") ? hexRgb(value(root, "replace-from")) : undefined,
    replaceTo: hexRgb(value(root, "replace-to")),
    replaceTolerance: number(root, "replace-tolerance"),
    gamma: number(root, "gamma"),
    saturation: number(root, "saturation"),
    hue: number(root, "hue"),
    invert: checked(root, "invert"),
    dither: number(root, "dither"),
    grayscale: checked(root, "grayscale"),
    pixelize: number(root, "pixelize"),
    boxBlur: checked(root, "box-blur"),
    gaussianBlur: number(root, "gaussian-blur"),
    smooth: root.querySelector("#smoothing-section").open,
    smoothCandidates: number(root, "smooth-candidates"),
    smoothOrders: number(root, "smooth-orders"),
    smoothShapes: checked(root, "smooth-shapes"),
    smoothNeighbors: checked(root, "smooth-neighbors"),
    // Both previews consume styled cells. Native mode builds fixed DOM cells;
    // bitmap mode lets the browser rasterize the same font with antialiasing.
    includeCells: true,
    overlays,
  };
}

export function updateRangeOutput(input) {
  const suffix = input.id === "rotate" ? "°" : "";
  input.nextElementSibling.value = `${input.value}${suffix}`;
}

export function resetAdjustments(root) {
  root.querySelector("#replace-from").value = "#ffffff";
  root.querySelector("#replace-to").value = "#000000";
  const defaults = {
    "luma-contrast": 0, "median-blur": 0, "line-thickness": 0, "replace-tolerance": 5,
    brightness: 0, contrast: 0, gamma: 0, saturation: 0, hue: 0, rotate: 0,
    "scale-x": 1, "scale-y": 1, pixelize: 0, "gaussian-blur": 0, dither: 0,
  };
  for (const [id, defaultValue] of Object.entries(defaults)) {
    const input = root.querySelector(`#${id}`);
    input.value = defaultValue;
    updateRangeOutput(input);
  }
  for (const id of ["flip-horizontal", "flip-vertical", "invert", "grayscale", "box-blur", "light-lines", "replace-enabled"]) {
    root.querySelector(`#${id}`).checked = false;
  }
}
