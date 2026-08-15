const HELP = Object.freeze({
  render: "Selects the output encoding and colour palette: IRC colours, ANSI 256 colours, or ANSI 24-bit RGB.",
  "auto-render": "Automatically starts a new render shortly after a control changes.",
  width: "Target output width in character cells. Leave blank to derive it from the height and source aspect ratio.",
  height: "Target output height in character cells. Leave blank to derive it from the width and source aspect ratio.",
  font: "The monospace font whose glyph shapes are used for matching. Google fonts are downloaded only when selected.",
  "font-size": "Raster resolution used to analyze and match glyph shapes. Larger values can preserve more detail but take longer; this does not set the displayed preview size.",
  filter: "Resampling algorithm used while resizing the source to the output grid. Sharper filters preserve detail; softer filters can reduce aliasing.",
  "grayscale-tolerance": "Controls when near-neutral colours are treated as grayscale during palette selection. Higher values classify more colours as gray.",
  "colour-space": "Colour model used when comparing and adjusting colours. Different models weight hue and brightness differences differently.",
  smooth: "Runs the more expensive continuity pass to reduce rough transitions between neighboring glyph cells.",
  "smooth-candidates": "Number of alternate glyph matches retained per cell for smoothing. More candidates can improve results and increase render time.",
  "smooth-orders": "Number of traversal patterns tried by smoothing. Additional patterns reduce directional bias but add work.",
  "smooth-shapes": "Allows smoothing to refine glyph shapes as well as neighbouring colour transitions.",
  "smooth-neighbors": "Rejects smoothing changes that improve one cell while making its surrounding cells substantially worse.",
  brightness: "Adds or removes overall light from the source before glyph matching.",
  contrast: "Increases or reduces the difference between dark and light source pixels.",
  gamma: "Adjusts midtones non-linearly while largely preserving the darkest and brightest values.",
  saturation: "Increases or reduces colour intensity before palette matching.",
  hue: "Rotates source colours around the hue wheel.",
  rotate: "Rotates the source image by the selected number of degrees.",
  "scale-x": "Stretches or compresses the source horizontally before fitting it to the character grid.",
  "scale-y": "Stretches or compresses the source vertically before fitting it to the character grid.",
  pixelize: "Groups source pixels into larger flat-colour regions before conversion.",
  "gaussian-blur": "Applies a soft blur before conversion; useful for suppressing fine noise that cannot be represented by glyphs.",
  dither: "Distributes palette quantization error into nearby pixels to reduce colour banding.",
  "flip-horizontal": "Mirrors the source from left to right.",
  "flip-vertical": "Mirrors the source from top to bottom.",
  invert: "Inverts the source colours before matching.",
  grayscale: "Removes colour from the source before matching.",
  "box-blur": "Applies a simple neighborhood-average blur before conversion.",
  braille: "Uses Unicode Braille cells, where each character represents a 2 × 4 dot matrix, instead of the selected glyph groups.",
  "glyph-search": "Filters the visible glyph-group list by name; it does not change the selected groups.",
  include: "Adds these exact characters to the glyph candidates, even when they are outside the selected groups.",
  exclude: "Removes these exact characters from the final glyph candidates.",
  "include-ranges": "Adds Unicode code-point ranges written as hexadecimal START-END values, one or more per line.",
  "exclude-ranges": "Removes Unicode code-point ranges written as hexadecimal START-END values.",
  "preview-mode": "Shows either img2irc's bitmap preview or real selectable browser glyphs. Both use the renderer's font advance and line-height metrics.",
  "output-font-size": "Changes only the displayed preview size. It does not rerender or change the encoded IRC/ANSI output.",
});

function captionFor(label, control) {
  const textNode = [...label.childNodes]
    .find((node) => node.nodeType === Node.TEXT_NODE && node.textContent.trim());
  if (textNode) {
    const caption = document.createElement("span");
    caption.className = "label-caption";
    caption.textContent = textNode.textContent.trim();
    textNode.replaceWith(caption);
    return caption;
  }
  const caption = [...label.children]
    .find((element) => element !== control && element.tagName === "SPAN");
  caption?.classList.add("label-caption");
  return caption;
}

export function initializeControlHelp(root) {
  const tooltip = document.createElement("div");
  tooltip.id = "control-help-tooltip";
  tooltip.className = "control-help-tooltip";
  tooltip.role = "tooltip";
  tooltip.hidden = true;
  document.body.append(tooltip);

  let active;
  const hide = () => {
    active = undefined;
    tooltip.hidden = true;
  };
  const show = (icon, text) => {
    active = icon;
    tooltip.textContent = text;
    tooltip.hidden = false;
    const anchor = icon.getBoundingClientRect();
    const bounds = tooltip.getBoundingClientRect();
    const below = anchor.bottom + 6;
    const above = anchor.top - bounds.height - 6;
    tooltip.style.top = `${below + bounds.height <= innerHeight - 8 ? below : Math.max(8, above)}px`;
    tooltip.style.left = `${Math.max(8, Math.min(anchor.left, innerWidth - bounds.width - 8))}px`;
  };

  for (const [id, text] of Object.entries(HELP)) {
    const control = root.querySelector(`#${id}`);
    const label = control?.closest("label");
    if (!control || !label) continue;
    const caption = captionFor(label, control);
    if (!caption) continue;

    const icon = document.createElement("span");
    icon.className = "info-button";
    icon.textContent = "i";
    icon.tabIndex = 0;
    icon.role = "button";
    icon.setAttribute("aria-label", `Explain ${caption.textContent}`);
    icon.setAttribute("aria-describedby", tooltip.id);
    icon.addEventListener("mouseenter", () => show(icon, text));
    icon.addEventListener("mouseleave", hide);
    icon.addEventListener("focus", () => show(icon, text));
    icon.addEventListener("blur", hide);
    icon.addEventListener("pointerdown", (event) => {
      event.preventDefault();
      event.stopPropagation();
    });
    icon.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      if (active === icon && !tooltip.hidden) hide();
      else show(icon, text);
    });
    icon.addEventListener("keydown", (event) => {
      if (event.key === "Escape") hide();
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        if (active === icon && !tooltip.hidden) hide();
        else show(icon, text);
      }
    });
    caption.append(icon);
  }

  addEventListener("scroll", hide, true);
  addEventListener("resize", hide);
}
