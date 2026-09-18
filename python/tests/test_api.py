import tempfile
import unittest
from pathlib import Path

import img2irc


class ApiTests(unittest.TestCase):
    PNG = bytes.fromhex(
        "89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c489"
        "0000000d49444154789c63f8cfc0f01f00050001ff89993d1d0000000049454e44ae426082"
    )

    def test_defaults_and_effect_builder(self):
        renderer = img2irc.Advanced(width=32, mode="ansi24")
        renderer.add_effect("contrast", 10).add_effect("sharpen")
        self.assertEqual(renderer.options["render"], "Ansi24")
        self.assertEqual(renderer.options["pipeline"], [{"Contrast": 10}, "Sharpen"])

    def test_native_glyph_catalog(self):
        catalog = img2irc.glyph_groups()
        self.assertIn("default", catalog)
        self.assertTrue(all(
            isinstance(name, str) and isinstance(chars, str)
            for name, chars in catalog.items()
        ))
        catalog.clear()
        self.assertIn("default", img2irc.glyph_groups())

    def test_current_pipeline_effect_builders(self):
        renderer = img2irc.Advanced()
        renderer.add_effect("luma contrast", 12).add_effect("median-blur", 2)
        renderer.add_effect("line thickness", -1).add_effect("oil", (3, 0.5))
        renderer.add_replace_colour((255, 0, 0), (0, 0, 255), tolerance=8)
        self.assertEqual(
            renderer.options["pipeline"],
            [
                {"LumaContrast": 12},
                {"MedianBlur": 2},
                {"DarkLines": -1},
                {"Oil": [3, 0.5]},
                {
                    "ReplaceColour": {
                        "from": [255, 0, 0],
                        "to": [0, 0, 255],
                        "tolerance": 8.0,
                    }
                },
            ],
        )

    def test_effect_builder_rejects_invalid_compound_values(self):
        with self.assertRaisesRegex(ValueError, "radius, intensity"):
            img2irc.Advanced().add_effect("oil", 3)
        with self.assertRaisesRegex(ValueError, "between 0 and 100"):
            img2irc.Advanced().add_replace_colour((0, 0, 0), (255, 255, 255), tolerance=101)

    def test_generates_content_and_png(self):
        output = img2irc.Simple(width=2).generate(self.PNG)
        self.assertTrue(output.content)
        self.assertEqual(output.encoded, (output.content + "\n").encode())
        self.assertEqual(output.columns, 2)
        self.assertIsInstance(output.timings, img2irc.Timings)
        self.assertGreaterEqual(output.timings["encode_ms"], 0)
        self.assertGreater(output.cell_width, 0)
        self.assertGreater(output.font_size, 0)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "preview.png"
            output.save_preview(path)
            self.assertTrue(path.read_bytes().startswith(b"\x89PNG"))

    def test_advanced_pipeline_cells_overlay_and_encoding(self):
        renderer = img2irc.Advanced(
            width=2,
            encoding="utf16",
            include_cells=True,
        )
        renderer.add_effect("contrast", 10).add_overlay(
            "X", 0, 0, foreground=(255, 255, 255), bold=True
        )
        output = renderer.generate(self.PNG)
        self.assertIsNotNone(output.cells)
        self.assertIsInstance(output.cells[0][0], img2irc.Cell)
        self.assertEqual(output.cells[0][0]["character"], "X")
        self.assertTrue(output.encoded.startswith((b"\xff\xfe", b"\xfe\xff")))
        self.assertEqual(output.options["pipeline"], [{"Contrast": 10}])

    def test_resolved_options_json_and_management(self):
        renderer = img2irc.Advanced.from_json('{"width": 12, "render": "Ansi24"}')
        self.assertEqual(renderer.resolved_options["width"], 12)
        self.assertIn("ocr_threads", renderer.resolved_options)
        renderer.add_effect("invert").add_effect("contrast", 2)
        renderer.remove_effect(0).clear_effects()
        renderer.add_overlay("x", 0, 0).clear_overlays()
        self.assertEqual(renderer.options["pipeline"], [])
        self.assertEqual(renderer.options["overlays"], [])
        self.assertIs(renderer.validate(), renderer)
        with self.assertRaises(ValueError):
            img2irc.Advanced(font_size=2).validate()

    def test_standalone_image_effect_and_inspection_apis(self):
        self.assertEqual(img2irc.image_dimensions(self.PNG), (1, 1))
        processed = img2irc.process(self.PNG, effects=["invert"])
        self.assertEqual((processed.width, processed.height), (1, 1))
        self.assertEqual(processed.rgba, b"\x00\xff\xff\xff")
        self.assertTrue(processed.png.startswith(b"\x89PNG"))

    def test_palettes_and_contour_scoring(self):
        self.assertEqual(len(img2irc.ANSI256), 256)
        self.assertEqual(len(img2irc.IRC99), 99)
        index, colour = img2irc.nearest_colour((255, 0, 0))
        self.assertEqual(colour, (255, 0, 0))
        self.assertEqual(img2irc.ANSI256[index], colour)
        score = img2irc.score_contours(self.PNG)
        self.assertIsInstance(score, img2irc.ContourScore)
        self.assertGreaterEqual(score.total, 0)

    def test_font_helpers(self):
        font = Path(__file__).parents[2] / "static" / "CascadiaCode-Regular.ttf"
        img2irc.validate_font(font)
        info = img2irc.font_info(font, blocks=[], include=" X")
        self.assertGreater(info.glyph_count, 0)
        self.assertIn("X", info.characters)

    def test_enum_and_functional_api(self):
        output = img2irc.generate(self.PNG, width=2, mode=img2irc.RenderMode.ANSI24)
        self.assertEqual(output.columns, 2)
        self.assertIn("width", img2irc.available_options())
        first = img2irc.defaults()
        first["width"] = 999
        self.assertNotEqual(img2irc.defaults()["width"], 999)

    def test_no_ocr_wheel_reports_unavailable_feature(self):
        if not img2irc.OCR_ENABLED:
            with self.assertRaisesRegex(RuntimeError, "not compiled"):
                img2irc.detect_text(self.PNG)


if __name__ == "__main__":
    unittest.main()
