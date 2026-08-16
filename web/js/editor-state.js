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

  get overlays() {
    return [...this.ocrOverlays, ...this.manualOverlays];
  }

  setSource(objectUrl, blob) {
    if (this.sourceObjectUrl) URL.revokeObjectURL(this.sourceObjectUrl);
    this.hasImage = true;
    this.sourceObjectUrl = objectUrl;
    this.sourceBlob = blob;
    this.sourceRevision += 1;
    this.result = undefined;
    this.ocrOverlays = [];
  }
}
