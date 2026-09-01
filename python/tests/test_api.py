import tempfile
import unittest
from pathlib import Path

import img2irc


class ApiTests(unittest.TestCase):
    def test_defaults_and_effect_builder(self):
        renderer = img2irc.Advanced(width=32, mode="ansi24")
        renderer.add_effect("contrast", 10).add_effect("sharpen")
        self.assertEqual(renderer.options["render"], "Ansi24")
        self.assertEqual(renderer.options["pipeline"], [{"Contrast": 10}, "Sharpen"])

    def test_generates_content_and_png(self):
        # A dependency-free 1x1 opaque red PNG.
        png = bytes.fromhex(
            "89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c489"
            "0000000d49444154789c63f8cfc0f01f00050001ff89993d1d0000000049454e44ae426082"
        )
        output = img2irc.Simple(width=2).generate(png)
        self.assertTrue(output.content)
        self.assertEqual(output.encoded, (output.content + "\n").encode())
        self.assertEqual(output.columns, 2)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "preview.png"
            output.save_preview(path)
            self.assertTrue(path.read_bytes().startswith(b"\x89PNG"))

    def test_advanced_pipeline_cells_overlay_and_encoding(self):
        png = bytes.fromhex(
            "89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c489"
            "0000000d49444154789c63f8cfc0f01f00050001ff89993d1d0000000049454e44ae426082"
        )
        renderer = img2irc.Advanced(
            width=2,
            encoding="utf16",
            include_cells=True,
        )
        renderer.add_effect("contrast", 10).add_overlay(
            "X", 0, 0, foreground=(255, 255, 255), bold=True
        )
        output = renderer.generate(png)
        self.assertIsNotNone(output.cells)
        self.assertTrue(output.encoded.startswith((b"\xff\xfe", b"\xfe\xff")))
        self.assertEqual(output.options["pipeline"], [{"Contrast": 10}])


if __name__ == "__main__":
    unittest.main()
