import { initializeControlHelp } from "./control-help.js";
import { EditorState } from "./editor-state.js";
import { FontService } from "./font-service.js";
import { GlyphCatalog } from "./glyph-catalog.js";
import { chooseGlyphPreset, filterGlyphGroups, populateGlyphGroups } from "./glyph-controls.js";
import { collectRenderOptions, resetAdjustments, updateRangeOutput } from "./render-options.js";
import { RendererClient } from "./renderer-client.js";
import { RenderStatus } from "./render-status.js";
import { OutputPreview } from "./output-preview.js";

const CASCADIA_URL = new URL("../assets/fonts/CascadiaCode-Regular.ttf", import.meta.url);
const IOSEVKA_FIXED_URL = new URL("../assets/fonts/IosevkaFixed-Regular.ttf", import.meta.url);
const IOSEVKA_FIXED_EXTENDED_URL = new URL(
  "../assets/fonts/IosevkaFixed-Extended.ttf",
  import.meta.url,
);
const GLYPH_CATALOG_URL = new URL("../glyphs.json", import.meta.url);
const HOSTED_FONTS = new Map([
  ["local:cascadia", CASCADIA_URL],
  ["local:iosevka-fixed", IOSEVKA_FIXED_URL],
  ["local:iosevka-fixed-extended", IOSEVKA_FIXED_EXTENDED_URL],
]);

export class EditorApp {
  #form = document.querySelector("#controls");
  #state = new EditorState();
  #fonts = new FontService(HOSTED_FONTS);
  #glyphs = new GlyphCatalog(GLYPH_CATALOG_URL);
  #renderStatus = new RenderStatus(document.querySelector("#render-stats"));
  #preview = new OutputPreview(
    document.querySelector("#output-canvas"),
    document.querySelector("#output-text"),
    document.querySelector("#output-placeholder"),
  );
  #renderer = new RendererClient();
  #rendererFontKey;
  #fontLoadRevision = 0;
  #fontLoadKey;
  #fontLoadPromise;
  #sourceLoadRevision = 0;
  #documentRevision = 0;
  #renderInFlight = false;
  #renderQueued = false;
  #renderTimer;

  async start() {
    initializeControlHelp(this.#form);
    const glyphGroups = await this.#glyphs.load();
    populateGlyphGroups(this.#element("glyph-groups"), glyphGroups);
    this.#bindEvents();
    this.#updateOutputFontSize();
    await Promise.all([
      this.#fonts.populateGoogleFonts(this.#element("font"), this.#element("font-status")),
      this.#ensureRenderer(),
    ]);
    this.#setStatus("Ready — open, paste, or drop an image");
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
    return true;
  }

  #scheduleRender() {
    if (!this.#element("auto-render").checked || !this.#state.hasImage) return;
    clearTimeout(this.#renderTimer);
    this.#renderTimer = setTimeout(() => this.render(), 350);
  }

  #markRenderDirty() {
    this.#documentRevision += 1;
    this.#scheduleRender();
  }

  #updateOutputFontSize() {
    const control = this.#element("output-font-size");
    if (!control.validity.valid) return;
    this.#state.outputFontSize = Number(control.value);
    this.#preview.setOutputFontSize(this.#state.outputFontSize);
  }

  async #loadSourceImage(blob, label = "clipboard image") {
    if (!blob?.type?.startsWith("image/")) {
      this.#setStatus("The selected clipboard item is not an image", true);
      return;
    }
    const sourceRevision = ++this.#sourceLoadRevision;
    this.#documentRevision += 1;
    clearTimeout(this.#renderTimer);
    this.#setStatus("Loading image…", false, true);
    try {
      const bytes = await blob.arrayBuffer();
      if (sourceRevision !== this.#sourceLoadRevision) return;
      if (!await this.#ensureRenderer()) return;
      if (sourceRevision !== this.#sourceLoadRevision) return;
      // Ownership of this buffer moves to the worker. It decodes and retains
      // the source once, then reuses it for subsequent renders.
      await this.#renderer.setImage(bytes);
      if (sourceRevision !== this.#sourceLoadRevision) return;
    } catch (error) {
      if (sourceRevision === this.#sourceLoadRevision) {
        this.#setStatus(`Could not open image: ${error}`, true);
      }
      return;
    }
    this.#state.setSource(URL.createObjectURL(blob));
    const sourcePreview = this.#element("source-preview");
    sourcePreview.src = this.#state.sourceObjectUrl;
    sourcePreview.alt = `Selected source: ${label}`;
    sourcePreview.hidden = false;
    this.#element("source-placeholder").hidden = true;
    if (this.#element("auto-render").checked) this.#scheduleRender();
    else this.#setStatus("Image ready — click Render");
  }

  async render() {
    if (!this.#state.hasImage) {
      this.#setStatus("Open an image to render");
      return;
    }
    if (this.#renderInFlight) {
      this.#renderQueued = true;
      return;
    }
    if (!this.#form.reportValidity()) return;

    const renderRevision = this.#documentRevision;
    this.#renderInFlight = true;
    try {
      const options = collectRenderOptions(this.#form, this.#glyphs, this.#state.overlays);
      if (!await this.#ensureRenderer()) return;
      if (renderRevision !== this.#documentRevision) return;
      this.#setStatus("Rendering output…", false, true);
      const started = performance.now();
      const { result, renderMs } = await this.#renderer.render(options);
      if (renderRevision !== this.#documentRevision) {
        if (!this.#element("auto-render").checked
          && this.#element("status").textContent === "Rendering output…") {
          this.#setStatus("Settings changed — click Render");
        }
        return;
      }
      this.#state.result = result;
      this.#element("raw-output").value = result.content;
      this.#element("copy-output").disabled = false;
      this.#element("download-output").disabled = false;
      this.#element("output-size").textContent = `${result.columns} × ${result.rows} cells`;
      this.#preview.draw(result);
      // The canvas owns the painted pixels now; keep the document result small
      // instead of retaining a multi-megabyte RGBA transfer until next render.
      delete result.previewRgba;
      result.workerRenderMs = renderMs;
      const elapsed = performance.now() - started;
      this.#renderStatus.update(result, elapsed);
      this.#setStatus(`${result.columns} × ${result.rows} cells · ${Math.round(elapsed)} ms`);
      const status = this.#element("status");
      status.dataset.renderMs = renderMs.toFixed(1);
      status.dataset.totalMs = elapsed.toFixed(1);
    } catch (error) {
      if (renderRevision === this.#documentRevision) {
        this.#setStatus(`Render failed: ${error}`, true);
      }
    } finally {
      this.#renderInFlight = false;
      if (this.#renderQueued) {
        this.#renderQueued = false;
        queueMicrotask(() => this.render());
      }
    }
  }

  #bindEvents() {
    this.#form.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.render();
    });
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
      if (["font", "image", "glyph-search"].includes(event.target.id)) return;
      this.#markRenderDirty();
    });

    this.#element("image").addEventListener("change", async () => {
      const file = this.#element("image").files[0];
      if (!file) return;
      await this.#loadSourceImage(file, file.name);
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
        .find((file) => file.type.startsWith("image/"));
      if (image) void this.#loadSourceImage(image, image.name);
      else this.#setStatus("The dropped files did not contain an image", true);
    });
    window.addEventListener("blur", () => {
      dragDepth = 0;
      setDropActive(false);
    });

    this.#element("font").addEventListener("change", async () => {
      this.#documentRevision += 1;
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
    this.#element("smooth").addEventListener("change", () => {
      this.#element("smoothing-options").hidden = !this.#element("smooth").checked;
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
        chooseGlyphPreset(this.#element("glyph-groups"), preset);
        this.#markRenderDirty();
      });
    }
    this.#element("reset-adjustments").addEventListener("click", () => {
      resetAdjustments(this.#form);
      this.#markRenderDirty();
    });
    this.#element("copy-output").addEventListener("click", () => this.#copyOutput());
    this.#element("download-output").addEventListener("click", () => this.#downloadOutput());
    window.addEventListener("beforeunload", () => this.#renderer.terminate(), { once: true });
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

  #downloadOutput() {
    if (!this.#state.result) return;
    const extension = this.#element("render").value === "irc" ? "irc" : "ans";
    const url = URL.createObjectURL(new Blob(
      [this.#state.result.content],
      { type: "text/plain;charset=utf-8" },
    ));
    const link = document.createElement("a");
    link.href = url;
    link.download = `img2irc.${extension}`;
    link.click();
    URL.revokeObjectURL(url);
  }
}
