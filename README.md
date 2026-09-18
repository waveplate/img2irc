# img2irc (2.0.0-alpha)

*img2irc* converts images to IRC and ANSI art using real font glyphs. it has a CLI, an interactive terminal editor, a browser editor, and Python/JavaScript APIs.

![img2irc block example](https://i.imgur.com/ew513lc.png)

choose from Unicode blocks, braille, or custom glyphs; adjust the image with filters, smooth the contours, and add text overlays. OCR preserves image text, with FIGlet for larger labels. output supports IRC's 99 colours, 256-colour ANSI, and 24-bit truecolour.

## install

from a clone of this repository:

```bash
cargo install --path . --locked
```

see [installation](docs/installation.md) for release downloads, AUR packages, and build requirements. rendering fonts are bundled; no font installation is needed.

## usage

```bash
img2irc photo.png --width 80
img2irc photo.png --render irc --width 80 > art.irc
img2irc --tui photo.png
```

paths and HTTP(S) URLs work as input. `ansi` is the default colour mode. `img2irc --help` lists all CLI options.

## docs

- [usage](docs/usage.md) — CLI, terminal editor, text, and OCR
- [fonts and configuration](docs/configuration.md) — custom fonts, glyph sets, and layouts
- [Python](docs/python.md) — API and wheel builds
- [web and JavaScript](docs/web.md) — browser editor, deployment, and WASM API

> if you like this project, i would appreciate a vote on the [AUR](https://aur.archlinux.org/packages/img2irc-bin)!

licensed under [GPL-3.0-only](LICENSE).
