#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ "${1:-}" == "--help" || "${1:-}" == "-h" || $# -gt 1 ]]; then
    printf 'Usage: %s [output-directory]\n\nBuild and package the static x86-64 Linux binary with OCR.\n' "$0"
    if (( $# > 1 )); then exit 2; fi
    exit 0
fi

output_dir="${1:-${root_dir}/target/packages}"
mkdir -p -- "${output_dir}"
output_dir="$(cd -- "${output_dir}" && pwd)"
cd "${root_dir}"

metadata="$(cargo metadata --format-version 1 --no-deps --locked)"
version="$(python3 -c 'import json, sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "img2irc-rs"))' <<< "${metadata}")"
target_dir="$(python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])' <<< "${metadata}")"
target=x86_64-unknown-linux-musl
name="img2irc-${version}-${target}"
archive="${name}.tar.gz"
export CARGO_TARGET_DIR="${target_dir}"
"${root_dir}/scripts/build-static-musl.sh"

stage="$(mktemp -d "${output_dir}/.${name}.XXXXXX")"
trap 'rm -rf -- "${stage}"' EXIT
package="${stage}/${name}"
mkdir -p "${package}/docs" "${package}/examples" "${package}/licenses"
install -m 755 "${target_dir}/${target}/release/img2irc" "${package}/img2irc"
install -m 644 LICENSE README.md glyphs.toml layout.toml figlet.toml "${package}/"
install -m 644 docs/*.md "${package}/docs/"
install -m 644 examples/README.md examples/gpu_blur_bench.cu "${package}/examples/"
install -m 644 static/fonts/LICENSE-* static/ocr/LICENSE-PaddleOCR.txt "${package}/licenses/"

ort_source="$("${root_dir}/scripts/build-static-musl.sh" --ort-source-dir)"
install -m 644 "${ort_source}/LICENSE" "${package}/licenses/ONNX-Runtime-LICENSE"
install -m 644 "${ort_source}/ThirdPartyNotices.txt" "${package}/licenses/ONNX-Runtime-ThirdPartyNotices.txt"

(
    cd "${stage}"
    tar -czf "${archive}" "${name}"
    sha256sum "${archive}" > "${archive}.sha256"
)
mv -- "${stage}/${archive}" "${stage}/${archive}.sha256" "${output_dir}/"
printf '%s\n' "${output_dir}/${archive}" "${output_dir}/${archive}.sha256"
