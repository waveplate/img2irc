export class EditorState {
  hasImage = false;
  sourceObjectUrl = undefined;
  result = undefined;
  fontKey = "local:cascadia";
  outputFontSize = 16;

  // This is deliberately part of the document state now. The text tool can
  // add selection/drag behavior without moving overlay ownership into the DOM.
  overlays = [];

  setSource(objectUrl) {
    if (this.sourceObjectUrl) URL.revokeObjectURL(this.sourceObjectUrl);
    this.hasImage = true;
    this.sourceObjectUrl = objectUrl;
    this.result = undefined;
  }
}
