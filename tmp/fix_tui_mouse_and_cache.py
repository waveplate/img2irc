import sys

def main():
    with open('src/tui.rs', 'r') as f:
        content = f.read()

    # 1. Update App struct to cache the rendered grid
    # We'll add base_grid: Option<Vec<Vec<crate::draw::RenderCell>>>
    # Wait, RenderedCell is public.
    app_struct_start = """    text_drag_start: Option<(i32, i32)>,
    text_field_focus: TextFieldFocus,"""
    app_struct_new = """    text_drag_start: Option<(i32, i32)>,
    base_grid: Option<Vec<Vec<crate::draw::RenderedCell>>>,
    text_field_focus: TextFieldFocus,"""
    content = content.replace(app_struct_start, app_struct_new)
    
    app_init_start = """            text_drag_start: None,
            text_field_focus: TextFieldFocus::List,"""
    app_init_new = """            text_drag_start: None,
            base_grid: None,
            text_field_focus: TextFieldFocus::List,"""
    content = content.replace(app_init_start, app_init_new)

    # 2. Update Mouse handling to pass is_right and handle dragging feedback
    old_mouse_down = """                        if let Some(cp_state) = &mut app.color_picker_state {
                            if mouse.kind == crossterm::event::MouseEventKind::Down(MouseButton::Left) {
                                handled = cp_state.handle_mouse_down(mouse.column, mouse.row, cp_rect);
                            }"""
    new_mouse_down = """                        if let Some(cp_state) = &mut app.color_picker_state {
                            if mouse.kind == crossterm::event::MouseEventKind::Down(MouseButton::Left) {
                                handled = cp_state.handle_mouse_down(mouse.column, mouse.row, cp_rect, false);
                            } else if mouse.kind == crossterm::event::MouseEventKind::Down(MouseButton::Right) {
                                handled = cp_state.handle_mouse_down(mouse.column, mouse.row, cp_rect, true);
                            }"""
    content = content.replace(old_mouse_down, new_mouse_down)

    # 3. Optimize rendering loop to use cached grid
    # This is more complex. We need to split render_output or add a way to just apply overlays.
    # For now, let's just fix the "ghost box" and the corruption.
    
    # 4. Add Ghost Box rendering in ui()
    # We'll find where the image is rendered and overlay a ghost box if dragging_mode == Sizing
    ghost_logic = """
    // Render dragging ghost box
    if app.current_tab == 3 && app.text_drag_mode == TextDragMode::Sizing {
        if let Some((sx, sy)) = app.text_drag_start {
            let inner_height = app.output_area.height.saturating_sub(2);
            let line_count = app.cached_render.lines().count() as u16;
            let pad_y = if line_count < inner_height { inner_height.saturating_sub(line_count) / 2 } else { 0 };
            
            // Re-calculate current mouse cell
            // (This is a bit redundant but safe)
            // We'll just use the last overlay's dimensions
            if let Some(ov) = app.render_args.overlays.last() {
                let rx = app.output_area.x + 1 + (ov.x * 2) as u16; // Assuming 2 chars per cell
                let ry = app.output_area.y + 1 + pad_y + ov.y as u16;
                let rw = (ov.w * 2) as u16;
                let rh = ov.h as u16;
                let ghost_rect = Rect::new(rx, ry, rw, rh);
                f.render_widget(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Yellow)), ghost_rect);
            }
        }
    }
"""
    # Find end of ui function
    ui_end = """    app.text_list_area = cp_rect; 
}"""
    # Wait, that's render_text_tab. ui ends with `render_auto_optimize_dialog(f, app, overlay_area);` or something.
    
    with open('src/tui.rs', 'w') as f:
        f.write(content)

if __name__ == "__main__":
    main()
