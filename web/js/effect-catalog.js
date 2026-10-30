// Extra browser controls feed the same Rust ImageEffect variants as the TUI.
export const EXTRA_EFFECTS = [
  ["darken-outlines", "Darken Outlines", "Effects", "DarkenOutlines", [
    ["outline-strength", "Strength %", 0, 100, 1, 100],
    ["outline-radius", "Radius (pixels)", 1, 64, 1, 3],
    ["outline-threshold", "Maximum luminance %", 0, 100, 1, 40],
    ["outline-contrast", "Minimum contrast %", 0, 100, 1, 8],
  ]],
  ["normalize", "Normalize", "Effects", "Normalize"],
  ["sharpen", "Sharpen", "Effects", "Sharpen"],
  ["lofi", "Lofi", "Stylize", "Lofi"],
  ["cali", "Cali", "Stylize", "Cali"],
];

export function installExtraEffectControls(root) {
  for (const [id, title, , , parameters] of EXTRA_EFFECTS) {
    const node = document.createElement("div");
    node.className = "pipeline-effect";
    node.dataset.pipelineEffect = id;
    node.hidden = true;
    const fields = document.createElement("div");
    fields.className = "effect-parameters range-grid";
    for (const [key, labelText, min, max, step, value] of parameters ?? [[id, title]]) {
      const label = document.createElement("label");
      const input = document.createElement("input");
      input.id = key;
      input.type = parameters ? "range" : "checkbox";
      if (parameters) {
        Object.assign(input, { min, max, step, value });
        const output = document.createElement("output");
        output.value = String(value);
        label.append(labelText, input, output);
      } else label.append(input, labelText);
      fields.append(label);
    }
    node.append(fields);
    root.querySelector("#pipeline-effects").append(node);
  }
}

export const effectControl = (root, id) => root.querySelector(`[data-effect-control="${id}"]`) ?? root.querySelector(`#${id}`);

export const hexRgb = (hex) => {
  if (!/^#[0-9a-f]{6}$/i.test(hex)) throw new Error(`Invalid colour "${hex}"; use #RRGGBB`);
  return [1, 3, 5].map((start) => parseInt(hex.slice(start, start + 2), 16));
};

export function parsePalette(text) {
  const unique = [...new Set(text.trim().split(/[\s,;]+/).filter(Boolean).map((item) => item.replace(/^#/, "").toLowerCase()))];
  if (unique.length > 256) throw new Error("Use at most 256 palette colours");
  return unique.map((item) => hexRgb(`#${item}`));
}

// Every entry owns its controls. Send native Rust effects in displayed order,
// rather than looking up a single shared value for each effect name.
export function collectPipelineEffects(root, beforeNode) {
  const result = [];
  for (const node of root.querySelector("#pipeline-effects").children) {
    if (node === beforeNode) break;
    if (node.hidden || node.dataset.disabled === "true") continue;
    const number = (id) => Number(effectControl(node, id).value);
    const checked = (id) => effectControl(node, id).checked;
    const id = node.dataset.pipelineEffect;
    const variants = { brightness: "Brightness", contrast: "Contrast", "luma-contrast": "LumaContrast",
      saturation: "Saturation", gamma: "Gamma", hue: "Hue", rotate: "Rotate",
      "gaussian-blur": "GaussBlur", "median-blur": "MedianBlur", pixelize: "Pixelize", dither: "Dither" };
    const toggles = { "flip-horizontal": "FlipH", "flip-vertical": "FlipV", "box-blur": "BoxBlur",
      invert: "Invert", grayscale: "Grayscale", ...Object.fromEntries(EXTRA_EFFECTS.filter(([, , , , parameters]) => !parameters).map(([key, , , variant]) => [key, variant])) };
    if (id === "darken-outlines") result.push({ DarkenOutlines: {
      strength: number("outline-strength"), radius: number("outline-radius"),
      threshold: number("outline-threshold"), contrast: number("outline-contrast"),
    } });
    else if (variants[id]) result.push({ [variants[id]]: number(id) });
    else if (toggles[id]) result.push(toggles[id]);
    else if (id === "scale") result.push({ ScaleX: Math.round(number("scale-x") * 100) }, { ScaleY: Math.round(number("scale-y") * 100) });
    else if (id === "crop") result.push({ Crop: Object.fromEntries(["top", "right", "bottom", "left"].map((side) => [side, number(`crop-${side}`)])) });
    else if (id === "line-thickness") result.push({ DarkLines: number(id) * (checked("light-lines") ? -1 : 1) });
    else if (id === "replace") result.push({ ReplaceColour: {
      from: hexRgb(effectControl(node, "replace-from").value), to: hexRgb(effectControl(node, "replace-to").value), tolerance: number("replace-tolerance"),
    } });
    else if (id === "quantize") result.push({ QuantizePalette: {
      colours: number("quantize-colours"), threshold: number("quantize-threshold"),
      palette: parsePalette(effectControl(node, "quantize-palette").value), source: checked("quantize-source"),
    } });
  }
  return result;
}

export function isOutputColourEffect(effect) {
  return Boolean(effect.ReplaceColour || (effect.QuantizePalette && !effect.QuantizePalette.source));
}
