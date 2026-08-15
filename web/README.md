# Web editor architecture

The browser build is a local editor: image data, fonts, document state and
rendering stay in the browser, and `img2irc_rs.wasm` runs inside a module Web
Worker. There is no application server in the interaction loop, and expensive
renders do not block controls, status animation, or canvas interaction.

HTMX is therefore not used. HTMX is strongest when controls issue HTTP requests
and the server responds with HTML fragments; using it here would add an HTTP
round trip or a second client-side state mechanism without replacing the code
needed for WASM rendering, canvas interaction, or text-overlay dragging.

The web code instead follows a small functional-core / imperative-shell split:

- `main.js` starts the editor; the render worker initializes WASM.
- `js/editor-app.js` coordinates state, controls and render scheduling.
- `js/editor-state.js` owns document state, including the future overlay list.
- `js/render-options.js` is the form-to-WASM options boundary.
- `js/renderer-client.js` owns the typed main-thread/worker request protocol.
- `js/render-worker.js` owns WASM, decoded-image state and renderer caches.
- `js/control-help.js` owns accessible per-control explanations.
- `js/font-service.js` owns hosted and Google font discovery/caching.
- `js/glyph-catalog.js` loads and resolves the deployed glyph catalog.
- `js/glyph-controls.js` builds the glyph-group browser from WASM metadata.
- `js/output-preview.js` owns bitmap/browser-text display and cell hit-testing.
- `js/render-status.js` formats the TUI-style post-render statistics line.

`npm run build:web` compiles the WASM module and stages these source files into
`dist/`. The resulting directory is a complete static web root; deployment does
not require the repository's `web/` source directory.

The build resolves `glyphs.toml` into `dist/glyphs.json`. The editor fetches
that JSON at startup and passes selected characters explicitly to WASM, keeping
glyph-set definitions out of the renderer binary and allowing the deployed
catalog to be replaced independently.

The WASM result always includes encoded text and an RGBA raster rendered from
img2irc's actual glyph bitmaps. Bitmap display passes those pixels directly to
`ImageData`, avoiding a PNG encode/decode cycle. Browser-text display requests
the optional styled cell matrix and groups adjacent cells with identical style
into DOM runs. It uses the same downloaded font bytes as glyph matching,
disables kerning and ligatures, and applies the renderer's measured advance and
line height at the selected display size. Each row clips glyph paint to its
cell-height box, preventing block glyphs from leaking into the next row. The
result is selectable real text with the same grid aspect ratio as bitmap mode.

The source image is decoded only when it changes, and per-cell result
serialization is opt-in. Preview display size is controlled independently of
the glyph raster size used for matching, so changing output font size does not
trigger another WASM render.

Render requests are coalesced in the controller: if controls change while the
worker is busy, the obsolete result is discarded and only one render using the
latest state follows it. RGBA buffers are transferred rather than cloned, and
font changes retain the already decoded source image inside WASM. The renderer
also retains the most recent prepared image canvas, so output-mode and smoothing
changes skip repeated resizing, effects, and palette mapping. Image, font, and
image-adjustment changes invalidate that canvas.

Every render reports timings for image preparation, glyph matching, smoothing
preparation/search, optional shape refinement, final scoring, encoding, RGBA
preview generation, and worker transfer/UI work. Contour-cache hit counts are
included as well. These appear in the editor footer and make profiling a normal
release-build operation rather than requiring a debug-only build.

Smoothing keeps bounded thread-local caches for exact contour topology and for
repeat pixel rasters. The topology key packs two three-bit labels per byte, and
the warm-raster fast path uses a two-word fingerprint. This particularly helps
repeat renders and common candidate patches without tying the cache to one
image or font.

## Image adjustment pipeline

Image adjustments intentionally remain in the Rust pipeline for now. CSS
filters only alter what the user sees; glyph matching needs the adjusted pixels.
A browser-native replacement would therefore need an `OffscreenCanvas` or WebGL
pipeline in the worker, a `getImageData` readback, and a full RGBA transfer into
WASM. It would also need parity implementations for gamma, dithering, rotation,
pixelization, and the existing filters. The prepared-canvas cache already makes
unchanged adjustments show as `Prepare: cached`, while the stage timings show
whether a future worker-side GPU path would justify that extra pipeline.

## Text overlay direction

Text overlays should remain document data in `EditorState.overlays`, not DOM
state. A text-tool module can use `OutputPreview.clientPointToCell()` for
selection, placement, resizing and dragging. The app controller can pass the
overlay list through `collectRenderOptions()` once the web binding accepts it.
This keeps overlay editing independent of the control layout and avoids a later
rewrite of the preview coordinate system.
