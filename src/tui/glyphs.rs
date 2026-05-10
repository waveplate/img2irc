use super::*;

pub(crate) fn render_glyphs_tab(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    let max_tree = area.height.saturating_sub(12).max(5);
    app.glyph_tree_height = app.glyph_tree_height.clamp(5, max_tree);
    let max_list = area.height.saturating_sub(app.glyph_tree_height + 7).max(5);
    app.glyph_list_height = app.glyph_list_height.clamp(5, max_list);

    let chunks = Layout::vertical([
        Constraint::Length(app.glyph_tree_height),
        Constraint::Length(1),
        Constraint::Length(app.glyph_list_height),
        Constraint::Length(1),
        Constraint::Min(5),
    ])
    .split(area);

    let splitter_style = Style::new().fg(BORDER).bg(BG);
    f.render_widget(Block::default().style(splitter_style), chunks[1]);
    f.render_widget(Block::default().style(splitter_style), chunks[3]);
    app.splitter_areas.push(SplitterArea {
        kind: SplitterKind::GlyphTreeRows,
        area: chunks[1],
    });
    app.splitter_areas.push(SplitterArea {
        kind: SplitterKind::GlyphListRows,
        area: chunks[3],
    });

    // Render tree in the first section
    let tree_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(BORDER))
        .border_set(symbols::border::ROUNDED)
        .style(Style::new().bg(SURFACE));
    let tree_area = tree_block.inner(chunks[0]);
    f.render_widget(tree_block, chunks[0]);
    render_tree(f, app, tree_area);

    app.glyph_list_area = chunks[2];
    app.preview_area = chunks[4];

    let is_focused = app.current_tab == 2 && app.selected_control == 1;
    let border_style = if is_focused {
        Style::new().fg(ACCENT)
    } else {
        Style::new().fg(BORDER)
    };
    let glyph_block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(if is_focused {
            " Glyphs ◄► "
        } else {
            " Glyphs "
        });

    if let Some(node) = app.tree_state.get_selected_node() {
        let (config_groups, _) = crate::font::load_blocks_config();

        let target_key = if !node.full_path.is_empty() {
            &node.full_path
        } else {
            &node.name
        };

        if let Some(codes) = config_groups.get(target_key) {
            let items: Vec<ListItem> = codes
                .iter()
                .map(|&code| {
                    let ch = char::from_u32(code).unwrap_or('?');
                    let is_excluded = app.render_args.exclude.contains(&ch);
                    let marker = if is_excluded { "☐ " } else { "☑ " };
                    ListItem::new(format!("{}{} U+{:04X}", marker, ch, code))
                })
                .collect();

            let highlight_style = Style::default()
                .bg(ACCENT)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD);

            let list = List::new(items)
                .block(glyph_block)
                .highlight_style(highlight_style)
                .highlight_symbol("▸ ");
            f.render_stateful_widget(list, chunks[2], &mut app.glyph_list_state);

            // Preview bitmap below
            let idx = app.glyph_list_state.selected().unwrap_or(0);
            if let Some(&code) = codes.get(idx) {
                if let Some(ch) = char::from_u32(code) {
                    render_block_bitmap(f, app, chunks[4], ch);
                }
            }
        } else {
            f.render_widget(
                Paragraph::new("No known glyphs").block(glyph_block),
                chunks[2],
            );
        }
    }
}

pub(crate) fn render_tree(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    app.tree_area = area;
    let buf = f.buffer_mut();
    // Get flattened nodes (this might be expensive to do every frame, but fine for now)
    // 1. Update Scroll
    {
        let nodes = app.tree_state.flatten();
        let _total = nodes.len();
        let height = area.height as usize;
        let selected = app.tree_state.selected_idx;
        let current_scroll = app.tree_state.scroll as usize;

        if selected >= current_scroll + height {
            app.tree_state.scroll = (selected - height + 1) as u16;
        } else if selected < current_scroll {
            app.tree_state.scroll = selected as u16;
        }
    }

    // 2. Render
    let nodes = app.tree_state.flatten();
    let total = nodes.len();
    let start_idx = app.tree_state.scroll as usize;
    let end_idx = (start_idx + area.height as usize).min(total);

    let visible_nodes = nodes
        .iter()
        .skip(start_idx)
        .take(end_idx - start_idx)
        .enumerate();

    for (i, node) in visible_nodes {
        let y = area.y + i as u16;
        if y >= area.y + area.height {
            break;
        }

        // Calculate actual index for selection check
        let actual_idx = start_idx + i;
        let selected = actual_idx == app.tree_state.selected_idx;

        let is_tree_focused = app.current_tab == 2 && app.selected_control == 0;
        let style = if selected && is_tree_focused {
            Style::new()
                .bg(ACCENT)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else if selected {
            Style::new().bg(SURFACE).fg(Color::White)
        } else {
            Style::new().fg(TEXT)
        };

        // Background for the whole line
        for x in area.x..area.x + area.width {
            buf[(x, y)].set_style(style);
        }

        // Indentation
        let indent_len = (node.level as u16) * 2;
        let x = area.x + indent_len;

        // Marker
        let marker = if !node.is_leaf {
            if node.expanded {
                "▼ "
            } else {
                "▶ "
            }
        } else {
            // Check if selected in render_args
            let name = &node.full_path;
            if app.render_args.blocks.contains(name) {
                "☑ "
            } else {
                "☐ "
            }
        };

        // Draw marker
        buf.set_string(x, y, marker, style.fg(ACCENT));

        // Draw name
        // +2 for marker width
        buf.set_string(x + 2, y, &node.name, style);
    }
}

pub(crate) fn render_block_bitmap(f: &mut ratatui::Frame, app: &mut App, area: Rect, ch: char) {
    let mut font_name = if app.font_idx < app.fonts.len() {
        app.fonts[app.font_idx].clone()
    } else {
        "Default".to_string()
    };

    // Use current store to find bitmap and ACTUAL font name
    let mut found_bitmap = None;
    if let Some(g) = &app.arc_glyphs {
        if let Some((_, bitmap, actual_font)) = g.glyphs.iter().find(|(c, _, _)| *c == ch) {
            found_bitmap = Some(bitmap);
            font_name = actual_font.clone();
        }
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Preview: {} ({}) ", ch, font_name));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if let Some(bitmap) = found_bitmap {
        let mut text = ratatui::text::Text::default();
        for row in bitmap {
            let mut line = Line::from(vec![]);
            for &pixel in row {
                if pixel > 0 {
                    line.spans
                        .push(Span::styled("██", Style::new().fg(Color::Cyan)));
                } else {
                    // eye friendly distinct background color
                    line.spans
                        .push(Span::styled("░░", Style::new().fg(Color::Rgb(60, 60, 80))));
                }
            }
            text.lines.push(line);
        }
        let p = Paragraph::new(text);
        f.render_widget(p, inner);
    } else {
        // Not in current store, maybe need to reload or it's just missing
        let p = Paragraph::new("Glyph not in current store.\nPress Space in left panel to reload.")
            .alignment(Alignment::Center);
        f.render_widget(p, inner);
    }
}
