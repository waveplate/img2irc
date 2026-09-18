#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Build a manylinux_2_28 x86_64 Python wheel with statically linked ONNX Runtime.

Usage: scripts/build-wheel.sh [OPTIONS] [-- MATURIN_OPTIONS...]

Options:
  --no-ocr       Build the smaller wheel without OCR support.
  --out DIR      Write wheels to DIR (default: dist or $WHEEL_DIR).
  --python PATH  Python interpreter to target (default: $PYTHON or python3).
  -h, --help     Show this help.

Requires Docker. The normal build includes OCR, builds ONNX Runtime from source
in the official manylinux image, and caches the build under target/.
Set IMG2IRC_BUILD_JOBS to limit parallelism or IMG2IRC_MANYLINUX_IMAGE to select
a compatible manylinux_2_28 x86_64 build image.
EOF
}

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
project_dir=$(cd -- "$script_dir/.." && pwd)
wheel_dir=${WHEEL_DIR:-"$project_dir/dist"}
python_bin=${PYTHON:-python3}
with_ocr=1
maturin_options=()

while (($#)); do
    case "$1" in
        --no-ocr)
            with_ocr=0
            shift
            ;;
        --out)
            if (($# < 2)); then
                echo "error: --out requires a directory" >&2
                exit 2
            fi
            wheel_dir=$2
            shift 2
            ;;
        --python)
            if (($# < 2)); then
                echo "error: --python requires an interpreter" >&2
                exit 2
            fi
            python_bin=$2
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        --)
            shift
            maturin_options+=("$@")
            break
            ;;
        *)
            maturin_options+=("$1")
            shift
            ;;
    esac
done

if ! command -v "$python_bin" >/dev/null 2>&1; then
    echo "error: Python interpreter not found: $python_bin" >&2
    exit 1
fi
python_bin=$(command -v "$python_bin")
if [[ $python_bin != /* ]]; then
    python_bin="$(cd -- "$(dirname -- "$python_bin")" && pwd)/$(basename -- "$python_bin")"
fi

if [[ $wheel_dir != /* ]]; then
    wheel_dir="$(pwd)/$wheel_dir"
fi

if [[ $(uname -s) != Linux || $(uname -m) != x86_64 ]]; then
    echo "error: this build currently targets x86_64 Linux" >&2
    exit 1
fi
if ! command -v docker >/dev/null 2>&1; then
    echo "error: Docker is required for the manylinux build" >&2
    exit 1
fi

python_tag=$("$python_bin" -c '
import sys, sysconfig
if sys.implementation.name != "cpython":
    raise SystemExit("error: select a CPython interpreter with --python")
version = f"cp{sys.version_info.major}{sys.version_info.minor}"
print(version + "-" + version + ("t" if sysconfig.get_config_var("Py_GIL_DISABLED") else ""))
')

mkdir -p -- "$wheel_dir" "$project_dir/target/wheel-build"
wheel_dir=$(cd -- "$wheel_dir" && pwd)
base_image=${IMG2IRC_MANYLINUX_IMAGE:-quay.io/pypa/manylinux_2_28_x86_64}
build_image=img2irc-manylinux_2_28-builder
build_jobs=${IMG2IRC_BUILD_JOBS:-6}

docker build \
    --build-arg "MANYLINUX_IMAGE=$base_image" \
    --tag "$build_image" \
    --file "$script_dir/wheel-builder.Dockerfile" "$script_dir"

docker run --rm \
    --user "$(id -u):$(id -g)" \
    --mount "type=bind,source=$project_dir,target=/io" \
    --mount "type=bind,source=$wheel_dir,target=/wheelhouse" \
    --workdir /io \
    --env "IMG2IRC_BUILD_JOBS=$build_jobs" \
    --env IMG2IRC_MANYLINUX_IN_CONTAINER=1 \
    "$build_image" \
    bash /io/scripts/build-manylinux-wheel.sh "$python_tag" "$with_ocr" \
    "${maturin_options[@]}"

printf 'Wheel output: %s\n' "$wheel_dir"
