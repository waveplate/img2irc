import sys

def main():
    with open('src/tui.rs', 'r') as f:
        content = f.read()

    # The bad mapping appears a few times:
    old_mapping = """match app.render_mode_idx {
                                        0 => crate::colorpicker::PaletteMode::Ansi24,
                                        1 => crate::colorpicker::PaletteMode::Ansi,
                                        2 => crate::colorpicker::PaletteMode::Ansi24,
                                        _ => crate::colorpicker::PaletteMode::Irc,
                                    };"""
    new_mapping = """match app.render_mode_idx {
                                        0 => crate::colorpicker::PaletteMode::Irc,
                                        1 => crate::colorpicker::PaletteMode::Ansi,
                                        2 => crate::colorpicker::PaletteMode::Ansi24,
                                        _ => crate::colorpicker::PaletteMode::Irc,
                                    };"""

    # There might be variations in indentation, so let's use regex or just be careful.
    # It's better to just replace the lines.
    
    # We will do a generic replace
    old_code = """match app.render_mode_idx {
                                            0 => crate::colorpicker::PaletteMode::Ansi24,
                                            1 => crate::colorpicker::PaletteMode::Ansi,
                                            2 => crate::colorpicker::PaletteMode::Ansi24,
                                            _ => crate::colorpicker::PaletteMode::Irc,
                                        };"""
                                        
    # Let's just find and replace the whole block ignoring whitespace if needed, or just write a small parser.
    import re
    content = re.sub(
        r"match app\.render_mode_idx \{\s*0 => crate::colorpicker::PaletteMode::Ansi24,\s*1 => crate::colorpicker::PaletteMode::Ansi,\s*2 => crate::colorpicker::PaletteMode::Ansi24,\s*_ => crate::colorpicker::PaletteMode::Irc,\s*\};",
        "match app.render_mode_idx { 0 => crate::colorpicker::PaletteMode::Irc, 1 => crate::colorpicker::PaletteMode::Ansi, 2 => crate::colorpicker::PaletteMode::Ansi24, _ => crate::colorpicker::PaletteMode::Irc, };",
        content
    )

    with open('src/tui.rs', 'w') as f:
        f.write(content)

if __name__ == "__main__":
    main()
