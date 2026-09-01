from __future__ import annotations

import copy
import json
import os
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, BinaryIO, Mapping
from urllib.parse import urlparse
from urllib.request import urlopen

from . import _native

ImageSource = str | os.PathLike[str] | bytes | bytearray | memoryview | BinaryIO
FontSource = str | os.PathLike[str] | bytes | bytearray | memoryview

_ENUM_VALUES = {
    "render": {"irc": "Irc", "ansi": "Ansi", "ansi24": "Ansi24"},
    "filter": {
        "nearest": "Nearest",
        "triangle": "Triangle",
        "catmullrom": "CatmullRom",
        "catmull-rom": "CatmullRom",
        "gaussian": "Gaussian",
        "lanczos3": "Lanczos3",
    },
    "colorspace": {"hsl": "HSL", "hsv": "HSV", "hsluv": "HSLUV", "lch": "LCH"},
    "encoding": {
        "utf8": "Utf8",
        "utf-8": "Utf8",
        "utf16": "Utf16",
        "utf-16": "Utf16",
        "utf16be": "Utf16be",
        "utf-16be": "Utf16be",
        "utf16le": "Utf16le",
        "utf-16le": "Utf16le",
        "cesu8": "Cesu8",
        "cesu-8": "Cesu8",
    },
    "ocr_model_tier": {"tiny": "Tiny", "small": "Small", "medium": "Medium"},
}

_EFFECT_NAMES = {
    "brightness": "Brightness",
    "contrast": "Contrast",
    "saturation": "Saturation",
    "gamma": "Gamma",
    "hue": "Hue",
    "invert": "Invert",
    "box_blur": "BoxBlur",
    "gaussian_blur": "GaussBlur",
    "luma_blur": "LumaBlur",
    "pixelize": "Pixelize",
    "halftone": "Halftone",
    "sepia": "Sepia",
    "solarize": "Solarize",
    "normalize": "Normalize",
    "noise": "Noise",
    "sharpen": "Sharpen",
    "edge_detection": "EdgeDetect",
    "emboss": "Emboss",
    "frosted_glass": "FrostGlass",
    "grayscale": "Grayscale",
    "identity": "Identity",
    "laplace": "Laplace",
    "cali": "Cali",
    "dramatic": "Dramatic",
    "firenze": "Firenze",
    "golden": "Golden",
    "lix": "Lix",
    "lofi": "Lofi",
    "neue": "Neue",
    "obsidian": "Obsidian",
    "pastel_pink": "PastelPink",
    "ryo": "Ryo",
    "oil": "Oil",
    "dither": "Dither",
    "crop_top": "CropTop",
    "crop_bottom": "CropBottom",
    "crop_left": "CropLeft",
    "crop_right": "CropRight",
    "flip_horizontal": "FlipH",
    "flip_vertical": "FlipV",
    "rotate": "Rotate",
    "scale_x": "ScaleX",
    "scale_y": "ScaleY",
}

_UNIT_EFFECTS = {
    "Invert",
    "BoxBlur",
    "Halftone",
    "Sepia",
    "Solarize",
    "Normalize",
    "Noise",
    "Sharpen",
    "EdgeDetect",
    "Emboss",
    "FrostGlass",
    "Grayscale",
    "Identity",
    "Laplace",
    "Cali",
    "Dramatic",
    "Firenze",
    "Golden",
    "Lix",
    "Lofi",
    "Neue",
    "Obsidian",
    "PastelPink",
    "Ryo",
    "FlipH",
    "FlipV",
}


@dataclass(frozen=True)
class Output:
    """A generated terminal-art result and its pixel-exact preview."""

    content: str
    save_content: str | None
    encoded: bytes = field(repr=False)
    columns: int
    rows: int
    preview_width: int
    preview_height: int
    preview_rgba: bytes = field(repr=False)
    preview_png: bytes = field(repr=False)
    cells: list[list[dict[str, Any]]] | None = field(default=None, repr=False)
    options: dict[str, Any] = field(default_factory=dict, repr=False)
    timings: dict[str, Any] = field(default_factory=dict, repr=False)
    metadata: dict[str, Any] = field(default_factory=dict, repr=False)

    def __str__(self) -> str:
        return self.content

    def write(self, path: str | os.PathLike[str]) -> None:
        """Write bytes using the renderer's selected output encoding."""
        Path(path).write_bytes(self.encoded)

    def save_preview(self, path: str | os.PathLike[str]) -> None:
        if not self.preview_png:
            raise ValueError("this output was generated without a preview")
        Path(path).write_bytes(self.preview_png)


def _read_bytes(source: ImageSource | FontSource, *, what: str) -> bytes:
    if isinstance(source, bytes):
        return source
    if isinstance(source, (bytearray, memoryview)):
        return bytes(source)
    if hasattr(source, "read"):
        data = source.read()
        if not isinstance(data, (bytes, bytearray, memoryview)):
            raise TypeError(f"{what} file object must be opened in binary mode")
        return bytes(data)
    if isinstance(source, (str, os.PathLike)):
        value = os.fspath(source)
        parsed = urlparse(value)
        if what == "image" and parsed.scheme in {"http", "https"}:
            with urlopen(value, timeout=30) as response:
                return response.read()
        return Path(value).read_bytes()
    raise TypeError(f"{what} must be bytes, a path, or a binary file object")


def _normalise_option(name: str, value: Any) -> Any:
    if name == "mode":
        name = "render"
    if name in _ENUM_VALUES and isinstance(value, str):
        try:
            return _ENUM_VALUES[name][value.lower()]
        except KeyError as error:
            choices = ", ".join(_ENUM_VALUES[name])
            raise ValueError(f"invalid {name} {value!r}; choose one of: {choices}") from error
    if name in {"width", "height"} and value == 0:
        return None
    if name == "exclude" and isinstance(value, str):
        return list(value)
    return value


def _colour(value: int | tuple[int, int, int] | list[int] | None) -> Any:
    if value is None:
        return None
    if isinstance(value, int):
        if not 0 <= value <= 255:
            raise ValueError("indexed colours must be between 0 and 255")
        return {"Index": value}
    channels = list(value)
    if len(channels) != 3 or any(not 0 <= channel <= 255 for channel in channels):
        raise ValueError("RGB colours must contain three values between 0 and 255")
    return {"Rgb": channels}


class Advanced:
    """Full-fidelity generator exposing every serializable TUI render option.

    Keyword names match ``RenderArgs`` in the default-options dictionary. Use
    :meth:`add_effect` for ordered pipeline operations and :meth:`add_overlay`
    for editable terminal text layers.
    """

    def __init__(
        self,
        options: Mapping[str, Any] | None = None,
        *,
        font: FontSource | None = None,
        system_fonts: bool = False,
        include_cells: bool = False,
        include_preview: bool = True,
        **overrides: Any,
    ) -> None:
        self._options: dict[str, Any] = {}
        self.font = font
        self.system_fonts = system_fonts
        self.include_cells = include_cells
        self.include_preview = include_preview
        if options:
            self.update(options)
        self.update(overrides)

    @staticmethod
    def defaults() -> dict[str, Any]:
        return json.loads(_native.default_options_json())

    @staticmethod
    def available_options() -> tuple[str, ...]:
        return tuple(Advanced.defaults())

    @property
    def options(self) -> dict[str, Any]:
        return copy.deepcopy(self._options)

    def set(self, name: str, value: Any) -> Advanced:
        if name == "mode":
            name = "render"
        if name not in self.defaults():
            raise KeyError(f"unknown img2irc option {name!r}")
        self._options[name] = _normalise_option(name, value)
        return self

    def update(self, options: Mapping[str, Any] | None = None, **values: Any) -> Advanced:
        merged = dict(options or {})
        merged.update(values)
        for name, value in merged.items():
            self.set(name, value)
        return self

    def add_effect(self, name: str, value: Any = None) -> Advanced:
        key = name.strip().lower().replace("-", " ").replace(" ", "_")
        rust_name = _EFFECT_NAMES.get(key, name)
        if rust_name in _UNIT_EFFECTS:
            if value is not None:
                raise ValueError(f"effect {name!r} does not take a value")
            effect: Any = rust_name
        else:
            if value is None:
                raise ValueError(f"effect {name!r} requires a value")
            effect = {rust_name: value}
        self._options.setdefault("pipeline", []).append(effect)
        return self

    def add_overlay(
        self,
        text: str,
        x: int,
        y: int,
        *,
        width: int | None = None,
        height: int | None = None,
        foreground: int | tuple[int, int, int] | None = None,
        background: int | tuple[int, int, int] | None = None,
        wrap: bool = False,
        auto_grow: bool = True,
        transparent_spaces: bool = False,
        bold: bool = False,
        italic: bool = False,
        underline: bool = False,
    ) -> Advanced:
        lines = text.splitlines() or [""]
        overlay = {
            "text": text,
            "source_text": None,
            "figlet_font": None,
            "x": x,
            "y": y,
            "w": width if width is not None else max(map(len, lines)),
            "h": height if height is not None else len(lines),
            "fg": _colour(foreground),
            "bg": _colour(background),
            "wrap": wrap,
            "auto_grow": auto_grow,
            "transparent_spaces": transparent_spaces,
            "bold": bold,
            "italic": italic,
            "underline": underline,
        }
        self._options.setdefault("overlays", []).append(overlay)
        return self

    def generate(self, image: ImageSource, **overrides: Any) -> Output:
        options = self.options
        for name, value in overrides.items():
            if name == "mode":
                name = "render"
            if name not in self.defaults():
                raise KeyError(f"unknown img2irc option {name!r}")
            options[name] = _normalise_option(name, value)
        image_data = _read_bytes(image, what="image")
        font_data = None if self.font is None else _read_bytes(self.font, what="font")
        metadata_json, preview_rgba, preview_png, encoded = _native.generate_json(
            image_data,
            json.dumps(options),
            font_data,
            self.system_fonts,
            self.include_cells,
            self.include_preview,
        )
        metadata = json.loads(metadata_json)
        return Output(
            content=metadata["content"],
            save_content=metadata["save_content"],
            encoded=bytes(encoded),
            columns=metadata["columns"],
            rows=metadata["rows"],
            preview_width=metadata["preview_width"],
            preview_height=metadata["preview_height"],
            preview_rgba=bytes(preview_rgba),
            preview_png=bytes(preview_png),
            cells=metadata.pop("cells"),
            options=metadata.pop("options"),
            timings=metadata.pop("timings"),
            metadata=metadata,
        )

    __call__ = generate


class Simple:
    """Convenient generator for the options most applications commonly need."""

    def __init__(
        self,
        width: int | None = 80,
        *,
        height: int | None = None,
        mode: str = "ansi24",
        font_size: float = 0,
        braille: bool = False,
        blocks: list[str] | tuple[str, ...] | None = None,
        characters: str | None = None,
        smooth: bool = False,
        ocr: bool = False,
        font: FontSource | None = None,
    ) -> None:
        options: dict[str, Any] = {
            "width": width,
            "height": height,
            "render": mode,
            "font_size": font_size,
            "braille": braille,
            "score_fix": smooth,
            "ocr": ocr,
        }
        if blocks is not None:
            options["blocks"] = list(blocks)
        if characters is not None:
            options["blocks"] = []
            options["include"] = characters
        self._advanced = Advanced(options, font=font)

    @property
    def advanced(self) -> Advanced:
        return self._advanced

    def generate(self, image: ImageSource, **overrides: Any) -> Output:
        return self._advanced.generate(image, **overrides)

    __call__ = generate
