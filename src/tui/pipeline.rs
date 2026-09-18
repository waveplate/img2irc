use super::*;

pub(crate) fn render_pipeline_catalog_list(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    let catalog = &app.pipeline_catalog;
    let no_image = app
        .render_args
        .image
        .as_deref()
        .map(|p| p.is_empty())
        .unwrap_or(true);
    let is_focused = app.current_tab == 1 && app.pipeline_expanded.is_some();
    let border_color = if no_image {
        Color::Rgb(80, 40, 40)
    } else if is_focused {
        ACCENT
    } else {
        BORDER
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            " Library ",
            Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if no_image {
        f.render_widget(
            Paragraph::new("Load an image\nfirst")
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::Rgb(180, 80, 80))),
            inner,
        );
        return;
    }

    let scroll = app.pipeline_scroll as usize;
    let items: Vec<ListItem> = catalog
        .iter()
        .enumerate()
        .skip(scroll)
        .map(|(idx, e)| {
            if e.effect.is_none() {
                // section header
                ListItem::new(format!("{}", e.label))
                    .style(Style::default().fg(ACCENT_DIM).add_modifier(Modifier::BOLD))
            } else {
                let is_selected = idx == app.pipeline_library_selected;
                let style = if is_selected {
                    let mut style = Style::default()
                        .fg(Color::White)
                        .bg(SELECTED_BG)
                        .add_modifier(Modifier::BOLD);
                    if !is_focused {
                        style = style.remove_modifier(Modifier::BOLD);
                    }
                    style
                } else {
                    Style::default().fg(TEXT_DIM)
                };

                let mut spans = vec![Span::styled(format!("  {}", e.label), style)];

                if is_selected {
                    if let Some(effect) = &e.effect {
                        if let Some(v) = effect.param_f32() {
                            let (min, max) = effect.param_range().unwrap_or((0.0, 255.0));
                            let units = quantized_slider_units_f32(v, min, max, SLIDER_BAR_CELLS);
                            let bar = render_eighth_block_bar(units, SLIDER_BAR_CELLS);
                            spans.push(Span::styled("  ", style));
                            spans.push(Span::styled(
                                bar,
                                Style::default().fg(ACCENT).bg(Color::Rgb(15, 15, 20)),
                            ));
                            spans.push(Span::styled(format!(" {:>4.0}", v), style));
                        }
                    }
                }
                ListItem::new(Line::from(spans))
            }
        })
        .collect();

    f.render_widget(List::new(items), inner);
}

pub(crate) fn render_active_pipeline_list(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    app.ensure_pipeline_selection();
    app.replace_colour_button_area = Rect::default();
    let area = if app.pipeline_expanded.is_none() {
        if let Some(crate::pipeline::ImageEffectNode {
            effect: crate::pipeline::ImageEffect::ReplaceColour { from, to, tolerance }, ..
        }) = app.pipeline_nodes.get(app.selected_control) {
            let parts = Layout::vertical([Constraint::Min(3), Constraint::Length(4)]).split(area);
            let block = Block::default().borders(Borders::ALL).title(" Replace Colour ");
            let inner = block.inner(parts[1]);
            f.render_widget(block, parts[1]);
            let rows = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(inner);
            f.render_widget(Paragraph::new(Line::from(vec![
                Span::styled(format!(" #{:02x}{:02x}{:02x}", from[0], from[1], from[2]), Style::default().fg(Color::Rgb(from[0], from[1], from[2]))),
                Span::raw(" → "),
                Span::styled(format!("#{:02x}{:02x}{:02x}", to[0], to[1], to[2]), Style::default().fg(Color::Rgb(to[0], to[1], to[2]))),
                Span::raw(format!("  Tolerance: {tolerance:.0}%")),
            ])), rows[0]);
            f.render_widget(Paragraph::new(" [ Eyedropper / Edit colours… ]  Enter / c").style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)), rows[1]);
            app.replace_colour_button_area = rows[1];
            parts[0]
        } else { area }
    } else { area };
    app.pipeline_active_area = area;
    let is_focused = app.current_tab == 1;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if is_focused { ACCENT } else { BORDER }))
        .border_set(ratatui::symbols::border::ROUNDED)
        .style(Style::new().bg(SURFACE))
        .title(Span::styled(
            " Active Pipeline ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut items = Vec::new();
    let total_nodes = app.pipeline_nodes.len();

    let mut target_visual_index = 0;
    let mut current_visual_index = 0;

    for i in 0..=total_nodes {
        let is_add_node = i == total_nodes;
        let is_selected = app.selected_control == i && app.pipeline_expanded.is_none();
        let is_expanded = app.pipeline_expanded == Some(i);

        if is_selected {
            target_visual_index = current_visual_index;
        }

        let line_bg = if is_selected || is_expanded {
            SELECTED_BG
        } else {
            SURFACE
        };

        let (label, is_disabled, active_f32, active_range) = if is_add_node {
            ("＋ Add Effect...", false, None, None)
        } else {
            let node = &app.pipeline_nodes[i];
            (
                node.effect.name(),
                node.disabled,
                node.effect.param_f32(),
                node.effect.param_range(),
            )
        };

        let (display_f32, display_range) = if is_expanded {
            if let Some(preview) = &app.pipeline_preview_effect {
                (preview.param_f32(), preview.param_range())
            } else {
                (None, None)
            }
        } else {
            (active_f32, active_range)
        };

        let label = if !is_add_node
            && matches!(app.pipeline_nodes[i].effect, crate::pipeline::ImageEffect::ReplaceColour { .. })
        {
            "Replace Colour [c]"
        } else {
            label
        };

        let text_fg = if is_disabled {
            TEXT_DIM
        } else if is_add_node {
            if is_selected {
                Color::White
            } else {
                TEXT_DIM
            }
        } else {
            TEXT
        };
        let icon_str = if is_add_node { "  " } else { "● " };
        let icon_fg = if is_disabled { TEXT_DIM } else { TOGGLE_ON };

        // 2 chars for icon_str. We want 22 total chars before slider. Pad label to 20.
        let remaining_len = 20_usize.saturating_sub(label.chars().count());
        let padding = " ".repeat(remaining_len);

        let mut spans = vec![
            Span::styled(icon_str, Style::default().fg(icon_fg)),
            Span::styled(
                format!("{}{}", label, padding),
                Style::default().fg(text_fg).add_modifier(
                    if (is_selected || is_expanded) && !is_disabled && !is_add_node {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    },
                ),
            ),
        ];

        if let Some(v) = display_f32 {
            let (min, max) = display_range.unwrap_or((0.0, 255.0));
            let bar_cells = SLIDER_BAR_CELLS;
            let units = quantized_slider_units_f32(v, min, max, bar_cells);
            let bar = render_eighth_block_bar(units, bar_cells);

            let display_str = format!("{}", v);
            let text_len = display_str.chars().count();
            let center_idx = bar_cells.saturating_sub(text_len) / 2;
            let is_filled = (units / 8) >= center_idx + text_len / 2;

            let fg_color = if is_selected || is_expanded {
                BAR_FG
            } else {
                ACCENT
            };
            let bg_color = Color::Rgb(15, 15, 20);

            let left_part: String = bar.chars().take(center_idx).collect();
            let right_part: String = bar.chars().skip(center_idx + text_len).collect();

            let text_bg = if is_filled { fg_color } else { bg_color };
            let text_fg = if is_filled { bg_color } else { fg_color };

            spans.push(Span::styled(
                left_part,
                Style::default().fg(fg_color).bg(bg_color),
            ));
            spans.push(Span::styled(
                display_str,
                Style::default()
                    .fg(text_fg)
                    .bg(text_bg)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(
                right_part,
                Style::default().fg(fg_color).bg(bg_color),
            ));
        }

        items.push(ListItem::new(Line::from(spans)).style(Style::default().bg(line_bg)));
        current_visual_index += 1;

        if is_expanded {
            let catalog = &app.pipeline_catalog;
            for (lib_idx, e) in catalog.iter().enumerate() {
                let lib_selected = lib_idx == app.pipeline_library_selected;
                let lib_style = if lib_selected {
                    Style::default()
                        .fg(Color::White)
                        .bg(SELECTED_BG)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(TEXT_DIM)
                };

                if lib_selected {
                    target_visual_index = current_visual_index;
                }

                if e.effect.is_none() {
                    let clean_label = e.label.replace("── ", "").replace(" ──", "");
                    items.push(
                        ListItem::new(format!("   {}", clean_label)).style(
                            Style::default()
                                .fg(ACCENT_DIM)
                                .add_modifier(Modifier::BOLD)
                                .bg(SURFACE),
                        ),
                    );
                } else {
                    let lib_spans = vec![Span::styled(format!("      {}", e.label), lib_style)];
                    items.push(ListItem::new(Line::from(lib_spans)).style(
                        Style::default().bg(if lib_selected { SELECTED_BG } else { SURFACE }),
                    ));
                }
                current_visual_index += 1;
            }
        }
    }

    if app.pipeline_list_state.selected() != Some(target_visual_index) {
        app.pipeline_list_state.select(Some(target_visual_index));
    }

    // We pass list to stateful widget
    let list = List::new(items);
    f.render_stateful_widget(list, inner, &mut app.pipeline_list_state);
}

pub(crate) fn add_pipeline_effect(app: &mut App, catalog_idx: usize) {
    if app
        .render_args
        .image
        .as_deref()
        .map(|path| path.is_empty())
        .unwrap_or(true)
    {
        return;
    }

    let effect = if let Some(preview) = app.pipeline_preview_effect.clone() {
        preview
    } else {
        let Some(effect) = app
            .pipeline_catalog
            .get(catalog_idx)
            .and_then(|item| item.effect.clone())
        else {
            return;
        };
        effect
    };

    let new_id = app.pipeline_next_id;
    app.pipeline_nodes.push(crate::pipeline::ImageEffectNode {
        id: new_id,
        effect,
        disabled: false,
    });
    app.pipeline_next_id += 1;
    app.pipeline_selected = Some(new_id);
    app.selected_control = app.pipeline_nodes.len().saturating_sub(1);

    // Refresh preview for next selection
    app.pipeline_preview_effect = app
        .pipeline_catalog
        .get(catalog_idx)
        .and_then(|item| item.effect.clone());

    app.mark_args_dirty();
}

pub(crate) fn select_pipeline_library(app: &mut App, catalog_idx: usize) {
    app.pipeline_library_selected =
        catalog_idx.min(app.pipeline_catalog.len().saturating_sub(1));
    app.pipeline_preview_effect = app
        .pipeline_catalog
        .get(app.pipeline_library_selected)
        .and_then(|item| item.effect.clone());
    app.mark_args_dirty();
}

pub(crate) fn move_pipeline_library_selection(app: &mut App, direction: i32) {
    if app.pipeline_catalog.is_empty() {
        app.pipeline_library_selected = 0;
        return;
    }

    let mut iter: Box<dyn Iterator<Item = usize>> = if direction < 0 {
        Box::new((0..app.pipeline_library_selected).rev())
    } else {
        Box::new((app.pipeline_library_selected + 1)..app.pipeline_catalog.len())
    };

    if let Some(idx) = iter.find(|idx| app.pipeline_catalog[*idx].effect.is_some()) {
        select_pipeline_library(app, idx);
    }

    let visible_rows = 20_usize;
    let scroll = app.pipeline_scroll as usize;
    if visible_rows > 0 {
        if app.pipeline_library_selected < scroll {
            app.pipeline_scroll = app.pipeline_library_selected as u16;
        } else if app.pipeline_library_selected >= scroll + visible_rows {
            app.pipeline_scroll =
                app.pipeline_library_selected
                    .saturating_sub(visible_rows.saturating_sub(1)) as u16;
        }
    }
}

pub(crate) fn adjust_pipeline_library_preview(app: &mut App, delta: f32) {
    let Some(effect) = app.pipeline_preview_effect.as_mut() else {
        return;
    };
    let Some(value) = effect.param_f32() else {
        return;
    };
    let (min, max) = effect.param_range().unwrap_or((-9999.0, 9999.0));
    effect.set_param((value + delta).clamp(min, max));
    app.mark_args_dirty();
}

pub(crate) fn adjust_selected_pipeline_param(app: &mut App, delta: f32) {
    let Some(id) = app.pipeline_selected else {
        return;
    };

    let Some(node) = app.pipeline_nodes.iter_mut().find(|node| node.id == id) else {
        return;
    };

    let Some(value) = node.effect.param_f32() else {
        return;
    };
    let (min, max) = node.effect.param_range().unwrap_or((-9999.0, 9999.0));
    node.effect.set_param((value + delta).clamp(min, max));
    app.mark_args_dirty();
}
