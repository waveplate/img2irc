import os
import re
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock, patch

from img2irc import Cell
from img2irc.tui import (
    CONTROLS,
    PythonTui,
    SECTIONS,
    _default_output_path,
    _default_preview_path,
    _checklist,
    _cell_sgr,
    _draw,
    _preview_ansi,
)


class TuiTests(unittest.TestCase):
    def test_tui_defaults_and_controls(self):
        app = PythonTui("picture.png", width=72)
        self.assertEqual(app.options["width"], 72)
        self.assertTrue(app.options["ocr_auto_width"])

        app.selected = next(
            i for i, control in enumerate(app.controls) if control.key == "width"
        )
        self.assertTrue(app.change_selected(1))
        self.assertEqual(app.options["width"], 76)

        self.select(app, "invert")
        self.assertTrue(app.change_selected(1))
        self.assertTrue(app.options["invert"])

    def test_output_names(self):
        self.assertEqual(
            _default_output_path("picture.jpg", "Ansi24"), Path("picture.img2irc.ans")
        )
        self.assertEqual(
            _default_output_path("picture.jpg", "Irc"), Path("picture.img2irc.txt")
        )
        self.assertEqual(
            _default_output_path("https://example.com/picture.jpg", "Ansi24"),
            Path("img2irc-output.ans"),
        )

    @staticmethod
    def select(app, key):
        control = next(control for control in CONTROLS if control.key == key)
        app.section_index = SECTIONS.index(control.section)
        app.selected = next(i for i, item in enumerate(app.controls) if item.key == key)

    @staticmethod
    def result():
        return SimpleNamespace(encoded=b"terminal output", preview_png=b"PNG preview")

    def test_default_saves_preserve_source_for_all_input_suffixes(self):
        with tempfile.TemporaryDirectory() as directory:
            for suffix in (".png", ".PNG", ".jpg", ".ans", ".txt", ""):
                with self.subTest(suffix=suffix):
                    source = Path(directory) / ("picture" + suffix)
                    source.write_bytes(b"original image")
                    app = PythonTui(str(source))
                    self.assertNotEqual(app.output_path.resolve(), source.resolve())
                    self.assertNotEqual(app.preview_path.resolve(), source.resolve())
                    app.result = self.result()
                    app.save_output()
                    app.save_preview()
                    self.assertEqual(source.read_bytes(), b"original image")
                    self.assertEqual(app.output_path.read_bytes(), b"terminal output")
                    self.assertEqual(app.preview_path.read_bytes(), b"PNG preview")

    def test_custom_png_output_uses_a_different_preview_name(self):
        output = Path("custom.png")
        self.assertEqual(_default_preview_path(output), Path("custom.preview.png"))
        app = PythonTui("input.jpg", output=output)
        self.assertNotEqual(app.output_path, app.preview_path)

    def test_explicit_source_destinations_are_rejected_without_truncation(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "input.png"
            source.write_bytes(b"original image")
            for target in ("output", "preview"):
                with self.subTest(target=target):
                    app = PythonTui(str(source), **{target: source})
                    app.result = self.result()
                    with self.assertRaisesRegex(ValueError, "source file"):
                        getattr(app, "save_" + target)()
                    self.assertEqual(source.read_bytes(), b"original image")

    def test_url_looking_local_path_is_still_protected(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory) / "file:"
            parent.mkdir()
            source = parent / "input.png"
            source.write_bytes(b"original image")
            app = PythonTui(str(parent) + "//input.png", preview=source)
            app.result = self.result()
            with self.assertRaisesRegex(ValueError, "source file"):
                app.save_preview()
            self.assertEqual(source.read_bytes(), b"original image")

    def test_symlink_and_hardlink_to_source_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "input.png"
            source.write_bytes(b"original image")
            symlink = Path(directory) / "symlink.png"
            symlink.symlink_to(source)
            hardlink = Path(directory) / "hardlink.png"
            os.link(source, hardlink)
            for alias in (symlink, hardlink):
                for target in ("output", "preview"):
                    with self.subTest(alias=alias, target=target):
                        app = PythonTui(str(source), **{target: alias})
                        app.result = self.result()
                        with self.assertRaisesRegex(ValueError, "source file"):
                            getattr(app, "save_" + target)()
                        self.assertEqual(source.read_bytes(), b"original image")

    def test_old_source_remains_protected_after_opening_another_image(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "first.png"
            source.write_bytes(b"original image")
            app = PythonTui(str(source))
            app.result = self.result()
            app.open_image(str(Path(directory) / "second.png"))
            self.assertIsNone(app.result)
            self.assertEqual(app.output_path.name, "second.img2irc.ans")
            self.assertEqual(app.preview_path.name, "second.img2irc.png")
            app.preview_path = source
            app.result = self.result()
            with self.assertRaisesRegex(ValueError, "source file"):
                app.save_preview()
            self.assertEqual(source.read_bytes(), b"original image")

    def test_open_image_keeps_explicit_destinations(self):
        app = PythonTui("first.png", output=Path("out.ans"), preview=Path("out.png"))
        app.open_image("second.png")
        self.assertEqual(app.output_path, Path("out.ans"))
        self.assertEqual(app.preview_path, Path("out.png"))

    def test_colliding_output_paths_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            destination = Path(directory) / "same.png"
            destination.write_bytes(b"existing output")
            app = PythonTui("input.jpg", output=destination, preview=destination)
            app.result = self.result()
            for save in (app.save_output, app.save_preview):
                with self.assertRaisesRegex(ValueError, "different files"):
                    save()
                self.assertEqual(destination.read_bytes(), b"existing output")

    def test_source_identity_is_checked_before_truncating_open_file(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "input.png"
            source.write_bytes(b"original image")
            alias = Path(directory) / "alias.png"
            os.link(source, alias)
            app = PythonTui(str(source), preview=alias)
            app.result = self.result()
            # Simulate a path changing after the initial path-alias check.
            with patch("img2irc.tui._same_file", return_value=False):
                with self.assertRaisesRegex(ValueError, "source file"):
                    app.save_preview()
            self.assertEqual(source.read_bytes(), b"original image")

    def test_source_inode_remains_protected_after_rename(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "input.png"
            source.write_bytes(b"original image")
            app = PythonTui(str(source))
            moved = source.with_name("moved.png")
            source.rename(moved)
            app.preview_path = moved
            app.result = self.result()
            with self.assertRaisesRegex(ValueError, "source file"):
                app.save_preview()
            self.assertEqual(moved.read_bytes(), b"original image")

    def test_selected_font_file_is_protected(self):
        with tempfile.TemporaryDirectory() as directory:
            font = Path(directory) / "font.ttf"
            font.write_bytes(b"original font")
            app = PythonTui("input.png", font=str(font), output=font)
            app.result = self.result()
            with self.assertRaisesRegex(ValueError, "source file"):
                app.save_output()
            self.assertEqual(font.read_bytes(), b"original font")

    def test_repeated_saves_replace_output_without_appending(self):
        with tempfile.TemporaryDirectory() as directory:
            destination = Path(directory) / "output.ans"
            destination.write_bytes(b"longer preexisting terminal output")
            app = PythonTui("input.png", output=destination)
            app.result = self.result()
            app.save_output()
            app.save_output()
            self.assertEqual(destination.read_bytes(), b"terminal output")

    def test_failed_render_does_not_leave_a_savable_stale_result(self):
        app = PythonTui("missing.png")
        app.result = self.result()
        with patch("img2irc.tui.Advanced") as renderer:
            renderer.return_value.generate.side_effect = ValueError("bad image")
            with self.assertRaisesRegex(ValueError, "bad image"):
                app.render()
        self.assertIsNone(app.result)
        with self.assertRaisesRegex(RuntimeError, "nothing"):
            app.save_preview()

    def test_smoothing_includes_all_native_controls(self):
        keys = {control.key for control in CONTROLS if control.section == "Smoothing"}
        self.assertEqual(
            keys,
            {
                "score_fix",
                "score_fix_candidates",
                "score_fix_orderings",
                "score_fix_geometry_first",
                "score_fix_neighborhood_guard",
                "contour_bending_weight",
                "contour_endpoint_weight",
                "contour_junction_weight",
                "contour_fragment_weight",
                "contour_fidelity_weight",
                "contour_boundary_weight",
                "contour_peak_weight",
            },
        )
        app = PythonTui("picture.png")
        self.select(app, "score_fix_candidates")
        self.assertFalse(app.change_selected(1))
        self.select(app, "score_fix")
        self.assertTrue(app.change_selected(1))
        for key in keys - {"score_fix"}:
            self.select(app, key)
            # Bending/junction already start at the top of their sliders.
            direction = (
                -1
                if key
                in {
                    "contour_bending_weight",
                    "contour_junction_weight",
                    "score_fix_neighborhood_guard",
                }
                else 1
            )
            self.assertTrue(app.change_selected(direction), key)
        self.assertEqual(app.options["score_fix_candidates"], 13)
        self.assertEqual(app.options["score_fix_orderings"], 2)
        self.assertTrue(app.options["score_fix_geometry_first"])
        self.assertFalse(app.options["score_fix_neighborhood_guard"])

    def test_glyph_sets_and_individual_characters_change_render_options(self):
        app = PythonTui("picture.png")
        app._glyph_catalog = {"default": " ▀", "blocks.full": "█"}
        app.set_glyph_groups(["blocks.full"])
        self.assertEqual(app.options["blocks"], ["blocks.full"])
        app.set_characters(" ▀█")
        self.assertEqual(app.options["blocks"], [])
        self.assertEqual(app.options["include"], " ▀█")
        app.set_selected_characters(" ▀█", {" ", "█"})
        self.assertEqual(app.options["exclude"], ["▀"])
        app.set_glyph_groups(["default"])
        self.assertIsNone(app.options["include"])
        with self.assertRaisesRegex(ValueError, "at least one"):
            app.set_selected_characters(" ▀█", set())
        with self.assertRaisesRegex(ValueError, "at least one"):
            app.set_glyph_groups([])

    def test_glyph_picker_applies_a_staged_selection_and_can_cancel(self):
        screen = Mock()
        screen.getmaxyx.return_value = (24, 80)
        original = {"first"}
        screen.get_wch.side_effect = ["n", "j", " ", "\n"]
        items = [("first", "first set"), ("second", "second set")]
        self.assertEqual(_checklist(screen, "Glyphs", items, original), {"second"})
        self.assertEqual(original, {"first"})
        screen.get_wch.side_effect = ["n", "q"]
        self.assertIsNone(_checklist(screen, "Glyphs", items, original))
        self.assertEqual(original, {"first"})

    def test_all_control_values_are_valid_native_options(self):
        from img2irc import Advanced

        app = PythonTui("picture.png")
        for control in CONTROLS:
            with self.subTest(control=control.key):
                self.assertIn(control.key, app.options)
                values = (
                    control.choices
                    if control.kind == "choice"
                    else (
                        control.change(app.options[control.key], -1),
                        control.change(app.options[control.key], 1),
                    )
                )
                for value in values:
                    Advanced({control.key: value}).validate()

    def test_real_render_and_save_keep_png_source_unchanged(self):
        from test_api import ApiTests

        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source.png"
            source.write_bytes(ApiTests.PNG)
            app = PythonTui(str(source), width=8, characters=" ▀█")
            characters, selected = app.glyph_characters()
            self.assertIn("▀", selected)
            app.set_selected_characters(characters, {" ", "█"})
            app.options.update(
                score_fix=True,
                score_fix_candidates=3,
                score_fix_orderings=2,
                score_fix_geometry_first=True,
                score_fix_neighborhood_guard=False,
                contour_bending_weight=0.5,
                contour_peak_weight=0.2,
            )
            result = app.render()
            self.assertEqual(result.options["score_fix_candidates"], 3)
            self.assertEqual(result.options["score_fix_orderings"], 2)
            self.assertEqual(result.options["include"], " ▀█")
            self.assertEqual(result.options["exclude"], ["▀"])
            self.assertTrue(result.options["score_fix_geometry_first"])
            self.assertFalse(result.options["score_fix_neighborhood_guard"])
            self.assertEqual(result.options["contour_bending_weight"], 0.5)
            self.assertAlmostEqual(result.options["contour_peak_weight"], 0.2)
            self.assertTrue(
                all(cell.character != "▀" for row in result.cells for cell in row)
            )
            app.save_output()
            app.save_preview()
            self.assertEqual(source.read_bytes(), ApiTests.PNG)
            self.assertTrue(app.preview_path.read_bytes().startswith(b"\x89PNG"))

    @staticmethod
    def cell(foreground, background=(0, 0, 0), character="▀", **styles):
        return Cell(
            character, foreground, background,
            **{**dict(inverted=False, bold=False, italic=False, underline=False),
               **styles},
        )

    @staticmethod
    def decode_preview(output):
        """Small terminal model: decode cursor positions and truecolour SGR."""
        cells = {}
        y = x = 0
        foreground = background = None
        for token in re.findall(r"\x1b\[[0-9;]*[Hm]|\x1b[78]|[^\x1b]", output):
            if token.startswith("\x1b["):
                values = [int(value) for value in token[2:-1].split(";")]
                if token.endswith("H"):
                    y, x = (value - 1 for value in values)
                else:
                    while values:
                        code = values.pop(0)
                        if code == 0:
                            foreground = background = None
                        elif code in (38, 48):
                            assert values.pop(0) == 2
                            rgb = tuple(values[:3])
                            del values[:3]
                            if code == 38:
                                foreground = rgb
                            else:
                                background = rgb
            elif not token.startswith("\x1b"):
                cells[y, x] = (token, foreground, background)
                x += 1
        return cells

    def test_preview_preserves_more_than_256_colour_pairs(self):
        # Python curses color_pair(256) wraps to zero; the old preview failed
        # even when COLOR_PAIRS advertised 65536. Exercise 600 distinct pairs.
        grid = [
            [self.cell((x, y * 8, 125), (y, x, 181)) for x in range(100)]
            for y in range(6)
        ]
        output = _preview_ansi(grid, top=2, left=39, height=6, width=100)
        decoded = self.decode_preview(output)
        for y, row in enumerate(grid):
            for x, cell in enumerate(row):
                self.assertEqual(
                    decoded[y + 2, x + 39],
                    (cell.character, cell.foreground, cell.background),
                )
        self.assertTrue(output.startswith("\x1b7"))
        self.assertTrue(output.endswith("\x1b[0m\x1b8"))

    def test_preview_uses_exact_irc_colours_and_cell_styles(self):
        cell = self.cell(
            (125, 181, 0), (156, 255, 156), bold=True, italic=True,
            underline=True,
        )
        self.assertEqual(
            _cell_sgr(cell),
            "\x1b[0;1;3;4;38;2;125;181;0;48;2;156;255;156m",
        )
        self.assertNotIn(";7;", _cell_sgr(cell))
        self.assertIn(";7;", _cell_sgr(self.cell((1, 2, 3), inverted=True)))
        self.assertNotIn("48;", _cell_sgr(self.cell((1, 2, 3), None)))
        self.assertEqual(_cell_sgr(cell, colour=False), "\x1b[0;1;3;4m")

    def test_preview_crops_pans_and_clears_unused_viewport(self):
        grid = [
            [self.cell((y, x, 0), character=str(x)) for x in range(4)]
            for y in range(3)
        ]
        output = _preview_ansi(
            grid, top=2, left=39, height=4, width=5,
            row_offset=1, column_offset=2,
        )
        decoded = self.decode_preview(output)
        self.assertEqual(len(decoded), 20)
        self.assertEqual(decoded[2, 39], ("2", (1, 2, 0), (0, 0, 0)))
        self.assertEqual(decoded[2, 40][0], "3")
        for y in range(2, 6):
            for x in range(39, 44):
                if y >= 4 or x >= 41:
                    self.assertEqual(decoded[y, x], (" ", None, None))
        self.assertEqual(_preview_ansi(grid, top=0, left=0, height=0, width=5), "")
        self.assertEqual(_preview_ansi(grid, top=0, left=0, height=5, width=0), "")

    def test_preview_sanitizes_control_glyphs(self):
        grid = [[self.cell((1, 2, 3), character="\x1b")]]
        output = _preview_ansi(grid, top=0, left=0, height=1, width=1)
        self.assertEqual(self.decode_preview(output)[0, 0][0], " ")

    def test_draw_writes_rgb_after_curses_refresh_without_colour_pairs(self):
        screen = Mock()
        screen.getmaxyx.return_value = (24, 100)
        app = PythonTui("picture.png")
        app.result = SimpleNamespace(cells=[[self.cell((125, 181, 0))]])
        with patch("img2irc.tui.sys.stdout") as stdout, \
             patch("img2irc.tui.curses.init_pair") as init_pair:
            stdout.write.side_effect = lambda _: self.assertTrue(screen.refresh.called)
            _draw(screen, app, True)
            init_pair.assert_not_called()
            output = stdout.write.call_args.args[0]
            self.assertEqual(self.decode_preview(output)[2, 39][1], (125, 181, 0))
            stdout.flush.assert_called_once()

    def test_draw_without_result_clears_the_previous_rgb_preview(self):
        screen = Mock()
        screen.getmaxyx.return_value = (24, 100)
        with patch("img2irc.tui.sys.stdout") as stdout:
            _draw(screen, PythonTui("picture.png"), True)
            screen.clear.assert_called_once()
            stdout.write.assert_not_called()


if __name__ == "__main__":
    unittest.main()
