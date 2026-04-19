import sys

def main():
    with open('src/tui.rs', 'r') as f:
        content = f.read()

    # 1. Update `text_btn_areas` logic to handle the new Text Tab layout buttons
    old_text_tab = """fn render_text_tab(f: &mut ratatui::Frame, app: &mut App, area: Rect) {"""
    
    new_text_tab = """
#[derive(PartialEq, Eq)]
enum TextDragMode {
    None,
    Moving,
    Sizing,
}

fn render_text_tab(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    let main_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .border_set(ratatui::symbols::border::ROUNDED)
        .title(Span::styled(
            " Text Settings ",
            Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(SURFACE));
    let inner = main_block.inner(area);
    f.render_widget(main_block, area);

    let chunks = Layout::vertical([
        Constraint::Length(19), // Color picker
        Constraint::Length(3),  // Options (Wrap toggle / Delete button)
        Constraint::Min(0),     // Instructions
    ])
    .split(inner);

    // Color Picker always visible
    let cp_rect = Rect::new(chunks[0].x, chunks[0].y, chunks[0].width, 18);
    
    // Ensure CP is instantiated with correct mode
    let mode = match app.render_mode_idx { 0 => crate::colorpicker::PaletteMode::Irc, 1 => crate::colorpicker::PaletteMode::Ansi, 2 => crate::colorpicker::PaletteMode::Ansi24, _ => crate::colorpicker::PaletteMode::Irc, };
    if app.color_picker_state.is_none() {
        app.color_picker_state = Some(crate::colorpicker::ColorPickerState::new(mode, true));
    } else {
        app.color_picker_state.as_mut().unwrap().mode = mode;
        app.color_picker_state.as_mut().unwrap().is_open = true; // Force open
    }
    
    // Sync CP with selection
    if let Some(cp_state) = &mut app.color_picker_state {
        if let Some(sel) = app.text_selected {
            if sel > 0 && sel <= app.render_args.overlays.len() {
                let ov = &app.render_args.overlays[sel - 1];
                if cp_state.editing_fg {
                    if let Some(fg) = &ov.fg {
                        match fg {
                            crate::args::ColorSpec::Rgb(rgb) => cp_state.set_rgb(rgb[0], rgb[1], rgb[2]),
                            crate::args::ColorSpec::Index(i) => cp_state.set_grid_idx(*i as usize),
                        }
                    } else {
                        cp_state.fg_idx = None;
                    }
                } else {
                    if let Some(bg) = &ov.bg {
                        match bg {
                            crate::args::ColorSpec::Rgb(rgb) => {
                                cp_state.bg_rgb = Some((rgb[0], rgb[1], rgb[2]));
                                cp_state.set_rgb(rgb[0], rgb[1], rgb[2]);
                            },
                            crate::args::ColorSpec::Index(i) => {
                                cp_state.bg_idx = Some(*i as usize);
                                cp_state.set_grid_idx(*i as usize);
                            }
                        }
                    } else {
                        cp_state.bg_idx = None;
                        cp_state.bg_rgb = None;
                    }
                }
            }
        }
        f.render_widget(crate::colorpicker::ColorPickerWidget { state: cp_state }, cp_rect);
    }
    
    app.text_btn_areas.clear(); // We use index 0 for Wrap, index 1 for Delete
    
    let has_selection = app.text_selected.map_or(false, |s| s > 0 && s <= app.render_args.overlays.len());
    if has_selection {
        let ov = &app.render_args.overlays[app.text_selected.unwrap() - 1];
        let wrap_style = if ov.wrap { Style::default().fg(Color::Green) } else { Style::default().fg(Color::Gray) };
        let options_chunks = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(chunks[1]);
        
        let wrap_btn = Block::default().borders(Borders::ALL).style(wrap_style);
        let wrap_p = Paragraph::new(if ov.wrap { "[x] Wrap" } else { "[ ] Wrap" }).block(wrap_btn).alignment(Alignment::Center);
        f.render_widget(wrap_p, options_chunks[0]);
        app.text_btn_areas.push(options_chunks[0]);
        
        let del_btn = Block::default().borders(Borders::ALL).style(Style::default().fg(Color::Red));
        let del_p = Paragraph::new("Delete").block(del_btn).alignment(Alignment::Center);
        f.render_widget(del_p, options_chunks[1]);
        app.text_btn_areas.push(options_chunks[1]);
    } else {
        app.text_btn_areas.push(Rect::default());
        app.text_btn_areas.push(Rect::default());
    }

    let inst_text = "Instructions:\\n\\
    - Click image: Select/Create Text\\n\\
    - Drag: Move or Resize Box\\n\\
    - Type: Enter Text\\n\\
    - Escape: Deselect";
    
    let inst_p = Paragraph::new(inst_text).style(Style::default().fg(Color::DarkGray));
    f.render_widget(inst_p, chunks[2]);
    
    // Store where the CP is for mouse events later
    app.text_list_area = cp_rect; 
}
"""
    
    start_idx = content.find("fn render_text_tab(f: &mut ratatui::Frame, app: &mut App, area: Rect) {")
    end_idx = content.find("fn render_title_bar", start_idx)
    if end_idx == -1:
        end_idx = content.find("fn main(", start_idx)
    
    if start_idx != -1 and end_idx != -1:
        content = content[:start_idx] + new_text_tab + content[end_idx:]
        
    app_struct_start = """    text_list_area: Rect,
    text_selected: Option<usize>,
    text_field_focus: TextFieldFocus,"""
    app_struct_new = """    text_list_area: Rect,
    text_selected: Option<usize>,
    text_drag_mode: u8, // 0: None, 1: Move, 2: Resize
    text_drag_start: Option<(i32, i32)>,
    text_field_focus: TextFieldFocus,"""
    content = content.replace(app_struct_start, app_struct_new)
    
    app_init_start = """            text_list_area: Rect::default(),
            text_selected: None,
            text_field_focus: TextFieldFocus::List,"""
    app_init_new = """            text_list_area: Rect::default(),
            text_selected: None,
            text_drag_mode: 0,
            text_drag_start: None,
            text_field_focus: TextFieldFocus::List,"""
    content = content.replace(app_init_start, app_init_new)
    
    old_normal_keys = """            if app.input_mode == InputMode::Normal {
                if let Event::Key(key) = event {
                    if key.kind == KeyEventKind::Press {
                        match key.code {"""
    
    new_normal_keys = """            if app.input_mode == InputMode::Normal {
                if let Event::Key(key) = event {
                    if key.kind == KeyEventKind::Press {
                        if app.current_tab == 3 {
                            if let Some(sel) = app.text_selected {
                                if sel > 0 && sel <= app.render_args.overlays.len() {
                                    let mut handled = true;
                                    let ov = &mut app.render_args.overlays[sel - 1];
                                    match key.code {
                                        KeyCode::Backspace => {
                                            ov.text.pop();
                                            app.mark_args_dirty();
                                        }
                                        KeyCode::Char(c) => {
                                            ov.text.push(c);
                                            app.mark_args_dirty();
                                        }
                                        KeyCode::Esc => {
                                            app.text_selected = None;
                                        }
                                        _ => {
                                            handled = false;
                                        }
                                    }
                                    if handled {
                                        continue;
                                    }
                                }
                            }
                        }
                        match key.code {"""
    content = content.replace(old_normal_keys, new_normal_keys)
    
    old_cp_modal = """            // --- Color Picker Modal ---
            if app.color_picker_state.is_some() {
                if let Event::Mouse(mouse) = event {
                    let mut handled = false;
                    let terminal_area = app.terminal_area;
                    let cp_width = 56;
                    let cp_height = 17;
                    let area_x = (terminal_area.width.saturating_sub(cp_width)) / 2;
                    let area_y = (terminal_area.height.saturating_sub(cp_height)) / 2;
                    let cp_rect = Rect::new(area_x, area_y, cp_width, cp_height);

                    if let Some(cp_state) = &mut app.color_picker_state {
                        if mouse.kind == crossterm::event::MouseEventKind::Down(MouseButton::Left) {
                            handled = cp_state.handle_mouse_down(mouse.column, mouse.row, cp_rect);
                        } else if mouse.kind
                            == crossterm::event::MouseEventKind::Drag(MouseButton::Left)
                        {
                            handled = cp_state.handle_mouse_drag(mouse.column, mouse.row, cp_rect);
                        } else if mouse.kind
                            == crossterm::event::MouseEventKind::Up(MouseButton::Left)
                        {
                            cp_state.is_dragging_main = false;
                            cp_state.is_dragging_hue = false;
                            handled = true;
                        }

                        if !cp_state.is_open {
                            if cp_state.ok_trigger {
                                if let Some(sel) = app.text_selected {
                                    if sel > 0 && sel <= app.render_args.overlays.len() {
                                        let ov = &mut app.render_args.overlays[sel - 1];
                                        if let Some(fg_idx) = cp_state.fg_idx {
                                            ov.fg =
                                                Some(crate::args::ColorSpec::Index(fg_idx as u8));
                                        } else {
                                            ov.fg = Some(crate::args::ColorSpec::Rgb([
                                                cp_state.fg_rgb.0,
                                                cp_state.fg_rgb.1,
                                                cp_state.fg_rgb.2,
                                            ]));
                                        }
                                        if let Some(bg_rgb) = cp_state.bg_rgb {
                                            if let Some(bg_idx) = cp_state.bg_idx {
                                                ov.bg = Some(crate::args::ColorSpec::Index(
                                                    bg_idx as u8,
                                                ));
                                            } else {
                                                ov.bg = Some(crate::args::ColorSpec::Rgb([
                                                    bg_rgb.0, bg_rgb.1, bg_rgb.2,
                                                ]));
                                            }
                                        } else {
                                            ov.bg = None;
                                        }
                                        app.mark_args_dirty();
                                    }
                                }
                            }
                            // Important: need to delay setting to None to avoid borrow checker conflicts
                        }
                    }
                    if app
                        .color_picker_state
                        .as_ref()
                        .map(|s| !s.is_open)
                        .unwrap_or(false)
                    {
                        app.color_picker_state = None;
                    }
                    if handled {
                        continue;
                    }
                } else if let Event::Key(key) = event {
                    if key.code == KeyCode::Esc {
                        app.color_picker_state = None;
                    } else if key.code == KeyCode::Enter {
                        if let Some(cp_state) = &mut app.color_picker_state {
                            cp_state.ok_trigger = true;
                            cp_state.is_open = false;
                        }
                        if let Some(cp_state) = &app.color_picker_state {
                            if cp_state.ok_trigger {
                                if let Some(sel) = app.text_selected {
                                    if sel > 0 && sel <= app.render_args.overlays.len() {
                                        let ov = &mut app.render_args.overlays[sel - 1];
                                        if let Some(fg_idx) = cp_state.fg_idx {
                                            ov.fg =
                                                Some(crate::args::ColorSpec::Index(fg_idx as u8));
                                        } else {
                                            ov.fg = Some(crate::args::ColorSpec::Rgb([
                                                cp_state.fg_rgb.0,
                                                cp_state.fg_rgb.1,
                                                cp_state.fg_rgb.2,
                                            ]));
                                        }
                                        if let Some(bg_rgb) = cp_state.bg_rgb {
                                            if let Some(bg_idx) = cp_state.bg_idx {
                                                ov.bg = Some(crate::args::ColorSpec::Index(
                                                    bg_idx as u8,
                                                ));
                                            } else {
                                                ov.bg = Some(crate::args::ColorSpec::Rgb([
                                                    bg_rgb.0, bg_rgb.1, bg_rgb.2,
                                                ]));
                                            }
                                        } else {
                                            ov.bg = None;
                                        }
                                        app.mark_args_dirty();
                                    }
                                }
                            }
                        }
                        app.color_picker_state = None;
                    }
                    continue;
                }
            }"""
    content = content.replace(old_cp_modal, "")
    
    old_modal_render = """    if let Some(cp_state) = &mut app.color_picker_state {
        if cp_state.is_open {
            let cp_width = 56;
            let cp_height = 17;
            let area_x = (app.terminal_area.width.saturating_sub(cp_width)) / 2;
            let area_y = (app.terminal_area.height.saturating_sub(cp_height)) / 2;
            let cp_rect = Rect::new(area_x, area_y, cp_width, cp_height);
            f.render_widget(Clear, cp_rect);
            f.render_widget(crate::colorpicker::ColorPickerWidget { state: cp_state }, cp_rect);
        }
    }"""
    content = content.replace(old_modal_render, "")

    mouse_start = """            match event {
                Event::Mouse(mouse) => {
                    match mouse.kind {"""
    
    new_mouse = """            match event {
                Event::Mouse(mouse) => {
                    if app.current_tab == 3 {
                        let cp_rect = app.text_list_area; // We saved the CP rect here
                        let mut handled = false;
                        
                        if let Some(cp_state) = &mut app.color_picker_state {
                            if mouse.kind == crossterm::event::MouseEventKind::Down(MouseButton::Left) {
                                handled = cp_state.handle_mouse_down(mouse.column, mouse.row, cp_rect);
                            } else if mouse.kind == crossterm::event::MouseEventKind::Drag(MouseButton::Left) {
                                handled = cp_state.handle_mouse_drag(mouse.column, mouse.row, cp_rect);
                            } else if mouse.kind == crossterm::event::MouseEventKind::Up(MouseButton::Left) {
                                cp_state.is_dragging_main = false;
                                cp_state.is_dragging_hue = false;
                                // Automatically apply changes
                                handled = true;
                            }
                            
                            if handled {
                                if let Some(sel) = app.text_selected {
                                    if sel > 0 && sel <= app.render_args.overlays.len() {
                                        let ov = &mut app.render_args.overlays[sel - 1];
                                        if cp_state.editing_fg {
                                            if let Some(fg_idx) = cp_state.fg_idx {
                                                ov.fg = Some(crate::args::ColorSpec::Index(fg_idx as u8));
                                            } else {
                                                if let Some(rgb) = cp_state.fg_rgb {
                                                    ov.fg = Some(crate::args::ColorSpec::Rgb([rgb.0, rgb.1, rgb.2]));
                                                }
                                            }
                                        } else {
                                            if let Some(bg_idx) = cp_state.bg_idx {
                                                ov.bg = Some(crate::args::ColorSpec::Index(bg_idx as u8));
                                            } else {
                                                if let Some(rgb) = cp_state.bg_rgb {
                                                    ov.bg = Some(crate::args::ColorSpec::Rgb([rgb.0, rgb.1, rgb.2]));
                                                }
                                            }
                                        }
                                        app.mark_args_dirty();
                                    }
                                }
                                continue;
                            }
                        }
                    }
                    
                    match mouse.kind {"""
    content = content.replace(mouse_start, new_mouse)
    
    old_output_click = """                                                if app.current_tab == 3 {
                                                    let cell_x = content_x as i32;
                                                    let cell_y = content_y as i32;

                                                    let mut found = None;
                                                    for (idx, ov) in
                                                        app.render_args.overlays.iter().enumerate()
                                                    {
                                                        if ov.y == cell_y
                                                            && cell_x >= ov.x
                                                            && cell_x
                                                                < ov.x
                                                                    + ov.text.chars().count() as i32
                                                        {
                                                            found = Some(idx);
                                                            break;
                                                        }
                                                    }

                                                    if let Some(idx) = found {
                                                        app.text_selected = Some(idx + 1);
                                                        app.text_list_state.select(Some(idx + 1));
                                                    } else {
                                                        app.render_args.overlays.push(
                                                            crate::args::TextOverlay {
                                                                text: "Text".to_string(),
                                                                x: cell_x,
                                                                y: cell_y,
                                                                w: 0,
                                                                h: 0,
                                                                fg: None,
                                                                bg: None,
                                                                wrap: false,
                                                            },
                                                        );
                                                        app.text_selected =
                                                            Some(app.render_args.overlays.len());
                                                        app.text_list_state
                                                            .select(app.text_selected);
                                                        app.mark_args_dirty();
                                                    }
                                                }"""
    
    new_output_click = """                                                if app.current_tab == 3 {
                                                    let cell_x = content_x as i32;
                                                    let cell_y = content_y as i32;

                                                    let mut found = None;
                                                    for (idx, ov) in app.render_args.overlays.iter().enumerate().rev() {
                                                        if (ov.w > 0 && ov.h > 0 && cell_x >= ov.x && cell_x < ov.x + ov.w && cell_y >= ov.y && cell_y < ov.y + ov.h)
                                                        || ((ov.w == 0 || ov.h == 0) && ov.y == cell_y && cell_x >= ov.x && cell_x < ov.x + ov.text.chars().count() as i32) {
                                                            found = Some(idx);
                                                            break;
                                                        }
                                                    }

                                                    if let Some(idx) = found {
                                                        app.text_selected = Some(idx + 1);
                                                        app.text_drag_mode = 1; // Move
                                                        app.text_drag_start = Some((cell_x - app.render_args.overlays[idx].x, cell_y - app.render_args.overlays[idx].y));
                                                    } else {
                                                        app.render_args.overlays.push(crate::args::TextOverlay {
                                                            text: "".to_string(),
                                                            x: cell_x,
                                                            y: cell_y,
                                                            w: 0,
                                                            h: 0,
                                                            fg: None,
                                                            bg: None,
                                                            wrap: false,
                                                        });
                                                        app.text_selected = Some(app.render_args.overlays.len());
                                                        app.text_drag_mode = 2; // Resize
                                                        app.text_drag_start = Some((cell_x, cell_y));
                                                        app.mark_args_dirty();
                                                    }
                                                }"""
    content = content.replace(old_output_click, new_output_click)
    
    old_drag = """                        MouseEventKind::Drag(MouseButton::Left) => {
                            if let Some(splitter) = app.dragging_splitter {"""
    
    new_drag = """                        MouseEventKind::Drag(MouseButton::Left) => {
                            if app.current_tab == 3 && app.text_drag_mode != 0 {
                                if let Some(res) = &app.cached_result {
                                    let inner_height = app.output_area.height.saturating_sub(2);
                                    let line_count = app.cached_render.lines().count() as u16;
                                    let pad_y = if line_count < inner_height { inner_height.saturating_sub(line_count) / 2 } else { 0 };
                                    if mouse.row >= app.output_area.y + 1 + pad_y {
                                        let c_y = mouse.row - (app.output_area.y + 1 + pad_y);
                                        let c_x = mouse.column.saturating_sub(app.output_area.x + 1);
                                        if let Some(sel) = app.text_selected {
                                            if sel > 0 && sel <= app.render_args.overlays.len() {
                                                let cell_x = if app.render_args.render == crate::args::Render::Irc || app.render_args.render == crate::args::Render::Preview { c_x / 2 } else { c_x } as i32;
                                                let cell_y = c_y as i32;
                                                
                                                let ov = &mut app.render_args.overlays[sel - 1];
                                                if app.text_drag_mode == 1 {
                                                    // Move
                                                    if let Some((sx, sy)) = app.text_drag_start {
                                                        ov.x = cell_x - sx;
                                                        ov.y = cell_y - sy;
                                                    }
                                                } else if app.text_drag_mode == 2 {
                                                    // Resize
                                                    if let Some((sx, sy)) = app.text_drag_start {
                                                        ov.w = (cell_x - sx).max(0);
                                                        ov.h = (cell_y - sy).max(0);
                                                    }
                                                }
                                                app.mark_args_dirty();
                                            }
                                        }
                                    }
                                }
                            } else if let Some(splitter) = app.dragging_splitter {"""
    content = content.replace(old_drag, new_drag)
    
    old_up = """                        MouseEventKind::Up(MouseButton::Left) => {
                            app.dragging_splitter = None;
                        }"""
    
    new_up = """                        MouseEventKind::Up(MouseButton::Left) => {
                            app.dragging_splitter = None;
                            app.text_drag_mode = 0;
                            app.text_drag_start = None;
                        }"""
    content = content.replace(old_up, new_up)
    
    old_btn_handling = """                            let pos = ratatui::layout::Position {
                                x: mouse.column,
                                y: mouse.row,
                            };
                            // Check text overlay list
                            if app.text_list_area.contains(pos) && app.current_tab == 3 {"""
                            
    new_btn_handling = """                            let pos = ratatui::layout::Position {
                                x: mouse.column,
                                y: mouse.row,
                            };
                            
                            // Check Text Tab options buttons (Wrap, Delete)
                            if app.current_tab == 3 && app.text_btn_areas.len() >= 2 {
                                if app.text_btn_areas[0].contains(pos) {
                                    if let Some(sel) = app.text_selected {
                                        if sel > 0 && sel <= app.render_args.overlays.len() {
                                            app.render_args.overlays[sel - 1].wrap = !app.render_args.overlays[sel - 1].wrap;
                                            app.mark_args_dirty();
                                        }
                                    }
                                    continue;
                                } else if app.text_btn_areas[1].contains(pos) {
                                    if let Some(sel) = app.text_selected {
                                        if sel > 0 && sel <= app.render_args.overlays.len() {
                                            app.render_args.overlays.remove(sel - 1);
                                            app.text_selected = None;
                                            app.mark_args_dirty();
                                        }
                                    }
                                    continue;
                                }
                            }
                            
                            // Check text overlay list
                            if app.text_list_area.contains(pos) && app.current_tab == 3 {"""
    content = content.replace(old_btn_handling, new_btn_handling)

    # I'll strip the old button click logic out with regex because it's messy.
    import re
    # We want to remove the block starting from `if app.text_btn_areas.len() >= 3 {` to `} else if app.get_current_control_id() == ControlId::Eyedropper {`
    # We can match it if we know the exact structure
    pattern = r"if app\.text_btn_areas\.len\(\) >= 3 \{.*?\}(?=\s*else if app\.get_current_control_id\(\) == ControlId::Eyedropper \{)"
    content = re.sub(pattern, "", content, flags=re.DOTALL)
    
    # Also strip out the `if app.text_field_areas.len() >= 3` logic
    pattern2 = r"if app\.text_field_areas\.len\(\) >= 3 \{.*?\}(?=\s*else if app\.text_btn_areas\.len\(\) >= 3)"
    content = re.sub(pattern2, "", content, flags=re.DOTALL)

    with open('src/tui.rs', 'w') as f:
        f.write(content)

if __name__ == "__main__":
    main()
