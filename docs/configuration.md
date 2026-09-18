# fonts and configuration

Cascadia Code, Iosevka Fixed, and GNU Unifont are embedded for native rendering, including supplementary Unicode symbols. no system font installation is required. select a bundled font or match your terminal's font:

```bash
img2irc photo.png --font "Iosevka Fixed"
img2irc photo.png --font "DejaVu Sans Mono"
img2irc photo.png --font /path/to/font.ttf
```

`--font` accepts installed monospace families or font files and can be repeated. the receiving terminal/client still needs to display the chosen glyphs. `--font-size` controls the matching raster height in pixels; `0` is automatic. `--clear-cache` clears native font caches. `--font-licenses` prints bundled font notices; provenance is listed in [font sources](font-sources.md).

## configuration

files are searched independently in `--config-dir` / `$IMG2IRC_CONFIG_DIR`, the current directory, then `$XDG_CONFIG_HOME/img2irc` (normally `~/.config/img2irc`), with `~/.config/img2irc` as a final fallback.

| file | purpose |
| --- | --- |
| `config.toml` | default font and glyph groups |
| `glyphs.toml` | glyph definitions, exclusions, and native TUI selection |
| `layout.toml` | TUI order, visibility, and initial pane sizes |
| `figlet.toml` | OCR FIGlet fonts by height |

without `glyphs.toml`, native rendering uses the [shipped catalog](../glyphs.toml). to customize it, copy it from the repository root:

```bash
mkdir -p "${XDG_CONFIG_HOME:-$HOME/.config}/img2irc"
cp glyphs.toml "${XDG_CONFIG_HOME:-$HOME/.config}/img2irc/"
```

a minimal `config.toml`:

```toml
font = "Cascadia Code"
blocks = ["half", "full", "space"]
```

custom glyph groups support `glyphs` (code points), `ranges` (inclusive pairs), `include` (other groups), and `exclude` (code points).

in `layout.toml`, `[general]` and `[pipeline]` accept `order` and `hidden` lists. ordered entries come first; unlisted entries follow. `[panes]` sets initial widths. the [included layout](../layout.toml) lists stable IDs.
