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

wasm_bindgen_version=$(wasm-bindgen --version)
if [[ "$wasm_bindgen_version" != "wasm-bindgen 0.2.127" ]]; then
    echo "wasm-bindgen-cli 0.2.127 is required (found: $wasm_bindgen_version); install it with:" >&2
    echo "  cargo install wasm-bindgen-cli --version 0.2.127 --force" >&2
    exit 1
fi

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

mkdir -p "$staging_dir/js" "$staging_dir/assets/fonts"
install -m 0644 web/index.html "$staging_dir/index.html"
install -m 0644 web/main.js "$staging_dir/main.js"
install -m 0644 web/styles.css "$staging_dir/styles.css"
for source in web/js/*.js; do
    install -m 0644 "$source" "$staging_dir/js/$(basename "$source")"
done
install -m 0644 static/CascadiaCode-Regular.ttf \
    "$staging_dir/assets/fonts/CascadiaCode-Regular.ttf"
for source in static/fonts/*; do
    install -m 0644 "$source" "$staging_dir/assets/fonts/$(basename "$source")"
done

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
