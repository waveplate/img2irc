import sys

def main():
    with open('src/draw.rs', 'r') as f:
        content = f.read()

    # Find the IRC block in emit_colourized
    import re
    # We want to replace all write!(out, "{}", idx) or write!(out, "{}", i) with {:02} in the IRC block
    # Actually, we can just find and replace the whole block carefully.
    
    old_fg = 'Colour::Index(i) => write!(out, "{}", i).unwrap(),'
    new_fg = 'Colour::Index(i) => write!(out, "{:02}", i).unwrap(),'
    content = content.replace(old_fg, new_fg)
    
    # Do it again for BG
    content = content.replace(old_fg, new_fg) # replace same pattern again if multiple

    with open('src/draw.rs', 'w') as f:
        f.write(content)

if __name__ == "__main__":
    main()
