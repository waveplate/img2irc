import { parsePalette } from "./render-options.js";
import { effectControl } from "./effect-catalog.js";
import { colourHex as hex, visiblePalette } from "./artwork-colours.js";
import { ArtworkColourPicker } from "./artwork-colour-picker.js";

export { visiblePalette };

export function initializePaletteControls(root, result, changed) {
  const element = (id) => effectControl(root, id);
  const input = element("quantize-palette");
  const swatches = element("quantize-swatches");
  const addColour = (colour) => {
    try {
      const colours = parsePalette(input.value).map(hex);
      if (!colours.includes(colour)) colours.push(colour);
      parsePalette(colours.join(" "));
      input.value = colours.join(" ");
    } catch (error) {
      input.setCustomValidity(error.message);
      input.reportValidity();
      return false;
    }
    element("quantize-colour").value = colour;
    element("quantize-enabled").checked = true;
    refresh();
    changed();
  };
  const picker = new ArtworkColourPicker(root, "quantize", result, addColour);
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
  picker.start();
  element("quantize-add").addEventListener("click", () => addColour(element("quantize-colour").value));
  element("quantize-pick").addEventListener("click", () => {
    picker.open("Choose a current art colour below, or click the artwork to add it to the explicit palette");
  });
  element("quantize-current").addEventListener("click", () => {
    const colours = visiblePalette(result());
    if (!colours.length) {
      picker.open("Choose a current art colour below, or click the artwork to add it to the explicit palette");
      return;
    }
    input.value = colours.join(" ");
    element("quantize-enabled").checked = true;
    picker.close();
    refresh();
    changed();
  });
  element("quantize-auto").addEventListener("click", () => {
    input.value = "";
    picker.close();
    refresh();
    changed();
  });
  root.addEventListener("effectreset", refresh);
  refresh();
}
