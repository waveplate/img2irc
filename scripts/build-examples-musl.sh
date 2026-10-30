#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_root"

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
    echo "Usage: $0 [EXAMPLE ...]"
    echo "Build all examples as static musl executables, or only the named examples."
    exit 0
fi

for tool in cargo x86_64-unknown-linux-musl-gcc x86_64-unknown-linux-musl-g++ x86_64-unknown-linux-musl-ar; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "error: required build tool '$tool' was not found" >&2
        exit 1
    fi
done

targets=(--examples)
if (( $# > 0 )); then
    targets=()
    for example in "$@"; do
        targets+=(--example "$example")
    done
fi

# The repository's .cargo/config.toml selects the musl C/C++ toolchain.
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS="-C target-feature=+crt-static"
cargo build \
    --release \
    --locked \
    --no-default-features \
    --target x86_64-unknown-linux-musl \
    "${targets[@]}"

echo "Examples built in ${CARGO_TARGET_DIR:-target}/x86_64-unknown-linux-musl/release/examples/"
