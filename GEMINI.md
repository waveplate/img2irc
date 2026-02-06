# Gemini Context

## Procedures
- **Commit Changes**: Always commit changes to the git repository after reaching a stable state or completing a request feature to prevent data loss.
- **Build with Release**: When building the project (e.g., using `cargo build`), always include the `--release` flag and `-j12` (e.g., `cargo build --release -j12`) to ensure optimal performance and utilize multiple CPU cores for faster builds.
