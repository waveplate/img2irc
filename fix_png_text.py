import sys

def main():
    with open('src/font.rs', 'r') as f:
        content = f.read()

    # Find the place where include_chars is processed
    old_include = """    if let Some(inc) = include_chars {
        for ch in inc.chars() {
            if !blocks_to_exclude.contains(&ch) {
                blocks_to_select.insert(ch);
            }
        }
    }"""
    
    new_include = """    if let Some(inc) = include_chars {
        for ch in inc.chars() {
            if !blocks_to_exclude.contains(&ch) {
                blocks_to_select.insert(ch);
            }
        }
    }
    
    // Always include standard printable ASCII for text overlays in PNG export
    for ch in 32..=126 {
        if let Some(c) = std::char::from_u32(ch) {
            blocks_to_select.insert(c);
        }
    }"""

    content = content.replace(old_include, new_include)

    with open('src/font.rs', 'w') as f:
        f.write(content)

if __name__ == "__main__":
    main()
