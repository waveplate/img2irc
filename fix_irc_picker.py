import sys

def main():
    with open('src/colorpicker.rs', 'r') as f:
        content = f.read()

    # Find the old Irc apply_main_area_click logic
    old_click = """        if self.mode == PaletteMode::Irc {
            if dx < 20 && dy < 5 {
                let col = dx / 2;
                // Since we can't detect sub-cell, we'll just pick top half
                // (or bottom half if we want to add a toggle, but default to top)
                // Actually, if dx % 2 == 0 it's top, dx % 2 == 1 it's bottom? 
                // That's a neat trick! Left char -> Top color, Right char -> Bottom color.
                let is_bottom = (dx % 2) == 1;
                let row = dy * 2 + if is_bottom { 1 } else { 0 };
                let idx = row * 10 + col;
                if idx < 99 {
                    self.set_grid_idx(idx as usize);
                }
            }"""

    new_click = """        if self.mode == PaletteMode::Irc {
            // IRC layout: 12 columns x 7 rows, starting at index 16. Full block (2 chars wide).
            if dx < 24 && dy < 7 {
                let col = dx / 2;
                let row = dy;
                let idx = 16 + row * 12 + col;
                if idx < 99 {
                    self.set_grid_idx(idx as usize);
                }
            }"""

    content = content.replace(old_click, new_click)

    # Find the old Irc render logic
    old_render = """        if self.state.mode == PaletteMode::Irc {
            for row in 0..5 {
                for col in 0..10 {
                    let top_idx = row * 2 * 10 + col;
                    let bot_idx = (row * 2 + 1) * 10 + col;

                    let mut fg = Color::Reset;
                    let mut bg = Color::Reset;

                    if top_idx < 99 {
                        let (r, g, b) = self.state.rgb_from_idx(top_idx as usize);
                        fg = Color::Rgb(r, g, b);
                    }
                    if bot_idx < 99 {
                        let (r, g, b) = self.state.rgb_from_idx(bot_idx as usize);
                        bg = Color::Rgb(r, g, b);
                    }

                    let x = main_rect.x + (col * 2);
                    let y = main_rect.y + row;
                    
                    buf.get_mut(x, y).set_char('▀').set_fg(fg).set_bg(bg);
                    buf.get_mut(x + 1, y).set_char('▀').set_fg(fg).set_bg(bg);

                    if top_idx as usize == self.state.grid_idx {
                        buf.get_mut(x, y).set_char('▛').set_fg(Color::White).set_bg(fg);
                    }
                    if bot_idx as usize == self.state.grid_idx {
                        buf.get_mut(x + 1, y).set_char('▙').set_fg(bg).set_bg(Color::White);
                    }
                }
            }"""

    new_render = """        if self.state.mode == PaletteMode::Irc {
            for row in 0..7 {
                for col in 0..12 {
                    let idx = 16 + row * 12 + col;
                    if idx >= 99 {
                        continue;
                    }
                    let (r, g, b) = self.state.rgb_from_idx(idx as usize);
                    let fg = Color::Rgb(r, g, b);

                    let x = main_rect.x + (col * 2);
                    let y = main_rect.y + row;

                    buf.get_mut(x, y).set_char('█').set_fg(fg);
                    buf.get_mut(x + 1, y).set_char('█').set_fg(fg);

                    if idx as usize == self.state.grid_idx {
                        let contrast = if (r as u32 + g as u32 + b as u32) > 380 { Color::Black } else { Color::White };
                        buf.get_mut(x, y).set_char('X').set_fg(contrast).set_bg(fg);
                        buf.get_mut(x + 1, y).set_char('X').set_fg(contrast).set_bg(fg);
                    }
                }
            }"""

    content = content.replace(old_render, new_render)

    with open('src/colorpicker.rs', 'w') as f:
        f.write(content)

if __name__ == "__main__":
    main()
