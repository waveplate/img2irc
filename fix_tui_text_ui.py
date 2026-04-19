import sys

def main():
    with open('src/tui.rs', 'r') as f:
        content = f.read()

    # 1. Update render_text_tab to include list and status info
    old_text_tab = """fn render_text_tab(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
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
    .split(inner);"""
    
    new_text_tab = """fn render_text_tab(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
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
        Constraint::Length(18), // Color picker
        Constraint::Length(3),  // Options (Wrap toggle / Delete button)
        Constraint::Length(8),  // Mini list of overlays
        Constraint::Min(0),     // Instructions / Status
    ])
    .split(inner);"""
    content = content.replace(old_text_tab, new_text_tab)

    # 2. Add mini-list and status info logic
    old_inst = """    let inst_text = "Instructions:\\n\\
    - Click image: Select/Create Text\\n\\
    - Drag: Move or Resize Box\\n\\
    - Type: Enter Text\\n\\
    - Escape: Deselect";
    
    let inst_p = Paragraph::new(inst_text).style(Style::default().fg(Color::DarkGray));
    f.render_widget(inst_p, chunks[2]);"""

    new_inst = """    // Mini-list of overlays
    let mut items: Vec<ListItem> = Vec::new();
    for (i, ov) in app.render_args.overlays.iter().enumerate() {
        let is_selected = app.text_selected == Some(i + 1);
        let text_trunc: String = if ov.text.chars().count() > 20 {
            format!("{}…", ov.text.chars().take(20).collect::<String>())
        } else {
            ov.text.clone()
        };
        let label = format!(" #{} {} ({},{}) {}x{}", i+1, text_trunc, ov.x, ov.y, ov.w, ov.h);
        let style = if is_selected { Style::default().fg(ACCENT).add_modifier(Modifier::BOLD) } else { Style::default().fg(TEXT) };
        items.push(ListItem::new(label).style(style));
    }
    let list_block = Block::default().borders(Borders::ALL).title(" Overlays ");
    let list = List::new(items).block(list_block);
    f.render_stateful_widget(list, chunks[2], &mut app.text_list_state);
    // Note: We don't update text_list_area here because we want mouse to work on image

    let mut status_text = String::from("Instructions:\\n\\
    - Click canvas: Create Text\\n\\
    - Left Click: Select FG / Right Click: Select BG\\n\\
    - Drag Box: Move / Drag Corner: Resize\\n\\
    - Type: Enter Text / Esc: Deselect");

    if let Some(sel) = app.text_selected {
        if sel > 0 && sel <= app.render_args.overlays.len() {
            let ov = &app.render_args.overlays[sel - 1];
            status_text = format!("Selected #{} | Pos: {},{} | Size: {}x{}\\n", sel, ov.x, ov.y, ov.w, ov.h) + &status_text;
        }
    }
    
    let inst_p = Paragraph::new(status_text).style(Style::default().fg(Color::DarkGray));
    f.render_widget(inst_p, chunks[3]);"""
    content = content.replace(old_inst, new_inst)

    with open('src/tui.rs', 'w') as f:
        f.write(content)

if __name__ == "__main__":
    main()
