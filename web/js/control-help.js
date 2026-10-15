const HELP = Object.freeze({
  render: "Selects the output encoding and colour palette: IRC colours, ANSI 256 colours, or ANSI 24-bit RGB.",
  "auto-render": "Automatically starts a new render shortly after a control changes.",
  width: "Target output width in character cells. Enter zero or leave blank to derive it from the source dimensions and the selected glyph size.",
  height: "Target output height in character cells. Leave blank to derive it from the width, source aspect ratio, and glyph size.",
  font: "The monospace font whose glyph shapes are used for matching. Google fonts are downloaded only when selected.",
  "font-size": "Raster resolution used to analyze and match glyph shapes. Larger values can preserve more detail but take longer; this does not set the displayed preview size.",
  "optimize-font-size": "Compare every integer font size from 9 through 30 and select the lowest Perceptual score. The scan skips smoothing, edge repair, region guidance, warp and OCR; your enabled features are preserved for the final render.",
  filter: "Resampling algorithm used while resizing the source to the output grid. Sharper filters preserve detail; softer filters can reduce aliasing.",
  "grayscale-tolerance": "Controls when near-neutral colours are treated as grayscale during palette selection. Higher values classify more colours as gray.",
  "colour-space": "Colour model used when comparing and adjusting colours. Different models weight hue and brightness differences differently.",
  smooth: "Runs the more expensive continuity pass to reduce rough transitions between neighboring glyph cells.",
  "perceptual-score": "Experimental quality score combining contour regularity, unsupported thin gaps and tips, missing or invented edges, and coarse colour agreement. Allows small edge movements. Lower is better; values use a different scale from Contour. Changes scoring, not the rendered image.",
  "glyph-pair-search": "Repair gaps, jagged edges and stray tips by trying neighbouring character combinations. Keeps only improvements to the full-image Perceptual score. Works independently of smoothing.",
  "discontinuity-threshold": "Minimum defect strength to search. Lower includes more cells; 0 searches every non-flat cell and disables skipping unchanged contexts. Default 0.5.",
  "glyph-search-candidates": "Alternatives per cell, split between source fit and neighbouring boundary agreement, plus its current glyph. Pair combinations grow roughly with the square of this count. Duplicate rendered shapes are removed.",
  "glyph-search-passes": "Maximum sweeps over neighbouring glyph pairs. Stops after a pass finds no improvement. Later passes can combine earlier changes.",
  "glyph-search-finalists": "Best local combinations checked against the full-image score per pair. 0 checks every shortlisted combination globally and can be much slower.",
  "smooth-candidates": "Number of alternate glyph matches retained per cell for smoothing. More candidates can improve results and increase render time.",
  "smooth-orders": "Number of traversal patterns tried by smoothing. Additional patterns reduce directional bias but add work.",
  "smooth-shapes": "Allows smoothing to refine glyph shapes as well as neighbouring colour transitions.",
  "smooth-neighbors": "Rejects smoothing changes that improve one cell while making its surrounding cells substantially worse.",
  "ocr-model-tier": "Choose the Tiny, Small, or Medium PP-OCRv6 detection and recognition models. Larger models take longer to download and initialize. Downloads are kept in IndexedDB for reuse. If detections are locked or edited, use Regenerate OCR to apply the selected model.",
  brightness: "Adds or removes overall light from the source before glyph matching.",
  "luma-contrast": "Changes brightness contrast while preserving RGB colour differences. Highly saturated colours change less when there is no room to shift them without changing chroma.",
  "median-blur": "Median neighbourhood radius in source pixels. Removes isolated speckles while keeping edges firmer than Gaussian blur. 0 disables it.",
  "line-thickness": "Grow lines with positive values; shrink them with negative values. Adjust in 0.1 source-pixel steps; fractional values blend adjacent radii for finer changes. Choose light lines for pale strokes on a dark background.",
  "light-lines": "Adjust light strokes on dark backgrounds. Leave off for dark strokes on light backgrounds.",
  "replace-enabled": "Replace colours in the finished art, including text overlays. Edit hex values or sample the art with the inline eyedropper.",
  "replace-from": "The visible output colour to replace. IRC and ANSI selections snap to their output palette; 24-bit uses exact RGB.",
  "quantize-enabled": "Merge nearby shades to clean flat fills, then limit their palette. Choose source cleanup or finished-art edits, or supply an explicit palette.",
  "quantize-colours": "Maximum automatically chosen colours after merging similar shades. Zero disables the colour cap. An explicit palette overrides this control.",
  "quantize-threshold": "Similarity radius in perceptual OKLab space, expressed as a percentage. Increase gradually to merge compression noise without merging distinct fills. Zero disables merging.",
  "quantize-source": "Apply quantization to image pixels before glyph matching. Uncheck to edit finished-art colours, including text overlays, without changing glyphs.",
  "quantize-palette": "Enter #RRGGBB colours separated by spaces or commas. Pixels or art colours map to their nearest chosen colour; count and threshold are ignored. Leave blank for automatic selection.",
  "replace-to": "The visible replacement colour. IRC and ANSI selections snap to their output palette; 24-bit uses exact RGB.",
  "replace-tolerance": "Distance from the selected output colour: 0 matches exactly, while higher values include nearby shades.",
  "ocr-figlet-fill": "Choose the largest font that fits the allowed extra width and height, nearby text and canvas bounds. The output width is unchanged by this option.",
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
  "preview-mode": "Shows either a bitmap or selectable browser text. Both align the selected font to fixed cells with shared pixel boundaries, preserving the grid's aspect ratio.",
  "output-font-size": "Changes only the displayed preview size. It does not rerender or change the encoded IRC/ANSI output.",
  "ocr-section": "Expand to view OCR settings. Use its ON/OFF switch to run PP-OCRv6 in a dedicated browser worker, remove recognized text from the source, and restore it as editable character-cell overlays.",
  "ocr-enabled": "Enables OCR without changing whether its settings are expanded.",
  "glyph-search-section": "Expand to view Repair Edges settings. Its ON/OFF switch enables the search independently.",
  "ocr-lock": "Freezes untouched OCR detections. Editing any OCR overlay automatically preserves the complete overlay set until Regenerate OCR is used.",
  "ocr-auto-width": "Uses the smallest render width at which the requested percentage of accepted OCR text fits completely inside its detected boxes. Height follows the image aspect ratio. Limited to 4096 columns.",
  "ocr-text-coverage": "Percentage of recognized non-space characters whose complete text lines must fit. Lower values can ignore a few unusually narrow or inaccurate boxes and keep the output smaller.",
  "ocr-confidence": "Rejects recognized lines below this confidence score. Higher values reduce false positives but may omit faint text.",
  "ocr-ascii-ratio": "Rejects lines whose text contains too few printable ASCII characters. Lower this when preserving non-Latin text.",
  "ocr-width": "Resizes the image to this pixel width before feeding into OCR detection and text removal. Set to 0 to use the full image width.",
  "ocr-megapixels": "Recognition detail in millions of pixels, independent of output width. Preserves aspect ratio and never enlarges the source. Zero enables the legacy width and maximum-side settings in Advanced.",
  "ocr-max-side": "Resizes large images to this maximum side length for detection. Larger values can find smaller text but cost more memory and time.",
  "ocr-max-height-ratio": "Rejects text regions taller than this multiple of the median detected line. Zero disables this filter.",
  "ocr-figlet": "Uses hosted FIGlet fonts when a detected label is large enough to fit multi-row text. Ordinary one-row text remains the fallback.",
  "ocr-figlet-min-height": "Minimum number of output rows in a detected text box before FIGlet may be used.",
  "ocr-figlet-min-height-ratio": "Requires a detected label to be this much taller than the median text line before FIGlet may be used. Zero disables the relative check.",
  "ocr-figlet-max-width-ratio": "Maximum emergency width enlargement when no FIGlet font fits the detected box (e.g. 0.5 allows up to 50% wider). Growth starts at the box's left edge and is constrained by text to its right and the canvas. Fit largest FIGlet always uses the available allowance.",
  "ocr-figlet-max-height-ratio": "Maximum emergency height enlargement when no FIGlet font fits the detected box (e.g. 0.5 allows up to 50% taller). Taller art stays centered on the detected box and is constrained by nearby text and canvas edges. Fit largest FIGlet always uses the available allowance.",
  "ocr-box-score": "Minimum confidence for a detected text region before recognition is attempted.",
  "ocr-box-threshold": "Pixel threshold used by PaddleOCR while constructing text regions.",
  "ocr-unclip-ratio": "Expands detected text boxes before recognition. Larger values include more surrounding context.",
  "overlay-text": "Text written directly into output character cells. Newlines create additional rows.",
  "overlay-x": "Zero-based output column where this overlay begins.",
  "overlay-y": "Zero-based output row where this overlay begins.",
  "overlay-width": "Width of the overlay box in character cells. It is managed automatically while Fit to text is enabled.",
  "overlay-height": "Height of the overlay box in character cells. It is managed automatically while Fit to text is enabled.",
  "overlay-auto-grow": "Sizes the overlay box to its text automatically.",
  "overlay-wrap": "Wraps text at the overlay box width using Unicode line-break opportunities.",
  "overlay-figlet": "Treats the typed text as one movable FIGlet object. Choose its font below; OCR text continues to select a font automatically.",
  "overlay-figlet-font": "Selects the exact hosted FIGlet font used by this manually created overlay. Resize the box if the selected font does not fit.",
  "overlay-foreground-enabled": "Overrides the foreground colour of characters in this overlay.",
  "overlay-background-enabled": "Fills the entire text or FIGlet bounding box with the selected background colour, including spaces, padding and blank rows.",
  "pipeline-add": "Append an effect to the active pipeline. Drag its grip to reorder it, toggle it off to keep its settings, or remove it with ×. Source effects run before matching; finished-art colour edits run afterwards.",
  "pipeline-filter": "Filters the effect catalog by name or category. Select an effect to add its controls to the active pipeline.",
  "reset-adjustments": "Removes all active effects and restores their parameters to the defaults.",
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
  const helpTitle = root.querySelector("#control-help-title");
  const helpText = root.querySelector("#control-help-text");
  const context = (title, text) => {
    if (helpTitle) helpTitle.textContent = title;
    if (helpText) helpText.textContent = text;
  };
  const paneHelp = {
    general: ["General", "Set the output mode, width and font size. OCR, smoothing and edge repair expand to show their settings."],
    pipeline: ["Active Pipeline", "New effects append to the bottom. Drag the grip or focus it and press ↑/↓ to reorder. OFF keeps settings; × removes an effect. Space restores the last slider value."],
    glyphs: ["Glyphs", "Choose glyph sets or a preset. Focus a set to inspect its characters. Include and exclude fields refine the selection."],
    text: ["Text Settings", "Add an overlay or select an existing one. Drag a box in the output to move it; drag its corner to resize it."],
  };
  root.addEventListener("controltabchange", (event) => {
    const help = paneHelp[event.detail.tab];
    if (help) context(...help);
  });
  const followControl = (event) => {
    const target = event.target.closest("label, summary, input, select, textarea, button");
    if (!target) return;
    let control = target.matches("label") ? target.querySelector("input, select, textarea") : target;
    if (control?.matches("summary")) control = control.closest("details");
    if (!control?.id) control = target.closest("[data-pipeline-effect]")?.querySelector("input[id], select[id]");
    const text = HELP[control?.id] ?? (control?.id === "smoothing-section" ? HELP.smooth : undefined);
    if (!text) return;
    const caption = control.closest("label")?.querySelector(".label-caption");
    const title = caption?.firstChild?.textContent?.trim() || control.getAttribute("aria-label")
      || (control.id === "ocr-section" ? "OCR" : control.id === "smoothing-section" ? "Smoothing" : control.id);
    context(title, text);
  };
  root.addEventListener("focusin", followControl);
  root.addEventListener("pointerover", followControl);

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
    if (!control || !label || label.classList.contains("feature-enable")) continue;
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
