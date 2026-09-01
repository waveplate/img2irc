# img2irc for Python

The package exposes two APIs backed directly by img2irc's Rust renderer:

```python
from img2irc import Simple

output = Simple(width=80, mode="ansi24").generate("photo.png")
print(output.content)
output.write("art.ans")  # honours the selected UTF-8/UTF-16/CESU-8 encoding
output.save_preview("preview.png")
```

`Advanced` accepts every serializable renderer option used by the native TUI,
plus ordered pipeline effects and text overlays:

```python
from img2irc import Advanced

renderer = (
    Advanced(width=100, render="ansi24", score_fix=True)
    .add_effect("contrast", 18)
    .add_effect("sharpen")
    .add_overlay("hello", 2, 1, foreground=(255, 255, 255), bold=True)
)
output = renderer.generate("photo.png")
```

Images may be paths, HTTP(S) URLs, bytes, or binary file objects. The package
uses an embedded Cascadia Code font by default; pass `font=` for another
monospace OpenType/TrueType font, or set `system_fonts=True` on `Advanced` and
use the renderer's `font` option.

OCR is included in normal package builds. Set `ocr=True` (or configure the
detailed `ocr_*` options on `Advanced`) to use it. OCR models are resolved and
cached by the same model hub as the CLI.

Build and install an editable development package from the repository root:

```bash
maturin develop --features python
```
