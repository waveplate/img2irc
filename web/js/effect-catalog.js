// Extra browser controls feed the same Rust ImageEffect variants as the TUI.
export const EXTRA_EFFECTS = [
  ["luma-blur", "Luma Blur", "Effects", "LumaBlur", [["luma-blur", "Radius", 0, 20, 1, 0]]],
  ["oil", "Oil Painting", "Effects", "Oil", [["oil-radius", "Radius", 1, 16, 1, 3], ["oil-intensity", "Intensity", 0, 255, 0.5, 55]]],
  ...[
    ["halftone", "Halftone", "Halftone"], ["normalize", "Normalize", "Normalize"],
    ["noise", "Noise", "Noise"], ["sharpen", "Sharpen", "Sharpen"],
    ["edge-detection", "Edge Detection", "EdgeDetect"], ["emboss", "Emboss", "Emboss"],
    ["frosted-glass", "Frosted Glass", "FrostGlass"], ["laplace", "Laplace", "Laplace"],
    ["identity", "Identity", "Identity"],
  ].map(([id, title, variant]) => [id, title, "Effects", variant]),
  ...[
    ["sepia", "Sepia", "Sepia"], ["solarize", "Solarize", "Solarize"],
    ["cali", "Cali", "Cali"], ["dramatic", "Dramatic", "Dramatic"],
    ["firenze", "Firenze", "Firenze"], ["golden", "Golden", "Golden"],
    ["lix", "Lix", "Lix"], ["lofi", "Lofi", "Lofi"], ["neue", "Neue", "Neue"],
    ["obsidian", "Obsidian", "Obsidian"], ["pastel-pink", "Pastel Pink", "PastelPink"], ["ryo", "Ryo", "Ryo"],
  ].map(([id, title, variant]) => [id, title, "Stylize", variant]),
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

export function collectExtraEffects(root) {
  return EXTRA_EFFECTS.flatMap(([id, , , variant, parameters]) => {
    const node = root.querySelector(`[data-pipeline-effect="${id}"]`);
    if (node.hidden || node.dataset.disabled === "true") return [];
    if (!parameters) return [variant];
    const values = parameters.map(([key]) => Number(root.querySelector(`#${key}`).value));
    return [{ [variant]: values.length === 1 ? values[0] : values }];
  });
}
