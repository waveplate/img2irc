# Gemini Context

This file contains useful context for Gemini when working on this project.

## Recent Changes
- **Added `--font` CLI argument**: Allows specifying the font to use directly from the command line, overriding the default or environment variable.
- **Implemented Full Block Calibration**:
  - The glyph grid metrics are now calibrated using the "Full Block" (U+2588) character.
  - The `cell_height` is set to the height of the Full Block bitmap.
  - The `global_ascender` is set to the top bearing of the Full Block.
  - This ensures that block characters fill the cell vertically and other characters are aligned relative to this "block grid", fixing issues with blank lines at the top and cut-off bottoms.
- **Fixed Font Metrics Calculation**:
  - Added robust handling for positive descender values (using `.abs()`).
  - Added debug logging for resolved font metrics.
- **Added `--clear-cache` flag**: CLI argument to clear the font cache.
- **Added `--show-block-bitmaps` flag**: CLI argument to visualize character bitmaps.
- **Implemented `print_block_bitmaps`**: Helper to print bitmaps with borders.

## Key Concepts
- **Full Block Alignment**: Since this tool generates IRC/ANSI art, the "Full Block" is treated as the canonical "pixel". The grid is adjusted to match its dimensions exactly.

## Procedures
- **Commit Changes**: Always commit changes to the git repository after reaching a stable state or completing a request feature to prevent data loss.