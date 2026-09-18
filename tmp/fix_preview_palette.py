import sys

def main():
    with open('src/draw.rs', 'r') as f:
        content = f.read()

    # Find the problematic line
    # let is_irc = render == Render::Irc || render == Render::Preview;
    
    old_line = "let is_irc = render == Render::Irc || render == Render::Preview;"
    new_line = "let is_irc = args.render == crate::args::Render::Irc;"
    content = content.replace(old_line, new_line)

    with open('src/draw.rs', 'w') as f:
        f.write(content)

if __name__ == "__main__":
    main()
