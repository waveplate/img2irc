export class EditorState {
  hasImage = false;
  sourceObjectUrl = undefined;
  result = undefined;
  fontKey = "local:iosevka-fixed-extended";
  outputFontSize = 16;
  sourceBlob = undefined;
  sourceRevision = 0;

  manualOverlays = [];
  ocrOverlays = [];
  ocrOverlaysDirty = false;
  ocrOverlaysEnabled = true;

  get overlays() {
    return [
      ...(this.ocrOverlaysEnabled ? this.ocrOverlays : []),
      ...this.manualOverlays,
    ];
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

  setSource(objectUrl, blob) {
    if (this.sourceObjectUrl) URL.revokeObjectURL(this.sourceObjectUrl);
    this.hasImage = true;
    this.sourceObjectUrl = objectUrl;
    this.sourceBlob = blob;
    this.sourceRevision += 1;
    this.result = undefined;
    this.clearOcrOverlays();
    this.ocrOverlaysEnabled = true;
  }
}
