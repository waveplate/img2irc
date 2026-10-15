import json
import struct
import tempfile
import unittest
import zlib
from pathlib import Path
from unittest.mock import patch

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
        catalog = img2irc.glyph_sets()
        self.assertIn("default", catalog)
        self.assertIn("smooth", catalog)
        self.assertTrue(catalog["smooth"])
        self.assertTrue(all(
            isinstance(name, str) and isinstance(chars, str)
            for name, chars in catalog.items()
        ))
        catalog.clear()
        self.assertIn("default", img2irc.glyph_groups())

    def test_smoothing_and_scoring_defaults(self):
        for options in (img2irc.defaults(), img2irc.Simple().advanced.resolved_options):
            self.assertEqual(options["smooth_candidates"], 5)
            self.assertEqual(options["smooth_orders"], 5)
            self.assertTrue(options["perceptual_score"])

    def test_cli_setting_names_are_available_and_resolve(self):
        from img2irc import _native

        defaults = img2irc.defaults()
        aliases = json.loads(_native.option_aliases_json())
        components = {"scalex", "scaley", "crop_x1", "crop_y1", "crop_x2", "crop_y2"}
        for name, target in aliases.items():
            with self.subTest(option=name):
                self.assertIn(name, img2irc.available_options())
                if name in components:
                    continue
                resolved = img2irc.Advanced({name: defaults[target]}).resolved_options
                self.assertEqual(resolved[target], defaults[target])

    def test_smooth_settings_match_across_python_apis(self):
        settings = dict(
            smooth=True, smooth_candidates=3,
            smooth_orders=2, smooth_shapes=True, smooth_neighbors=False,
        )
        for renderer in (img2irc.Advanced(**settings), img2irc.Simple(**settings).advanced):
            with self.subTest(renderer=renderer):
                renderer.validate()
                for name, value in settings.items():
                    self.assertEqual(renderer.resolved_options[name], value)
                self.assertEqual(json.loads(renderer.to_json()), renderer.options)
        for orders in (0, 6):
            with self.assertRaisesRegex(ValueError, "smooth_orders"):
                img2irc.Advanced(smooth_orders=orders).validate()

    def test_all_ocr_settings_work_in_simple_and_advanced(self):
        settings = {
            key: value for key, value in img2irc.defaults().items()
            if key == "ocr" or key.startswith("ocr_")
        }
        settings.update(
            ocr=True, ocr_auto_width=True, ocr_model_tier=img2irc.OcrModelTier.SMALL,
            ocr_threads=2, ocr_megapixels=2.5, ocr_min_confidence=0.8,
            ocr_min_ascii_ratio=0.9, ocr_box_threshold=0.3,
            ocr_box_score_threshold=0.7, ocr_unclip_ratio=1.8,
            ocr_figlet=False, ocr_figlet_fonts="2=phm-c64,phm-cga",
        )
        expected = img2irc.Advanced(**settings).resolved_options
        renderer = img2irc.Simple(**settings).advanced
        self.assertEqual(renderer.resolved_options, {**expected, "width": 80, "render": "Ansi24"})
        output = renderer.generate(self.PNG, width=2, ocr=False, ocr_threads=3)
        self.assertEqual(output.options["ocr_threads"], 3)
        self.assertEqual(output.options["ocr_megapixels"], 2.5)
        self.assertEqual(output.options["ocr_min_confidence"], expected["ocr_min_confidence"])
        self.assertEqual(output.options["ocr_figlet_fonts"], expected["ocr_figlet_fonts"])
        self.assertEqual(renderer.options["ocr_threads"], 2)

    def test_ocr_model_paths_accept_path_objects(self):
        paths = (
            "ocr_model_cache", "ocr_det_model", "ocr_cls_model", "ocr_rec_model",
            "ocr_dict", "ocr_doc_orientation_model",
        )
        for name in paths:
            with self.subTest(option=name):
                path = Path("models") / name
                renderer = img2irc.Simple(**{name: path}).advanced
                self.assertEqual(renderer.resolved_options[name], str(path))

    def test_figlet_font_declarations_accept_cli_and_python_forms(self):
        expected = [{"height": 2, "fonts": ["phm-c64", "phm-cga"]}]
        for value in ("2=phm-c64,phm-cga", ["2=phm-c64,phm-cga"],
                      {2: ["phm-c64", "phm-cga"]}, expected):
            with self.subTest(value=value):
                self.assertEqual(
                    img2irc.Simple(ocr_figlet_fonts=value).advanced.resolved_options["ocr_figlet_fonts"],
                    expected,
                )
        for value in ("plain", "0=plain", "2=", [{"height": 2, "fonts": []}],
                      {2.5: ["plain"]}, {True: ["plain"]}):
            with self.subTest(value=value), self.assertRaises(ValueError):
                img2irc.Advanced(ocr_figlet_fonts=value).validate()

    def test_detect_text_forwards_ocr_controls_and_overrides(self):
        values = [{"text": "hello", "confidence": 0.9, "polygon": [[0, 0], [1, 0], [1, 1], [0, 1]]}]
        with patch("img2irc.api._native.detect_text_json", return_value=json.dumps(values)) as detect:
            renderer = img2irc.Simple(ocr_threads=2, ocr_model_tier="small").advanced
            result = renderer.detect_text(self.PNG, ocr_threads=3, ocr_megapixels=2)
            self.assertEqual(result[0].text, "hello")
            image, options_json = detect.call_args.args
            self.assertEqual(image, self.PNG)
            options = json.loads(options_json)
            self.assertEqual(options["ocr_threads"], 3)
            self.assertEqual(options["ocr_model_tier"], "Small")
            self.assertEqual(options["ocr_megapixels"], 2)
            img2irc.detect_text(self.PNG, ocr_threads=4, ocr_figlet=False)
            self.assertEqual(json.loads(detect.call_args.args[1])["ocr_threads"], 4)
            self.assertEqual(renderer.options["ocr_threads"], 2)

    def test_cli_aliases_and_component_overrides(self):
        renderer = img2irc.Advanced(boxblur=True, gaussianblur=2, edgedetection=True,
                                   scalex=2, scaley=3, crop_x1=1, crop_y1=2,
                                   crop_x2=10, crop_y2=20)
        self.assertTrue(renderer.resolved_options["box_blur"])
        self.assertEqual(renderer.resolved_options["gaussian_blur"], 2)
        self.assertTrue(renderer.resolved_options["edge_detection"])
        self.assertEqual(renderer.resolved_options["scale"], [2, 3])
        self.assertEqual(renderer.resolved_options["crop"], [1, 2, 10, 20])
        renderer.set("--ocr-min-confidence", 0.75)
        self.assertEqual(renderer.resolved_options["ocr_min_confidence"], 0.75)
        output = img2irc.generate(self.PNG, width=2, smooth=True, smooth_orders=1)
        self.assertTrue(output.options["smooth"])
        self.assertEqual(output.options["smooth_orders"], 1)

    def test_ocr_megapixel_validation_matches_cli(self):
        for value in (0, 0.09, 16.01, -1):
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, "ocr_megapixels"):
                img2irc.Simple(ocr_megapixels=value).advanced.validate()
        for value in (0.1, 1.5, 16):
            img2irc.Advanced(ocr_megapixels=value).validate()

    def test_output_colour_edits_include_overlays(self):
        renderer = img2irc.Advanced(width=4, height=2, render="ansi24", include_cells=True)
        renderer.add_overlay("▀", 0, 0, foreground=(0, 255, 0))
        renderer.add_replace_colour((0, 255, 0), (12, 34, 56), tolerance=0)
        replaced = renderer.generate(self.PNG)
        self.assertEqual(tuple(replaced.cells[0][0].foreground), (12, 34, 56))
        self.assertIn("38;2;12;34;56", replaced.content)
        renderer.add_effect("quantize_colours", 1)
        reduced = renderer.generate(self.PNG)
        pixels = reduced.preview_rgba
        colours = {tuple(pixels[i:i + 3]) for i in range(0, len(pixels), 4)}
        self.assertEqual(colours, {(255, 0, 0)})
        self.assertEqual(replaced.cells[0][0].character, reduced.cells[0][0].character)
        for count in (0, -1, 257, 1.5, True, None):
            with self.assertRaises(ValueError):
                img2irc.Advanced().add_effect("quantize_colors", count)

    def test_structured_quantization_supports_pixels_and_finished_art(self):
        palette = [(255, 255, 255), (0, 0, 0)]
        renderer = img2irc.Advanced(width=4, height=2, render="ansi24", include_cells=True)
        renderer.add_overlay("▀", 0, 0, foreground=(0, 255, 0))
        renderer.add_quantize_colours(colours=1, threshold=100, palette=palette, source=True)
        self.assertEqual(renderer.resolved_options["pipeline"][0]["QuantizePalette"]["palette"], [list(rgb) for rgb in palette])
        source = renderer.generate(self.PNG)
        self.assertEqual(tuple(source.cells[0][0].foreground), (0, 255, 0))
        pixels = renderer.process(self.PNG).rgba
        self.assertIn(tuple(pixels[:3]), palette)
        renderer.clear_effects().add_effect("quantize_colors", {
            "colours": 1, "threshold": 100, "palette": palette,
        })
        output = renderer.generate(self.PNG)
        self.assertIn(tuple(output.cells[0][0].foreground), palette)
        self.assertEqual(output.cells[0][0].character, source.cells[0][0].character)
        self.assertIn("38;2;", output.content)

    def test_structured_quantization_validates_settings(self):
        for settings in ({"colours": -1}, {"colours": 257}, {"colours": True},
                         {"threshold": -1}, {"threshold": 101}, {"threshold": float("nan")},
                         {"palette": [(256, 0, 0)]}, {"palette": [(0, 0, 0)] * 257}):
            with self.subTest(settings=settings), self.assertRaises((ValueError, TypeError)):
                img2irc.Advanced().add_quantize_colours(**settings)
        img2irc.Advanced().add_quantize_colors(colours=0, threshold=0).validate()
        with self.assertRaises(TypeError):
            img2irc.Advanced().add_quantize_colours(source=1)
        renderer = img2irc.Advanced(pipeline=[{"QuantizePalette": {
            "colours": 257, "threshold": 0, "palette": [], "source": False,
        }}])
        with self.assertRaisesRegex(ValueError, "quantization"):
            renderer.validate()

    def test_glyph_names_and_deprecated_alias_accept_a_single_string(self):
        for key in ("glyphs", "blocks"):
            for value in ("smooth", ["smooth"], ("smooth",)):
                with self.subTest(key=key, value=value):
                    renderer = img2irc.Simple(width=2, **{key: value}).advanced
                    self.assertEqual(renderer.resolved_options["glyphs"], ["smooth"])
                    self.assertTrue(renderer.font_info().selected_count)
                    self.assertTrue(renderer.generate(self.PNG).content)
            renderer = img2irc.Advanced(**{key: "smooth"})
            self.assertEqual(renderer.resolved_options["glyphs"], ["smooth"])
            self.assertEqual(renderer.generate(self.PNG, width=2, **{key: "default"}).options["glyphs"], ["default"])
        self.assertIn("glyphs", img2irc.available_options())
        self.assertIn("blocks", img2irc.available_options())

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

    def test_median_blur_keyword_filters_pixels_and_matches_pipeline(self):
        def chunk(kind, data):
            return (
                struct.pack(">I", len(data)) + kind + data
                + struct.pack(">I", zlib.crc32(kind + data))
            )

        for noisy_pixels in ({12}, {7, 11, 12, 13, 17}):
            with self.subTest(noisy_pixels=noisy_pixels):
                rgba = b"".join(
                    b"\x00\x00\x00\xff" if index in noisy_pixels else b"\xff" * 4
                    for index in range(25)
                )
                image = (
                    b"\x89PNG\r\n\x1a\n"
                    + chunk(b"IHDR", struct.pack(">IIBBBBB", 5, 5, 8, 6, 0, 0, 0))
                    + chunk(b"IDAT", zlib.compress(b"".join(
                        b"\x00" + rgba[row * 20:(row + 1) * 20] for row in range(5)
                    )))
                    + chunk(b"IEND", b"")
                )
                self.assertEqual(img2irc.process(image, median_blur=0).rgba, rgba)
                filtered = img2irc.process(image, median_blur=1)
                expected = bytearray(b"\xff" * 100)
                if len(noisy_pixels) == 5:
                    # One median pass leaves the cross's centre; a second
                    # pass removes it, so this also catches double filtering.
                    expected[48:51] = b"\x00" * 3
                self.assertEqual(filtered.rgba, bytes(expected))
                self.assertEqual(
                    filtered.rgba,
                    img2irc.process(image, effects=[("median_blur", 1)]).rgba,
                )

                settings = dict(width=5, height=5, font_size=8, render="ansi24")
                pipeline = img2irc.Advanced(**settings).add_effect("median_blur", 1).generate(image)
                for output in (
                    img2irc.Advanced({"median_blur": 1}, **settings).generate(image),
                    img2irc.Simple(median_blur=1, **settings).generate(image),
                    img2irc.generate(image, median_blur=1, **settings),
                    img2irc.Advanced(**settings).generate(image, median_blur=1),
                ):
                    self.assertEqual(output.content, pipeline.content)
                    self.assertEqual(output.preview_rgba, pipeline.preview_rgba)
                    self.assertEqual(output.options["median_blur"], 1)

    def test_median_blur_keyword_defaults_and_validation(self):
        self.assertEqual(img2irc.defaults()["median_blur"], 0)
        self.assertIn("median_blur", img2irc.available_options())
        self.assertEqual(img2irc.Advanced(medianblur=2).resolved_options["median_blur"], 2)
        for radius in (-1, 11, 1.5, True):
            with self.subTest(radius=radius), self.assertRaises(ValueError):
                img2irc.Advanced(median_blur=radius).validate()

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
