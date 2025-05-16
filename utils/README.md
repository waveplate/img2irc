This tool converts an image containing Unicode glyphs into Rust-compatible bitmap arrays for use in the `img2irc` project.

## Workflow Overview

1. **Create input files:**

   * An **image file** with glyphs rendered horizontally, without gaps. Essentially a cropped screenshot of the rendered characters with no spaces in between.
   * A **text file** listing the same glyphs separated by spaces.

2. **Generate JSON bitmap data (`mkbitmap.py`).**

3. **Convert JSON to Rust source (`mkrust.py`).**

4. Replace `src/chars.rs` in `img2irc` with the generated Rust file and recompile.

---

## `mkbitmap.py`

Slices an image into bitmaps and outputs JSON.

**Arguments:**

| Argument      | Description                                      |
| ------------- | ------------------------------------------------ |
| `chars_file`  | Text file containing glyphs separated by spaces. |
| `image_file`  | Horizontal image of glyphs (no gaps).            |
| `--icw`       | Input character width (pixels).                  |
| `--ich`       | Input character height (pixels).                 |
| `--threshold` | Black/white pixel threshold (0–255).             |
| `--obw`       | Output bitmap width.                             |
| `--obh`       | Output bitmap height.                            |
| `--output`    | Output JSON filename.                            |

**Example:**

```bash
./mkbitmap.py chars.txt chars.png --icw 20 --ich 40 --threshold 128 --obw 10 --obh 20 --output chars.json
```

---

## `mkrust.py`

Converts JSON bitmap data to Rust source code.

**Arguments:**

| Argument     | Description                   |
| ------------ | ----------------------------- |
| `input_json` | JSON file from `mkbitmap.py`. |
| `output_rs`  | Output Rust source file.      |

**Example:**

```bash
./mkrust.py chars.json chars.rs
```

Replace the resulting `chars.rs` in your `img2irc` project and recompile.

You can use the included `chars.txt` and `chars.png` as an example.