import sys

def main():
    with open('src/draw.rs', 'r') as f:
        content = f.read()

    # Find the text_overlay application block
    old_block = """            let mut final_draw_fg = draw_fg.clone();
            let mut final_draw_bg = draw_bg.clone();

            if let Some(ov) = text_overlay {
                if let Some(fg_param) = &ov.fg {
                    final_draw_fg = match fg_param {
                        crate::args::ColorSpec::Index(i) => Colour::Index(*i),
                        crate::args::ColorSpec::Rgb(rgb) => Colour::RGB(*rgb),
                    };
                }
                if let Some(bg_param) = &ov.bg {
                    final_draw_bg = Some(match bg_param {
                        crate::args::ColorSpec::Index(i) => Colour::Index(*i),
                        crate::args::ColorSpec::Rgb(rgb) => Colour::RGB(*rgb),
                    });
                }
                final_char = overlay_char.unwrap();
            }

            row_vec.push(RenderedCell {
                char: final_char,
                fg: draw_fg_raw,
                bg: draw_bg_raw,
                inverted: inv,
            });"""

    new_block = """            let mut final_draw_fg = draw_fg.clone();
            let mut final_draw_bg = draw_bg.clone();
            let mut final_draw_fg_raw = draw_fg_raw;
            let mut final_draw_bg_raw = draw_bg_raw;

            if let Some(ov) = text_overlay {
                if let Some(fg_param) = &ov.fg {
                    final_draw_fg = match fg_param {
                        crate::args::ColorSpec::Index(i) => Colour::Index(*i),
                        crate::args::ColorSpec::Rgb(rgb) => Colour::RGB(*rgb),
                    };
                    final_draw_fg_raw = match fg_param {
                        crate::args::ColorSpec::Index(i) => {
                            let idx = (*i as usize).min(if render == Render::Irc { 98 } else { 255 });
                            if render == Render::Irc {
                                unpack_rgb(IRC99[idx])
                            } else {
                                unpack_rgb(ANSI256[idx])
                            }
                        },
                        crate::args::ColorSpec::Rgb(rgb) => *rgb,
                    };
                }
                if let Some(bg_param) = &ov.bg {
                    final_draw_bg = Some(match bg_param {
                        crate::args::ColorSpec::Index(i) => Colour::Index(*i),
                        crate::args::ColorSpec::Rgb(rgb) => Colour::RGB(*rgb),
                    });
                    final_draw_bg_raw = Some(match bg_param {
                        crate::args::ColorSpec::Index(i) => {
                            let idx = (*i as usize).min(if render == Render::Irc { 98 } else { 255 });
                            if render == Render::Irc {
                                unpack_rgb(IRC99[idx])
                            } else {
                                unpack_rgb(ANSI256[idx])
                            }
                        },
                        crate::args::ColorSpec::Rgb(rgb) => *rgb,
                    });
                }
                final_char = overlay_char.unwrap();
            }

            row_vec.push(RenderedCell {
                char: final_char,
                fg: final_draw_fg_raw,
                bg: final_draw_bg_raw,
                inverted: inv,
            });"""

    content = content.replace(old_block, new_block)

    with open('src/draw.rs', 'w') as f:
        f.write(content)

if __name__ == "__main__":
    main()
