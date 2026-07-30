use super::*;

pub(crate) fn render_glyphs_tab(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    app.glyph_select_all_area = Rect::default();
    app.glyph_select_none_area = Rect::default();
    app.glyph_add_area = Rect::default();
    app.glyph_delete_area = Rect::default();

    let max_tree = area.height.saturating_sub(14).max(5);
    app.glyph_tree_height = app.glyph_tree_height.clamp(5, max_tree);
    let max_list = area.height.saturating_sub(app.glyph_tree_height + 7).max(7);
    app.glyph_list_height = app.glyph_list_height.clamp(7, max_list);

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
    let tree_focused = app.current_tab == 2 && app.selected_control == 0;
    let tree_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(if tree_focused { ACCENT } else { BORDER }))
        .border_set(symbols::border::ROUNDED)
        .title(if tree_focused {
            " Sets · ↑↓ navigate · ←→ fold · Space toggle "
        } else {
            " Sets "
        })
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
            " Glyphs · ↑↓ navigate · Space toggle "
        } else {
            " Glyphs "
        });

    if let Some((target_key, is_leaf)) = app.tree_state.get_selected_node().map(|node| {
        (
            if node.full_path.is_empty() {
                node.name.clone()
            } else {
                node.full_path.clone()
            },
            node.is_leaf,
        )
    }) {
        let (config_groups, _) = crate::font::load_blocks_config_unfiltered();
        let (enabled_groups, _) = crate::font::load_blocks_config();

        if app.last_glyph_list_key.as_deref() != Some(target_key.as_str()) {
            app.glyph_list_state = ListState::default();
            app.glyph_list_state.select(Some(0));
            app.last_glyph_list_key = Some(target_key.clone());
        }

        let selected_codes = if is_leaf {
            config_groups.get(&target_key)
        } else {
            None
        };
        if let Some(codes) = selected_codes {
            let glyph_chunks = Layout::vertical([
                Constraint::Min(0),
                Constraint::Length(3), // Selection buttons
                Constraint::Length(3), // Add/delete range buttons
            ])
            .split(chunks[2]);

            app.glyph_list_area = glyph_block.inner(glyph_chunks[0]);

            let items: Vec<ListItem> = codes
                .iter()
                .map(|&code| {
                    let is_saved_excluded = enabled_groups
                        .get(&target_key)
                        .is_none_or(|enabled| !enabled.contains(&code));
                    let marker = if is_saved_excluded { "☐ " } else { "🗹 " };
                    let ch = char::from_u32(code).unwrap_or('?');
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
            f.render_stateful_widget(list, glyph_chunks[0], &mut app.glyph_list_state);

            render_glyph_action_rows(f, app, glyph_chunks[1], glyph_chunks[2], true);

            // Preview bitmap below
            let idx = app.glyph_list_state.selected().unwrap_or(0);
            if let Some(&code) = codes.get(idx) {
                if let Some(ch) = char::from_u32(code) {
                    render_block_bitmap(f, app, chunks[4], ch);
                }
            }
        } else {
            app.glyph_list_state.select(None);
            let glyph_chunks = Layout::vertical([
                Constraint::Min(0),
                Constraint::Length(3),
                Constraint::Length(3),
            ])
            .split(chunks[2]);
            app.glyph_list_area = glyph_block.inner(glyph_chunks[0]);
            f.render_widget(
                Paragraph::new(if is_leaf {
                    "No known glyphs"
                } else {
                    "Folder: use ○ to enable its sets, or choose a nested range"
                })
                .block(glyph_block),
                glyph_chunks[0],
            );
            render_glyph_action_rows(f, app, glyph_chunks[1], glyph_chunks[2], is_leaf);
        }
    } else {
        let glyph_chunks = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(3),
            Constraint::Length(3),
        ])
        .split(chunks[2]);
        app.glyph_list_state.select(None);
        app.glyph_list_area = glyph_block.inner(glyph_chunks[0]);
        f.render_widget(
            Paragraph::new("No glyph ranges yet. Add one to get started.").block(glyph_block),
            glyph_chunks[0],
        );
        render_glyph_action_rows(f, app, glyph_chunks[1], glyph_chunks[2], false);
    }
}

fn render_glyph_action_rows(
    f: &mut ratatui::Frame,
    app: &mut App,
    select_area: Rect,
    action_area: Rect,
    is_leaf: bool,
) {
    let select_chunks =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(select_area);
    let action_chunks =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(action_area);

    app.glyph_select_all_area = select_chunks[0];
    app.glyph_select_none_area = select_chunks[1];
    app.glyph_add_area = action_chunks[0];
    app.glyph_delete_area = action_chunks[1];

    let button_style = |focused| {
        if focused {
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(TEXT)
        }
    };
    let select_all_style = button_style(app.selected_control == 2);
    let select_none_style = button_style(app.selected_control == 3);
    let add_style = button_style(app.selected_control == 4);
    let delete_style = if !is_leaf {
        Style::default().fg(TEXT_DIM)
    } else if app.selected_control == 5 {
        Style::default()
            .fg(Color::LightRed)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Red)
    };

    let button = |label, style| {
        Paragraph::new(label)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_set(ratatui::symbols::border::ROUNDED)
                    .border_style(style),
            )
            .alignment(Alignment::Center)
            .style(style)
    };
    f.render_widget(
        button(
            if is_leaf {
                " Select All "
            } else {
                " Enable Sets "
            },
            select_all_style,
        ),
        select_chunks[0],
    );
    f.render_widget(
        button(
            if is_leaf {
                " Select None "
            } else {
                " Disable Sets "
            },
            select_none_style,
        ),
        select_chunks[1],
    );
    f.render_widget(button(" Add Range ", add_style), action_chunks[0]);
    f.render_widget(
        button(
            if is_leaf {
                " Delete Range "
            } else {
                " Select a range "
            },
            delete_style,
        ),
        action_chunks[1],
    );
}

pub(crate) fn render_tree(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    app.tree_area = area;
    let (all_groups, _) = crate::font::load_blocks_config_unfiltered();
    let (enabled_groups, _) = crate::font::load_blocks_config();
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
        let indent_len = (node.level as u16)
            .saturating_mul(2)
            .min(area.width.saturating_sub(4));
        let x = area.x + indent_len;

        let mut leaves = Vec::new();
        App::collect_leaves(node, &mut leaves);
        let selected_count = leaves
            .iter()
            .filter(|leaf| app.render_args.blocks.contains(*leaf))
            .count();
        let selection_marker = if selected_count == 0 {
            '○'
        } else if selected_count == leaves.len() {
            '●'
        } else {
            '◐'
        };
        let expansion_marker = if node.children.is_empty() {
            ' '
        } else if node.expanded {
            '▼'
        } else {
            '▶'
        };
        let marker = format!("{expansion_marker} {selection_marker} ");
        buf.set_string(x, y, marker, style.fg(ACCENT));

        let total_glyphs = leaves
            .iter()
            .filter_map(|leaf| all_groups.get(leaf))
            .map(Vec::len)
            .sum::<usize>();
        let enabled_glyphs = leaves
            .iter()
            .filter_map(|leaf| enabled_groups.get(leaf))
            .map(Vec::len)
            .sum::<usize>();
        let excluded = total_glyphs.saturating_sub(enabled_glyphs);
        let count_text = if excluded > 0 {
            format!("{}  −{}", total_glyphs, excluded)
        } else {
            total_glyphs.to_string()
        };
        let count_width = count_text.chars().count() as u16;
        let count_x = area
            .x
            .saturating_add(area.width)
            .saturating_sub(count_width);
        let name_x = x + 4;
        let max_name_width = count_x.saturating_sub(name_x).saturating_sub(1) as usize;
        let display_name = node.name.chars().take(max_name_width).collect::<String>();

        buf.set_string(name_x, y, display_name, style);
        if count_x > name_x {
            buf.set_string(
                count_x,
                y,
                count_text,
                style.fg(if excluded > 0 {
                    Color::Yellow
                } else {
                    TEXT_DIM
                }),
            );
        }
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
        let lines = sextant_preview_lines(bitmap, inner.width as usize, inner.height as usize);
        if lines.is_empty() {
            return;
        }
        let preview_height = lines.len().min(inner.height as usize) as u16;
        let preview_area = Rect::new(
            inner.x,
            inner.y + inner.height.saturating_sub(preview_height) / 2,
            inner.width,
            preview_height,
        );
        let style = Style::new().fg(Color::Cyan).bg(SURFACE);
        let text = Text::from(
            lines
                .into_iter()
                .map(|line| Line::from(Span::styled(line, style)))
                .collect::<Vec<_>>(),
        );
        f.render_widget(Paragraph::new(text).alignment(Alignment::Center), preview_area);
    } else {
        let p = Paragraph::new(
            "No bitmap was rendered for this glyph.\nIt may be unsupported by the selected and fallback fonts.",
        )
        .alignment(Alignment::Center);
        f.render_widget(p, inner);
    }
}

/// Convert one 2x3 binary patch to the corresponding Unicode sextant. Unicode
/// reuses older block elements for the two half blocks and the full block.
fn sextant_char(mask: u8) -> char {
    match mask & 0x3f {
        0 => ' ',
        0b01_01_01 => '▌',
        0b10_10_10 => '▐',
        0b11_11_11 => '█',
        mask => {
            let omitted_before = u32::from(mask > 0b01_01_01) + u32::from(mask > 0b10_10_10);
            char::from_u32(0x1FB00 + mask as u32 - 1 - omitted_before).unwrap_or(' ')
        }
    }
}

/// Scale the glyph to the preview pane, then pack six binary pixels into each
/// terminal cell. Nearest-neighbour scaling preserves the actual cached mask.
fn sextant_preview_lines(
    bitmap: &[Vec<u8>],
    max_cell_width: usize,
    max_cell_height: usize,
) -> Vec<String> {
    let source_height = bitmap.len();
    let source_width = bitmap.iter().map(Vec::len).max().unwrap_or(0);
    if source_width == 0 || source_height == 0 || max_cell_width == 0 || max_cell_height == 0 {
        return Vec::new();
    }

    let max_pixel_width = max_cell_width.saturating_mul(2);
    let max_pixel_height = max_cell_height.saturating_mul(3);
    let scale = (max_pixel_width as f64 / source_width as f64)
        .min(max_pixel_height as f64 / source_height as f64);
    let scaled_width = ((source_width as f64 * scale).floor() as usize)
        .clamp(1, max_pixel_width);
    let scaled_height = ((source_height as f64 * scale).floor() as usize)
        .clamp(1, max_pixel_height);
    let cell_width = (scaled_width + 1) / 2;
    let cell_height = (scaled_height + 2) / 3;
    let mut lines = Vec::with_capacity(cell_height);

    for cell_y in 0..cell_height {
        let mut line = String::with_capacity(cell_width * 4);
        for cell_x in 0..cell_width {
            let mut mask = 0u8;
            for sub_y in 0..3 {
                let scaled_y = cell_y * 3 + sub_y;
                if scaled_y >= scaled_height {
                    continue;
                }
                let source_y = scaled_y * source_height / scaled_height;
                for sub_x in 0..2 {
                    let scaled_x = cell_x * 2 + sub_x;
                    if scaled_x >= scaled_width {
                        continue;
                    }
                    let source_x = scaled_x * source_width / scaled_width;
                    if bitmap
                        .get(source_y)
                        .and_then(|row| row.get(source_x))
                        .copied()
                        .unwrap_or(0)
                        > 0
                    {
                        mask |= 1 << (sub_y * 2 + sub_x);
                    }
                }
            }
            line.push(sextant_char(mask));
        }
        lines.push(line);
    }
    lines
}
