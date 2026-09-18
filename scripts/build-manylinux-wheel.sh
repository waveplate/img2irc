#!/usr/bin/env bash
set -euo pipefail

# Container entry point used by build-wheel.sh; do not run on the host.
if [[ ${IMG2IRC_MANYLINUX_IN_CONTAINER:-} != 1 ]]; then
    echo "error: run scripts/build-wheel.sh to use the manylinux build image" >&2
    exit 1
fi

python_tag=${1:?Python ABI tag is required}
with_ocr=${2:?OCR selection is required}
shift 2
python_bin="/opt/python/$python_tag/bin/python"
if [[ ! -x $python_bin ]]; then
    echo "error: $python_tag is not provided by this manylinux image" >&2
    exit 1
fi

# These options would bypass the build's static-link/portability checks.
for option in "$@"; do
    case "$option" in
        --all-features)
            if ((!with_ocr)); then
                echo "error: --all-features cannot be combined with --no-ocr" >&2
                exit 1
            fi
            ;;
        --target*|--compatibility*|--manylinux*|--auditwheel*|--zig)
            echo "error: $option overrides the enforced manylinux build settings" >&2
            exit 1
            ;;
    esac
done

build_jobs=${IMG2IRC_BUILD_JOBS:-6}
if [[ ! $build_jobs =~ ^[1-9][0-9]*$ ]]; then
    echo "error: IMG2IRC_BUILD_JOBS must be a positive integer" >&2
    exit 1
fi

export CARGO_HOME=/io/target/wheel-build/cargo
export RUSTUP_HOME=/io/target/wheel-build/rustup
export CARGO_TARGET_DIR=/io/target/wheel-build/rust-target
export PIP_CACHE_DIR=/io/target/wheel-build/pip-cache
build_venv="/io/target/wheel-build/venv-$python_tag"
if [[ ! -x $build_venv/bin/python ]]; then
    "$python_bin" -m venv "$build_venv"
fi
export PATH="$CARGO_HOME/bin:$build_venv/bin:$PATH"
if ! "$build_venv/bin/python" -m maturin --version >/dev/null 2>&1; then
    "$build_venv/bin/python" -m pip install 'maturin>=1.10,<2'
fi
if ! command -v ninja >/dev/null 2>&1; then
    "$build_venv/bin/python" -m pip install ninja
fi
# ONNX Runtime 1.20's pinned dependencies still need pre-CMake-4 policies.
if [[ ! -x $build_venv/bin/cmake ]]; then
    "$build_venv/bin/python" -m pip install 'cmake>=3.28,<4'
fi
if ! command -v cargo >/dev/null 2>&1; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs |
        sh -s -- -y --profile minimal --no-modify-path
fi

# Rust needs an explicit native search path to embed the static C++ archive;
# GCC's implicit search directories alone only help at the final link step.
stdcxx_archive=$(g++ -print-file-name=libstdc++.a)
if [[ ! -f $stdcxx_archive ]]; then
    echo "error: the manylinux compiler has no static C++ runtime" >&2
    exit 1
fi
# The host config selects lld, which isn't required in the manylinux image.
RUSTFLAGS="-C link-arg=-fuse-ld=bfd -L native=$(dirname -- "$stdcxx_archive")"
export RUSTFLAGS
export OPENSSL_STATIC=1
# Match native C++ dependencies which use the cc crate, including without OCR.
export CXXSTDLIB=static=stdc++
features=python,vendored-openssl

if ((with_ocr)); then
    ort_version=1.20.0
    ort_source="/io/target/onnxruntime-$ort_version"
    ort_build=/io/target/onnxruntime-manylinux_2_28
    ort_lib="$ort_build/Release"
    ort_stamp="$ort_build/.img2irc-complete-$ort_version"
    if [[ ! -d $ort_source/.git ]]; then
        git clone --branch "v$ort_version" --depth 1 --recursive \
            https://github.com/microsoft/onnxruntime.git "$ort_source"
    fi
    if [[ ! -f $ort_stamp ]]; then
        # Fetch the exact pinned Eigen commit from its Git mirror. GitLab's
        # generated zip no longer matches ONNX 1.20's recorded archive hash.
        eigen_commit=e7248b26a1ed53fa030c5c459f7ea095dfd276ac
        eigen_source="/io/target/eigen-$eigen_commit"
        if [[ ! -d $eigen_source/.git ]]; then
            git init "$eigen_source"
            git -C "$eigen_source" fetch --depth 1 \
                https://github.com/eigen-mirror/eigen.git "$eigen_commit"
            git -C "$eigen_source" checkout --detach "$eigen_commit"
        fi
        if [[ $(git -C "$eigen_source" rev-parse HEAD) != "$eigen_commit" ]]; then
            echo "error: the cached Eigen source does not match the pinned commit" >&2
            exit 1
        fi
        "$ort_source/build.sh" \
            --update --config Release --build_dir "$ort_build" \
            --parallel "$build_jobs" --skip_tests --skip_submodule_sync \
            --use_preinstalled_eigen --eigen_path "$eigen_source" \
            --compile_no_warning_as_error --cmake_generator Ninja \
            --cmake_extra_defines \
                CMAKE_POSITION_INDEPENDENT_CODE=ON \
                CMAKE_CXX_FLAGS=-include\ cstdint \
                FETCHCONTENT_TRY_FIND_PACKAGE_MODE=NEVER \
                protobuf_WITH_ZLIB=OFF
        cmake --build "$ort_lib" --parallel "$build_jobs" --target \
            onnxruntime_common onnxruntime_flatbuffers onnxruntime_framework \
            onnxruntime_graph onnxruntime_lora onnxruntime_mlas \
            onnxruntime_optimizer onnxruntime_providers onnxruntime_session \
            onnxruntime_util
        touch "$ort_stamp"
    fi
    # Static consumers must also link these dependencies explicitly. They are
    # not all built by ONNX's individual archive targets (unlike shared builds).
    cmake --build "$ort_lib" --parallel "$build_jobs" --target \
        cpuinfo re2 nsync_cpp absl_kernel_timeout_internal \
        absl_graphcycles_internal absl_str_format_internal absl_string_view
    for component in common flatbuffers framework graph lora mlas optimizer providers session util; do
        if [[ ! -f $ort_lib/libonnxruntime_$component.a ]]; then
            echo "error: missing static ONNX Runtime archive: $component" >&2
            exit 1
        fi
    done
    export ORT_LIB_LOCATION="$ort_lib"
    export ORT_PREFER_DYNAMIC_LINK=0
    # ort-sys emits this directly as cargo:rustc-link-lib.
    export ORT_CXX_STDLIB=static=stdc++
    features+=,ocr
fi

staging_dir=$(mktemp -d /io/target/wheel-build/wheels.XXXXXX)
"$build_venv/bin/python" -m maturin build \
    --release --locked --interpreter "$python_bin" \
    --target x86_64-unknown-linux-gnu --jobs "$build_jobs" \
    --no-default-features --features "$features" \
    --compatibility manylinux_2_28 --auditwheel check \
    --out "$staging_dir" "$@"

native_library="$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/libimg2irc_rs.so"
readelf -d "$native_library"
if readelf -d "$native_library" | grep -Eq 'NEEDED.*(libonnxruntime|libstdc\+\+|libssl|libcrypto)'; then
    echo "error: the extension still depends on a non-static native library" >&2
    exit 1
fi

shopt -s nullglob
wheels=("$staging_dir"/*.whl)
if ((${#wheels[@]} != 1)); then
    echo "error: expected exactly one audited wheel in $staging_dir" >&2
    exit 1
fi
mv -- "${wheels[0]}" /wheelhouse/
rmdir -- "$staging_dir"
