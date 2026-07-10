import sys

def main():
    with open('src/colorpicker.rs', 'r') as f:
        content = f.read()

    # 1. Update set_grid_idx to allow selecting for BG
    # We'll add an optional bool or just use the current editing_fg state
    # Actually, let's just make a generic method that handles both.
    
    # 2. Update handle_mouse_down to accept button
    old_sig = "pub fn handle_mouse_down(&mut self, x: u16, y: u16, area: Rect) -> bool {"
    new_sig = "pub fn handle_mouse_down(&mut self, x: u16, y: u16, area: Rect, is_right: bool) -> bool {"
    content = content.replace(old_sig, new_sig)
    
    # 3. Update the logic in handle_mouse_down
    old_main_area = """        if main_rect.contains(ratatui::layout::Position { x, y }) {
            self.focus = Focus::MainArea;
            self.is_dragging_main = true;
            self.apply_main_area_click(x, y, main_rect);
            return true;
        }"""
        
    new_main_area = """        if main_rect.contains(ratatui::layout::Position { x, y }) {
            self.focus = Focus::MainArea;
            if is_right {
                self.editing_fg = false;
            } else {
                self.editing_fg = true;
            }
            self.is_dragging_main = true;
            self.apply_main_area_click(x, y, main_rect);
            return true;
        }"""
    content = content.replace(old_main_area, new_main_area)

    with open('src/colorpicker.rs', 'w') as f:
        f.write(content)

if __name__ == "__main__":
    main()
