import { collectPipelineEffects } from "./effect-catalog.js";

export { parsePalette } from "./effect-catalog.js";

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

function wholeNumber(root, id, minimum, label) {
  const input = value(root, id).trim();
  const result = Number(input);
  if (!input || !Number.isInteger(result) || result < minimum || result > 0xffffffff) {
    throw new Error(`${label} must be a whole number of at least ${minimum}`);
  }
  return result;
}

export function syncRenderModeControls(root) {
  const mode = value(root, "render-mode");
  const dither = mode === "dither";
  const glyph = mode === "glyph";
  const braille = mode === "braille";
  root.querySelector('#render option[value="ansi24"]').disabled = dither;
  root.querySelector('#render-mode option[value="dither"]').disabled =
    value(root, "render") === "ansi24" || checked(root, "foreground-only");
  root.querySelector("#foreground-only").disabled = dither;
  for (const label of root.querySelectorAll("[data-dither-control]")) {
    label.hidden = !dither;
    label.querySelector("input").disabled = !dither;
  }
  for (const control of root.querySelectorAll("#glyph-controls input, #glyph-controls textarea, #glyph-controls button")) {
    control.disabled = braille;
  }
  root.querySelector("#smooth").disabled = braille;
  root.querySelector("#glyph-pair-search").disabled = !glyph;
  root.querySelector("#perceptual-score").disabled = !glyph;
}

export function collectRenderOptions(root, glyphCatalog, overlays = []) {
  const includeRanges = lines(root, "include-ranges");
  const excludeRanges = lines(root, "exclude-ranges");
  validateRanges([...includeRanges, ...excludeRanges]);

  const renderMode = value(root, "render-mode");
  const render = value(root, "render");
  const foregroundOnly = checked(root, "foreground-only");
  const braille = renderMode === "braille";
  const dither = renderMode === "dither";
  if (dither && (render === "ansi24" || foregroundOnly)) {
    throw new Error("Dither needs IRC99 or ANSI 256 and both foreground and background colours");
  }
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
    render,
    renderMode,
    foregroundOnly,
    ...(dither ? {
      ditherDetail: wholeNumber(root, "dither-detail", 1, "Dither Detail"),
      ditherPaletteCandidates: wholeNumber(root, "dither-palette-candidates", 0, "Dither Palette Choices"),
    } : {}),
    glyphs: glyphCatalog.characters(braille ? ["braille"] : groups),
    pipeline: collectPipelineEffects(root),
    include: include || undefined,
    includeRanges,
    exclude: value(root, "exclude"),
    excludeRanges,
    filter: value(root, "filter"),
    blendColours: checked(root, "blend-colours"),
    grayscaleTolerance: number(root, "grayscale-tolerance"),
    colourSpace: value(root, "colour-space"),
    smooth: !braille && checked(root, "smooth"),
    perceptualScore: renderMode === "glyph" && checked(root, "perceptual-score"),
    glyphSearch: renderMode === "glyph" && checked(root, "glyph-pair-search"),
    discontinuityThreshold: number(root, "discontinuity-threshold"),
    glyphSearchCandidates: number(root, "glyph-search-candidates"),
    glyphSearchPasses: number(root, "glyph-search-passes"),
    glyphSearchFinalists: number(root, "glyph-search-finalists"),
    smoothCandidates: number(root, "smooth-candidates"),
    smoothOrders: number(root, "smooth-orders"),
    smoothShapes: checked(root, "smooth-shapes"),
    smoothNeighbors: checked(root, "smooth-neighbors"),
    includeCells: true,
    // Dither colours are matched against the renderer's coverage masks.
    includePreview: dither,
    livePreview: true,
    overlays,
  };
}

export function updateRangeOutput(input) {
  const suffix = (input.dataset.effectControl ?? input.id) === "rotate" ? "°" : "";
  input.nextElementSibling.value = `${input.value}${suffix}`;
}

export function resetAdjustments(root) {
  // The pipeline controller resets every instance, including its palette and
  // secondary controls. Keep the existing edge-repair reset here.
  root.querySelector("#glyph-pair-search").checked = false;
}
