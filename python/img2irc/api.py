from __future__ import annotations

import copy
import json
import math
import os
from dataclasses import asdict, dataclass, field
from enum import Enum
from functools import lru_cache
from pathlib import Path
from typing import Any, BinaryIO, Iterable, Mapping
from urllib.parse import urlparse
from urllib.request import urlopen

from . import _native

ImageSource = str | os.PathLike[str] | bytes | bytearray | memoryview | BinaryIO
FontSource = str | os.PathLike[str] | bytes | bytearray | memoryview
RGB = tuple[int, int, int]


class _StringEnum(str, Enum):
    def __str__(self) -> str:
        return self.value


class RenderMode(_StringEnum):
    IRC = "irc"
    ANSI = "ansi"
    ANSI24 = "ansi24"


class SamplingFilter(_StringEnum):
    NEAREST = "nearest"
    TRIANGLE = "triangle"
    CATMULL_ROM = "catmull-rom"
    GAUSSIAN = "gaussian"
    LANCZOS3 = "lanczos3"


class ColourSpace(_StringEnum):
    HSL = "hsl"
    HSV = "hsv"
    HSLUV = "hsluv"
    LCH = "lch"


class Encoding(_StringEnum):
    UTF8 = "utf-8"
    UTF16 = "utf-16"
    UTF16_BE = "utf-16be"
    UTF16_LE = "utf-16le"
    CESU8 = "cesu-8"


class OcrModelTier(_StringEnum):
    TINY = "tiny"
    SMALL = "small"
    MEDIUM = "medium"


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
    "replace_colour": "ReplaceColour",
    "replace_color": "ReplaceColour",
    "brightness": "Brightness",
    "contrast": "Contrast",
    "luma_contrast": "LumaContrast",
    "saturation": "Saturation",
    "gamma": "Gamma",
    "hue": "Hue",
    "invert": "Invert",
    "box_blur": "BoxBlur",
    "gaussian_blur": "GaussBlur",
    "gauss_blur": "GaussBlur",
    "luma_blur": "LumaBlur",
    "median_blur": "MedianBlur",
    "line_thickness": "DarkLines",
    "dark_lines": "DarkLines",
    "light_lines": "LightLines",
    "pixelize": "Pixelize",
    "halftone": "Halftone",
    "sepia": "Sepia",
    "solarize": "Solarize",
    "normalize": "Normalize",
    "noise": "Noise",
    "sharpen": "Sharpen",
    "edge_detection": "EdgeDetect",
    "edge_detect": "EdgeDetect",
    "emboss": "Emboss",
    "frosted_glass": "FrostGlass",
    "frost_glass": "FrostGlass",
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
    "flip_h": "FlipH",
    "flip_vertical": "FlipV",
    "flip_v": "FlipV",
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

EFFECTS = tuple(_EFFECT_NAMES)
ANSI256: tuple[RGB, ...] = tuple(tuple(colour) for colour in _native.ANSI256)
IRC99: tuple[RGB, ...] = tuple(tuple(colour) for colour in _native.IRC99)
OCR_ENABLED: bool = _native.OCR_ENABLED


class _RecordMapping(Mapping[str, Any]):
    """Give typed result records backwards-compatible dictionary access."""

    def __getitem__(self, key: str) -> Any:
        if key not in self.__dataclass_fields__:
            raise KeyError(key)
        return getattr(self, key)

    def __iter__(self):
        return iter(self.__dataclass_fields__)

    def __len__(self) -> int:
        return len(self.__dataclass_fields__)


@dataclass(frozen=True)
class Cell(_RecordMapping):
    """One styled terminal cell in a generated output grid."""

    character: str
    foreground: RGB
    background: RGB | None
    inverted: bool
    bold: bool
    italic: bool
    underline: bool

    @classmethod
    def _from_dict(cls, value: Mapping[str, Any]) -> Cell:
        return cls(
            character=value["character"],
            foreground=tuple(value["foreground"]),
            background=None if value["background"] is None else tuple(value["background"]),
            inverted=value["inverted"],
            bold=value["bold"],
            italic=value["italic"],
            underline=value["underline"],
        )


@dataclass(frozen=True)
class Timings(_RecordMapping):
    prepare_canvas_ms: float
    glyph_match_ms: float
    smoothing_prepare_ms: float
    smoothing_search_ms: float
    shape_refine_ms: float
    final_score_ms: float
    encode_ms: float
    contour_cache_hits: int
    contour_cache_misses: int


@dataclass(frozen=True)
class OcrDetection:
    text: str
    confidence: float
    polygon: tuple[tuple[float, float], ...]

    @classmethod
    def _from_dict(cls, value: Mapping[str, Any]) -> OcrDetection:
        return cls(
            text=value["text"],
            confidence=value["confidence"],
            polygon=tuple(tuple(point) for point in value["polygon"]),
        )


@dataclass(frozen=True)
class ProcessedImage:
    """RGBA image produced by img2irc's ordered effect pipeline."""

    width: int
    height: int
    rgba: bytes = field(repr=False)
    png: bytes = field(repr=False)

    def save(self, path: str | os.PathLike[str]) -> None:
        if not self.png:
            raise ValueError("this image was generated without PNG encoding")
        Path(path).write_bytes(self.png)


@dataclass(frozen=True)
class FontInfo:
    glyph_count: int
    selected_count: int
    cell_width: int
    cell_height: int
    cell_advance: float
    line_height: float
    font_names: tuple[str, ...]
    groups: dict[str, str]
    characters: str


@dataclass(frozen=True)
class ContourWeights:
    bending: float = 1.0
    endpoints: float = 1.0
    junctions: float = 2.0
    fragments: float = 1.0
    fidelity: float = 0.1
    boundary_balance: float = 0.1
    peak_sensitivity: float = 0.4


@dataclass(frozen=True)
class ContourScore:
    total: float
    bending: float
    endpoints: float
    junctions: float
    fragments: float
    boundary_length: int
    endpoint_count: int
    junction_count: int
    component_count: int
    fidelity: float
    boundary_balance: float


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
    error_count: int
    total_pixels: int
    dynamic_scaling_count: int
    score: float
    longest_line_bytes: int
    font_size: float
    cell_width: int
    cell_height: int
    cell_advance: float
    line_height: float
    timings: Timings
    cells: list[list[Cell]] | None = field(default=None, repr=False)
    ocr_detections: tuple[OcrDetection, ...] = field(default=(), repr=False)
    options: dict[str, Any] = field(default_factory=dict, repr=False)
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
    if name in {"config_dir", "figlet_dir", "ocr_model_cache"} and isinstance(
        value, os.PathLike
    ):
        return os.fspath(value)
    if name in {"scale", "crop", "trim"} and isinstance(value, tuple):
        return list(value)
    return value


def _rgb(value: tuple[int, int, int] | list[int]) -> list[int]:
    if not isinstance(value, (tuple, list)):
        raise TypeError("RGB colours must be a tuple or list")
    channels = list(value)
    if len(channels) != 3 or any(
        isinstance(channel, bool) or not isinstance(channel, int) or not 0 <= channel <= 255
        for channel in channels
    ):
        raise ValueError("RGB colours must contain three integers between 0 and 255")
    return channels


def _colour(value: int | tuple[int, int, int] | list[int] | None) -> Any:
    if value is None:
        return None
    if isinstance(value, int) and not isinstance(value, bool):
        if not 0 <= value <= 255:
            raise ValueError("indexed colours must be between 0 and 255")
        return {"Index": value}
    return {"Rgb": _rgb(value)}


@lru_cache(maxsize=1)
def _default_options() -> dict[str, Any]:
    return json.loads(_native.default_options_json())


def _resolve_options(options: Mapping[str, Any]) -> dict[str, Any]:
    return json.loads(_native.resolved_options_json(json.dumps(options)))


def _output_from_native(
    metadata_json: str,
    preview_rgba: bytes,
    preview_png: bytes,
    encoded: bytes,
) -> Output:
    metadata = json.loads(metadata_json)
    raw_cells = metadata["cells"]
    cells = (
        None
        if raw_cells is None
        else [[Cell._from_dict(cell) for cell in row] for row in raw_cells]
    )
    timings = Timings(**metadata["timings"])
    detections = tuple(
        OcrDetection._from_dict(detection) for detection in metadata["ocr_detections"]
    )
    output_metadata = {
        key: value
        for key, value in metadata.items()
        if key not in {"cells", "options", "timings", "ocr_detections"}
    }
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
        error_count=metadata["error_count"],
        total_pixels=metadata["total_pixels"],
        dynamic_scaling_count=metadata["dynamic_scaling_count"],
        score=metadata["score"],
        longest_line_bytes=metadata["longest_line_bytes"],
        font_size=metadata["font_size"],
        cell_width=metadata["cell_width"],
        cell_height=metadata["cell_height"],
        cell_advance=metadata["cell_advance"],
        line_height=metadata["line_height"],
        timings=timings,
        cells=cells,
        ocr_detections=detections,
        options=metadata["options"],
        metadata=output_metadata,
    )


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
        font_names: Iterable[str] | None = None,
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
        if font_names is not None:
            self.set("font", list(font_names))
        self.update(overrides)

    @classmethod
    def from_json(cls, value: str, **kwargs: Any) -> Advanced:
        """Create a renderer from a JSON object containing partial options."""
        options = json.loads(value)
        if not isinstance(options, dict):
            raise ValueError("generation options must be a JSON object")
        return cls(options, **kwargs)

    @staticmethod
    def defaults() -> dict[str, Any]:
        return copy.deepcopy(_default_options())

    @staticmethod
    def available_options() -> tuple[str, ...]:
        return tuple(_default_options())

    @property
    def options(self) -> dict[str, Any]:
        return copy.deepcopy(self._options)

    @property
    def resolved_options(self) -> dict[str, Any]:
        """All options after merging this renderer with native defaults."""
        return _resolve_options(self._options)

    def to_json(self, *, resolved: bool = False, indent: int | None = None) -> str:
        """Serialize partial or fully resolved renderer options."""
        options = self.resolved_options if resolved else self.options
        return json.dumps(options, indent=indent)

    def validate(self) -> Advanced:
        """Validate the current configuration without rendering an image."""
        _resolve_options(self._options)
        return self

    def copy(self) -> Advanced:
        return Advanced(
            self._options,
            font=self.font,
            system_fonts=self.system_fonts,
            include_cells=self.include_cells,
            include_preview=self.include_preview,
        )

    def set(self, name: str, value: Any) -> Advanced:
        if name == "mode":
            name = "render"
        if name not in _default_options():
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
        if key in {"replace_color", "replace_colour"}:
            if not isinstance(value, Mapping):
                raise ValueError(
                    "replace_colour requires a mapping with 'from', 'to', and optional "
                    "'tolerance' values"
                )
            try:
                from_colour = value["from"]
                to_colour = value["to"]
            except KeyError as error:
                raise ValueError("replace_colour requires both 'from' and 'to' colours") from error
            return self.add_replace_colour(
                from_colour,
                to_colour,
                tolerance=value.get("tolerance", 5.0),
            )
        rust_name = _EFFECT_NAMES.get(key, name)
        known_effects = set(_EFFECT_NAMES.values())
        if rust_name not in known_effects:
            choices = ", ".join(EFFECTS)
            raise ValueError(f"unknown effect {name!r}; choose one of: {choices}")
        if rust_name in _UNIT_EFFECTS:
            if value is not None:
                raise ValueError(f"effect {name!r} does not take a value")
            effect: Any = rust_name
        else:
            if value is None:
                raise ValueError(f"effect {name!r} requires a value")
            if rust_name == "Oil":
                if not isinstance(value, (tuple, list)) or len(value) != 2:
                    raise ValueError("oil requires a (radius, intensity) pair")
                value = list(value)
            effect = {rust_name: value}
        self._options.setdefault("pipeline", []).append(effect)
        return self

    def remove_effect(self, index: int) -> Advanced:
        """Remove a pipeline effect by its zero-based index."""
        self._options.setdefault("pipeline", []).pop(index)
        return self

    def clear_effects(self) -> Advanced:
        self._options["pipeline"] = []
        return self

    def add_replace_colour(
        self,
        from_colour: tuple[int, int, int] | list[int],
        to_colour: tuple[int, int, int] | list[int],
        *,
        tolerance: float = 5.0,
    ) -> Advanced:
        """Replace pixels near one RGB colour with another colour."""
        if isinstance(tolerance, bool) or not isinstance(tolerance, (int, float)):
            raise TypeError("colour replacement tolerance must be a number")
        tolerance = float(tolerance)
        if not math.isfinite(tolerance) or not 0 <= tolerance <= 100:
            raise ValueError("colour replacement tolerance must be between 0 and 100")
        effect = {
            "ReplaceColour": {
                "from": _rgb(from_colour),
                "to": _rgb(to_colour),
                "tolerance": tolerance,
            }
        }
        self._options.setdefault("pipeline", []).append(effect)
        return self

    add_replace_color = add_replace_colour

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
        source_text: str | None = None,
        figlet_font: str | None = None,
    ) -> Advanced:
        lines = text.splitlines() or [""]
        overlay = {
            "text": text,
            "source_text": source_text,
            "figlet_font": figlet_font,
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

    def remove_overlay(self, index: int) -> Advanced:
        self._options.setdefault("overlays", []).pop(index)
        return self

    def clear_overlays(self) -> Advanced:
        self._options["overlays"] = []
        return self

    def font_info(self) -> FontInfo:
        """Inspect the selected glyph set and its measured cell geometry."""
        font_data = None if self.font is None else _read_bytes(self.font, what="font")
        raw = json.loads(
            _native.font_info_json(
                json.dumps(self._options),
                font_data,
                self.system_fonts,
            )
        )
        font_names = tuple(raw.pop("font_names"))
        return FontInfo(**raw, font_names=font_names)

    def process(
        self,
        image: ImageSource,
        *,
        include_png: bool = True,
        **overrides: Any,
    ) -> ProcessedImage:
        """Apply image effects without converting the result to terminal art."""
        options = self._options_with_overrides(overrides)
        width, height, rgba, png = _native.process_json(
            _read_bytes(image, what="image"), json.dumps(options), include_png
        )
        return ProcessedImage(width, height, bytes(rgba), bytes(png))

    def detect_text(self, image: ImageSource, **overrides: Any) -> tuple[OcrDetection, ...]:
        """Run OCR and return raw detections without rendering terminal art."""
        options = self._options_with_overrides(overrides)
        values = json.loads(
            _native.detect_text_json(_read_bytes(image, what="image"), json.dumps(options))
        )
        return tuple(OcrDetection._from_dict(value) for value in values)

    def _options_with_overrides(self, overrides: Mapping[str, Any]) -> dict[str, Any]:
        options = self.options
        for name, value in overrides.items():
            if name == "mode":
                name = "render"
            if name not in _default_options():
                raise KeyError(f"unknown img2irc option {name!r}")
            options[name] = _normalise_option(name, value)
        return options

    def generate(self, image: ImageSource, **overrides: Any) -> Output:
        options = self._options_with_overrides(overrides)
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
        return _output_from_native(metadata_json, preview_rgba, preview_png, encoded)

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


def defaults() -> dict[str, Any]:
    """Return a copy of all native render defaults."""
    return Advanced.defaults()


def available_options() -> tuple[str, ...]:
    """Return every option name accepted by :class:`Advanced`."""
    return Advanced.available_options()


def glyph_groups() -> dict[str, str]:
    """Return named glyph sets from the native glyph catalog.

    The strings include all configured members, before per-glyph exclusions.
    Pass set names through ``blocks=`` or characters through ``include=``.
    Font coverage is reported separately by :func:`font_info`.
    """
    return json.loads(_native.glyph_groups_json())


def generate(image: ImageSource, **options: Any) -> Output:
    """Generate terminal art with a one-shot functional API."""
    return Advanced(**options).generate(image)


def process(
    image: ImageSource,
    *,
    effects: Iterable[str | tuple[str, Any] | Mapping[str, Any]] = (),
    include_png: bool = True,
    **options: Any,
) -> ProcessedImage:
    """Apply an effect sequence and return RGBA and optional PNG bytes.

    Effects may be names, ``(name, value)`` pairs, or their raw serialized Rust
    mappings. Raw mappings are useful for round-tripping saved configurations.
    """
    renderer = Advanced(**options)
    for effect in effects:
        if isinstance(effect, str):
            renderer.add_effect(effect)
        elif isinstance(effect, Mapping):
            renderer._options.setdefault("pipeline", []).append(copy.deepcopy(dict(effect)))
        else:
            try:
                name, value = effect
            except (TypeError, ValueError) as error:
                raise TypeError(
                    "effects must contain names, (name, value) pairs, or mappings"
                ) from error
            renderer.add_effect(name, value)
    return renderer.process(image, include_png=include_png)


def image_dimensions(image: ImageSource) -> tuple[int, int]:
    """Decode an image and return its ``(width, height)``."""
    return _native.image_dimensions(_read_bytes(image, what="image"))


def validate_font(font: FontSource) -> None:
    """Raise :class:`ValueError` unless *font* is usable and monospace."""
    _native.validate_font(_read_bytes(font, what="font"))


def font_info(
    font: FontSource | None = None,
    *,
    font_names: Iterable[str] | None = None,
    system_fonts: bool = False,
    **options: Any,
) -> FontInfo:
    """Inspect glyph coverage and metrics for a font configuration."""
    return Advanced(
        font=font,
        font_names=font_names,
        system_fonts=system_fonts,
        **options,
    ).font_info()


def detect_text(image: ImageSource, **options: Any) -> tuple[OcrDetection, ...]:
    """Run OCR without generating terminal art.

    This raises :class:`RuntimeError` in a wheel built with ``--no-ocr``.
    """
    return Advanced(**options).detect_text(image)


def nearest_colour(
    colour: RGB | list[int],
    *,
    palette: str = "ansi256",
    preserve_grayscale: bool = False,
    grayscale_tolerance: int = 0,
) -> tuple[int, RGB]:
    """Return the renderer's nearest palette index and its RGB colour."""
    rgb = _rgb(colour)
    if isinstance(grayscale_tolerance, bool) or not isinstance(grayscale_tolerance, int):
        raise TypeError("grayscale_tolerance must be an integer")
    if not 0 <= grayscale_tolerance <= 255:
        raise ValueError("grayscale_tolerance must be between 0 and 255")
    index, matched = _native.nearest_palette_colour(
        tuple(rgb), palette, preserve_grayscale, grayscale_tolerance
    )
    return index, tuple(matched)


def score_contours(
    image: ImageSource,
    *,
    reference: ImageSource | None = None,
    cell_scale: int = 8,
    weights: ContourWeights | Mapping[str, float] | None = None,
) -> ContourScore:
    """Calculate the same contour-quality score used by render optimization."""
    if weights is None:
        weights_dict: Mapping[str, float] = {}
    elif isinstance(weights, ContourWeights):
        weights_dict = asdict(weights)
    elif isinstance(weights, Mapping):
        weights_dict = weights
    else:
        raise TypeError("weights must be ContourWeights, a mapping, or None")
    value = _native.contour_score_json(
        _read_bytes(image, what="image"),
        cell_scale,
        None if reference is None else _read_bytes(reference, what="image"),
        json.dumps(dict(weights_dict)),
    )
    return ContourScore(**json.loads(value))


def contour_cache_stats() -> tuple[int, int]:
    """Return global contour-score cache ``(hits, misses)`` counters."""
    return _native.contour_cache_stats()


def clear_contour_cache() -> None:
    """Clear in-memory contour-score entries (the counters remain cumulative)."""
    _native.clear_contour_cache()
