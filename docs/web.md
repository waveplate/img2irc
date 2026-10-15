# web and JavaScript

the browser editor opens, pastes, and drops images; adjusts colours and glyphs; and edits text overlays. rendering and OCR stay in browser workers. copy/download preserves encoded IRC/ANSI codes. the text preview is selectable; bitmap display avoids browser text-layout differences.

SVG files can be opened or dropped like raster images. The browser rasterizes
them before passing the pixels to the Rust renderer and OCR worker.

**Auto** beside Font Size checks all integer sizes from 9 through 30 and selects
the lowest **Perceptual** score. The Rust scan skips smoothing, Repair Edges,
region guidance, warp and OCR without changing their controls; the final render
uses the winning size and your enabled features.

The output toolbar has **Smooth**, **Balanced** and **Detailed** glyph presets.
Smooth includes polygons, thirds, horizontal eighth blocks and space. Balanced
includes polygons, eighths, full and half blocks, octants, quadrants, sextants
and space. Detailed adds large, math, misc, misc2, arrows and box glyphs.
Enabling smoothing or edge repair offers a one-time hint to use Smooth.

The File menu offers **Save text…** and **Save PNG…**. Each remembers its last
filename separately in browser storage. Browsers with a native save picker
prefill it there; other browsers show a prefilled filename prompt before the
download.

The editor saves the source image (including the original SVG), controls,
glyph selection, ordered and disabled pipeline effects, manual and OCR text
overlays, open sections, selected tab, pane widths and completed output locally.
Refreshing restores the document and its output without rerunning a completed
search. An unfinished render restarts using the saved settings.

Startup loads the render font and WASM alongside the glyph catalog; Google
Fonts and FIGlet lists load in the background. Render fonts and WASM have
compressed `.gz` copies decoded by browsers supporting `DecompressionStream`,
with the original assets retained for other browsers. Downloaded assets are
cached locally under a content version, so a new deployment cannot reuse an
older WASM/font combination. Upload the complete generated `dist/` directory.

The effect catalog uses the available width for multiple columns and scrolls
with the main panel when needed, rather than imposing a small inner viewport.
It stays open while adding effects.

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

`regionGuidance: true` enables experimental connected-region colour and glyph
fitting, with a local source-error budget and boundary constraints on subsequent
smoothing. Its editor control is currently hidden. The API option defaults
to false and works independently of smoothing and
sextant alignment. It searches glyph shapes and observed region colours together,
without averaging colours across glyph boundaries. Rebuild WASM when updating
the editor to use this option.

## editor code

The editor follows the native TUI layout and theme: General, Pipeline, Glyphs,
and Text tabs across the title bar, a control pane beside the output, contextual
help below the controls, and a status/shortcut bar along the bottom. Drag the
divider to resize a pane; each tab keeps its width until the page reloads.
Double-click the divider to restore that tab's default width. Alt+1–4 switches
tabs, Alt+Up/Down navigates controls, and Space toggles slider values.

Pipeline shows active effects. **Add Effect** appends an effect to the bottom;
ON/OFF preserves its parameters, and × removes it. Drag an effect's grip to
reorder it, or focus the grip and press Up/Down. Rendering follows that order
within the source-image and finished-art colour stages, as in the TUI. Effects
still appear once each. Glyphs previews the characters of the focused set.

OCR, Smoothing and Repair Edges each have a disclosure triangle and a separate
ON/OFF switch. Expanding or collapsing settings does not enable, disable or
restart a render; enabled sections keep running when collapsed.

**Stop** in the output toolbar (or Esc) interrupts rendering. Changing settings
during refinement also stops the superseded search. The worker restores the
loaded image and font before accepting another render, so interruption cannot
leave its request queue blocked. Stopping preserves the last completed output.
An inline, dismissible hint offers **Use Smooth** while smoothing or edge repair
is enabled with another glyph set. Dismissal is remembered across refreshes.

The pipeline includes the native image effects, including Luma Blur, Oil
Painting, Halftone, Sharpen, Edge Detection, Emboss and the colour filters.
**Crop** has four pixel margins plus **Select area…**: drag a rectangle to keep
in the image preview, then Apply. The preview includes source effects preceding
Crop, at their original pixel resolution. Reset crop clears all four sides.

The output toolbar shows rendering status and the TUI's glyph-search progress
bar, including the current pass, pair count, percentage, elapsed time, score,
and accepted moves. Preparation and verification display an indeterminate bar;
the percentage covers the current pass, and search may finish before the maximum
number of passes. WASM's `renderCurrent(options, onProgress)` accepts an optional
callback with the same search diagnostics. `pipelineOrder` optionally specifies
the enabled effect names in execution order; omitting it preserves the original
API ordering.

The editor enables `livePreview: true` with `includeCells: true`. Search progress
then includes a `preview` grid at the start of each pass and roughly every two
seconds, using the existing Rust search snapshots. Unchanged snapshots are sent
only once. Overlays and finished-art colour effects apply to these display copies;
the search and completed-result cache are unaffected. API callers can omit or
disable `livePreview` for progress messages without snapshots.

Bitmap and text previews share irc2html's native font drawing and grid layout.
Both preserve the rendered grid geometry when changing display size or font.
The renderer measures fractional font metrics, aligns the baseline once per
font and shares pixel boundaries between adjacent cells. This prevents font
leading and fractional row placement from drawing horizontal seams.
Rust's resize cache also includes the active font's raster and floating-point
cell metrics, so changing browser font bytes cannot reuse the previous font's
image dimensions.
The web build uses `../irc2html/src/index.js` when available, with the bundled
`web/vendor/irc2html.js` as fallback. Set `IRC2HTML_SOURCE` to another source entry
point when developing the renderer elsewhere.

General exposes **Repair Edges**. The API options remain
`glyphSearch`, `glyphSearchCandidates` (6 alternatives by default),
`glyphSearchPasses` (2), and `glyphSearchFinalists` (3). It searches neighbouring
glyph pairs with fixed colours and keeps only full-image Perceptual improvements.
Search first detects suspect gaps, tips and mismatched edges, then explores their
neighbours, including diagonal pairs. `discontinuityThreshold` defaults to 0.5;
0 searches all non-flat cells and disables skipping unchanged contexts. Pools
combine source and boundary fits, and source-only scoring data is cached across
trials. Finalists 0 checks every shortlisted pair combination globally and can
be slow. More stats includes the new `thin_defects` score component, evaluated
before geometry downsampling and blur.
The feature defaults to off, works independently of smoothing and region
guidance, and reports `glyphSearchMs` in timings. Rebuild WASM for these options.

The Rust/WASM renderer retains up to four completed base outputs in a 32 MiB
session cache. Returning to a cached combination of render settings skips canvas
preparation, matching, smoothing and glyph search. Text overlays and finished-art
colours are reapplied to the cached base. `resultCacheHit` in timings reports reuse.
Changing the source image or font, calling `clearCache()`, or recreating the
renderer clears this cache; older entries are evicted when it fills.

Quantize Colours offers separate colour-count and merge-threshold sliders,
source cleanup versus finished-art editing, and an inline hex palette with
removable swatches. Use current art palette freezes visible colours for editing.
Replace Colour's eyedropper also opens inline. The WASM options are
`quantizeEnabled`, `quantizeColours`, `quantizeThreshold` (0–100),
`quantizePalette` (RGB triples), and `quantizeSource`. A zero count only merges
shades; an explicit palette overrides count and threshold. Existing API calls
with only `quantizeColours` retain their finished-art, exact-colour behaviour.

`editor-app.js` coordinates controls and render scheduling; `editor-state.js` owns document/overlay state; `render-options.js` maps controls into API options. `renderer-client.js` talks to `render-worker.js`, which owns WASM and caches. `output-preview.js`, `text-controls.js`, and `ocr-service.js` handle display, overlays, and recognition. stale renders are discarded and changes coalesced.

geometry regressions can be checked with `node --test web/js/ocr-service.test.mjs`.

OCR megapixels defaults to 1.5, like the native editor. It controls detection
resolution independently of output width and never enlarges the source.
Set it to zero to use the legacy width/maximum-side controls in Advanced.
Advanced also retains ASCII and detector thresholds, box expansion and detailed
FIGlet fitting controls. The OCR model selector offers Tiny (default), Small,
and Medium, using both the detector and recognizer for the selected tier.
Recognition runs in one browser worker; switching models releases the previous
worker. Locked or edited detections stay intact until Regenerate OCR is used.

Tiny archives are hosted with the app. Small and Medium archives download from
the official PaddleOCR model host when first selected (about 31 MB and 139 MB
respectively). Image data stays in the browser. Model archives are cached as
Blobs in IndexedDB (`img2irc-ocr-models`), with separate versioned entries per
model URL. Later visits reuse those downloads; ONNX sessions still need to
initialize after reloading. If storage is unavailable or full, OCR continues
using downloaded bytes. Clearing this site's browser storage removes the cache.
