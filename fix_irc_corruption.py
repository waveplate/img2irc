import sys

def main():
    with open('src/draw.rs', 'r') as f:
        content = f.read()

    # 1. Always use 2-digit IRC color codes to prevent digit-bleeding
    old_irc_fg = 'Colour::Index(i) => write!(out, "{}", i).unwrap(),'
    new_irc_fg = 'Colour::Index(i) => write!(out, "{:02}", i).unwrap(),'
    content = content.replace(old_irc_fg, new_irc_fg)
    
    old_irc_bg = 'Colour::Index(i) => write!(out, "{}", i).unwrap(),'
    new_irc_bg = 'Colour::Index(i) => write!(out, "{:02}", i).unwrap(),'
    # Be careful with multiple occurrences. 
    # Actually, let's use a more specific replace.
    
    import re
    content = re.sub(r'Colour::Index\(i\) => write!\(out, "{}", i\)\.unwrap\(\),', r'Colour::Index(i) => write!(out, "{:02}", i).unwrap(),', content)

    # 2. Fix the panic in alt_row_cells
    old_alt_char = "alt_final_char = overlay_char.unwrap();"
    new_alt_char = """if let Some(c) = overlay_char {
                        alt_final_char = c;
                    } else {
                        alt_final_char = ' ';
                    }"""
    content = content.replace(old_alt_char, new_alt_char)

    with open('src/draw.rs', 'w') as f:
        f.write(content)

if __name__ == "__main__":
    main()
