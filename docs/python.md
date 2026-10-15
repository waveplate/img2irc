# Python

```python
from img2irc import Simple

output = Simple(width=80, mode="ansi24", smooth=True).generate("photo.png")
output.write("art.ans")
output.save_preview("preview.png")
```

Both `Simple` and `Advanced` accept the CLI render settings using underscores
in place of hyphens: `--ocr-min-confidence` becomes `ocr_min_confidence`,
`--smooth-orders` becomes `smooth_orders`, and so on. `generate()` and per-call
overrides use the same names. CLI aliases such as `boxblur` also work; existing
Python names such as `box_blur` remain available.

`median_blur=2` applies a median filter before glyph matching. It accepts an
integer radius of 0–10 source pixels, with 0 disabled. The same setting is
`--median-blur 2` in the CLI. For an explicit ordered pipeline, use
`.add_effect("median_blur", 2)`; that pipeline takes precedence over scalar
effect settings.

Smoothing uses `smooth`, `smooth_candidates`, `smooth_orders`,
`smooth_shapes`, and `smooth_neighbors` throughout the APIs and serialized
options. It always uses glyph-feature smoothing; the former `smooth_features`
mode switch has been removed.

`region_guidance=True` enables experimental connected-region colour and glyph
fitting, with a local source-error budget. It works independently of smoothing;
when combined, it also constrains smoothing to preserve confident boundaries.
Colours and glyph shapes are searched together using dominant observed region
colours; it does not average colours across a glyph boundary.
Its editor controls are hidden; the API option defaults to false.

`glyph_search=True` enables neighbouring glyph-pair search, keeping colours
fixed and accepting only lower full-image Perceptual scores. It works without
smoothing or region guidance. `glyph_search_candidates` defaults to 6 alternatives,
`glyph_search_passes` to 2, and `glyph_search_finalists` to 3. Set finalists to 0
for full-image scoring of every shortlisted pair combination. Search first maps
suspect gaps, tips and mismatched edges, then tries adjacent cells including
diagonals. `discontinuity_threshold` defaults to 0.5; 0 searches all non-flat cells
and disables skipping unchanged contexts. Candidate pools combine source and
boundary fits, and source-only scoring data is cached across trials. Larger pools
can be slow. Timings include `glyph_search_ms`; the Python TUI exposes these controls
in General. The feature defaults to false and always reports Perceptual scoring.

inputs can be paths, HTTP(S) URLs, bytes, or binary file objects. Cascadia Code is embedded; `font=` accepts another font file or bytes.

Glyph sets are embedded in the wheel. Use `glyphs="smooth"` for one set or
`glyphs=["smooth", "ascii"]` for several; `blocks` remains a deprecated alias.
Both `Simple` and `Advanced` accept these forms, as do generation overrides.

```python
import img2irc

sets = img2irc.glyph_sets()  # name -> complete character string
print(sorted(sets))
output = img2irc.Simple(width=80, glyphs="smooth").generate("photo.png")
```

`glyph_groups()` is also supported. Catalog membership is independent of font
coverage; `font_info(glyphs="smooth")` reports the characters supported by the
selected font. Local `glyphs.toml` definitions override bundled sets with the
same name; sets missing from an older local catalog remain available from the
wheel. Serialized render options now use `glyphs`; older `blocks` JSON is accepted.

`Advanced` exposes all native render settings, an ordered pipeline, and overlays:

```python
from img2irc import Advanced

output = (
    Advanced(width=100, render="ansi24", smooth=True, include_cells=True)
    .add_effect("contrast", 18)
    .add_replace_colour((255, 0, 0), (20, 20, 20), tolerance=8)
    .add_effect("sharpen")
    .add_effect("quantize_colours", 16)
    .add_overlay("hello", 2, 1, foreground=(255, 255, 255), bold=True)
    .generate("photo.png")
)
```

`available_options()` lists settings; `resolved_options`, `validate()`, and `to_json()` inspect, check, and serialize them. output contains encoded text, RGBA/PNG previews, scores/timings, OCR text, and optional styled cells. `process()`, `detect_text()`, `nearest_colour()`, and `score_contours()` expose individual subsystems. see the [gradient example](../python/examples/gradient.py).

OCR controls work in both constructors and generation overrides:

```python
from pathlib import Path
from img2irc import Simple, detect_text

renderer = Simple(
    width=100,
    ocr=True,
    ocr_auto_width=True,
    max_width=160,
    ocr_model_tier="small",
    ocr_model_cache=Path("models"),
    ocr_threads=2,
    ocr_megapixels=2.5,
    ocr_min_confidence=0.8,
    ocr_figlet=False,
)
output = renderer.generate("photo.png", ocr_box_threshold=0.3)
detections = detect_text("photo.png", ocr_threads=2, ocr_megapixels=2.5)
```

All `ocr_*` settings are exposed, including detector thresholds, model paths,
orientation, FIGlet selection and sizing, and debug boxes. Model paths accept
strings or `Path` objects. `ocr_figlet_fonts` accepts CLI declarations such as
`["2=phm-c64,phm-cga", "3=future"]`, a height-to-fonts mapping such as
`{2: ["phm-c64", "phm-cga"], 3: ["future"]}`, or serialized
`[{"height": 2, "fonts": ["phm-c64"]}]` entries.

As in the CLI, `ocr_auto_width` defaults to false. Enabling it lets accepted
text determine the output width, subject to `min_width` and `max_width`.
`ocr_megapixels` controls detection resolution independently of output width
and accepts 0.1–16; its effective default is 1.5. `OCR_ENABLED` reports whether
the installed extension includes OCR.

The mapping covers render settings. Python uses image arguments, `Output.write()`,
`Output.save_preview()`, and `Output.timings` for input, output, and profiling;
CLI launch/reporting flags and `start:end:step` parameter sweeps are separate
workflows. `font=` accepts font bytes or a file; use `font_names=["Cascadia Code"]`
with `system_fonts=True` for the CLI's font-family selection. Scalar `scalex`,
`scaley`, and `crop_x1`/`crop_y1`/`crop_x2`/`crop_y2` update their corresponding
`scale` or `crop` sequence.

`generate()` applies colour replacement and quantization to the finished art,
including overlays. Quantization keeps the most-used colours by visible area
and maps rarer shades to the nearest retained colour; put it last to enforce
the final limit. `process()` has no glyph stage and applies these effects to
source pixels in pipeline order. `quantize_colors` is also accepted.

To merge compression shades before reducing the palette, use the structured
quantizer. `source=True` cleans image pixels before glyph matching; the default
edits finished-art colours. A zero count only merges shades. An explicit
palette overrides the count and threshold:

```python
renderer.add_quantize_colours(colours=16, threshold=3, source=True)
renderer.add_quantize_colours(palette=[(255, 255, 255), (0, 0, 0)], source=False)
# The effect builder accepts the same settings:
renderer.add_effect("quantize_colours", {"colours": 8, "threshold": 4})
```

Threshold is a percentage of OKLab distance (0–100); increase it gradually to
merge slight shade variation. Source cleanup can produce additional colours
during resizing or glyph matching; use a finished-art quantizer to cap final
output colours. `add_quantize_colors` is also accepted. Integer
`add_effect("quantize_colours", 16)` keeps its existing exact-colour behaviour.

Smoothing defaults to 5 candidates and 5 scan patterns; enable
it with `smooth=True`. The Perceptual score is on by default. Explicit
`perceptual_score=False` selects the older contour scoring mode.

`superpixel_warp` enables the experimental feature-alignment pass on generated
art. It accepts 0–100: 0 disables it, 100 requests full alignment, and 50 applies
half the displacement. It attracts strong boundaries toward the nearest
sextant half-cell and third-cell intersections, including boundaries halfway
between grid lines. Conflicting movements are reduced locally to prevent
folding; a global reduction is reserved for cases where local corrections
are insufficient.

In the Python TUI, Space toggles numeric controls between zero and their previous
value (the minimum for controls that cannot be zero). Arrow keys still adjust
the value normally.

## wheels

```bash
./scripts/build-wheel.sh --python python3.12
python3.12 -m pip install dist/<generated-wheel>.whl
```

the script requires Docker and x86_64 Linux, and produces a manylinux 2.28 wheel. OCR, ONNX Runtime, and OpenSSL are included; the first build compiles native dependencies and later builds reuse caches under `target/`. `--no-ocr` omits OCR, `--out DIR` changes the wheel directory, and `IMG2IRC_BUILD_JOBS` limits parallelism. install with the Python version selected for the build. Python 3.10 or later is required.

a small testing front end is also available as `img2irc-tui photo.png`.
Its OCR tab keeps detector thresholds, model/performance options and detailed
FIGlet controls under **Advanced OCR**. Collapsing it preserves their values.
