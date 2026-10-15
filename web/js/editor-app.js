import { ColourReplacementPicker } from "./colour-replacement.js";
import { initializeControlHelp } from "./control-help.js";
import { EditorState } from "./editor-state.js";
import { FontService } from "./font-service.js";
import { GlyphCatalog } from "./glyph-catalog.js";
import { chooseGlyphPreset, filterGlyphGroups, populateGlyphGroups } from "./glyph-controls.js";
import { collectRenderOptions, resetAdjustments, updateRangeOutput } from "./render-options.js";
import { RendererClient } from "./renderer-client.js";
import { RenderStatus } from "./render-status.js";
import { OutputPreview } from "./output-preview.js";
import { FigletService } from "./figlet-service.js";
import { OcrService, ocrItemsToOverlays, ocrFitWidth } from "./ocr-service.js";
import { renderableOverlays, TextControls } from "./text-controls.js";
import { initializeNumberScrubbers } from "./number-scrubber.js";
import { initializeSliderToggles } from "./slider-toggle.js";
import { initializePaletteControls } from "./palette-controls.js";
import { initializePipelineControls } from "./pipeline-controls.js";
import { initializeCropControls } from "./crop-controls.js";
import { initializeWorkspaceControls } from "./workspace-controls.js";
import { isImageFile, rasterizeSvg } from "./image-input.js";
import { saveFile } from "./file-export.js";
import { EditorPersistence, captureControls, restoreControls } from "./editor-persistence.js";

const CASCADIA_URL = new URL("../assets/fonts/CascadiaCode-Regular.ttf", import.meta.url);
const IOSEVKA_FIXED_URL = new URL("../assets/fonts/IosevkaFixed-Regular.ttf", import.meta.url);
const IOSEVKA_FIXED_EXTENDED_URL = new URL(
  "../assets/fonts/IosevkaFixed-Extended.ttf",
  import.meta.url,
);
const UNIFONT_URL = new URL("../assets/fonts/unifont-17.0.05.otf", import.meta.url);
const GLYPH_CATALOG_URL = new URL("../glyphs.json", import.meta.url);
const HOSTED_FONTS = new Map([
  ["local:cascadia", CASCADIA_URL],
  ["local:iosevka-fixed", IOSEVKA_FIXED_URL],
  ["local:iosevka-fixed-extended", IOSEVKA_FIXED_EXTENDED_URL],
  ["local:unifont", UNIFONT_URL],
]);

export class EditorApp {
  #form = document.querySelector("#controls");
  #state = new EditorState();
  #fonts = new FontService(HOSTED_FONTS);
  #glyphs = new GlyphCatalog(GLYPH_CATALOG_URL);
  #renderStatus = new RenderStatus(document.querySelector("#render-stats"));
  #preview = new OutputPreview(
    document.querySelector("#preview-stage"),
    document.querySelector("#output-canvas"),
    document.querySelector("#output-text"),
    document.querySelector("#output-placeholder"),
  );
  #renderer = new RendererClient();
  #ocr = new OcrService();
  #figlet = new FigletService(this.#renderer);
  #textControls = new TextControls(
    this.#form,
    this.#state,
    () => this.#markOverlayDirty(),
    this.#preview,
  );
  #colourPicker = new ColourReplacementPicker(
    this.#form,
    () => this.#state.result,
    () => this.#element("render").value,
    () => this.#markRenderDirty(),
  );
  #rendererFontKey;
  #rendererSourceKey;
  #fontLoadRevision = 0;
  #fontLoadKey;
  #fontLoadPromise;
  #sourceLoadRevision = 0;
  #documentRevision = 0;
  #renderInFlight = false;
  #renderRequestInFlight = false;
  #renderQueued = false;
  #overlayRenderQueued = false;
  #overlayRenderOptions;
  #renderTimer;
  #ocrRefreshRevision = 0;
  #ocrOverlayKey;
  #lockedOcr;
  #lockedOcrGrid;
  #fontScanInFlight = false;
  #fontScanQueued = false;
  #sourceName = "img2irc";
  #smoothHintSeen = false;
  #sourceLabel = "clipboard image";
  #scrollPositions = {};
  #pipelineControls;
  #workspaceControls;
  #restoring = true;
  #saveErrorShown = false;
  #persistence = new EditorPersistence((error) => {
    if (!this.#saveErrorShown) {
      this.#saveErrorShown = true;
      this.#setStatus(`Could not save refresh recovery: ${error}`, true);
    }
  });

  async start() {
    const saved = this.#persistence.metadata();
    this.#scrollPositions = saved?.scroll ?? {};
    restoreControls(this.#form, saved?.controls);
    this.#pipelineControls = initializePipelineControls(this.#form);
    restoreControls(this.#form, saved?.controls);
    if (saved?.pipeline) this.#pipelineControls.restore(saved.pipeline);
    this.#workspaceControls = initializeWorkspaceControls(this.#form);
    initializeControlHelp(this.#form);
    initializeNumberScrubbers(this.#form);
    // Optional catalogs cannot hold up the image editor. A saved Google font
    // list is installed synchronously before its background refresh begins.
    const googleFonts = this.#fonts.populateGoogleFonts(this.#element("font"), this.#element("font-status"));
    if (saved?.controls?.font?.startsWith("google:")
      && ![...this.#element("font").options].some((option) => option.value === saved.controls.font)) {
      await googleFonts;
    }
    restoreControls(this.#form, { font: saved?.controls?.font ?? this.#element("font").value });
    const fontReady = this.#ensureRenderer();
    const documentReady = saved ? this.#persistence.loadDocument() : Promise.resolve({});
    const glyphGroups = await this.#glyphs.load();
    populateGlyphGroups(this.#element("glyph-groups"), glyphGroups);
    if (saved?.glyphGroups) {
      this.#element("glyph-groups").querySelectorAll("input").forEach((input) => {
        input.checked = saved.glyphGroups.includes(input.value);
      });
    }
    this.#textControls.start();
    this.#bindEvents();
    this.#syncOcrAutoWidthControls();
    this.#colourPicker.start();
    initializePaletteControls(this.#form, () => this.#state.result, () => this.#markRenderDirty());
    initializeCropControls(this.#form, async () => {
      if (!this.#state.hasImage) throw new Error("Open an image first");
      if (this.#renderRequestInFlight || this.#fontScanInFlight) this.#stopRender();
      await this.#ensureRenderer();
      await this.#ensureRendererSource(this.#state.sourceBlob, `original:${this.#state.sourceRevision}`);
      const options = collectRenderOptions(this.#form, this.#glyphs, []);
      const index = options.pipelineOrder.indexOf("crop");
      return this.#renderer.getTransformedImage({ ...options,
        pipelineOrder: index < 0 ? [] : options.pipelineOrder.slice(0, index), sourceResolution: true });
    });
    this.#selectControlTab(saved?.activeTab ?? "general");
    this.#element("view-controls").checked = saved?.controls?.["view-controls"] ?? true;
    this.#form.dataset.controlsVisible = String(this.#element("view-controls").checked);
    this.#workspaceControls.restore(saved?.dockWidths);
    for (const [id, open] of Object.entries(saved?.sections ?? {})) {
      const section = this.#element(id);
      if (section?.tagName === "DETAILS") section.open = Boolean(open);
    }
    this.#form.querySelectorAll('input[type="range"]').forEach(updateRangeOutput);
    this.#preview.setMode(this.#element("preview-mode").value);
    this.#updateOutputFontSize();
    void this.#figlet.fontOptions()
      .then((fonts) => this.#textControls.setFigletFonts(fonts))
      .catch(() => this.#textControls.setFigletFonts([]));
    const [, restored] = await Promise.all([fontReady, documentReady]);
    if (restored.source?.blob) {
      const source = restored.source;
      this.#state.setSource(URL.createObjectURL(source.originalBlob ?? source.blob), source.blob, source.originalBlob);
      this.#state.sourceRevision = source.revision;
      this.#sourceName = source.name;
      this.#sourceLabel = source.label;
      await this.#ensureRendererSource(source.blob, `original:${source.revision}`);
      this.#showSourceLink();
      this.#element("replace-pick").disabled = false;
      this.#element("replace-pick-to").disabled = false;
    }
    this.#documentRevision = saved?.revision ?? 0;
    this.#state.manualOverlays = saved?.manualOverlays ?? [];
    this.#state.ocrOverlays = saved?.ocrOverlays ?? [];
    this.#state.ocrOverlaysDirty = Boolean(saved?.ocrOverlaysDirty);
    this.#state.ocrOverlaysEnabled = this.#element("ocr-enabled").checked;
    const output = restored.result;
    const matchesSource = output && output.sourceRevision === this.#state.sourceRevision && this.#state.hasImage;
    if (matchesSource) {
      this.#lockedOcr = output.lockedOcr;
      this.#lockedOcrGrid = output.lockedOcrGrid;
      if (output.revision === this.#documentRevision) this.#overlayRenderOptions = output.options;
      this.#commitRenderResult(output.result, output.result.workerRenderMs ?? 0, performance.now());
    }
    this.#textControls.restoreSelection(saved?.overlaySelection);
    this.#syncGlyphPresetButtons();
    this.#syncSmoothHint();
    this.#bindSessionSaving();
    this.#restoring = false;
    this.#saveSession();
    requestAnimationFrame(() => {
      for (const [id, position] of Object.entries(saved?.scroll ?? {})) {
        const element = this.#element(id);
        if (element) { element.scrollLeft = position.left; element.scrollTop = position.top; }
      }
    });
    this.#setStatus(this.#state.hasImage ? "Restored saved session" : "Ready — open, paste, or drop an image");
    if (this.#state.hasImage && saved?.pendingFontScan) {
      void this.#optimizeFontSize();
    } else if (this.#state.hasImage && (saved?.pendingRender
      || (this.#element("auto-render").checked && (!matchesSource || output.revision !== this.#documentRevision)))) {
      void this.render();
    }
  }

  #showSourceLink() {
    const link = this.#element("source-link");
    link.href = this.#state.sourceObjectUrl;
    link.textContent = this.#sourceLabel;
    link.title = `Open ${this.#sourceLabel} in a new tab`;
    link.hidden = false;
    this.#element("source-placeholder").hidden = true;
  }

  #saveSession() {
    if (this.#restoring) return;
    for (const element of this.#form.querySelectorAll(".control-panel[id], .canvas-viewport")) {
      if (element.getClientRects().length) {
        this.#scrollPositions[element.id] = { left: element.scrollLeft, top: element.scrollTop };
      }
    }
    this.#persistence.saveMetadata({
      controls: captureControls(this.#form),
      glyphGroups: [...this.#element("glyph-groups").querySelectorAll("input:checked")].map((input) => input.value),
      pipeline: this.#pipelineControls.snapshot(), dockWidths: this.#workspaceControls.snapshot(),
      activeTab: this.#form.dataset.activeTab,
      sections: Object.fromEntries([...this.#form.querySelectorAll('details[id]:not([data-app-menu])')].map((section) => [section.id, section.open])),
      scroll: this.#scrollPositions,
      revision: this.#documentRevision, sourceRevision: this.#state.sourceRevision,
      manualOverlays: this.#state.manualOverlays, ocrOverlays: this.#state.ocrOverlays,
      ocrOverlaysDirty: this.#state.ocrOverlaysDirty, overlaySelection: this.#textControls.selection(),
      pendingRender: this.#renderInFlight || this.#fontScanInFlight,
      pendingFontScan: this.#fontScanInFlight || this.#fontScanQueued,
    });
  }

  #bindSessionSaving() {
    const save = () => queueMicrotask(() => this.#saveSession());
    for (const event of ["input", "change", "toggle", "workspacechange", "controltabchange"]) {
      this.#form.addEventListener(event, save, true);
    }
    this.#form.addEventListener("click", () => queueMicrotask(save), true);
    let scrollTimer;
    this.#form.addEventListener("scroll", () => {
      clearTimeout(scrollTimer); scrollTimer = setTimeout(() => this.#saveSession(), 150);
    }, true);
    window.addEventListener("pagehide", () => this.#saveSession());
  }

  #element(id) {
    return this.#form.querySelector(`#${id}`);
  }

  #setStatus(message, isError = false, isBusy = false) {
    const status = this.#element("status");
    status.textContent = message;
    status.dataset.error = String(isError);
    status.dataset.busy = String(isBusy);
    if (isBusy) {
      delete status.dataset.renderMs;
      delete status.dataset.totalMs;
    }
    this.#form.setAttribute("aria-busy", String(isBusy));
    this.#element("render-button").disabled = isBusy;
    this.#element("render-cancel").hidden = !isBusy || (!this.#renderInFlight && !this.#fontScanInFlight);
    this.#element("render-progress").hidden = true;
  }

  async #renderOutput(options, revision) {
    this.#renderRequestInFlight = true;
    try { return await this.#renderer.render(options, (progress) => {
      if (!this.#renderInFlight || revision !== this.#documentRevision) return;
      if (progress.preview) this.#preview.draw(progress.preview);
      this.#setStatus("Rendering output…", false, true);
      const bar = this.#element("render-progress");
      const determinate = progress.stage === "searching" || progress.stage === "finished";
      const percent = Math.max(0, Math.min(100, progress.fraction * 100));
      bar.hidden = false;
      bar.dataset.indeterminate = String(!determinate);
      bar.style.setProperty("--progress", `${percent}%`);
      bar.setAttribute("aria-valuemin", "0");
      bar.setAttribute("aria-valuemax", "100");
      if (determinate) bar.setAttribute("aria-valuenow", String(percent));
      else bar.removeAttribute("aria-valuenow");
      bar.setAttribute("aria-valuetext", progress.message);
      this.#element("render-progress-label").textContent = progress.message;
    }); } finally { this.#renderRequestInFlight = false; }
  }

  #interruptRenderer() {
    this.#renderer.restart();
    this.#rendererSourceKey = undefined;
    this.#rendererFontKey = undefined;
    this.#fontLoadRevision += 1;
    this.#fontLoadPromise = undefined;
    this.#fontLoadKey = undefined;
    this.#element("render-progress").hidden = true;
  }

  #stopRender() {
    this.#documentRevision += 1;
    clearTimeout(this.#renderTimer);
    this.#renderQueued = this.#overlayRenderQueued = this.#fontScanQueued = false;
    if (this.#renderInFlight || this.#fontScanInFlight) this.#interruptRenderer();
    if (this.#state.result) this.#preview.draw(this.#state.result);
    this.#setStatus("Rendering stopped — ready to render again");
    this.#saveSession();
  }

  async #ensureRenderer() {
    const key = this.#element("font").value;
    if (this.#rendererFontKey === key) return true;
    if (this.#fontLoadPromise && this.#fontLoadKey === key) return this.#fontLoadPromise;

    const revision = ++this.#fontLoadRevision;
    this.#fontLoadKey = key;
    const promise = this.#loadRendererFont(key, revision);
    this.#fontLoadPromise = promise;
    try {
      return await promise;
    } finally {
      if (this.#fontLoadPromise === promise) {
        this.#fontLoadPromise = undefined;
        this.#fontLoadKey = undefined;
      }
    }
  }

  async #loadRendererFont(key, revision) {
    this.#setStatus("Loading font…", false, true);
    const bytes = await this.#fonts.bytes(key);
    if (revision !== this.#fontLoadRevision) return false;
    const [, displayFamily] = await Promise.all([
      this.#renderer.setFont(bytes),
      this.#fonts.displayFamily(key),
    ]);
    if (revision !== this.#fontLoadRevision) return false;
    this.#rendererFontKey = key;
    this.#state.fontKey = key;
    this.#preview.setFontFamily(displayFamily);
    this.#element("glyph-preview-grid").style.fontFamily = displayFamily;
    return true;
  }

  #scheduleRender() {
    if (this.#fontScanInFlight || this.#fontScanQueued) {
      if (this.#element("auto-render").checked) this.#renderQueued = true;
      return;
    }
    if (!this.#element("auto-render").checked || !this.#state.hasImage) return;
    clearTimeout(this.#renderTimer);
    this.#renderTimer = setTimeout(() => this.render(), 350);
  }

  #syncOcrAutoWidthControls() {
    const disabled = !this.#element("ocr-auto-width").checked;
    this.#element("ocr-text-coverage").disabled = disabled;
    this.#element("ocr-text-coverage-control").classList.toggle("disabled-control", disabled);
  }

  #selectControlTab(tab) {
    if (!this.#restoring) this.#saveSession();
    this.#form.dataset.activeTab = tab;
    this.#form.dataset.controlsVisible = "true";
    this.#element("view-controls").checked = true;
    this.#form.querySelectorAll("[data-control-tab]").forEach((button) => {
      button.setAttribute("aria-selected", String(button.dataset.controlTab === tab));
      if (button.getAttribute("role") === "tab") button.tabIndex = button.dataset.controlTab === tab ? 0 : -1;
    });
    const position = this.#scrollPositions[`panel-${tab}`];
    if (position) requestAnimationFrame(() => {
      const panel = this.#element(`panel-${tab}`);
      panel.scrollLeft = position.left; panel.scrollTop = position.top;
    });
    this.#form.querySelectorAll("[data-control-tab-panel]").forEach((panel) => {
      panel.hidden = panel.dataset.controlTabPanel !== tab;
    });
    this.#textControls.setInteractive(tab === "text");
    this.#form.dispatchEvent(new CustomEvent("controltabchange", { detail: { tab } }));
  }

  #ocrSettings() {
    return {
      modelTier: this.#element("ocr-model-tier").value,
      minConfidence: Number(this.#element("ocr-confidence").value),
      minAsciiRatio: Number(this.#element("ocr-ascii-ratio").value),
      ocrWidth: Number(this.#element("ocr-width")?.value || 0),
      megapixels: Number(this.#element("ocr-megapixels")?.value ?? 1.5),
      maxSideLength: Number(this.#element("ocr-max-side").value),
      maxTextHeightRatio: Number(this.#element("ocr-max-height-ratio").value),
      boxScoreThreshold: Number(this.#element("ocr-box-score").value),
      boxThreshold: Number(this.#element("ocr-box-threshold").value),
      unclipRatio: Number(this.#element("ocr-unclip-ratio").value),
      refreshRevision: this.#ocrRefreshRevision,
    };
  }

  #ocrFigletSettings() {
    return {
      enabled: this.#element("ocr-figlet").checked,
      fillAvailable: this.#element("ocr-figlet-fill").checked,
      minHeight: Number(this.#element("ocr-figlet-min-height").value),
      minHeightRatio: Number(this.#element("ocr-figlet-min-height-ratio").value),
      maxWidthRatio: Number(this.#element("ocr-figlet-max-width-ratio")?.value || 0.5),
      maxHeightRatio: Number(this.#element("ocr-figlet-max-height-ratio")?.value || 0.5),
    };
  }

  #setOcrStatus(message, isError = false) {
    const status = this.#element("ocr-status");
    status.textContent = message;
    status.dataset.error = String(isError);
  }

  async #resolveFigletOverlays(columns, rows) {
    const candidates = this.#state.overlays.filter((overlay) => overlay.figlet
      && (!overlay.renderedText || !overlay.figletFont || overlay.figletFont === "plain" || (overlay.figletFontChoice && overlay.figletFont !== overlay.figletFontChoice)));
    if (candidates.length === 0) return undefined;
    const requests = candidates.map((overlay) => ({
      text: overlay.text,
      maxWidth: Math.max(1, Math.min(
        Number(overlay.width) || 1,
        columns - Math.max(0, Number(overlay.x) || 0),
      )),
      maxHeight: Math.max(1, Math.min(
        Number(overlay.height) || 1,
        rows - Math.max(0, Number(overlay.y) || 0),
      )),
      useFiglet: true,
      fontName: overlay.figletFontChoice || undefined,
    }));
    const rendered = await this.#figlet.render(requests, (message) => {
      this.#setStatus(message, false, true);
    });
    candidates.forEach((overlay, index) => {
      const art = rendered.arts[index];
      overlay.renderedText = art?.text ?? overlay.text;
      overlay.figletFont = art?.source ?? "plain";
    });
    this.#textControls.refresh();
    return rendered;
  }

  async #ensureRendererSource(bytes, key) {
    if (this.#rendererSourceKey === key) return;
    const buffer = bytes instanceof Blob ? await bytes.arrayBuffer() : bytes.slice(0);
    await this.#renderer.setImage(buffer);
    this.#rendererSourceKey = key;
  }

  #markRenderDirty() {
    this.#documentRevision += 1;
    this.#overlayRenderOptions = undefined;
    if (this.#renderRequestInFlight || this.#fontScanInFlight) {
      this.#interruptRenderer();
      this.#setStatus("Settings changed — click Render");
    }
    this.#syncSmoothHint();
    this.#saveSession();
    this.#scheduleRender();
  }

  #selectGlyphPreset(preset) {
    this.#element("braille").checked = false;
    this.#element("glyph-controls").querySelectorAll("input, textarea, button").forEach((control) => { control.disabled = false; });
    chooseGlyphPreset(this.#element("glyph-groups"), preset);
    this.#syncGlyphPresetButtons();
    this.#markRenderDirty();
  }

  #syncGlyphPresetButtons() {
    const selected = [...this.#element("glyph-groups").querySelectorAll("input:checked")].map((input) => input.value);
    const custom = this.#element("include").value || this.#element("include-ranges").value;
    for (const button of this.#form.querySelectorAll("[data-glyph-preset]")) {
      button.setAttribute("aria-pressed", String(!this.#element("braille").checked && !custom
        && selected.length === 1 && selected[0] === button.dataset.glyphPreset));
    }
  }

  #syncSmoothHint() {
    const selected = this.#element("glyph-groups").querySelectorAll("input:checked");
    const smooth = selected.length === 1 && selected[0].value === "smooth";
    const refining = this.#element("smooth").checked || this.#element("glyph-pair-search").checked;
    let dismissed = this.#smoothHintSeen;
    try { dismissed ||= Boolean(localStorage.getItem("img2irc.smooth-hint-dismissed")); } catch {}
    this.#element("smooth-glyph-hint").hidden = !refining || smooth || dismissed || this.#element("braille").checked;
  }

  async #optimizeFontSize() {
    if (!this.#state.hasImage || this.#fontScanInFlight) return;
    if (this.#renderInFlight) { this.#fontScanQueued = true; return; }
    clearTimeout(this.#renderTimer);
    const revision = this.#documentRevision;
    this.#fontScanInFlight = true;
    this.#saveSession();
    const button = this.#element("optimize-font-size");
    button.disabled = true;
    this.#setStatus("Scanning font sizes 9–30…", false, true);
    try {
      if (!await this.#ensureRenderer()) return;
      const options = collectRenderOptions(this.#form, this.#glyphs, []);
      await this.#ensureRendererSource(this.#state.sourceBlob, `original:${this.#state.sourceRevision}`);
      const best = await this.#renderer.optimizeFontSize(options, (progress) => {
        if (revision !== this.#documentRevision) return;
        this.#setStatus(`Font size ${progress.fontSize} · ${progress.completed}/${progress.total} · best ${progress.best?.fontSize ?? "—"}`, false, true);
        const bar = this.#element("render-progress");
        bar.hidden = false; bar.dataset.indeterminate = "false";
        bar.style.setProperty("--progress", `${progress.completed / progress.total * 100}%`);
        bar.setAttribute("aria-valuemin", "0"); bar.setAttribute("aria-valuemax", "100");
        bar.setAttribute("aria-valuenow", String(progress.completed / progress.total * 100));
        bar.setAttribute("aria-valuetext", `Font scan ${progress.completed} of ${progress.total}`);
        this.#element("render-progress-label").textContent = `Font scan ${progress.completed}/${progress.total} · score ${progress.score?.toFixed(3) ?? "—"}`;
      });
      if (revision !== this.#documentRevision) { this.#setStatus("Settings changed — scan result discarded"); return; }
      this.#element("font-size").value = best.fontSize;
      this.#documentRevision += 1;
      this.#overlayRenderOptions = undefined;
      this.#setStatus(`Selected font size ${best.fontSize} · Perceptual ${best.score.toFixed(3)}`);
      this.#renderQueued = true;
    } catch (error) {
      if (revision === this.#documentRevision && error.name !== "AbortError") this.#setStatus(`Font scan failed: ${error}`, true);
    }
    finally {
      this.#fontScanInFlight = false; button.disabled = false;
      this.#saveSession();
      this.#element("render-progress").hidden = true;
      if (this.#renderQueued) { this.#renderQueued = false; queueMicrotask(() => void this.render()); }
    }
  }

  #markOverlayDirty() {
    this.#documentRevision += 1;
    this.#saveSession();
    if (!this.#state.hasImage) return;
    if (!this.#overlayRenderOptions || !this.#state.result) {
      this.#scheduleRender();
      return;
    }
    clearTimeout(this.#renderTimer);
    this.#renderTimer = setTimeout(() => void this.#renderOverlaysOnly(), 75);
  }

  #commitRenderResult(result, renderMs, started, figletFailures = new Set()) {
    this.#state.result = result;
    this.#element("raw-output").value = result.content;
    this.#form.querySelectorAll("[data-copy-output]").forEach((button) => {
      button.disabled = false;
    });
    this.#element("download-output").disabled = false;
    this.#element("download-png").disabled = false;
    this.#preview.draw(result);
    this.#textControls.setGrid(result.columns, result.rows);
    // The canvas owns the painted pixels now; keep the document result small
    // instead of retaining a multi-megabyte RGBA transfer until next render.
    delete result.previewRgba;
    result.workerRenderMs = renderMs;
    if (!this.#restoring) {
      void this.#persistence.store("result", {
        sourceRevision: this.#state.sourceRevision, revision: this.#documentRevision,
        result, options: this.#overlayRenderOptions, lockedOcr: this.#lockedOcr, lockedOcrGrid: this.#lockedOcrGrid,
      });
      this.#saveSession();
    }
    const elapsed = performance.now() - started;
    this.#renderStatus.update(result, elapsed);
    const figletNote = figletFailures.size
      ? ` · ${figletFailures.size} FIGlet files unavailable`
      : "";
    this.#setStatus(`${result.columns} × ${result.rows} cells · ${Math.round(elapsed)} ms${figletNote}`);
    const status = this.#element("status");
    status.dataset.renderMs = renderMs.toFixed(1);
    status.dataset.totalMs = elapsed.toFixed(1);
  }

  async #renderOverlaysOnly() {
    if (this.#fontScanInFlight || this.#fontScanQueued) { this.#renderQueued = true; return; }
    if (!this.#state.hasImage) return;
    if (!this.#overlayRenderOptions || !this.#state.result) {
      this.#scheduleRender();
      return;
    }
    if (this.#renderInFlight) {
      this.#overlayRenderQueued = true;
      return;
    }

    const renderRevision = this.#documentRevision;
    const options = this.#overlayRenderOptions;
    const { columns, rows } = this.#state.result;
    this.#renderInFlight = true;
    this.#saveSession();
    this.#setStatus("Updating text overlays…", false, true);
    const started = performance.now();
    try {
      const figlet = await this.#resolveFigletOverlays(columns, rows);
      if (renderRevision !== this.#documentRevision) return;
      const { result, renderMs } = await this.#renderOutput({
        ...options,
        overlays: renderableOverlays(this.#state.overlays),
      }, renderRevision);
      if (renderRevision !== this.#documentRevision) return;
      this.#commitRenderResult(result, renderMs, started, new Set(figlet?.failed ?? []));
    } catch (error) {
      if (renderRevision === this.#documentRevision) {
        this.#setStatus(`Could not update text overlays: ${error}`, true);
      }
    } finally {
      this.#element("render-progress").hidden = true;
      this.#renderInFlight = false;
      if (renderRevision !== this.#documentRevision && this.#element("status").dataset.busy === "true") {
        this.#setStatus("Settings changed — click Render");
      }
      this.#saveSession();
      if (this.#fontScanQueued) {
        this.#fontScanQueued = false;
        queueMicrotask(() => void this.#optimizeFontSize());
      } else if (this.#renderQueued) {
        this.#renderQueued = false;
        this.#overlayRenderQueued = false;
        queueMicrotask(() => this.render());
      } else if (this.#overlayRenderQueued) {
        this.#overlayRenderQueued = false;
        queueMicrotask(() => void this.#renderOverlaysOnly());
      }
    }
  }

  #updateOutputFontSize() {
    const control = this.#element("output-font-size");
    if (!control.validity.valid) return;
    this.#state.outputFontSize = Number(control.value);
    this.#preview.setOutputFontSize(this.#state.outputFontSize);
  }

  async #transformedImageBlob(options) {
    const transformed = await this.#renderer.getTransformedImage(options);
    const canvas = typeof OffscreenCanvas === "function"
      ? new OffscreenCanvas(transformed.width, transformed.height)
      : Object.assign(document.createElement("canvas"), {
        width: transformed.width,
        height: transformed.height,
      });
    const context = canvas.getContext("2d");
    if (!context) throw new Error("A 2D canvas is required to prepare OCR");
    context.putImageData(new ImageData(
      new Uint8ClampedArray(
        transformed.rgba.buffer,
        transformed.rgba.byteOffset,
        transformed.rgba.byteLength,
      ),
      transformed.width,
      transformed.height,
    ), 0, 0);
    return typeof canvas.convertToBlob === "function"
      ? canvas.convertToBlob({ type: "image/png" })
      : new Promise((resolve, reject) => canvas.toBlob(
        (blob) => blob ? resolve(blob) : reject(new Error("Could not encode the OCR image")),
        "image/png",
      ));
  }

  #optionsForTransformedImage(options) {
    return {
      ...options,
      // Only finished-art colour edits remain after source effects and OCR.
      pipelineOrder: options.pipelineOrder?.filter((id) => id === "replace"
        || (id === "quantize" && !options.quantizeSource)),
      extraEffects: [], cropTop: 0, cropRight: 0, cropBottom: 0, cropLeft: 0,
      quantizeEnabled: options.quantizeSource ? false : options.quantizeEnabled,
      quantizeColours: options.quantizeSource ? 0 : options.quantizeColours,
      rotate: 0,
      flipHorizontal: false,
      flipVertical: false,
      scaleX: 1,
      scaleY: 1,
      brightness: 0,
      contrast: 0,
      lumaContrast: 0,
      medianBlur: 0,
      lineThickness: 0,
      gamma: 0,
      saturation: 0,
      hue: 0,
      invert: false,
      dither: 0,
      grayscale: false,
      noGrayscale: false,
      pixelize: 0,
      boxBlur: false,
      gaussianBlur: 0,
    };
  }

  async #loadSourceImage(blob, label = "clipboard image") {
    if (!isImageFile(blob) && !/\.svg$/i.test(label)) {
      this.#setStatus("The selected clipboard item is not an image", true);
      return;
    }
    const sourceRevision = ++this.#sourceLoadRevision;
    this.#documentRevision += 1;
    this.#overlayRenderOptions = undefined;
    if (this.#renderRequestInFlight || this.#fontScanInFlight) this.#interruptRenderer();
    clearTimeout(this.#renderTimer);
    this.#setStatus("Loading image…", false, true);
    try {
      const originalBlob = /\.svg$/i.test(label) && blob.type !== "image/svg+xml"
        ? new Blob([blob], { type: "image/svg+xml" }) : blob;
      blob = await rasterizeSvg(blob, label);
      const bytes = await blob.arrayBuffer();
      if (sourceRevision !== this.#sourceLoadRevision) return;
      if (!await this.#ensureRenderer()) return;
      if (sourceRevision !== this.#sourceLoadRevision) return;
      // Ownership of this buffer moves to the worker. It decodes and retains
      // the source once, then reuses it for subsequent renders.
      await this.#renderer.setImage(bytes);
      if (sourceRevision !== this.#sourceLoadRevision) return;
      this.#sourceName = label.replace(/\.[^.]+$/, "") || "img2irc";
      this.#sourceLabel = label;
      this.#state.setSource(URL.createObjectURL(originalBlob), blob, originalBlob);
      await this.#persistence.store("source", {
        blob, originalBlob, label, name: this.#sourceName, revision: this.#state.sourceRevision,
      });
      if (sourceRevision !== this.#sourceLoadRevision) return;
    } catch (error) {
      if (sourceRevision === this.#sourceLoadRevision) {
        this.#setStatus(`Could not open image: ${error}`, true);
      }
      return;
    }
    this.#form.querySelectorAll("[data-copy-output]").forEach((button) => {
      button.disabled = true;
    });
    this.#element("download-output").disabled = true;
    this.#element("download-png").disabled = true;
    this.#element("replace-pick").disabled = false;
    this.#element("replace-pick-to").disabled = false;
    this.#colourPicker.close();
    this.#rendererSourceKey = `original:${this.#state.sourceRevision}`;
    this.#ocrOverlayKey = undefined;
    this.#lockedOcr = undefined;
    this.#lockedOcrGrid = undefined;
    const ocrLock = this.#element("ocr-lock");
    if (ocrLock) ocrLock.checked = false;
    this.#textControls.clearOcrOverlays();
    this.#setOcrStatus("OCR loads only when enabled.");
    this.#showSourceLink();
    this.#setStatus("Image ready — click Render");
    this.#saveSession();
    if (this.#element("auto-render").checked) this.#scheduleRender();
  }

  async render() {
    if (this.#fontScanInFlight || this.#fontScanQueued) { this.#renderQueued = true; return; }
    if (!this.#state.hasImage) {
      this.#setStatus("Open an image to render");
      return;
    }
    if (this.#renderInFlight) {
      this.#renderQueued = true;
      return;
    }
    if (!this.#form.reportValidity()) {
      const invalid = this.#form.querySelector("input:invalid, select:invalid, textarea:invalid");
      const label = invalid?.labels?.[0]?.textContent.trim() || invalid?.id || "settings";
      this.#setStatus(`Check ${label}: ${invalid?.validationMessage || "invalid value"}`, true);
      return;
    }

    const renderRevision = this.#documentRevision;
    this.#renderInFlight = true;
    this.#saveSession();
    try {
      const ocrEnabled = this.#element("ocr-enabled").checked;
      this.#state.setOcrOverlaysEnabled(ocrEnabled);
      const ocrLockRequested = ocrEnabled && Boolean(this.#element("ocr-lock")?.checked);
      const options = collectRenderOptions(this.#form, this.#glyphs, []);
      if (!await this.#ensureRenderer()) return;
      const preserveEditedOcr = Boolean(this.#state.ocrOverlaysDirty);
      const useLockedOcr = (ocrLockRequested || preserveEditedOcr) && Boolean(this.#lockedOcr);

      let preparedOcr;
      let ocrWidthNote = "";
      let effectiveRenderOptions = options;
      const preparationKey = String(renderRevision);

      if (ocrEnabled) {
        this.#setStatus(
          useLockedOcr ? "Applying locked OCR regions…" : "Preparing transformed image for OCR…",
          false,
          true,
        );
        await this.#ensureRendererSource(
          this.#state.sourceBlob,
          `original:${this.#state.sourceRevision}`,
        );
        if (renderRevision !== this.#documentRevision) return;

        const sourceOptions = useLockedOcr
          && this.#element("ocr-auto-width").checked
          && this.#lockedOcrGrid
          ? {
            ...options,
            width: this.#lockedOcrGrid.width,
            height: this.#lockedOcrGrid.height,
          }
          : options;
        const transformedBlob = await this.#transformedImageBlob(sourceOptions);
        if (renderRevision !== this.#documentRevision) return;
        effectiveRenderOptions = this.#optionsForTransformedImage(sourceOptions);

        if (useLockedOcr) {
          preparedOcr = this.#lockedOcr;
          const processedBytes = await this.#ocr.reapplyBackground(transformedBlob, preparedOcr);
          if (renderRevision !== this.#documentRevision) return;
          await this.#ensureRendererSource(
            processedBytes ?? transformedBlob,
            `ocr-locked:${this.#state.sourceRevision}:${preparationKey}:${preparedOcr.key}`,
          );
        } else {
          this.#setStatus("Preparing OCR…", false, true);
          preparedOcr = await this.#ocr.prepare(
            transformedBlob,
            this.#ocrSettings(),
            (message) => {
              this.#setStatus(message, false, true);
              this.#setOcrStatus(message);
            },
          );
          if (renderRevision !== this.#documentRevision) return;

          if (preparedOcr.processedBytes) {
            await this.#ensureRendererSource(
              preparedOcr.processedBytes,
              `ocr:${this.#state.sourceRevision}:${preparationKey}:${preparedOcr.key}`,
            );
          } else {
            await this.#ensureRendererSource(
              transformedBlob,
              `transformed:${this.#state.sourceRevision}:${preparationKey}`,
            );
          }
          this.#lockedOcr = preparedOcr;
        }
      } else {
        if (!this.#state.ocrOverlaysDirty) {
          this.#lockedOcr = undefined;
          this.#lockedOcrGrid = undefined;
          this.#textControls.clearOcrOverlays();
        }
        await this.#ensureRendererSource(
          this.#state.sourceBlob,
          `original:${this.#state.sourceRevision}`,
        );
      }
      if (renderRevision !== this.#documentRevision) return;
      this.#setStatus("Rendering output…", false, true);
      if (preparedOcr && !useLockedOcr && this.#element("ocr-auto-width").checked) {
        const fit = ocrFitWidth(
          preparedOcr,
          1,
          4096,
          Number(this.#element("ocr-text-coverage").value),
        );
        if (fit) {
          effectiveRenderOptions = { ...effectiveRenderOptions, width: fit.width, height: undefined };
          const coverage = Math.floor(fit.coverage + 1e-6);
          ocrWidthNote = ` · auto width ${fit.width} · ${coverage}% text coverage`;
          if (fit.coverage + 1e-6 < fit.targetCoverage) {
            ocrWidthNote += ` (target ${fit.targetCoverage}%; all text needs ${fit.required})`;
          }
        }
      }
      const started = performance.now();
      const needsOverlayPass = Boolean(preparedOcr) || this.#state.overlays.length > 0;
      const firstOptions = needsOverlayPass
        ? { ...effectiveRenderOptions, includePreview: false, includeCells: false }
        : effectiveRenderOptions;
      let { result, renderMs } = await this.#renderOutput(firstOptions, renderRevision);
      if (preparedOcr && !useLockedOcr) {
        this.#lockedOcrGrid = { width: result.columns, height: result.rows };
      }
      let ocrFigletResult;
      if (preparedOcr && !useLockedOcr) {
        const figletSettings = this.#ocrFigletSettings();
        const overlayKey = JSON.stringify([
          this.#state.sourceRevision,
          preparationKey,
          preparedOcr.key,
          result.columns,
          result.rows,
          figletSettings,
        ]);
        if (this.#ocrOverlayKey !== overlayKey) {
          const mapped = await ocrItemsToOverlays(
            preparedOcr,
            result.columns,
            result.rows,
            figletSettings,
            (requests) => this.#figlet.render(requests, (message) => {
              this.#setStatus(message, false, true);
              this.#setOcrStatus(message);
            }),
          );
          this.#textControls.setOcrOverlays(mapped.overlays);
          this.#ocrOverlayKey = overlayKey;
          ocrFigletResult = mapped.figlet;
        }
      }
      const overlayFigletResult = await this.#resolveFigletOverlays(result.columns, result.rows);
      if (needsOverlayPass) {
        const overlaid = await this.#renderOutput({
          ...effectiveRenderOptions,
          overlays: renderableOverlays(this.#state.overlays),
        }, renderRevision);
        result = overlaid.result;
        renderMs += overlaid.renderMs;
      }
      const figletFailures = new Set([
        ...(ocrFigletResult?.failed ?? []),
        ...(overlayFigletResult?.failed ?? []),
      ]);
      if (preparedOcr) {
        const cacheNote = preserveEditedOcr
          ? " · edited overlays preserved"
          : useLockedOcr
            ? " · locked"
            : preparedOcr.cacheHit ? " · cached" : "";
        const figletCount = this.#state.ocrOverlays
          .filter((overlay) => overlay.figletFont && overlay.figletFont !== "plain").length;
        const figletNote = figletCount ? ` · ${figletCount} FIGlet` : "";
        const unavailableNote = figletFailures.size
          ? ` · ${figletFailures.size} FIGlet fonts unavailable`
          : "";
        this.#setOcrStatus(
          `${this.#state.ocrOverlays.length}/${preparedOcr.result.items.length} regions rendered · ${Math.round(preparedOcr.elapsedMs)} ms${cacheNote}${figletNote}${unavailableNote}${ocrWidthNote}`,
        );
      }
      if (renderRevision !== this.#documentRevision) {
        if (!this.#element("auto-render").checked
          && this.#element("status").dataset.busy === "true") {
          this.#setStatus("Settings changed — click Render");
        }
        return;
      }
      this.#overlayRenderOptions = { ...effectiveRenderOptions };
      this.#commitRenderResult(result, renderMs, started, figletFailures);
    } catch (error) {
      if (renderRevision === this.#documentRevision) {
        if (this.#element("ocr-enabled").checked) {
          this.#setOcrStatus(`OCR failed: ${error}`, true);
        }
        this.#setStatus(`Render failed: ${error}`, true);
      }
    } finally {
      this.#element("render-progress").hidden = true;
      this.#renderInFlight = false;
      if (renderRevision !== this.#documentRevision && this.#element("status").dataset.busy === "true") {
        this.#setStatus("Settings changed — click Render");
      }
      this.#saveSession();
      if (this.#fontScanQueued) {
        this.#fontScanQueued = false;
        queueMicrotask(() => void this.#optimizeFontSize());
      } else if (this.#renderQueued) {
        this.#renderQueued = false;
        this.#overlayRenderQueued = false;
        queueMicrotask(() => this.render());
      } else if (this.#overlayRenderQueued) {
        this.#overlayRenderQueued = false;
        queueMicrotask(() => void this.#renderOverlaysOnly());
      }
    }
  }

  #bindEvents() {
    initializeSliderToggles(this.#form);
    this.#element("render-cancel").addEventListener("click", () => this.#stopRender());
    this.#form.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.render();
    });
    this.#form.addEventListener("pipelinechange", () => this.#markRenderDirty());
    this.#form.addEventListener("input", (event) => {
      if (event.target.matches('input[type="range"]')) updateRangeOutput(event.target);
      if (event.target.id === "output-font-size") {
        this.#updateOutputFontSize();
        return;
      }
      if (event.target.id === "preview-mode") {
        this.#preview.setMode(event.target.value);
        if (event.target.value === "text" && this.#state.hasImage && !this.#preview.hasText()) {
          void this.render();
        }
        return;
      }
      if (event.target.id === "auto-render") {
        if (event.target.checked) this.#scheduleRender();
        else clearTimeout(this.#renderTimer);
        return;
      }
      if (event.target.id === "view-controls") {
        this.#form.dataset.controlsVisible = String(event.target.checked);
        return;
      }
      if (event.target.id === "ocr-lock") {
        if (!event.target.checked && !this.#state.ocrOverlaysDirty) {
          this.#ocrOverlayKey = undefined;
          this.#lockedOcr = undefined;
          this.#lockedOcrGrid = undefined;
        }
      }
      if (event.target.id === "ocr-auto-width") this.#syncOcrAutoWidthControls();
      if (event.target.closest("#glyph-groups") || ["braille", "include", "include-ranges"].includes(event.target.id)) this.#syncGlyphPresetButtons();
      if (["font", "image", "glyph-search", "pipeline-filter"].includes(event.target.id)) return;
      this.#markRenderDirty();
    });

    this.#element("image").addEventListener("change", async () => {
      const file = this.#element("image").files[0];
      if (!file) return;
      await this.#loadSourceImage(file, file.name);
    });
    this.#form.querySelectorAll("[data-control-tab]").forEach((button) => {
      button.addEventListener("click", () => this.#selectControlTab(button.dataset.controlTab));
    });
    this.#bindApplicationMenu();
    this.#element("ocr-refresh").addEventListener("click", () => {
      if (this.#state.ocrOverlaysDirty
        && !window.confirm("Regenerate OCR and discard your edited OCR overlays?")) {
        return;
      }
      const ocrLock = this.#element("ocr-lock");
      if (ocrLock) ocrLock.checked = false;
      this.#lockedOcr = undefined;
      this.#lockedOcrGrid = undefined;
      this.#ocrRefreshRevision += 1;
      this.#ocrOverlayKey = undefined;
      this.#textControls.clearOcrOverlays();
      this.#figlet.reload();
      this.#markRenderDirty();
      if (!this.#element("auto-render").checked && this.#state.hasImage) void this.render();
    });
    window.addEventListener("paste", (event) => {
      const imageItem = [...(event.clipboardData?.items ?? [])]
        .find((item) => item.kind === "file" && item.type.startsWith("image/"));
      const image = imageItem?.getAsFile();
      if (!image) return;
      event.preventDefault();
      void this.#loadSourceImage(image);
    });

    let dragDepth = 0;
    const isFileDrag = (event) => [...(event.dataTransfer?.types ?? [])].includes("Files");
    const setDropActive = (active) => {
      this.#form.dataset.dropActive = String(active);
    };
    window.addEventListener("dragenter", (event) => {
      if (!isFileDrag(event)) return;
      event.preventDefault();
      dragDepth += 1;
      setDropActive(true);
    });
    window.addEventListener("dragover", (event) => {
      if (!isFileDrag(event)) return;
      event.preventDefault();
      event.dataTransfer.dropEffect = "copy";
    });
    window.addEventListener("dragleave", (event) => {
      if (!isFileDrag(event)) return;
      dragDepth = Math.max(0, dragDepth - 1);
      if (dragDepth === 0) setDropActive(false);
    });
    window.addEventListener("drop", (event) => {
      if (!isFileDrag(event)) return;
      event.preventDefault();
      dragDepth = 0;
      setDropActive(false);
      const image = [...event.dataTransfer.files]
        .find(isImageFile);
      if (image) void this.#loadSourceImage(image, image.name);
      else this.#setStatus("The dropped files did not contain an image", true);
    });
    window.addEventListener("blur", () => {
      dragDepth = 0;
      setDropActive(false);
    });

    this.#element("font").addEventListener("change", async () => {
      this.#documentRevision += 1;
      if (this.#renderRequestInFlight || this.#fontScanInFlight) this.#interruptRenderer();
      this.#overlayRenderOptions = undefined;
      this.#rendererSourceKey = undefined;
      clearTimeout(this.#renderTimer);
      try {
        if (!await this.#ensureRenderer()) return;
        if (this.#element("auto-render").checked && this.#state.hasImage) {
          this.#scheduleRender();
        } else if (this.#state.hasImage) {
          this.#setStatus("Font ready — click Render");
        } else {
          this.#setStatus("Ready — open, paste, or drop an image");
        }
      } catch (error) {
        this.#setStatus(`Font failed: ${error}`, true);
      }
    });
    this.#element("ocr-enabled").addEventListener("input", (event) => {
      const enabled = event.currentTarget.checked;
      this.#state.setOcrOverlaysEnabled(enabled);
      this.#element("ocr-summary").textContent = "OCR";
      if (!enabled) {
        if (this.#state.ocrOverlaysDirty) {
          this.#setOcrStatus("Edited OCR overlays are hidden and preserved until OCR is enabled again.");
        } else {
          this.#textControls.clearOcrOverlays();
          this.#ocrOverlayKey = undefined;
          this.#lockedOcr = undefined;
          this.#lockedOcrGrid = undefined;
          this.#setOcrStatus("OCR loads only when enabled.");
        }
        const ocrLock = this.#element("ocr-lock");
        if (ocrLock) ocrLock.checked = false;
      }
      this.#textControls.refresh();
    });
    this.#element("braille").addEventListener("change", () => {
      const disabled = this.#element("braille").checked;
      this.#element("glyph-controls").querySelectorAll("input, textarea, button").forEach((element) => {
        element.disabled = disabled;
      });
    });
    this.#element("glyph-search").addEventListener("input", (event) => {
      filterGlyphGroups(this.#element("glyph-groups"), event.target.value);
    });
    for (const preset of ["default", "smooth", "all", "none"]) {
      this.#element(`glyph-${preset}`).addEventListener("click", () => {
        this.#selectGlyphPreset(preset);
      });
    }
    this.#form.querySelectorAll("[data-glyph-preset]").forEach((button) => {
      button.addEventListener("click", () => this.#selectGlyphPreset(button.dataset.glyphPreset));
    });
    this.#element("optimize-font-size").addEventListener("click", () => void this.#optimizeFontSize());
    this.#element("hint-use-smooth").addEventListener("click", () => {
      this.#selectGlyphPreset("smooth"); this.#element("smooth-glyph-hint").hidden = true;
    });
    this.#element("hint-dismiss").addEventListener("click", () => {
      this.#smoothHintSeen = true;
      try { localStorage.setItem("img2irc.smooth-hint-dismissed", "true"); } catch {}
      this.#syncSmoothHint();
    });
    this.#element("reset-adjustments").addEventListener("click", () => {
      resetAdjustments(this.#form);
      this.#markRenderDirty();
    });
    this.#form.querySelectorAll("[data-copy-output]").forEach((button) => {
      button.addEventListener("click", () => this.#copyOutput());
    });
    this.#element("download-output").addEventListener("click", () => void this.#downloadOutput("txt"));
    this.#element("download-png").addEventListener("click", () => void this.#downloadOutput("png"));
    window.addEventListener("beforeunload", () => {
      this.#renderer.terminate();
      void this.#ocr.dispose();
    }, { once: true });
  }

  #bindApplicationMenu() {
    const menus = [...this.#form.querySelectorAll("[data-app-menu]")];
    const closeMenus = (except) => {
      for (const menu of menus) {
        if (menu !== except) menu.open = false;
      }
    };

    for (const menu of menus) {
      menu.querySelector("summary").addEventListener("click", () => {
        if (!menu.open) closeMenus(menu);
      });
      menu.addEventListener("toggle", () => {
        if (menu.open) closeMenus(menu);
      });
      menu.querySelectorAll("[role='menuitem'], [role='menuitemcheckbox']").forEach((item) => {
        item.addEventListener("click", () => queueMicrotask(() => { menu.open = false; }));
      });
    }
    document.addEventListener("pointerdown", (event) => {
      if (!event.target.closest("[data-app-menu]")) closeMenus();
    });

    this.#form.querySelectorAll("[data-click-target]").forEach((item) => {
      item.addEventListener("click", () => this.#element(item.dataset.clickTarget)?.click());
    });
    this.#form.querySelectorAll("[data-preview-mode]").forEach((item) => {
      item.addEventListener("click", () => {
        const select = this.#element("preview-mode");
        select.value = item.dataset.previewMode;
        select.dispatchEvent(new Event("input", { bubbles: true }));
      });
    });
    this.#form.querySelectorAll("[data-toggle-target]").forEach((item) => {
      item.addEventListener("click", () => {
        const target = this.#element(item.dataset.toggleTarget);
        target.open = !target.open;
      });
    });

    window.addEventListener("keydown", (event) => {
      if (event.key === "Escape" && menus.some((menu) => menu.open)) {
        const openMenu = menus.find((menu) => menu.open);
        closeMenus();
        openMenu?.querySelector("summary")?.focus();
        return;
      }
      if (event.key === "Escape" && !document.querySelector("dialog[open]") && (this.#renderInFlight || this.#fontScanInFlight)) {
        this.#stopRender(); return;
      }
      if (!(event.ctrlKey || event.metaKey)) return;
      const key = event.key.toLowerCase();
      if (key === "o") {
        event.preventDefault();
        this.#element("image").click();
      } else if (key === "s") {
        event.preventDefault();
        this.#element("download-output").click();
      } else if (key === "c" && event.shiftKey) {
        event.preventDefault();
        this.#element("copy-output").click();
      } else if (event.key === "Enter") {
        event.preventDefault();
        this.#element("render-button").click();
      }
    });
  }

  async #copyOutput() {
    if (!this.#state.result) return;
    try {
      await navigator.clipboard.writeText(this.#state.result.content);
      this.#setStatus(`Copied ${this.#element("render").selectedOptions[0].textContent} output`);
    } catch {
      const raw = this.#element("raw-output");
      raw.focus();
      raw.select();
      if (document.execCommand("copy")) this.#setStatus("Copied encoded output");
      else this.#setStatus("Copy failed: clipboard permission was denied", true);
    }
  }

  async #downloadOutput(kind = "txt") {
    if (!this.#state.result) return;
    const result = this.#state.result;
    try {
      const name = await saveFile(kind, () => kind === "png" ? this.#preview.toPngBlob(result)
        : new Blob([result.content], { type: "text/plain;charset=utf-8" }), this.#sourceName);
      if (name) this.#setStatus(`Saved ${name}`);
    } catch (error) {
      if (error.name !== "AbortError") this.#setStatus(`Save failed: ${error}`, true);
    }
  }
}
