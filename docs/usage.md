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

smoothing tries alternate glyphs to improve contours between cells. `--smooth-candidates` (default 12) and `--smooth-orders` (default 1, maximum 5) trade render time for a wider search. `--smooth-shapes` adds shape refinement.

`--smooth --smooth-features` enables experimental **Fast features** smoothing (also a toggle in the terminal and web editors). For example:

```bash
img2irc photo.png --width 80 --glyphs smooth --smooth --smooth-features --profile
```

turn off `--smooth-features` to compare with the original contour method. Features are extracted from the actual font bitmaps at the same 8×16 preview size used by contour scoring; no glyph names or sextant coordinates are hard-coded. The fast search minimizes source error plus a blurred-Laplacian energy using cached glyph responses, up to four sweeps per scan pattern. It keeps the initial result available and caps added source error per cell at 1/16 of that cell's foreground/background contrast energy. This is a different smoothing objective and can soften fine details; it does not promise to improve the reported contour score or preserve topology.

Candidates and scan patterns apply to both modes. In Fast features, Protect neighbors additionally prevents increasing the total curvature energy in surrounding cells. The advanced contour weights apply to the original contour method and the optional Refine shapes pass. Refine shapes still runs the expensive existing refinement after Fast features, so leave it off when comparing speed. Python `Advanced` uses `score_fix=True, score_fix_features=True`; the JavaScript API uses `smooth: true, smoothFeatures: true`.

### experimental perceptual scoring

`--perceptual-score` switches the status score and the terminal auto-optimizer to an experimental reference-aware metric. The General panel has the same toggle in both editors. Lower is better. This changes evaluation, not glyph matching or the smoothing objective; leave it off to compare with the original Contour score. The two scores have different scales.

```bash
img2irc photo.png --width 80 --glyphs smooth --smooth --smooth-features --perceptual-score --profile
```

The score combines contrast-weighted contour roughness, excess fine texture, missing source edges, invented edges, and coarse color agreement. It uses all colors, includes short contours, and allows small edge movements (up to two pixels per axis in the normalized 8×16 cell preview, with a small displacement cost). Geometry is evaluated after area sampling to 4×8 per cell; fine texture is checked before that reduction. Roughness and edge matching use source-edge strength as a fixed denominator, so extra easy edges do not dilute those penalties. Small shifts and sharper boundaries are tolerated; losing a feature still costs. It is a heuristic under evaluation, not a calibrated predictor of human preferences or a topology guarantee.

The web editor's **More stats** shows all five components; native renders log them in `img2irc.log`. Python `Advanced` accepts `perceptual_score=True` and exposes the breakdown in `output.metadata["perceptual_score"]`. The JavaScript option is `perceptualScore: true`, with a breakdown in `result.perceptualScore`. Scoring currently applies to the glyph renderer, before text overlays; the separate braille renderer does not supply a quality score.

The experimental total is `0.8 × roughness + 0.15 × texture + (1.5 × missing² + 0.5 × extra² + 0.25 × colour²) / 100`. The squared structural terms give small approximation errors room to improve aesthetics while making large losses expensive. The components and total are costs, not percentages or probabilities; they can exceed 100. Their weights have not been fitted to human preference data.

The reference is the image after the requested transforms and color edits. Consequently this measures representation quality *within the chosen crop*. An optimizer can still discard difficult content by cropping it away; evaluating an automatic deformation/crop search against a fixed starting reference is a separate constraint. This implementation does not warp the source image.

## filters and output

```bash
img2irc photo.png --width 80 --contrast 18 --sharpen
img2irc photo.png --render ansi24 --save preview.png > art.ans
```

encoded text goes to stdout; `--save` additionally writes a PNG preview. `--profile` prints stage timings to stderr. `--encoding` selects text encoding (default UTF-8).

use the [TUI pipeline](#terminal-editor) or [Python](python.md) to control effect order, repeat effects, and replace colours. see [text and OCR](#text-and-ocr) for preserving readable lettering.

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

pipeline effects run in order. Enter opens the effect library; drag nodes to reorder, space disables/enables, and Delete removes. crop, flip, rotate, and scale act on the source before output sizing. select replace colour and press `c` for its picker; Tab switches from/to, and `e` enables the eyedropper.

native glyph selections and exclusions are saved to `glyphs.toml`. `--glyphs` overrides saved group selection. [layout.toml](../layout.toml) lists controls that can be reordered or hidden.

while editing overlay text, Enter inserts a newline and `Ctrl+Enter` finishes. see [text and OCR](#text-and-ocr) for FIGlet and recognition.

## text and OCR

manual overlays work in both editors and through the APIs. positions and boxes are measured in output cells; foreground/background colours, wrapping, and bold/italic/underline styles are editable. FIGlet banners retain their source text, and their whitespace leaves the image beneath visible.

### OCR

```bash
img2irc poster.png --ocr --width 100
img2irc poster.png --ocr --ocr-auto-width --max-width 200
```

OCR removes source lettering and overlays recognized text. nearby words and aligned lines are grouped for editing; original word boxes control removal. native PP-OCRv6 models download on first use and are cached. select `tiny`, `small`, or `medium` with `--ocr-model-tier`; `--ocr-model-cache` sets the cache. the browser uses a separate PaddleOCR.js worker with hosted tiny models.

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
