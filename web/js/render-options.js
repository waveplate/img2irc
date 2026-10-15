import { collectExtraEffects } from "./effect-catalog.js";

const value = (root, id) => root.querySelector(`#${id}`).value;
const disabledEffect = (input) => Boolean(input.closest?.('.pipeline-effect[data-disabled="true"]'));
const checked = (root, id) => {
  const input = root.querySelector(`#${id}`);
  return !disabledEffect(input) && input.checked;
};
const number = (root, id) => {
  const input = root.querySelector(`#${id}`);
  return Number(disabledEffect(input) ? input.dataset?.neutralValue ?? 0 : input.value);
};

const hexRgb = (hex) => {
  if (!/^#[0-9a-f]{6}$/i.test(hex)) throw new Error(`Invalid colour "${hex}"; use #RRGGBB`);
  return [1, 3, 5].map((start) => parseInt(hex.slice(start, start + 2), 16));
};

export function parsePalette(text) {
  const unique = [...new Set(text.trim().split(/[\s,;]+/).filter(Boolean).map((item) => item.replace(/^#/, "").toLowerCase()))];
  if (unique.length > 256) throw new Error("Use at most 256 palette colours");
  return unique.map((item) => hexRgb(`#${item}`));
}

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
    pipelineOrder: [...root.querySelectorAll('#pipeline-effects > .pipeline-effect:not([hidden])[data-disabled="false"]')]
      .map((node) => node.dataset.pipelineEffect),
    extraEffects: collectExtraEffects(root),
    cropTop: number(root, "crop-top"), cropRight: number(root, "crop-right"),
    cropBottom: number(root, "crop-bottom"), cropLeft: number(root, "crop-left"),
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
    replaceTo: checked(root, "replace-enabled") ? hexRgb(value(root, "replace-to")) : [0, 0, 0],
    replaceTolerance: number(root, "replace-tolerance"),
    quantizeColours: checked(root, "quantize-enabled") ? number(root, "quantize-colours") : 0,
    quantizeEnabled: checked(root, "quantize-enabled"),
    quantizeThreshold: number(root, "quantize-threshold"),
    quantizePalette: checked(root, "quantize-enabled") ? parsePalette(value(root, "quantize-palette")) : [],
    quantizeSource: checked(root, "quantize-source"),
    gamma: number(root, "gamma"),
    saturation: number(root, "saturation"),
    hue: number(root, "hue"),
    invert: checked(root, "invert"),
    dither: number(root, "dither"),
    grayscale: checked(root, "grayscale"),
    pixelize: number(root, "pixelize"),
    boxBlur: checked(root, "box-blur"),
    gaussianBlur: number(root, "gaussian-blur"),
    smooth: checked(root, "smooth"),
    perceptualScore: checked(root, "perceptual-score"),
    glyphSearch: checked(root, "glyph-pair-search"),
    discontinuityThreshold: number(root, "discontinuity-threshold"),
    glyphSearchCandidates: number(root, "glyph-search-candidates"),
    glyphSearchPasses: number(root, "glyph-search-passes"),
    glyphSearchFinalists: number(root, "glyph-search-finalists"),
    smoothCandidates: number(root, "smooth-candidates"),
    smoothOrders: number(root, "smooth-orders"),
    smoothShapes: checked(root, "smooth-shapes"),
    smoothNeighbors: checked(root, "smooth-neighbors"),
    // Both previews consume styled cells. Native mode builds fixed DOM cells;
    // bitmap mode lets the browser rasterize the same font with antialiasing.
    includeCells: true,
    livePreview: true,
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
  root.querySelector("#quantize-colours").value = 16;
  root.querySelector("#quantize-colours").nextElementSibling.value = "16";
  root.querySelector("#quantize-palette").value = "";
  root.querySelector("#quantize-swatches").replaceChildren();
  root.querySelector("#quantize-source").checked = true;
  const defaults = {
    "luma-contrast": 0, "median-blur": 0, "line-thickness": 0, "replace-tolerance": 5, "quantize-threshold": 3,
    brightness: 0, contrast: 0, gamma: 0, saturation: 0, hue: 0, rotate: 0,
    "scale-x": 1, "scale-y": 1, pixelize: 0, "gaussian-blur": 0, dither: 0,
    "crop-top": 0, "crop-right": 0, "crop-bottom": 0, "crop-left": 0,
  };
  for (const [id, defaultValue] of Object.entries(defaults)) {
    const input = root.querySelector(`#${id}`);
    input.value = defaultValue;
    if (input.type === "range") updateRangeOutput(input);
  }
  for (const id of ["flip-horizontal", "flip-vertical", "invert", "grayscale", "box-blur", "light-lines", "replace-enabled", "quantize-enabled", "glyph-pair-search"]) {
    root.querySelector(`#${id}`).checked = false;
  }
}
