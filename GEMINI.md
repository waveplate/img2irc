# Gemini Context

## Procedures
- **Commit Changes**: Always commit changes to the git repository after reaching a stable state or completing a request feature to prevent data loss.
- **Build with Release**: When building the project (e.g., using `cargo build`), always include the `--release` flag, `-j12`, and `--features gpu` (e.g., `cargo build --release -j12 --features gpu`) to ensure optimal performance, enable GPU acceleration features, and utilize multiple CPU cores for faster builds.
