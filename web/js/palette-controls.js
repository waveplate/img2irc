import { parsePalette } from "./render-options.js";

const hex = (rgb) => `#${rgb.map((c) => c.toString(16).padStart(2, "0")).join("")}`;

export function visiblePalette(result) {
  const counts = new Map();
  const pixels = result?.previewRgba ?? [];
  for (let i = 0; i < pixels.length; i += 4) {
    if (!pixels[i + 3]) continue;
    const key = hex([pixels[i], pixels[i + 1], pixels[i + 2]]);
    counts.set(key, (counts.get(key) ?? 0) + pixels[i + 3]);
  }
  return [...counts].sort(([a, na], [b, nb]) => nb - na || a.localeCompare(b)).slice(0, 256).map(([colour]) => colour);
}

export function initializePaletteControls(root, result, changed) {
  const element = (id) => root.querySelector(`#${id}`);
  const input = element("quantize-palette");
  const swatches = element("quantize-swatches");
  const refresh = () => {
    let colours;
    try {
      colours = parsePalette(input.value);
      input.setCustomValidity("");
    } catch (error) {
      input.setCustomValidity(element("quantize-enabled").checked ? error.message : "");
      swatches.replaceChildren();
      return;
    }
    for (const id of ["quantize-colours", "quantize-threshold"]) element(id).disabled = colours.length > 0;
    const buttons = colours.map((rgb, index) => {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = hex(rgb);
      button.title = `Remove ${hex(rgb)} from the palette`;
      button.style.borderLeft = `1.25rem solid ${hex(rgb)}`;
      button.addEventListener("click", () => {
        input.value = colours.filter((_, i) => i !== index).map(hex).join(" ");
        refresh();
        changed();
      });
      return button;
    });
    swatches.replaceChildren(...buttons);
  };
  input.addEventListener("input", refresh);
  element("quantize-enabled").addEventListener("change", refresh);
  element("quantize-current").addEventListener("click", () => {
    input.value = visiblePalette(result()).join(" ");
    element("quantize-enabled").checked = true;
    refresh();
    changed();
  });
  element("quantize-auto").addEventListener("click", () => {
    input.value = "";
    refresh();
    changed();
  });
  element("reset-adjustments").addEventListener("click", () => {
    input.value = "";
    refresh();
  });
  refresh();
}
