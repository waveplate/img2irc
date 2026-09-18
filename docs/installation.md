# installation

release binaries are available on the [releases page](https://github.com/waveplate/img2irc/releases). Arch Linux packages are available through the AUR: `yay -S img2irc-bin` for the binary package, or `yay -S img2irc` to compile the packaged version.

## source build

```bash
git clone https://github.com/waveplate/img2irc.git
cd img2irc
cargo build --release
./target/release/img2irc photo.png --width 80
```

to install on your Cargo `PATH`, run `cargo install --path . --locked`. Rust/Cargo and a C/C++ build toolchain are required. the repository selects `lld` on x86_64 GNU/Linux; OpenSSL development files and `pkg-config` may also be needed by native dependencies.

OCR is included by default. normal native builds download a prebuilt ONNX Runtime; use `cargo build --release --no-default-features` to omit OCR. rendering fonts are embedded in the binary.

## static musl build

```bash
rustup target add x86_64-unknown-linux-musl
./scripts/build-static-musl.sh
```

this builds ONNX Runtime from source and links it into the OCR-enabled binary. the host needs Git, Python 3, CMake, Ninja, and the `x86_64-unknown-linux-musl-{gcc,g++}` cross-toolchain. output is `target/x86_64-unknown-linux-musl/release/img2irc`.

the first build is long; later builds reuse `target/onnxruntime-musl`. `IMG2IRC_BUILD_JOBS` limits parallelism. `IMG2IRC_ORT_SOURCE_DIR` and `IMG2IRC_ORT_BUILD_DIR` override cache locations.

see [Python](python.md) for wheels and [web](web.md) for browser builds.
