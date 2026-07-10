import sys

def main():
    with open('src/draw.rs', 'r') as f:
        content = f.read()

    helper = """fn get_overlay_at(
    ov: &crate::args::TextOverlay,
    cx: i32,
    cy: i32,
) -> Option<Option<char>> {
    if ov.w > 0 && ov.h > 0 {
        if cx < ov.x || cx >= ov.x + ov.w || cy < ov.y || cy >= ov.y + ov.h {
            return None; // Outside the box
        }
        
        let dx = cx - ov.x;
        let dy = cy - ov.y;
        
        if ov.wrap {
            let idx = (dy * ov.w + dx) as usize;
            if idx < ov.text.chars().count() {
                return Some(Some(ov.text.chars().nth(idx).unwrap()));
            }
        } else {
            if dy == 0 {
                let idx = dx as usize;
                if idx < ov.text.chars().count() {
                    return Some(Some(ov.text.chars().nth(idx).unwrap()));
                }
            }
        }
        return Some(None); // Inside box, but no char
    } else {
        if cy != ov.y || cx < ov.x {
            return None;
        }
        let dx = cx - ov.x;
        let idx = dx as usize;
        if idx < ov.text.chars().count() {
            return Some(Some(ov.text.chars().nth(idx).unwrap()));
        }
        return None;
    }
}
"""
    if "fn get_overlay_at(" not in content:
        content = content.replace("pub fn render_blocks(", helper + "\npub fn render_blocks(")

    # Replace render_blocks logic
    old_blocks = """            let mut text_overlay: Option<&crate::args::TextOverlay> = None;
            let mut overlay_char = None;
            for ov in &args.overlays {
                if y as i32 == ov.y && (x as i32) >= ov.x {
                    let dx = (x as i32 - ov.x) as usize;
                    if let Some(c) = ov.text.chars().nth(dx) {
                        text_overlay = Some(ov);
                        overlay_char = Some(c);
                        break;
                    }
                }
            }"""
            
    new_blocks = """            let mut text_overlay: Option<&crate::args::TextOverlay> = None;
            let mut overlay_char = None;
            let mut in_overlay_box = false;
            for ov in &args.overlays {
                if let Some(opt_c) = get_overlay_at(ov, x as i32, y as i32) {
                    text_overlay = Some(ov);
                    overlay_char = opt_c;
                    in_overlay_box = true;
                    break;
                }
            }"""
    content = content.replace(old_blocks, new_blocks)

    old_blocks_2 = """                final_char = overlay_char.unwrap();"""
    new_blocks_2 = """                if let Some(c) = overlay_char {
                    final_char = c;
                } else {
                    final_char = ' ';
                }"""
    content = content.replace(old_blocks_2, new_blocks_2)
    
    # Replace render_braille logic 1
    old_braille = """            for ov in &args.overlays {
                if (y / 4) as i32 == ov.y && ((x / 2) as i32) >= ov.x {
                    let dx = ((x / 2) as i32 - ov.x) as usize;
                    if let Some(c) = ov.text.chars().nth(dx) {
                        if let Some(fg_param) = &ov.fg {
                            final_fg = match fg_param {
                                crate::args::ColorSpec::Index(i) => Colour::Index(*i),
                                crate::args::ColorSpec::Rgb(rgb) => Colour::RGB(*rgb),
                            };
                        }
                        if let Some(bg_param) = &ov.bg {
                            final_bg = Some(match bg_param {
                                crate::args::ColorSpec::Index(i) => Colour::Index(*i),
                                crate::args::ColorSpec::Rgb(rgb) => Colour::RGB(*rgb),
                            });
                        }
                        final_ch = c;
                        break;
                    }
                }
            }"""
            
    new_braille = """            for ov in &args.overlays {
                if let Some(opt_c) = get_overlay_at(ov, (x / 2) as i32, (y / 4) as i32) {
                    if let Some(fg_param) = &ov.fg {
                        final_fg = match fg_param {
                            crate::args::ColorSpec::Index(i) => Colour::Index(*i),
                            crate::args::ColorSpec::Rgb(rgb) => Colour::RGB(*rgb),
                        };
                    }
                    if let Some(bg_param) = &ov.bg {
                        final_bg = Some(match bg_param {
                            crate::args::ColorSpec::Index(i) => Colour::Index(*i),
                            crate::args::ColorSpec::Rgb(rgb) => Colour::RGB(*rgb),
                        });
                    }
                    if let Some(c) = opt_c {
                        final_ch = c;
                    } else {
                        final_ch = ' ';
                    }
                    break;
                }
            }"""
    content = content.replace(old_braille, new_braille)

    # Replace render_braille logic 2 (alt_out)
    old_braille_alt = """                for ov in &args.overlays {
                    if (y / 4) as i32 == ov.y && ((x / 2) as i32) >= ov.x {
                        let dx = ((x / 2) as i32 - ov.x) as usize;
                        if let Some(c) = ov.text.chars().nth(dx) {
                            if let Some(fg_param) = &ov.fg {
                                alt_fg = match fg_param {
                                    crate::args::ColorSpec::Index(i) => Colour::Index(*i),
                                    crate::args::ColorSpec::Rgb(rgb) => Colour::RGB(*rgb),
                                };
                            }
                            if let Some(bg_param) = &ov.bg {
                                alt_bg = Some(match bg_param {
                                    crate::args::ColorSpec::Index(i) => Colour::Index(*i),
                                    crate::args::ColorSpec::Rgb(rgb) => Colour::RGB(*rgb),
                                });
                            }
                            alt_ch = c;
                            break;
                        }
                    }
                }"""
                
    new_braille_alt = """                for ov in &args.overlays {
                    if let Some(opt_c) = get_overlay_at(ov, (x / 2) as i32, (y / 4) as i32) {
                        if let Some(fg_param) = &ov.fg {
                            alt_fg = match fg_param {
                                crate::args::ColorSpec::Index(i) => Colour::Index(*i),
                                crate::args::ColorSpec::Rgb(rgb) => Colour::RGB(*rgb),
                            };
                        }
                        if let Some(bg_param) = &ov.bg {
                            alt_bg = Some(match bg_param {
                                crate::args::ColorSpec::Index(i) => Colour::Index(*i),
                                crate::args::ColorSpec::Rgb(rgb) => Colour::RGB(*rgb),
                            });
                        }
                        if let Some(c) = opt_c {
                            alt_ch = c;
                        } else {
                            alt_ch = ' ';
                        }
                        break;
                    }
                }"""
    content = content.replace(old_braille_alt, new_braille_alt)

    with open('src/draw.rs', 'w') as f:
        f.write(content)

if __name__ == "__main__":
    main()
