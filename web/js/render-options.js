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
    pipeline: collectPipelineEffects(root),
    include: include || undefined,
    includeRanges,
    exclude: value(root, "exclude"),
    excludeRanges,
    filter: value(root, "filter"),
    blendColours: checked(root, "blend-colours"),
    grayscaleTolerance: number(root, "grayscale-tolerance"),
    colourSpace: value(root, "colour-space"),
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
  const suffix = (input.dataset.effectControl ?? input.id) === "rotate" ? "°" : "";
  input.nextElementSibling.value = `${input.value}${suffix}`;
}

export function resetAdjustments(root) {
  // The pipeline controller resets every instance, including its palette and
  // secondary controls. Keep the existing edge-repair reset here.
  root.querySelector("#glyph-pair-search").checked = false;
}
