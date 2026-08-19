import init, { Img2Irc } from "../pkg/img2irc_rs.js";

let renderer;
let pendingFigletFonts;

function errorMessage(error) {
  if (error instanceof Error) return error.message;
  return String(error);
}

function requireRenderer() {
  if (!renderer) throw new Error("A render font has not been loaded");
  return renderer;
}

function respond(id, value, transfer = []) {
  self.postMessage({ id, ok: true, value }, transfer);
}

self.addEventListener("message", (event) => {
  const { id, type } = event.data;
  try {
    switch (type) {
      case "set-font": {
        const bytes = new Uint8Array(event.data.bytes);
        if (renderer) renderer.setFont(bytes);
        else renderer = new Img2Irc(bytes);
        if (pendingFigletFonts) {
          renderer.setFigletFonts(pendingFigletFonts);
          pendingFigletFonts = null;
        }
        respond(id);
        break;
      }
      case "set-image":
        requireRenderer().setImage(new Uint8Array(event.data.bytes));
        respond(id);
        break;
      case "set-figlet-fonts":
        if (renderer) {
          renderer.setFigletFonts(event.data.fonts);
        } else {
          pendingFigletFonts = event.data.fonts;
        }
        respond(id);
        break;
      case "render-figlet":
        respond(id, requireRenderer().renderFiglet(event.data.requests));
        break;
      case "render": {
        const started = performance.now();
        const result = requireRenderer().renderCurrent(event.data.options);
        const renderMs = performance.now() - started;
        const transfer = result.previewRgba?.buffer ? [result.previewRgba.buffer] : [];
        respond(id, { result, renderMs }, transfer);
        break;
      }
      case "get-transformed-image": {
        const result = requireRenderer().getTransformedImage(event.data.options);
        const transfer = result.rgba?.buffer ? [result.rgba.buffer] : [];
        respond(id, result, transfer);
        break;
      }
      case "clear-cache":
        requireRenderer().clearCache();
        respond(id);
        break;
      default:
        throw new Error(`Unknown renderer request: ${type}`);
    }
  } catch (error) {
    self.postMessage({ id, ok: false, error: errorMessage(error) });
  }
});

try {
  await init();
  self.postMessage({ type: "ready" });
} catch (error) {
  self.postMessage({ type: "fatal", error: errorMessage(error) });
}
