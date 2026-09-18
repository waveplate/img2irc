"""A small curses front end for the img2irc Python bindings."""

from __future__ import annotations

import argparse
import curses
import os
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Sequence
from urllib.parse import urlparse

from .api import Advanced, Cell, OCR_ENABLED, Output, glyph_groups


@dataclass(frozen=True)
class Control:
    key: str
    label: str
    kind: str = "bool"
    step: float = 1
    minimum: float = 0
    maximum: float = 100
    choices: tuple[str, ...] = ()
    section: str = "General"
    depends_on: str | None = None

    def change(self, value: object, direction: int) -> object:
        if self.kind == "bool":
            return not bool(value)
        if self.kind == "choice":
            index = self.choices.index(str(value))
            return self.choices[(index + direction) % len(self.choices)]
        changed = max(
            self.minimum, min(self.maximum, float(value) + direction * self.step)
        )
        if self.key == "font_size" and 0 < changed < 6:
            changed = 6 if direction > 0 else 0
        return int(changed) if self.kind == "int" else changed

    def display(self, value: object) -> str:
        if self.kind == "bool":
            return "on" if value else "off"
        if self.kind == "float":
            return f"{float(value):g}"
        return str(value).lower()


SECTIONS = ("General", "Smoothing", "OCR", "Effects")

CONTROLS = (
    Control("width", "Width", "int", 4, 0, 4096),
    Control("render", "Mode", "choice", choices=("Ansi24", "Ansi", "Irc")),
    Control("braille", "Braille"),
    Control("font_size", "Font size (0=auto)", "int", 1, 0, 256),
    Control(
        "filter",
        "Filter",
        "choice",
        choices=("Nearest", "Triangle", "CatmullRom", "Gaussian", "Lanczos3"),
    ),
    Control(
        "colorspace", "Colorspace", "choice", choices=("HSL", "HSV", "HSLUV", "LCH")
    ),
    Control(
        "encoding",
        "Encoding",
        "choice",
        choices=("Utf8", "Utf16", "Utf16be", "Utf16le", "Cesu8"),
    ),
    Control("grayscale_tolerance", "Grayscale tolerance", "int", 1, 0, 255),
    Control("show_discontinuities", "Show discontinuities"),
    Control("discontinuity_threshold", "Discontinuity threshold", "float", 1, 0, 1000),
    Control("score_fix", "Smoothing", section="Smoothing"),
    Control(
        "score_fix_candidates",
        "Candidates",
        "int",
        1,
        1,
        256,
        section="Smoothing",
        depends_on="score_fix",
    ),
    Control(
        "score_fix_orderings",
        "Scan patterns",
        "int",
        1,
        1,
        5,
        section="Smoothing",
        depends_on="score_fix",
    ),
    Control(
        "score_fix_geometry_first",
        "Refine shapes",
        section="Smoothing",
        depends_on="score_fix",
    ),
    Control(
        "score_fix_neighborhood_guard",
        "Protect neighbors",
        section="Smoothing",
        depends_on="score_fix",
    ),
    Control(
        "contour_bending_weight",
        "Bending weight",
        "float",
        0.1,
        0,
        10,
        section="Smoothing",
        depends_on="score_fix",
    ),
    Control(
        "contour_endpoint_weight",
        "Endpoint weight",
        "float",
        0.1,
        0,
        10,
        section="Smoothing",
        depends_on="score_fix",
    ),
    Control(
        "contour_junction_weight",
        "Junction weight",
        "float",
        0.1,
        0,
        10,
        section="Smoothing",
        depends_on="score_fix",
    ),
    Control(
        "contour_fragment_weight",
        "Fragment weight",
        "float",
        0.1,
        0,
        10,
        section="Smoothing",
        depends_on="score_fix",
    ),
    Control(
        "contour_fidelity_weight",
        "Fidelity weight",
        "float",
        0.1,
        0,
        10,
        section="Smoothing",
        depends_on="score_fix",
    ),
    Control(
        "contour_boundary_weight",
        "Boundary weight",
        "float",
        0.1,
        0,
        10,
        section="Smoothing",
        depends_on="score_fix",
    ),
    Control(
        "contour_peak_weight",
        "Peak mix",
        "float",
        0.1,
        0,
        1,
        section="Smoothing",
        depends_on="score_fix",
    ),
    Control("ocr", "OCR", section="OCR"),
    Control("ocr_auto_width", "Auto width", section="OCR", depends_on="ocr"),
    Control(
        "ocr_megapixels",
        "Megapixels",
        "float",
        0.1,
        0.1,
        16,
        section="OCR",
        depends_on="ocr",
    ),
    Control(
        "ocr_min_confidence",
        "Min confidence",
        "float",
        0.05,
        0,
        1,
        section="OCR",
        depends_on="ocr",
    ),
    Control(
        "ocr_model_tier",
        "Model",
        "choice",
        choices=("Tiny", "Small", "Medium"),
        section="OCR",
        depends_on="ocr",
    ),
    Control(
        "ocr_threads",
        "Worker threads",
        "int",
        1,
        1,
        32,
        section="OCR",
        depends_on="ocr",
    ),
    Control(
        "ocr_max_text_height_ratio",
        "Max text size ratio",
        "float",
        0.1,
        0,
        10,
        section="OCR",
        depends_on="ocr",
    ),
    Control(
        "ocr_textline_orientation", "Line orientation", section="OCR", depends_on="ocr"
    ),
    Control("ocr_figlet", "Style large text", section="OCR", depends_on="ocr"),
    Control(
        "ocr_figlet_fill", "Fill style space", section="OCR", depends_on="ocr_figlet"
    ),
    Control(
        "ocr_figlet_min_height",
        "Min style rows",
        "float",
        1,
        0,
        12,
        section="OCR",
        depends_on="ocr_figlet",
    ),
    Control(
        "ocr_figlet_min_height_ratio",
        "Min relative size",
        "float",
        0.05,
        0,
        5,
        section="OCR",
        depends_on="ocr_figlet",
    ),
    Control(
        "ocr_figlet_max_width_ratio",
        "Extra style width",
        "float",
        0.05,
        0,
        2,
        section="OCR",
        depends_on="ocr_figlet",
    ),
    Control(
        "ocr_figlet_max_height_ratio",
        "Extra style height",
        "float",
        0.05,
        0,
        2,
        section="OCR",
        depends_on="ocr_figlet",
    ),
    Control("grayscale", "Grayscale", section="Effects"),
    Control("invert", "Invert", section="Effects"),
    Control("brightness", "Brightness", "float", 5, -100, 100, section="Effects"),
    Control("contrast", "Contrast", "float", 5, -100, 100, section="Effects"),
    Control("saturation", "Saturation", "float", 5, -100, 100, section="Effects"),
    Control("gamma", "Gamma", "float", 0.1, -10, 10, section="Effects"),
    Control("hue", "Hue", "float", 5, -180, 180, section="Effects"),
    Control("dither", "Dither", "int", 1, 0, 16, section="Effects"),
    Control("pixelize", "Pixelize", "int", 1, 0, 64, section="Effects"),
    Control("gaussian_blur", "Gaussian blur", "int", 1, 0, 32, section="Effects"),
    Control("box_blur", "Box blur", section="Effects"),
    Control("sharpen", "Sharpen", section="Effects"),
    Control("noise_reduction", "Noise reduction", section="Effects"),
    Control("halftone", "Halftone", section="Effects"),
    Control("sepia", "Sepia", section="Effects"),
    Control("normalize", "Normalize", section="Effects"),
    Control("emboss", "Emboss", section="Effects"),
    Control("edge_detection", "Edge detection", section="Effects"),
    Control("fliph", "Flip horizontal", section="Effects"),
    Control("flipv", "Flip vertical", section="Effects"),
    Control("rotate", "Rotate", "float", 5, -180, 180, section="Effects"),
)


class PythonTui:
    """Mutable application state, kept separate from curses for easy testing."""

    def __init__(
        self,
        image: str,
        *,
        width: int = 80,
        mode: str = "Ansi24",
        ocr: bool = False,
        output: Path | None = None,
        preview: Path | None = None,
        blocks: Sequence[str] | None = None,
        characters: str | None = None,
        font: str | None = None,
    ) -> None:
        self.image = image
        self.font = font
        self.options = Advanced.defaults()
        self.options.update(
            {
                "width": width,
                "render": mode,
                "ocr": ocr,
                # This is a TUI-specific default; the normal Python/CLI default is off.
                "ocr_auto_width": True,
                "ocr_megapixels": 1.5,
            }
        )
        if blocks is not None:
            self.options["blocks"] = list(blocks)
        if characters is not None:
            self.set_characters(characters)
        self._output_explicit = output is not None
        self._preview_explicit = preview is not None
        self.output_path = output or _default_output_path(image, mode)
        self.preview_path = preview or _default_preview_path(self.output_path)
        self._source_paths: set[Path] = set()
        self._source_identities: set[tuple[int, int]] = set()
        self._protect_source(image)
        if font is not None:
            self._protect_source(font, is_image=False)
        self.section_index = 0
        self.selected = 0
        self.result: Output | None = None
        self.status = "Ready"
        self.render_seconds = 0.0
        self._glyph_catalog: dict[str, str] | None = None
        self.preview_x = 0
        self.preview_y = 0

    @property
    def controls(self) -> tuple[Control, ...]:
        return tuple(
            control
            for control in CONTROLS
            if control.section == SECTIONS[self.section_index]
        )

    def next_section(self, direction: int = 1) -> None:
        self.section_index = (self.section_index + direction) % len(SECTIONS)
        self.selected = 0

    def disabled_reason(self, control: Control) -> str | None:
        if control.section == "OCR":
            if not OCR_ENABLED:
                return "OCR is not included in this wheel"
            if control.key != "ocr" and not self.options["ocr"]:
                return "Enable OCR first"
        if control.depends_on and not self.options[control.depends_on]:
            return (
                f"Enable {control.depends_on.replace('score_fix', 'smoothing')} first"
            )
        if (
            control.key == "width"
            and self.options["ocr"]
            and self.options["ocr_auto_width"]
        ):
            return "Disable OCR auto width to use manual width"
        return None

    def change_selected(self, direction: int) -> bool:
        control = self.controls[self.selected]
        reason = self.disabled_reason(control)
        if reason:
            self.status = reason
            return False
        old_value = self.options[control.key]
        self.options[control.key] = control.change(old_value, direction)
        if old_value == self.options[control.key]:
            return False
        if control.key == "render" and not self._output_explicit:
            self.output_path = _default_output_path(
                self.image, str(self.options["render"])
            )
            if not self._preview_explicit:
                self.preview_path = _default_preview_path(self.output_path)
        self.result = None
        return True

    def _protect_source(self, source: str, *, is_image: bool = True) -> None:
        if is_image and _is_remote_image(source):
            return
        path = Path(source).expanduser().resolve()
        self._source_paths.add(path)
        try:
            stat = path.stat()
            self._source_identities.add((stat.st_dev, stat.st_ino))
        except FileNotFoundError:
            pass

    def open_image(self, image: str) -> None:
        self._protect_source(image)
        self.image = image
        self.result = None
        if not self._output_explicit:
            self.output_path = _default_output_path(image, str(self.options["render"]))
        if not self._preview_explicit:
            self.preview_path = _default_preview_path(self.output_path)

    def set_font(self, font: str) -> None:
        self._protect_source(font, is_image=False)
        self.font = font
        self.result = None

    def glyph_catalog(self) -> dict[str, str]:
        if self._glyph_catalog is None:
            self._glyph_catalog = glyph_groups()
        return self._glyph_catalog.copy()

    def set_glyph_groups(self, groups: Sequence[str]) -> None:
        if not groups:
            raise ValueError(
                "Select at least one glyph set, or enter custom characters"
            )
        unknown = set(groups) - self.glyph_catalog().keys()
        if unknown:
            raise ValueError(f"Unknown glyph sets: {', '.join(sorted(unknown))}")
        self.options.update(blocks=list(groups), include=None)
        self.result = None

    def set_characters(self, characters: str) -> None:
        if not characters:
            raise ValueError("Select at least one character")
        self.options.update(blocks=[], include=characters, exclude=[])
        self.result = None

    def glyph_characters(self) -> tuple[str, set[str]]:
        options = {**self.options, "exclude": []}
        characters = Advanced(options, font=self.font).font_info().characters
        return characters, set(characters) - set(self.options["exclude"])

    def set_selected_characters(self, characters: str, selected: set[str]) -> None:
        if not (set(characters) & selected):
            raise ValueError("Select at least one character")
        self.options["exclude"] = sorted(set(characters) - selected)
        self.result = None

    def render(self) -> Output:
        started = time.monotonic()
        self._protect_source(self.image)
        self.result = None
        renderer = Advanced(
            self.options,
            font=self.font,
            include_cells=True,
            include_preview=True,
        )
        self.result = renderer.generate(self.image)
        self.render_seconds = time.monotonic() - started
        self.status = (
            f"Rendered {self.result.columns}x{self.result.rows} cells "
            f"in {self.render_seconds:.2f}s"
        )
        return self.result

    def save_output(self) -> None:
        if self.result is None:
            raise RuntimeError("nothing has been rendered yet")
        self._safe_write(self.output_path, self.result.encoded, self.preview_path)
        self.status = f"Saved {self.output_path}"

    def save_preview(self) -> None:
        if self.result is None:
            raise RuntimeError("nothing has been rendered yet")
        if not self.result.preview_png:
            raise ValueError("this output was generated without a preview")
        self._safe_write(self.preview_path, self.result.preview_png, self.output_path)
        self.status = f"Saved {self.preview_path}"

    def _safe_write(self, path: Path, data: bytes, other_output: Path) -> None:
        self._protect_source(self.image)
        if self.font is not None:
            self._protect_source(self.font, is_image=False)
        path = Path(path).expanduser()
        if any(_same_file(path, source) for source in self._source_paths):
            raise ValueError(f"Refusing to overwrite a source file: {path}")
        if _same_file(path, other_output):
            raise ValueError("Terminal output and PNG preview must use different files")
        # Open without truncating, then check the actual inode before modifying
        # anything. This also protects a source behind a hard link or symlink.
        with path.open("ab") as target:
            stat = os.fstat(target.fileno())
            if (stat.st_dev, stat.st_ino) in self._source_identities:
                raise ValueError(f"Refusing to overwrite a source file: {path}")
            target.truncate(0)
            target.write(data)


def _cell_sgr(cell: Cell, colour: bool = True) -> str:
    """Use final display colours from the bindings, without palette remapping."""
    codes = ["0"]
    for enabled, code in (
        (cell.bold, "1"),
        (cell.italic, "3"),
        (cell.underline, "4"),
        (cell.inverted, "7"),
    ):
        if enabled:
            codes.append(code)
    if colour:
        codes.append("38;2;" + ";".join(map(str, cell.foreground)))
        if cell.background is not None:
            codes.append("48;2;" + ";".join(map(str, cell.background)))
    return "\x1b[" + ";".join(codes) + "m"


def _preview_ansi(
    cells: Sequence[Sequence[Cell]],
    *,
    top: int,
    left: int,
    height: int,
    width: int,
    row_offset: int = 0,
    column_offset: int = 0,
    colour: bool = True,
) -> str:
    """Paint the complete viewport, including blank space after a smaller image.

    Python curses colour attributes only address 256 pairs, even on terminals
    advertising 65536. Writing RGB SGR directly avoids wrapped pair IDs and
    preserves IRC/ANSI palette colours as well as ANSI24's truecolour pixels.
    Curses still owns the controls and input; restore its cursor and style.
    """
    if width <= 0 or height <= 0:
        return ""
    output = ["\x1b7"]
    for y in range(height):
        output.append(f"\x1b[{top + y + 1};{left + 1}H\x1b[0m")
        row_index = row_offset + y
        row = cells[row_index] if row_index < len(cells) else ()
        visible = row[column_offset : column_offset + width]
        previous_style = None
        for cell in visible:
            style = _cell_sgr(cell, colour)
            if style != previous_style:
                output.append(style)
                previous_style = style
            # A custom alphabet must not inject terminal control characters.
            output.append(cell.character if cell.character.isprintable() else " ")
        output.append("\x1b[0m" + " " * (width - len(visible)))
    output.append("\x1b[0m\x1b8")
    return "".join(output)


def _default_output_path(image: str, mode: str) -> Path:
    if _is_remote_image(image):
        return Path("img2irc-output.txt" if mode == "Irc" else "img2irc-output.ans")
    suffix = ".txt" if mode == "Irc" else ".ans"
    path = Path(image).expanduser()
    return path.with_name(path.stem + ".img2irc" + suffix)


def _is_remote_image(image: str) -> bool:
    # Match the binding's HTTP(S)-only URL handling. Other strings are paths.
    return urlparse(image).scheme.lower() in {"http", "https"}


def _default_preview_path(output: Path) -> Path:
    # A custom --output ending in .png must not collide with the preview.
    return (
        output.with_name(output.stem + ".preview.png")
        if output.suffix.lower() == ".png"
        else output.with_suffix(".png")
    )


def _same_file(first: Path, second: Path) -> bool:
    if first.expanduser().resolve() == second.expanduser().resolve():
        return True
    try:
        return first.expanduser().samefile(second.expanduser())
    except FileNotFoundError:
        return False


def _put(screen: curses.window, y: int, x: int, text: str, attribute: int = 0) -> None:
    height, width = screen.getmaxyx()
    if not 0 <= y < height or x >= width - 1:
        return
    try:
        screen.addstr(y, max(0, x), text[: max(0, width - x - 1)], attribute)
    except curses.error:
        pass


def _draw(screen: curses.window, app: PythonTui, colour: bool) -> None:
    if app.result is None or app.result.cells is None:
        # Also remove an out-of-band preview after a failed/invalidated render.
        screen.clear()
    screen.erase()
    height, width = screen.getmaxyx()
    panel_width = min(38, max(24, width // 2))
    _put(screen, 0, 0, f" img2irc Python TUI — {app.image}", curses.A_BOLD)
    sections = "  ".join(
        f"[{name}]" if i == app.section_index else name
        for i, name in enumerate(SECTIONS)
    )
    _put(screen, 1, 0, sections, curses.A_BOLD)

    controls = app.controls
    available_rows = max(1, height - 6)
    scroll = max(
        0, min(app.selected - available_rows + 1, len(controls) - available_rows)
    )
    for row, index in enumerate(
        range(scroll, min(len(controls), scroll + available_rows)), 2
    ):
        control = controls[index]
        value = control.display(app.options[control.key])
        marker = ">" if index == app.selected else " "
        label = f"{marker} {control.label[:panel_width - 11]:<{panel_width - 11}} {value:>7}"
        attribute = curses.A_REVERSE if index == app.selected else curses.A_NORMAL
        if app.disabled_reason(control):
            attribute |= curses.A_DIM
        _put(screen, row, 0, label, attribute)

    preview_x = panel_width + 1
    if width > preview_x:
        for y in range(2, max(2, height - 3)):
            _put(screen, y, panel_width, "│", curses.A_DIM)
        if app.result is None or app.result.cells is None:
            _put(screen, 2, preview_x, "No preview yet", curses.A_DIM)

    _put(screen, height - 3, 0, app.status, curses.A_BOLD)
    _put(
        screen,
        height - 2,
        0,
        f"s: {app.output_path}   p: {app.preview_path}",
        curses.A_DIM,
    )
    _put(
        screen,
        height - 1,
        0,
        "Tab section  ↑↓ select  ←→ change  g sets  c chars  e custom  f font  o open  q quit",
        curses.A_DIM,
    )
    screen.refresh()
    if app.result is not None and app.result.cells is not None:
        sys.stdout.write(
            _preview_ansi(
                app.result.cells,
                top=2,
                left=preview_x,
                height=max(0, height - 5),
                width=max(0, width - preview_x - 1),
                row_offset=app.preview_y,
                column_offset=app.preview_x,
                colour=colour,
            )
        )
        sys.stdout.flush()


def _prompt(
    screen: curses.window, label: str, current: str, *, strip: bool = True
) -> str:
    height, width = screen.getmaxyx()
    screen.move(height - 1, 0)
    screen.clrtoeol()
    hint = current[: max(0, width - len(label) - 12)]
    prompt = f"{label} [{hint}]: "
    _put(screen, height - 1, 0, prompt)
    screen.refresh()
    curses.echo()
    try:
        curses.curs_set(1)
    except curses.error:
        pass
    try:
        value = screen.getstr(height - 1, min(width - 2, len(prompt)), 4096)
        decoded = value.decode(errors="replace")
        return (decoded.strip() if strip else decoded) or current
    finally:
        curses.noecho()
        try:
            curses.curs_set(0)
        except curses.error:
            pass


def _checklist(
    screen: curses.window,
    title: str,
    items: list[tuple[str, str]],
    selected: set[str],
) -> set[str] | None:
    """Edit a staged selection; Enter applies, Escape/q cancels."""
    selected = selected.copy()
    cursor = 0
    if not items:
        raise ValueError("No glyphs are available for the selected font/sets")
    # Curses cannot track RGB preview writes; force a clear before a modal.
    screen.clear()
    while True:
        screen.erase()
        height, _ = screen.getmaxyx()
        _put(screen, 0, 0, title, curses.A_BOLD)
        rows = max(1, height - 4)
        start = max(0, cursor - rows + 1)
        for y, index in enumerate(range(start, min(len(items), start + rows)), 2):
            value, label = items[index]
            check = "x" if value in selected else " "
            attribute = curses.A_REVERSE if index == cursor else curses.A_NORMAL
            _put(screen, y, 0, f"[{check}] {label}", attribute)
        _put(screen, height - 2, 0, f"{len(selected)} selected", curses.A_BOLD)
        _put(
            screen,
            height - 1,
            0,
            "↑↓ move  space toggle  a all  n none  Enter apply  q cancel",
        )
        screen.refresh()
        key = screen.get_wch()
        if key in ("q", "\x1b"):
            return None
        if key in ("\n", "\r", curses.KEY_ENTER):
            return selected
        if key in (curses.KEY_UP, "k"):
            cursor = (cursor - 1) % len(items)
        elif key in (curses.KEY_DOWN, "j"):
            cursor = (cursor + 1) % len(items)
        elif key == " ":
            value = items[cursor][0]
            if value in selected:
                selected.remove(value)
            else:
                selected.add(value)
        elif key == "a":
            selected = {value for value, _ in items}
        elif key == "n":
            selected.clear()


def _edit_glyphs(screen: curses.window, app: PythonTui, *, individual: bool) -> bool:
    if individual:
        characters, selected = app.glyph_characters()
        items = [
            (char, f"{char if char != ' ' else 'SPACE'}  U+{ord(char):04X}")
            for char in characters
        ]
        changed = _checklist(
            screen, "Individual glyphs (for current sets/font)", items, selected
        )
        if changed is None:
            return False
        app.set_selected_characters(characters, changed)
    else:
        catalog = app.glyph_catalog()
        items = [
            (name, f"{name} ({len(chars)} glyphs)")
            for name, chars in sorted(catalog.items())
        ]
        selected = {
            name
            for name in catalog
            if any(
                name == block or name.startswith(block + ".")
                for block in app.options["blocks"]
            )
        }
        changed = _checklist(screen, "Glyph sets", items, selected)
        if changed is None:
            return False
        app.set_glyph_groups(sorted(changed))
    return True


def _run(screen: curses.window, app: PythonTui) -> None:
    screen.keypad(True)
    try:
        curses.curs_set(0)
    except curses.error:
        pass
    colour = curses.has_colors()
    if colour:
        curses.start_color()
        try:
            curses.use_default_colors()
        except curses.error:
            pass
    dirty = True

    while True:
        if dirty:
            app.status = "Rendering…"
            _draw(screen, app, colour)
            try:
                app.render()
            except Exception as error:  # Keep the UI alive for bad paths/options.
                app.status = f"Error: {error}"
            dirty = False
        _draw(screen, app, colour)

        key = screen.get_wch()
        if key in ("q", "Q", "\x1b"):
            return
        if key == "\t":
            app.next_section()
        elif key == curses.KEY_RESIZE:
            # Clear the previous viewport after its position/size changes.
            screen.clear()
        elif key == curses.KEY_BTAB:
            app.next_section(-1)
        elif key in (curses.KEY_UP, "k"):
            app.selected = (app.selected - 1) % len(app.controls)
        elif key in (curses.KEY_DOWN, "j"):
            app.selected = (app.selected + 1) % len(app.controls)
        elif key in (curses.KEY_LEFT, "h", "-"):
            dirty = app.change_selected(-1)
        elif key in (curses.KEY_RIGHT, "l", "+", " "):
            dirty = app.change_selected(1)
        elif key in ("r", "R"):
            dirty = True
        elif key in ("o", "O"):
            try:
                app.open_image(_prompt(screen, "Image", app.image))
                dirty = True
            except Exception as error:
                app.status = f"Error: {error}"
        elif key in ("g", "c", "e", "f"):
            try:
                if key in ("g", "c"):
                    dirty = _edit_glyphs(screen, app, individual=key == "c")
                elif key == "e":
                    app.set_characters(
                        _prompt(
                            screen,
                            "Characters",
                            app.options["include"] or " ▀█",
                            strip=False,
                        )
                    )
                    dirty = True
                elif key == "f":
                    font = _prompt(screen, "Font path", app.font or "")
                    if font:
                        app.set_font(font)
                        dirty = True
            except Exception as error:
                app.status = f"Error: {error}"
        elif key in (curses.KEY_PPAGE, curses.KEY_NPAGE):
            limit = max(0, (app.result.rows if app.result else 0) - 1)
            app.preview_y = max(
                0, min(limit, app.preview_y + (-10 if key == curses.KEY_PPAGE else 10))
            )
        elif key in ("[", "]"):
            limit = max(0, (app.result.columns if app.result else 0) - 1)
            app.preview_x = max(
                0, min(limit, app.preview_x + (-10 if key == "[" else 10))
            )
        elif key == "s":
            try:
                app.save_output()
            except Exception as error:
                app.status = f"Error: {error}"
        elif key == "p":
            try:
                app.save_preview()
            except Exception as error:
                app.status = f"Error: {error}"


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("image", help="image path or HTTP(S) URL")
    parser.add_argument("--width", type=int, default=80, help="initial output width")
    parser.add_argument(
        "--mode",
        choices=("ansi24", "ansi", "irc"),
        default="ansi24",
        help="initial output format",
    )
    parser.add_argument("--ocr", action="store_true", help="start with OCR enabled")
    parser.add_argument(
        "--font", help="monospace font path (default: embedded Cascadia)"
    )
    parser.add_argument(
        "--glyphs", dest="blocks", action="append", metavar="GLYPHS", help="initial glyph set; repeat for multiple sets"
    )
    parser.add_argument(
        "--blocks", dest="blocks", action="append", help=argparse.SUPPRESS
    )
    parser.add_argument(
        "--characters", help="use exactly these characters instead of named sets"
    )
    parser.add_argument("--output", type=Path, help="path used by the save command")
    parser.add_argument(
        "--preview", type=Path, help="PNG path used by the preview command"
    )
    return parser


def main(argv: Sequence[str] | None = None) -> None:
    args = build_parser().parse_args(argv)
    if not 0 <= args.width <= 4096:
        raise SystemExit("--width must be between 0 (automatic) and 4096")
    mode = {"ansi24": "Ansi24", "ansi": "Ansi", "irc": "Irc"}[args.mode]
    app = PythonTui(
        args.image,
        width=args.width,
        mode=mode,
        ocr=args.ocr,
        output=args.output,
        preview=args.preview,
        blocks=args.blocks,
        characters=args.characters,
        font=args.font,
    )
    curses.wrapper(_run, app)


if __name__ == "__main__":
    main()
