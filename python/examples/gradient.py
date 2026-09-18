"""Generate ANSI art and a PNG preview without any third-party Python packages."""

from __future__ import annotations

import argparse
from pathlib import Path

from img2irc import Advanced


def gradient_ppm(width: int = 160, height: int = 90) -> bytes:
    """Create an encoded PPM gradient for img2irc's byte-input API."""
    pixels = bytearray()
    for y in range(height):
        for x in range(width):
            pixels.extend(
                (
                    255 * x // (width - 1),
                    255 * y // (height - 1),
                    255 * (width - 1 - x) // (width - 1),
                )
            )
    return f"P6\n{width} {height}\n255\n".encode() + pixels


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path("gradient.ans"))
    parser.add_argument("--preview", type=Path, default=Path("gradient-preview.png"))
    parser.add_argument("--width", type=int, default=64, help="terminal columns")
    args = parser.parse_args()

    renderer = (
        Advanced(width=args.width, render="ansi24")
        .add_effect("luma contrast", 12)
        .add_effect("median blur", 1)
        .add_replace_colour((255, 0, 0), (255, 120, 20), tolerance=6)
        .add_overlay(
            "img2irc Python",
            2,
            1,
            foreground=(255, 255, 255),
            background=(20, 30, 80),
            bold=True,
        )
    )
    result = renderer.generate(gradient_ppm())
    result.write(args.output)
    result.save_preview(args.preview)

    print(result.content)
    print(
        f"\nWrote {result.columns}x{result.rows} cells to {args.output} "
        f"and a {result.preview_width}x{result.preview_height} preview to {args.preview}."
    )


if __name__ == "__main__":
    main()
