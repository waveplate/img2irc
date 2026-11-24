export class EditorState {
  hasImage = false;
  sourceObjectUrl = undefined;
  result = undefined;
  fontKey = "local:iosevka-fixed-extended";
  outputFontSize = 16;
  sourceBlob = undefined;
  originalBlob = undefined;
  sourceRevision = 0;

  manualOverlays = [];
  ocrOverlays = [];
  ocrOverlaysDirty = false;
  ocrOverlaysEnabled = true;

  get overlays() {
    return this.#ordered(this.ocrOverlaysEnabled);
  }

  #ordered(includeOcr) {
    return [...(includeOcr ? this.ocrOverlays : []), ...this.manualOverlays]
      .map((overlay, index) => ({ overlay, index }))
      .sort((a, b) => (a.overlay.zOrder ?? a.index) - (b.overlay.zOrder ?? b.index) || a.index - b.index)
      .map(({ overlay }) => overlay);
  }

  reorderOverlay(overlay, action) {
    const visible = this.overlays;
    const index = visible.indexOf(overlay);
    if (index < 0) return false;
    const target = action === "back" ? 0 : action === "front" ? visible.length - 1
      : action === "lower" ? Math.max(0, index - 1) : Math.min(visible.length - 1, index + 1);
    if (index === target) return false;
    const all = this.#ordered(true);
    const from = all.indexOf(overlay);
    const to = all.indexOf(visible[target]);
    if (all.slice(Math.min(from, to), Math.max(from, to) + 1).some((item) => this.ocrOverlays.includes(item))) {
      this.markOcrOverlaysEdited();
    }
    all.splice(from, 1);
    all.splice(to, 0, overlay);
    all.forEach((item, rank) => { item.zOrder = rank; });
    return true;
  }

  addManualOverlay(overlay) {
    const ordered = this.#ordered(true);
    ordered.forEach((item, rank) => { item.zOrder = rank; });
    overlay.zOrder = ordered.length;
    this.manualOverlays.push(overlay);
  }

  replaceOcrOverlays(overlays) {
    this.ocrOverlays = overlays;
    this.ocrOverlaysDirty = false;
  }

  markOcrOverlaysEdited() {
    this.ocrOverlaysDirty = true;
  }

  setOcrOverlaysEnabled(enabled) {
    this.ocrOverlaysEnabled = Boolean(enabled);
  }

  clearOcrOverlays() {
    this.ocrOverlays = [];
    this.ocrOverlaysDirty = false;
  }

  setSource(objectUrl, blob, originalBlob = blob) {
    if (this.sourceObjectUrl) URL.revokeObjectURL(this.sourceObjectUrl);
    this.hasImage = true;
    this.sourceObjectUrl = objectUrl;
    this.sourceBlob = blob;
    this.originalBlob = originalBlob;
    this.sourceRevision += 1;
    this.result = undefined;
    this.clearOcrOverlays();
    this.ocrOverlaysEnabled = true;
  }
}
