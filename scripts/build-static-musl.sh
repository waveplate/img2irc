#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ORT_VERSION="1.20.0"
ORT_SOURCE_DIR="${IMG2IRC_ORT_SOURCE_DIR:-${ROOT_DIR}/target/onnxruntime-${ORT_VERSION}}"
ORT_BUILD_ROOT="${IMG2IRC_ORT_BUILD_DIR:-${ROOT_DIR}/target/onnxruntime-musl}"
ORT_LIB_DIR="${ORT_BUILD_ROOT}/Release"
ORT_BUILD_STAMP="${ORT_BUILD_ROOT}/.img2irc-complete-${ORT_VERSION}"
BUILD_JOBS="${IMG2IRC_BUILD_JOBS:-$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 1)}"

for tool in cargo git python3 cmake ninja x86_64-unknown-linux-musl-gcc x86_64-unknown-linux-musl-g++; do
    if ! command -v "${tool}" >/dev/null 2>&1; then
        echo "error: required build tool '${tool}' was not found" >&2
        exit 1
    fi
done

MUSL_RUNTIME_DIR="$(dirname "$(x86_64-unknown-linux-musl-g++ -print-file-name=libstdc++.so)")"

export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=x86_64-unknown-linux-musl-gcc
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS="-C target-feature=+crt-static"
export AR_x86_64_unknown_linux_musl=x86_64-unknown-linux-musl-ar
export CC_x86_64_unknown_linux_musl=x86_64-unknown-linux-musl-gcc
export CXX_x86_64_unknown_linux_musl=x86_64-unknown-linux-musl-g++

if [[ ! -d "${ORT_SOURCE_DIR}/.git" ]]; then
    git clone --branch "v${ORT_VERSION}" --depth 1 --recursive \
        https://github.com/microsoft/onnxruntime.git "${ORT_SOURCE_DIR}"
fi

if [[ ! -f "${ORT_BUILD_STAMP}" ]]; then
    "${ORT_SOURCE_DIR}/build.sh" \
        --update \
        --config Release \
        --build_dir "${ORT_BUILD_ROOT}" \
        --parallel "${BUILD_JOBS}" \
        --skip_tests \
        --compile_no_warning_as_error \
        --cmake_generator Ninja \
        --cmake_extra_defines \
            CMAKE_C_COMPILER=x86_64-unknown-linux-musl-gcc \
            CMAKE_CXX_COMPILER=x86_64-unknown-linux-musl-g++ \
            CMAKE_AR=x86_64-unknown-linux-musl-ar \
            CMAKE_RANLIB=x86_64-unknown-linux-musl-ranlib \
            CMAKE_STRIP=x86_64-unknown-linux-musl-strip \
            CMAKE_BUILD_RPATH="${MUSL_RUNTIME_DIR}" \
            CMAKE_CXX_FLAGS="-include cstdint -I${ROOT_DIR}/scripts/musl-compat" \
            CMAKE_TRY_COMPILE_TARGET_TYPE=STATIC_LIBRARY \
            FETCHCONTENT_TRY_FIND_PACKAGE_MODE=NEVER \
            protobuf_WITH_ZLIB=OFF
    cmake --build "${ORT_LIB_DIR}" \
        --parallel "${BUILD_JOBS}" \
        --target \
            onnxruntime_common \
            onnxruntime_flatbuffers \
            onnxruntime_framework \
            onnxruntime_graph \
            onnxruntime_lora \
            onnxruntime_mlas \
            onnxruntime_optimizer \
            onnxruntime_providers \
            onnxruntime_session \
            onnxruntime_util
    touch "${ORT_BUILD_STAMP}"
fi

ORT_LIB_LOCATION="${ORT_LIB_DIR}" \
ORT_CXX_STDLIB=stdc++ \
ORT_PREFER_DYNAMIC_LINK=0 \
PYO3_BUILD_EXTENSION_MODULE=1 \
cargo build \
    --manifest-path "${ROOT_DIR}/Cargo.toml" \
    --release \
    --locked \
    --target x86_64-unknown-linux-musl \
    --bin img2irc \
    --all-features

BINARY="${ROOT_DIR}/target/x86_64-unknown-linux-musl/release/img2irc"
file "${BINARY}"
if readelf -l "${BINARY}" | grep -q INTERP || readelf -d "${BINARY}" | grep -q NEEDED; then
    echo "error: the binary is not fully static" >&2
    exit 1
fi
if command -v ldd >/dev/null 2>&1; then
    ldd "${BINARY}" 2>&1 || true
fi
