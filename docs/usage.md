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
