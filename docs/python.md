# Python

```python
from img2irc import Simple

output = Simple(width=80, mode="ansi24", smooth=True).generate("photo.png")
output.write("art.ans")
output.save_preview("preview.png")
```

inputs can be paths, HTTP(S) URLs, bytes, or binary file objects. Cascadia Code is embedded; `font=` accepts another font file or bytes.

`Advanced` exposes all native render settings, an ordered pipeline, and overlays:

```python
from img2irc import Advanced

output = (
    Advanced(width=100, render="ansi24", score_fix=True, include_cells=True)
    .add_effect("contrast", 18)
    .add_replace_colour((255, 0, 0), (20, 20, 20), tolerance=8)
    .add_effect("sharpen")
    .add_overlay("hello", 2, 1, foreground=(255, 255, 255), bold=True)
    .generate("photo.png")
)
```

`available_options()` lists settings; `resolved_options`, `validate()`, and `to_json()` inspect, check, and serialize them. output contains encoded text, RGBA/PNG previews, scores/timings, OCR text, and optional styled cells. `process()`, `detect_text()`, `nearest_colour()`, and `score_contours()` expose individual subsystems. see the [gradient example](../python/examples/gradient.py).

## wheels

```bash
./scripts/build-wheel.sh --python python3.12
python3.12 -m pip install dist/<generated-wheel>.whl
```

the script requires Docker and x86_64 Linux, and produces a manylinux 2.28 wheel. OCR, ONNX Runtime, and OpenSSL are included; the first build compiles native dependencies and later builds reuse caches under `target/`. `--no-ocr` omits OCR, `--out DIR` changes the wheel directory, and `IMG2IRC_BUILD_JOBS` limits parallelism. install with the Python version selected for the build. Python 3.10 or later is required.

a small testing front end is also available as `img2irc-tui photo.png`.
