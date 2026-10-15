# usage

```bash
img2irc photo.png --width 80 --render ansi24
img2irc https://i.imgur.com/qP1uBCK.png --width 80
```

width is in columns and height is in rows. leaving one unset preserves aspect ratio; omitted width or `--width 0` selects automatic sizing. native loading also supports SVG. `--help` is the full option reference.

| `--render` | output |
| --- | --- |
| `irc` | 99-colour IRC codes |
| `ansi` | 256-colour ANSI; default |
| `ansi24` | 24-bit RGB ANSI |

## glyphs and smoothing

```bash
img2irc photo.png --width 80 --glyphs half,full,space
img2irc photo.png --width 80 --braille
img2irc photo.png --width 80 --smooth
```

group names come from [glyphs.toml](../glyphs.toml); prefixes select subgroups. `--include` / `--include-range` add characters, and `--exclude` / `--exclude-range` remove them. ranges use hexadecimal, e.g. `2500-257F`. `--print-glyph-chars` lists groups; `--print-glyph-bitmaps` shows their rasters.

smoothing tries alternate glyphs to improve contours between cells. `--smooth-candidates` (default 5) and `--smooth-orders` (default 5, maximum 5) trade render time for a wider search. `--smooth-shapes` adds shape refinement.

`--smooth` enables glyph-feature smoothing, with 5 candidates and 5 scan patterns by default (also available in the terminal and web editors). For example:

```bash
img2irc photo.png --width 80 --glyphs smooth --smooth --profile
```

Features are extracted from the actual font bitmaps at the same 8×16 preview size used by contour scoring; no glyph names or sextant coordinates are hard-coded. The search minimizes source error plus a blurred-Laplacian energy using cached glyph responses, up to four sweeps per scan pattern. It keeps the initial result available and caps added source error per cell at 1/16 of that cell's foreground/background contrast energy. This objective can soften fine details and does not guarantee improvement in the reported contour score. The former mode switch and `--smooth-features` option have been removed.

Protect neighbors additionally prevents increasing the total curvature energy in surrounding cells. Refine shapes adds the slower refinement pass after feature smoothing. Python `Advanced` uses `smooth=True`; the JavaScript API uses `smooth: true`.

`--region-guidance` enables experimental connected-region glyph fitting, inspired
by superpixel region growth. It groups nearby pixels using Oklab colour and
spatial distance, adding seeds for contrasting components missed by the regular
grid. It searches glyph shapes and colour pairs together using dominant observed
colours in coherent regions. It never adds averaged or blended shades to fit an
imperfect glyph boundary. Mixed edge samples and regions without a dominant
colour receive no colour guidance; the source image is not replaced by flat
region colours. Palette modes use the normal IRC/ANSI colour quantization.

```bash
img2irc photo.png --width 80 --glyphs smooth --region-guidance --smooth
img2irc --tui photo.png --region-guidance --optimize-log trials.jsonl
```

Region guidance works independently of smoothing and sextant alignment. Its
editor controls are currently hidden; the CLI/API option remains available.
A region change must improve the combined pixel/boundary fit
without increasing disagreement with confident region colours. With smoothing
enabled, candidate glyphs must stay within the resulting boundary-error limit
and source-pixel error budget. Added squared
RGB error is capped per guided cell at 1/64 of its original foreground/background
contrast energy, across colour fitting, shape fitting and smoothing together.
This is a local constraint, not a topology or perceptual-score guarantee.
Compare the toggle on and off on the same image and settings.

Analysis uses the same 8×16 pixels per cell at every font size and is skipped
above 1,048,576 analysis pixels. `img2irc.log` reports region count, colour/shape
changes and elapsed time. Python uses `region_guidance=True`; JavaScript uses
`regionGuidance: true`. This applies to glyph rendering; braille uses its own
renderer. The option defaults to off and adds segmentation and fitting work.

### experimental glyph combination search

`--glyph-search` first maps suspect narrow gaps/tips, missing or invented edges,
and excess contour roughness relative to the source. It searches around these
cells, prioritizing the strongest defects and including horizontal, vertical,
and diagonal neighbouring pairs. Colours stay fixed. Each cell keeps its current
glyph and up to six alternatives, split between source pixel fit and agreement
with neighbouring boundaries; duplicate rendered shapes are removed. The search
tries their Cartesian product, including changes to both glyphs simultaneously.
This can escape a fit where neither single-cell change is useful by itself.

```bash
img2irc --tui photo.png --glyph-search --region-guidance=false
img2irc photo.png --width 80 --glyphs smooth --glyph-search --profile
```

By default, combinations are ranked by Perceptual score in a window with two
cells of surrounding context. The best three are checked against the **full
image**. A pair is committed only when that full score improves. The starting
pair is always available, and the final output is rechecked through the actual
renderer. Colours are not averaged or refitted by this search.

`--disc-threshold` reuses the discontinuity threshold for search targeting
(default 0.5). Lower values include weaker defects; higher values concentrate on
stronger defects. The search also prepares alternatives for the eight neighbours
of each target, since a repair may require changing a cell that looks good by
itself. Source-only scoring data is cached across trials. Later passes skip a
pair when both its surrounding pixels and candidate pools are unchanged.
Set the threshold to **0** to search every non-flat cell and disable that skip.

On native builds, independent trial scores run across CPU cores while pairs
are committed in the same order. Output colour, narrow-feature and variation
data are cached; each trial refreshes only the changed cells and their immediate
pixel neighbours. Scoring buffers are reused. The full-image filters, score
formulas, candidate pools, ranking and improvement threshold stay the same.
Full-image checks also cache the smoothed pixels, edges, orientation dispersion,
boundary matches and coarse colour error. A proposal updates the affected rows,
columns and matching neighbours, then updates cached block totals. Filters
restart at the beginning of each affected row/column and
propagate bitwise changes, preserving floating-point results even outside the
usual filter footprint. Blocks containing only exact zero terms are skipped.
The full-image cache is shared between trials, avoiding image-sized copies for
each finalist. Repeated proposals reuse a score and its precision bounds until the next accepted
move; any accepted move invalidates that memo table. These optimizations apply
automatically.

The TUI retains up to four completed base outputs in a 32 MiB session cache.
Returning to the same source image, font/glyphs and render settings reuses a
stored output, including its smoothing and glyph search. Cancelled renders are
not stored. Changing text overlays or finished-art colours reapplies those
edits to the cached base without rerunning glyph search. Older entries can be
evicted when the cache fills; closing the TUI clears it.

`--glyph-search-candidates N` sets alternatives per cell (1–64, default 6).
`--glyph-search-passes N` sets sweeps (1–20, default 2); search stops after a
sweep makes no improvement. `--glyph-search-finalists N` sets full-image checks
per pair (default 3). Set finalists to **0** to score every combination in the
candidate pools against the full image. For a broader diagnostic search, also
disable defect targeting:

```bash
img2irc photo.png --glyphs smooth --glyph-search \
  --disc-threshold 0 --glyph-search-candidates 8 \
  --glyph-search-finalists 0 --glyph-search-passes 1
```

This is an exhaustive search over each shortlisted pair, not over every possible
whole-image arrangement. Local ranking and defect targeting may miss a global
improvement; finalists 0 removes local ranking, and threshold 0 disables targeting
and skipping unchanged contexts. Search time
grows roughly with the square of the alternatives and can be substantial.
The score remains a visual-quality heuristic; a lower score is not a guarantee
that every individual line looks better.

The native General panel has **Repair Edges** and its search controls.
It works independently of Smoothing; turn it off for an isolated comparison.
Search runs after smoothing and any region guidance enabled through the CLI
when combined. It always uses and
reports Perceptual scoring, including when the scoring toggle is off. Braille
uses a separate renderer. The option defaults to off.

`img2irc.log` records target/active cell counts, pairs skipped, progress,
combinations tried, computed/reused full-image scores, CPU worker counts,
accepted moves, how many changed both glyphs, initial/final scores, and separate
local-ranking/full-check timings. `--profile`
reports `glyph-search` time. Python uses `glyph_search=True` and the corresponding
underscore options; JavaScript uses `glyphSearch`, `glyphSearchCandidates`,
`glyphSearchPasses`, and `glyphSearchFinalists`. The targeting threshold is
`discontinuity_threshold` in Python and `discontinuityThreshold` in JavaScript.

Native Repair Edges parallelizes both local candidate ranking and independent
full-image finalist checks. Each finalist worker retains incremental
caches; extra cache memory is bounded to 256 MiB. Accepted changes are applied
in the original pair and candidate order, so parallel work does not change the
search. Very large images use fewer workers to stay within that budget.

`IMG2IRC_REPAIR_CHECK_THREADS=1` selects the previous serial full-image checking
path; the normal limit is four workers and respects the available CPU threads
and cache memory budget. These execution choices do not alter search settings.

Full-image checks store 64-bit block totals in a balanced tree. A local edit
updates its blocks and their ancestors; unchanged image suffixes are not
summed again. Fine-feature invalidation follows the changed rectangle and its
one-pixel halo rather than invalidating entire rows. Score bounds cover the
rounding difference from the original raster-order accumulation. Decisions
close enough for that rounding to matter are checked with the original scorer,
including the currently selected proposal. The score formulas, candidate pools,
pair order and improvement threshold remain the same. The final output score
is still calculated by the original scorer.

`IMG2IRC_REPAIR_PROFILE=1` adds per-cache timings for smoothing, gradients,
curvature, boundary matching, coarse colour and both score reductions to
`img2irc.log`, together with the number of precise borderline checks.

In the native TUI, **Show Search Defects** previews the actual search map. Red
cells are defect targets; amber cells are the neighbouring cells prepared for
possible joint changes. It works with Repair Edges off, so you can inspect
the plan before an expensive search. During search, the preview shows the map
at the start of each pass and updates the glyphs roughly every two seconds;
afterwards it retains the first-pass map over the completed result. The threshold
control remains available with this
preview on. Toggling an available map updates the display immediately. Native
copy/save/PNG export use the normal art.

```bash
img2irc --tui photo.png --show-glyph-defects
img2irc --tui photo.png --glyph-search --show-glyph-defects
```

The TUI shows the search stage, pass number, current-pass percentage, completed
pairs, elapsed search time, best score and accepted moves. Updates include work
inside large pair pools. A pass can finish early when no improvements remain;
the pass limit is not a promise that every pass will run. Interactive CLI
renders show the same progress on stderr; redirected output stays free of
progress text. CLI `--show-glyph-defects` prints an ANSI-coloured diagnostic view
to stdout, while `--save` still exports normal art.

With search enabled, the native TUI displays the new pre-search art as soon as
it is available, including when the defect overlay is off. Accepted repairs
appear in the live preview while the full search continues; the completed result
replaces it when finished. Source adjustments such as contrast still trigger a
fresh search, since they can change which glyphs score best. Rapid edits cancel
obsolete work immediately and wait for a 200 ms pause before starting the latest
render. Other renders keep the usual 50 ms delay. Copy/save use the completed
result while the progress indicator marks the live preview as still searching.

The native TUI caches up to four completed base renders within a 32 MiB budget.
Repeating the same source pixels, glyph bitmaps and render settings reuses the
result, skipping smoothing and glyph search; source or search-setting changes
recompute it. Text overlays, output colour edits and defect preview toggles reuse
the base result. Cancelled renders are not cached. The cache is per session and
does not make a fresh CLI process faster. A repeated cached render returns the
previous result; increase the pass/candidate limit to request a broader search.
Ordinary smoothing runs before glyph search and may reduce its target count,
but smoothing costs time too and does not guarantee a faster combined render.

### experimental perceptual scoring

The experimental reference-aware **Perceptual** score is enabled by default for the status bar and terminal auto-optimizer. Use `--perceptual-score=false`, or the General panel toggle in either editor, to compare with the original Contour score. Lower is better; the two scores have different scales. This setting changes evaluation without changing glyph matching or the smoothing objective.

```bash
img2irc photo.png --width 80 --glyphs smooth --smooth --perceptual-score --profile
```

`--superpixel-warp` enables an experimental two-pass geometry mode that moves
strong image boundaries toward the half-cell and third-cell intersections used
by the sextant glyphs. Use `--superpixel-warp=100` to request full alignment or
`--superpixel-warp=50` for half the displacement; omit it to leave the image
unchanged. The editor controls are currently hidden; the CLI/API option remains
available. Capture covers the entire region around each
intersection, including edges halfway between grid lines. The warp stays
continuous: conflicting movements are reduced locally so that one crowded
region does not weaken distant contours. A global reduction remains as a
fallback if local corrections cannot keep the mesh from folding.

Space on a slider toggles between zero and its previous value, including values
entered by typing or dragging. Scale controls use their neutral value instead;
counts that cannot be zero use their minimum. Active pipeline effects toggle
off without losing their parameters. The web editor also supports Space on its
numeric controls.

The score combines contrast-weighted contour roughness, excess fine texture, missing source edges, invented edges, coarse color agreement, and unsupported thin defects. It uses all colors, includes short contours, and allows small edge movements (up to two pixels per axis in the normalized 8×16 cell preview, with a small displacement cost). Geometry is evaluated after area sampling to 4×8 per cell; fine texture and thin defects are checked before that reduction. Thin defects measure narrow tips, strips and gaps using contrast and support from neighbouring pixels. Similar features in the source, within one preview pixel, provide an allowance, so a genuine thin source line has support. Roughness and edge matching use source-edge strength as a fixed denominator, so extra easy edges do not dilute those penalties. Small shifts and sharper boundaries are tolerated; losing a feature still costs. It is a heuristic under evaluation, not a calibrated predictor of human preferences or a topology guarantee.

The web editor's **More stats** shows all six components; native renders log them in `img2irc.log`. The added component is `thin_defects` in logs and API breakdowns. Python `Advanced` accepts `perceptual_score=True` and exposes the breakdown in `output.metadata["perceptual_score"]`. The JavaScript option is `perceptualScore: true`, with a breakdown in `result.perceptualScore`. Scoring currently applies to the glyph renderer, before text overlays and output colour edits; the separate braille renderer does not supply a quality score.

The experimental total is `0.8 × roughness + 0.15 × texture + 1.5 × thin_defects + (1.5 × missing² + 0.5 × extra² + 0.25 × colour²) / 100`. Scores from builds before the thin-defect term are not directly comparable. The squared structural terms give small approximation errors room to improve aesthetics while making large losses expensive. The components and total are costs, not percentages or probabilities; they can exceed 100. Their weights have not been fitted to human preference data.

The reference is the image after the requested transforms and color edits. Consequently this measures representation quality *within the chosen crop*. An optimizer can still discard difficult content by cropping it away; evaluating an automatic deformation/crop search against a fixed starting reference is a separate constraint. This implementation does not warp the source image.

## filters and output

```bash
img2irc photo.png --width 80 --contrast 18 --sharpen
img2irc photo.png --width 80 --median-blur 2
img2irc photo.png --render ansi24 --save preview.png > art.ans
```

encoded text goes to stdout; `--save` additionally writes a PNG preview. `--profile` prints stage timings to stderr. `--encoding` selects text encoding (default UTF-8).

`--median-blur` removes isolated pixel noise before glyph matching. Its integer
radius is 0–10 source pixels; 0 disables it. Python uses `median_blur=2` for
the same setting.

Quantization and replacement are also available directly from the CLI:

```bash
# Limit finished-art colours (merge threshold defaults to 3%).
img2irc photo.png --quantize-colours 8 --quantize-threshold 3

# Clean up source pixels before glyph matching instead.
img2irc photo.png --quantize-colours 16 --quantize-source

# Use an explicit palette; commas, spaces and semicolons are accepted.
img2irc photo.png --quantize-palette '#000000,#ffffff,#ff0000'

# Replace finished-art colours; repeat the option for multiple replacements.
img2irc photo.png --replace-colour '#ffffff=#ff0000' --replace-tolerance 5
img2irc photo.png --replace-from '#ffffff' --replace-to '#000000'
```

`--quantize-colours` accepts 0–256; 0 only merges nearby shades. Specifying a
threshold or palette without a count enables quantization with a default count
of 16. `--quantize-source` alone also enables 16-colour source cleanup; omit it
or use `--quantize-source=false` for finished art. Explicit palettes override
the count and merge threshold. Replacement tolerance defaults to 5% and accepts
0–100%; 0 requests an exact match. Hex colours accept an optional `#` prefix.
`--quantize`, `--quantize-colors`, `--replace`, and `--replace-color` are aliases.
Source cleanup runs after the other source filters. Finished-art replacements
run in the order supplied, followed by finished-art quantization, including
text overlays. A From/To pair runs after any compact replacement options.

Use the [TUI pipeline](#terminal-editor) or [Python](python.md) to set arbitrary
effect order and repeat other effects. See [text and OCR](#text-and-ocr) for
preserving readable lettering.

**Replace Colour** has inline From/To hex fields, a tolerance slider and an eyedropper. In the terminal, click a hex field or press `f` / `t` to edit it, then Enter to apply. Pick art samples foregrounds with a left click and backgrounds with a right click. The browser shows its sampling canvas inline. Replacement runs after glyph matching and text overlays, so the picked colour is the colour it replaces.

**Quantize Colours** has independent maximum-colour and merge-threshold sliders. Similar shades are grouped in OKLab before colours are ranked by visible area, so small compression variations share one palette slot. Start with a threshold of 3% and increase gradually; 0% preserves exact shades. Set maximum colours to 0 to merge shades without imposing a cap. Automatic palettes retain original colours; distinct low-frequency accents can still be removed if the cap is too small.

Choose **source image** to clean pixels before matching glyphs, or **finished art** to change colours while preserving glyphs and including text overlays. The colour cap applies at the chosen stage; source cleanup can still produce additional shades during resizing or matching. To cap the final palette, use a finished-art quantizer last. Enter an explicit palette as space-separated `#RRGGBB` values to map to exactly those colours (IRC/ANSI snap to their supported palettes); this overrides both sliders. **Use current art** freezes the visible palette for editing, and **Automatic palette** returns to shade grouping.

Effects expose their controls below the active pipeline. In the terminal,
Tab and Shift+Tab move through every input and action, including palette
swatches and eyedroppers. Up/Down also move between inputs while controls have
focus; Left/Right adjust numeric values, and Shift makes larger changes. Enter
edits a number or hex value, or activates the selected action. Tab applies an
edit and moves to the next input. Esc cancels an edit or picker; another Esc
returns to the effect list. Clicking elsewhere also leaves text editing.
`[` / `]` remain shortcuts for selecting numeric parameters. Oil has separate
radius and intensity controls.

Quantization shortcuts are `s` (source/art), `p` (edit palette), `i` (pick image
colours), `u` (use current art), and `a` (automatic). The image picker shows the
source after preceding source effects, before quantization. Click to add
colours repeatedly, or move with arrows and press Enter; Esc finishes. This
keeps the original colours available even after the output has been quantized.
Click a palette swatch to edit its hex colour; right click or focused Delete
removes it. Ctrl+U clears an edit and Ctrl+V pastes. Existing saved quantizers
retain their original exact-colour behaviour until edited.

**Crop** is one effect with Top, Right, Bottom and Left pixel margins. Press
`i` or activate **Select crop area** to drag the area to keep on the image.
Arrows move the selection; Shift+arrows resize it. Enter applies and Esc
cancels. Press `r` or select Reset crop to clear every side. Legacy pipelines
containing separate crop-side effects remain supported.

## terminal editor

```bash
img2irc --tui photo.png
```

CLI settings provide the initial state. the general, pipeline, glyphs, and text panes edit the output against a live preview. click pane headers to show/hide them; `Ctrl+Left` / `Ctrl+Right` navigates visible panes.

| shortcut | action |
| --- | --- |
| `Ctrl+O` / `Ctrl+V` | open / paste an image |
| `Ctrl+S` / `Ctrl+P` | save encoded text / PNG |
| `Ctrl+Q` | quit |
| `Tab` / `Shift+Tab` | cycle focus |
| arrows | select controls and change values |
| Enter / space | edit a value / toggle |
| Escape | cancel a dialog, render, or effect preview |

Pipeline effects run in order. With the effect list focused, Enter opens the
library; drag nodes to reorder, Space disables/enables, and Delete removes.
Crop, flip, rotate and scale act on the source before output sizing. Tab enters
the selected effect's controls; Esc returns to the list. Replace Colour offers
separate From/To hex fields and Pick art buttons in that same focus order.

Line Thickness adjusts in 0.1 source-pixel increments. Fractional values blend
the results of adjacent whole-pixel radii, so you can make smaller changes
between 1 and 2; whole-number values keep their original behaviour. Left/Right
changes it by 0.1, and Shift+Left/Right by 1.0. Positive values grow dark lines;
negative values grow light lines. The browser's Light lines toggle reverses
the sign. The Python pipeline accepts fractional values, for example
`renderer.add_effect("line_thickness", 1.3)`.

native glyph selections and exclusions are saved to `glyphs.toml`. `--glyphs` overrides saved group selection. [layout.toml](../layout.toml) lists controls that can be reordered or hidden.

Click **Auto** beside Font Size, or focus that slider and press `A`, to compare
every integer size from 9 through 30 and select the lowest **Perceptual** score.
This quick sweep skips smoothing, Repair Edges, region guidance, warp and OCR,
then renders the winning size with your existing settings. It also writes
trials to `--optimize-log` when supplied.

The file picker opens SVG images and shows SVG thumbnails. With the file list
focused, typing a letter jumps to the first entry beginning with that letter,
ignoring case; use arrows, Backspace and Enter to navigate. Text and PNG save
dialogs remember their last successful filename and directory separately for
the current editor session.

Auto-Optimize evaluates every enabled range at the requested step and retains
the lowest score, including the starting settings as a baseline. Each trial
uses the same rendering path as the live preview. Leave **Shortest Longest
Line** unchecked to minimize the quality score; checking it minimizes encoded
line length instead.

To record the search for plotting or analysis, start the editor with:

```bash
img2irc --tui photo.png --optimize-log trials.jsonl
```

The file uses JSON Lines and appends each search with a distinct `run_id`.
`start` records the objective, full settings and requested ranges; `baseline`
and `trial` records include the font size, width, trim, scale, score, perceptual
components, grid dimensions, render time and best score so far. Failed or
cancelled trials have a status and no objective score. `finish` records the
winner and completion counts. `trial` is the candidate's stable ID and
`completed` is its completion order, which can differ when rendering in
parallel or using Randomize.

For example, plot font size against score for the latest search (requires
Matplotlib):

```python
import json
import matplotlib.pyplot as plt

with open("trials.jsonl") as f:
    records = [json.loads(line) for line in f]
run = next(r["run_id"] for r in reversed(records) if r["event"] == "start")
trials = [r for r in records if r["run_id"] == run
          and r["event"] == "trial" and r["status"] == "ok"]
plt.scatter([r["font_size"] for r in trials], [r["score"] for r in trials])
plt.xlabel("Font size")
plt.ylabel("Objective score (lower is better)")
plt.show()
```

while editing overlay text, Enter inserts a newline and `Ctrl+Enter` finishes. see [text and OCR](#text-and-ocr) for FIGlet and recognition.

## text and OCR

manual overlays work in both editors and through the APIs. positions and boxes are measured in output cells; foreground/background colours, wrapping, and bold/italic/underline styles are editable. A selected background colour fills the entire overlay bounding box, including padding, blank rows and spaces in FIGlet banners. Without a background, FIGlet whitespace leaves the image beneath visible.

### OCR

```bash
img2irc poster.png --ocr --width 100
img2irc poster.png --ocr --ocr-auto-width --max-width 200
```

OCR removes source lettering and overlays recognized text. nearby words and aligned lines are grouped for editing; original word boxes control removal. native PP-OCRv6 models download on first use and are cached. select `tiny`, `small`, or `medium` with `--ocr-model-tier`; `--ocr-model-cache` sets the cache. the browser offers the same three model tiers through its OCR model selector, runs PaddleOCR.js in a worker, and caches model downloads in IndexedDB.

Multiline groups require nearby lines with consistent spacing. Paragraph breaks
and widely separated labels stay separate. Merged plain text includes blank
rows; merged FIGlet text uses one font and places each rendered line using its
source bounds. The first through last visible rows approximate the source
height to the nearest output row, excluding blank outer font margins.

The native and Python editors expose ASCII filtering, box confidence,
binarization and box expansion under **Advanced OCR**, alongside their model,
performance and FIGlet options. The browser keeps the same detector and styling
controls under **Advanced OCR settings**; its model and worker count are fixed.

`--ocr-megapixels` controls detection resolution (native default 1.5 MP, range 0.1–16), independently of output width. automatic output width is on in the editors and off in the CLI. `--ocr-auto-width false` disables it. native auto width grows from the source-size grid; the browser fits its text coverage target. use `--max-width` to bound native output.

lock OCR to preserve detections, edits, and fitted grid while adjusting the image. browser overlay edits also preserve the detected set; regenerate OCR discards those edits and detects again.

### FIGlet

larger text can use FIGlet; `--ocr-figlet false` keeps plain text. `--ocr-figlet-min-height` requires 2 output rows by default, and `--ocr-figlet-min-height-ratio` requires 1.25 times the median line height. `--ocr-figlet-fill` chooses the largest art fitting the allowed space. neighbour and canvas limits still apply.

shipped FIGlet fonts are bundled. `--figlet-dir` supplies additional fonts. font preferences are grouped by declared height in `figlet.toml`, or in `config.toml` as:

```toml
[ocr_figlet_fonts]
1 = ["plain"]
2 = ["phm-minecraft", "phm-lcdmatrix"]
3 = ["phm-largetype", "phm-shinonome"]
```

`--ocr-figlet-fonts '2=phm-minecraft,phm-lcdmatrix'` overrides a list from the CLI. the native text pane has a searchable FIGlet picker.

for recognition problems, inspect `--ocr-debug-boxes` and adjust confidence or detection resolution. the default `--ocr-min-ascii-ratio 0.7` filters text with few printable ASCII characters. offline models require `--ocr-det-model`, `--ocr-rec-model`, and `--ocr-dict` together.
