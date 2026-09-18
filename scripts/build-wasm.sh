#!/usr/bin/env bash
set -euo pipefail

project_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$project_root"

output_dir="$project_root/dist"
staging_dir=$(mktemp -d "$project_root/.img2irc-web.XXXXXX")
cleanup() {
    if [[ -n "${staging_dir:-}" && -d "$staging_dir" ]]; then
        rm -rf -- "$staging_dir"
    fi
}
trap cleanup EXIT

if ! command -v wasm-bindgen >/dev/null 2>&1; then
    echo "wasm-bindgen-cli is required; install it with:" >&2
    echo "  cargo install wasm-bindgen-cli --version 0.2.127" >&2
    exit 1
fi

if ! command -v python3 >/dev/null 2>&1; then
    echo "python3 is required to generate the runtime glyph catalog" >&2
    exit 1
fi

if [[ ! -x "$project_root/node_modules/.bin/esbuild" ]]; then
    echo "web dependencies are missing; run npm install" >&2
    exit 1
fi

wasm_bindgen_version=$(wasm-bindgen --version)
if [[ "$wasm_bindgen_version" != "wasm-bindgen 0.2.127" ]]; then
    echo "wasm-bindgen-cli 0.2.127 is required (found: $wasm_bindgen_version); install it with:" >&2
    echo "  cargo install wasm-bindgen-cli --version 0.2.127 --force" >&2
    exit 1
fi

CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS='--cfg getrandom_backend="wasm_js"' \
cargo build \
    --release \
    --target wasm32-unknown-unknown \
    --no-default-features \
    --features web \
    --lib

wasm-bindgen \
    target/wasm32-unknown-unknown/release/img2irc_rs.wasm \
    --out-dir "$staging_dir/pkg" \
    --target web \
    --typescript

mkdir -p \
    "$staging_dir/js" \
    "$staging_dir/assets/fonts" \
    "$staging_dir/assets/figlet" \
    "$staging_dir/assets/ocr" \
    "$staging_dir/vendor/ort" \
    "$staging_dir/vendor/paddleocr/assets"
install -m 0644 web/index.html "$staging_dir/index.html"
install -m 0644 web/main.js "$staging_dir/main.js"
install -m 0644 web/styles.css "$staging_dir/styles.css"
for icon in web/favicon* web/apple-touch-icon.png; do
    [[ -f "$icon" ]] || continue
    install -m 0644 "$icon" "$staging_dir/$(basename "$icon")"
done
python3 - "$project_root/figlet.toml" "$staging_dir/figlet-fonts.json" "$project_root/static/figlet" "$project_root/static/figlet-fonts.json" <<'PYTHON'
import json
import pathlib
import sys
import tomllib

source = pathlib.Path(sys.argv[1])
destination = pathlib.Path(sys.argv[2])
figlet_dir = pathlib.Path(sys.argv[3])
static_json = pathlib.Path(sys.argv[4]) if len(sys.argv) > 4 else None

with source.open("rb") as handle:
    raw = tomllib.load(handle)

fonts_dict = raw.get("fonts", raw.get("ocr_figlet_fonts", raw))
font_list = []

for height_str, font_names in sorted(fonts_dict.items(), key=lambda item: int(item[0]) if str(item[0]).isdigit() else 999):
    if not str(height_str).isdigit():
        continue
    height = int(height_str)
    for name in font_names:
        if name.lower() == "plain":
            continue
        ext = None
        for candidate in ["flf", "tlf"]:
            if (figlet_dir / f"{name}.{candidate}").exists():
                ext = candidate
                break
        if ext is None:
            print(f"Warning: FIGlet font file for '{name}' not found in {figlet_dir}", file=sys.stderr)
            continue
        font_path = figlet_dir / f"{name}.{ext}"
        actual_height = height
        try:
            import io, zipfile
            raw_bytes = font_path.read_bytes()
            if raw_bytes.startswith(b"PK\x03\x04"):
                with zipfile.ZipFile(io.BytesIO(raw_bytes)) as zf:
                    header = zf.read(zf.namelist()[0]).decode("utf-8", errors="ignore").split("\n")[0]
            else:
                header = raw_bytes.decode("utf-8", errors="ignore").split("\n")[0]
            parts = header.split()
            if len(parts) >= 2 and parts[1].isdigit():
                actual_height = int(parts[1])
        except Exception:
            pass
        font_list.append({
            "name": name,
            "height": actual_height,
            "url": f"./assets/figlet/{name}.{ext}"
        })

manifest = {
    "version": 1,
    "fonts": font_list
}

with destination.open("w", encoding="utf-8") as handle:
    json.dump(manifest, handle, indent=2)
    handle.write("\n")

if static_json:
    with static_json.open("w", encoding="utf-8") as handle:
        json.dump(manifest, handle, indent=2)
        handle.write("\n")
PYTHON
for source in web/js/*.js; do
    install -m 0644 "$source" "$staging_dir/js/$(basename "$source")"
done
install -m 0644 static/CascadiaCode-Regular.ttf \
    "$staging_dir/assets/fonts/CascadiaCode-Regular.ttf"
for source in static/fonts/*; do
    install -m 0644 "$source" "$staging_dir/assets/fonts/$(basename "$source")"
done
if [[ -d "$project_root/static/figlet" ]]; then
    for source in "$project_root"/static/figlet/*; do
        [[ -f "$source" ]] || continue
        install -m 0644 "$source" "$staging_dir/assets/figlet/$(basename "$source")"
    done
fi
for source in static/ocr/*; do
    install -m 0644 "$source" "$staging_dir/assets/ocr/$(basename "$source")"
done

# PaddleOCR is an optional, lazily imported browser component. Bundle its
# public SDK entry so the deployable web root has no bare npm imports, retain
# the SDK's own inference worker, and host the exact ONNX Runtime binary used
# by the pinned package version.
"$project_root/node_modules/.bin/esbuild" \
    "$project_root/node_modules/@paddleocr/paddleocr-js/dist/index.mjs" \
    --bundle \
    --format=esm \
    --platform=browser \
    --minify \
    --define:require=undefined \
    --outfile="$staging_dir/vendor/paddleocr/index.js"

ocr_worker=$(find \
    "$project_root/node_modules/@paddleocr/paddleocr-js/dist/assets" \
    -maxdepth 1 -type f -name 'worker-entry-*.js' -print -quit)
if [[ -z "$ocr_worker" ]]; then
    echo "PaddleOCR worker asset was not found" >&2
    exit 1
fi
install -m 0644 "$ocr_worker" \
    "$staging_dir/vendor/paddleocr/assets/$(basename "$ocr_worker")"
install -m 0644 \
    "$project_root/node_modules/onnxruntime-web/dist/ort-wasm-simd-threaded.wasm" \
    "$staging_dir/vendor/ort/ort-wasm-simd-threaded.wasm"
install -m 0644 \
    "$project_root/node_modules/onnxruntime-web/dist/ort-wasm-simd-threaded.mjs" \
    "$staging_dir/vendor/ort/ort-wasm-simd-threaded.js"

# Resolve the native TOML catalog into deployment data. The browser fetches
# this file at runtime; none of these group definitions are embedded in WASM.
python3 - "$project_root/glyphs.toml" "$staging_dir/glyphs.json" <<'PYTHON'
import json
import pathlib
import sys
import tomllib

source = pathlib.Path(sys.argv[1])
destination = pathlib.Path(sys.argv[2])
with source.open("rb") as handle:
    raw = tomllib.load(handle)

entries = {}
entry_keys = {"glyphs", "ranges", "include", "exclude"}

def flatten(prefix, value):
    if isinstance(value, dict) and entry_keys.intersection(value):
        entries[prefix] = value
        return
    if isinstance(value, dict):
        for name, child in value.items():
            flatten(f"{prefix}.{name}" if prefix else name, child)

flatten("", raw)
resolved = {}

def resolve(name, stack=()):
    if name in resolved:
        return resolved[name]
    if name in stack:
        chain = " -> ".join((*stack, name))
        raise ValueError(f"cyclic glyph group include: {chain}")
    entry = entries.get(name, {})
    codepoints = set(entry.get("glyphs", []))
    for start, end in entry.get("ranges", []):
        codepoints.update(range(start, end + 1))
    for included in entry.get("include", []):
        codepoints.update(resolve(included, (*stack, name)))
    codepoints.difference_update(entry.get("exclude", []))
    resolved[name] = sorted(
        codepoint for codepoint in codepoints
        if 0 <= codepoint <= 0x10FFFF and not 0xD800 <= codepoint <= 0xDFFF
    )
    return resolved[name]

catalog = {
    "version": 1,
    "groups": [
        {"name": name, "codepoints": resolve(name)}
        for name in sorted(entries)
    ],
}
with destination.open("w", encoding="utf-8") as handle:
    json.dump(catalog, handle, ensure_ascii=False, indent=2)
    handle.write("\n")
PYTHON

# `dist` is generated output owned by this script. Replace it only after every
# build and copy step succeeds, so a failed build leaves the prior web root
# intact.
rm -rf -- "$output_dir"
mv "$staging_dir" "$output_dir"
staging_dir=""

echo "Generated deployable web root in dist/ (upload its contents to your server)"
