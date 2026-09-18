# web and JavaScript

the browser editor opens, pastes, and drops images; adjusts colours and glyphs; and edits text overlays. rendering and OCR stay in browser workers. copy/download preserves encoded IRC/ANSI codes. the text preview is selectable; bitmap display avoids browser text-layout differences.

## build and serve

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.127
npm install
npm run build:web
python3 -m http.server --directory dist 8080
```

open `http://localhost:8080/`. the build needs Python 3.11 or later for TOML parsing. copy `dist/` to a static web server to deploy, including its fonts, OCR models, and worker assets.

`glyphs.json` comes from `glyphs.toml`; `figlet-fonts.json` lists hosted FIGlet files. manifest URLs resolve relative to the manifest and can point to same-origin or CORS-enabled hosts. extra monospace families are fetched from Google Fonts when selected. OCR assets load only when OCR is enabled.

## WASM API

```js
import init, { Img2Irc } from "./pkg/img2irc_rs.js";

await init();
const renderer = new Img2Irc(new Uint8Array(await fontFile.arrayBuffer()));
renderer.setImage(new Uint8Array(await imageFile.arrayBuffer()));
const result = renderer.renderCurrent({
  width: 80,
  render: "ansi24",
  glyphs: " .#@",
  includeCells: true,
});
console.log(result.content);
```

reuse the instance to retain decoded images and render caches. `setFont()` changes font bytes; `clearCache()` releases render caches. `render(bytes, options)` is the one-shot entry point. results contain encoded text, an RGBA preview, timings, and optional styled cells. generated TypeScript declarations describe the options. native OCR is excluded from WASM; overlays work independently.

## editor code

`editor-app.js` coordinates controls and render scheduling; `editor-state.js` owns document/overlay state; `render-options.js` maps controls into API options. `renderer-client.js` talks to `render-worker.js`, which owns WASM and caches. `output-preview.js`, `text-controls.js`, and `ocr-service.js` handle display, overlays, and recognition. stale renders are discarded and changes coalesced.

geometry regressions can be checked with `node --test web/js/ocr-service.test.mjs`.
