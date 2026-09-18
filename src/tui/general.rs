use super::*;

pub(crate) fn render_options_panel(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    if app.current_tab != 0 || !app.show_general {
        app.options_pane_area = Rect::default();
        app.options_areas.clear();
        return;
    }

    let id = app.get_current_control_id();
    let ctrl = get_control_def(id);
    if app.control_disabled_reason(id).is_some() {
        app.options_pane_area = Rect::default();
        app.options_areas.clear();
        return;
    }

    let (options, current) = match &ctrl.kind {
        ControlKind::Enum { options, .. } => (
            options.iter().map(|&s| s.to_string()).collect::<Vec<_>>(),
            app.get_value(id) as usize,
        ),
        ControlKind::FontSelector => (app.fonts.clone(), app.font_idx),
        _ => return,
    };

    // Use bottom 3 rows of output
    let pane_height = 3;
    let pane_area = Rect::new(
        area.x,
        area.y + area.height.saturating_sub(pane_height),
        area.width,
        pane_height,
    );
    app.options_pane_area = pane_area;
    app.options_areas.clear();
    f.render_widget(Clear, pane_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} Options ", ctrl.label))
        .border_style(Style::new().fg(ACCENT));
    f.render_widget(&block, pane_area);

    let inner = block.inner(pane_area);
    let mut x = inner.x;

    // We want to show options around the current one
    let visible_count = inner.width as usize / 12; // estimate more space
    let start = current.saturating_sub(visible_count / 2);
    let end = (start + visible_count).min(options.len());

    for (i, opt) in options.iter().enumerate().skip(start).take(end - start) {
        let is_selected = i == current;
        let style = if is_selected {
            Style::new()
                .bg(ACCENT)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(TEXT)
        };
        let text = format!(" {} ", opt);
        if x + text.len() as u16 > inner.x + inner.width {
            break;
        }
        let opt_area = Rect::new(x, inner.y, text.len() as u16, 1);
        if app.options_areas.len() <= i {
            app.options_areas.resize(i + 1, Rect::default());
        }
        app.options_areas[i] = opt_area;
        f.render_widget(Paragraph::new(text.clone()).style(style), opt_area);
        x += text.len() as u16 + 1;
    }
}

pub(crate) fn render_controls(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    // Panel border
    let panel = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(BORDER))
        .border_set(symbols::border::ROUNDED)
        .style(Style::new().bg(SURFACE));
    let panel_inner = panel.inner(area);
    let help_height = if panel_inner.height >= 12 { 6 } else if panel_inner.height >= 7 { 3 } else { 0 };
    let inner = Rect::new(panel_inner.x, panel_inner.y, panel_inner.width, panel_inner.height.saturating_sub(help_height));
    let help_area = Rect::new(panel_inner.x, inner.bottom(), panel_inner.width, help_height);
    f.render_widget(panel, area);

    app.control_areas.clear();

    let buf = f.buffer_mut();

    let controls = app.tab_controls(0);
    let avail_height = inner.height as usize;
    let item_height: u16 = 2; // 1 row content + 1 row spacing
    let max_visible = avail_height / item_height as usize;

    // Keep the viewport stationary while the selection remains visible. The
    // previous offset was derived directly from the selected row, which pinned
    // the selection to the bottom edge whenever the list had been scrolled.
    let selected_ctrl = if app.current_tab == 0 {
        app.selected_control
    } else {
        usize::MAX
    };
    let max_scroll_offset = controls.len().saturating_sub(max_visible);
    app.general_scroll_offset = app.general_scroll_offset.min(max_scroll_offset);
    if selected_ctrl != usize::MAX && max_visible > 0 {
        if selected_ctrl < app.general_scroll_offset {
            app.general_scroll_offset = selected_ctrl;
        } else if selected_ctrl >= app.general_scroll_offset + max_visible {
            app.general_scroll_offset = (selected_ctrl + 1 - max_visible).min(max_scroll_offset);
        }
    }
    let scroll_offset = app.general_scroll_offset;

    let mut control_rects = Vec::new();
    let mut y = inner.y;

    for (v_idx, ctrl) in controls.iter().enumerate() {
        if v_idx < scroll_offset {
            control_rects.push(Rect::default());
            continue;
        }
        if y + item_height > inner.y + inner.height {
            control_rects.push(Rect::default());
            continue;
        }

        let conditional_depth = ctrl.id.conditional_depth();
        let indent = conditional_depth.saturating_mul(2).min(inner.width);
        let ctrl_area = Rect::new(
            inner.x.saturating_add(indent),
            y,
            inner.width.saturating_sub(indent),
            1,
        );
        let selected = v_idx == selected_ctrl;
        let control_bg = match conditional_depth {
            0 => SURFACE,
            1 => CONDITIONAL_BG,
            _ => NESTED_CONDITIONAL_BG,
        };
        let group_area = Rect::new(
            inner.x,
            y,
            inner.width,
            item_height.min(inner.y + inner.height - y),
        );
        buf.set_style(group_area, Style::new().bg(control_bg));

        let label_text = if ctrl.id == ControlId::AutoOptimize && app.is_optimizing {
            "🛑 STOP AUTO-OPTIMIZE".to_string()
        } else {
            ctrl.label.to_string()
        };

        match ctrl.kind {
            ControlKind::Slider { min, max, .. } => {
                let value = app.get_control_value(0, v_idx);
                if selected && app.input_mode == InputMode::ValueEdit {
                    // Show edit buffer with cursor
                    let edit_text = format!("{}: {}▏", label_text, app.value_edit_buffer);
                    let style = Style::new()
                        .fg(Color::Yellow)
                        .bg(SELECTED_BG)
                        .add_modifier(Modifier::BOLD);
                    for (i, ch) in edit_text.chars().enumerate() {
                        if ctrl_area.x + i as u16 >= ctrl_area.x + ctrl_area.width {
                            break;
                        }
                        buf.cell_mut((ctrl_area.x + i as u16, ctrl_area.y))
                            .map(|cell| cell.set_char(ch).set_style(style));
                    }
                } else {
                    let dv = if ctrl.id == ControlId::Width && value == 0 {
                        Some("Auto".to_string())
                    } else if ctrl.id == ControlId::OcrMegapixels {
                        Some(format!("{:.1} MP", value as f32 / 10.0))
                    } else if ctrl.id == ControlId::FontSize {
                        Some((value / 10).to_string())
                    } else if matches!(
                        ctrl.id,
                        ControlId::DiscThreshold
                            | ControlId::ContourBendingWeight
                            | ControlId::ContourEndpointWeight
                            | ControlId::ContourJunctionWeight
                            | ControlId::ContourFragmentWeight
                            | ControlId::ContourFidelityWeight
                            | ControlId::ContourBoundaryWeight
                            | ControlId::ContourPeakWeight
                    ) {
                        Some(format!("{:.1}", value as f32 / 10.0))
                    } else {
                        None
                    };
                    render_bar(
                        buf,
                        ctrl_area,
                        &label_text,
                        value,
                        min,
                        max,
                        selected,
                        dv.as_deref(),
                        control_bg,
                    );
                }
            }
            ControlKind::Enum { options, .. } => {
                let value = app.get_control_value(0, v_idx) as usize;
                render_enum(
                    buf,
                    ctrl_area,
                    &label_text,
                    options,
                    value,
                    selected,
                    control_bg,
                );
            }
            ControlKind::Toggle { .. } => {
                let on = app.get_control_value(0, v_idx) != 0;
                render_toggle(buf, ctrl_area, &label_text, on, selected, control_bg);
            }
            ControlKind::FontSelector => {
                let font_name: &str = if app.font_idx < app.fonts.len() {
                    &app.fonts[app.font_idx]
                } else {
                    "Default"
                };
                render_enum(
                    buf,
                    ctrl_area,
                    &label_text,
                    &[font_name],
                    0,
                    selected,
                    control_bg,
                );
            }
            ControlKind::EyedropperWidget => {
                render_eyedropper_widget(
                    buf,
                    ctrl_area,
                    &label_text,
                    app.eyedropper_color,
                    selected,
                    control_bg,
                );
            }
            ControlKind::Action => {
                let status = if ctrl.id == ControlId::AutoOptimize && app.is_optimizing {
                    &app.optimization_progress
                } else {
                    ""
                };
                render_button(buf, ctrl_area, &label_text, selected, status, control_bg);
            }
        }

        if app.control_disabled_reason(ctrl.id).is_some() {
            buf.set_style(ctrl_area, Style::new().fg(TEXT_DIM).add_modifier(Modifier::DIM));
        }
        control_rects.push(ctrl_area);
        y += item_height;
    }

    app.control_areas = control_rects;

    // Draw scroll indicator if needed
    let total = controls.len();
    if total > max_visible {
        let indicator = format!(
            " {}/{} ",
            if selected_ctrl != usize::MAX {
                selected_ctrl + 1
            } else {
                1
            },
            total
        );
        let ix = inner.right().saturating_sub(indicator.len() as u16 + 1).max(inner.x);
        let iy = inner.bottom().saturating_sub(1);
        if inner.height > 0 && iy < area.y + area.height {
            buf.set_string(ix, iy, &indicator, Style::new().fg(TEXT_DIM).bg(SURFACE));
        }
    }
    if help_height > 0 {
        if let Some(control) = controls.get(app.selected_control) {
            let reason = app.control_disabled_reason(control.id);
            let text = reason.unwrap_or_else(|| control.id.help());
            let title = if reason.is_some() {
                format!(" {} · Disabled ", control.label)
            } else {
                format!(" {} ", control.label)
            };
            f.render_widget(
                Paragraph::new(text)
                    .block(Block::default().borders(Borders::TOP).title(title).border_style(Style::new().fg(BORDER)))
                    .style(Style::new().fg(if reason.is_some() { Color::Yellow } else { TEXT_DIM }))
                    .wrap(ratatui::widgets::Wrap { trim: true }),
                help_area,
            );
        }
    }

}

pub(crate) fn render_auto_optimize_dialog(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    if let Some(state) = &mut app.auto_optimize_state {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Auto Optimize (wheel: adjust, Shift+wheel: 10) ")
            .style(Style::default().fg(Color::Yellow));

        let inner_area = block.inner(area);
        f.render_widget(Clear, area);
        f.render_widget(block, area);

        // Eight parameter rows, two option rows, and one action row.
        let mut constraints = vec![Constraint::Length(3); 8];
        constraints.push(Constraint::Length(3));
        constraints.push(Constraint::Length(3));
        constraints.push(Constraint::Length(3));
        let rows = Layout::vertical(constraints).split(inner_area);

        let titles = [
            "Width",
            "Crop Top",
            "Crop Right",
            "Crop Bottom",
            "Crop Left",
            "Font Size",
            "Scale X",
            "Scale Y",
        ];

        for i in 0..8 {
            let row_area = rows[i];
            let cols = Layout::horizontal([
                Constraint::Length(5),
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
            ])
            .split(row_area);

            let title = titles.get(i).unwrap_or(&"");
            let enabled = state.enabled_rows[i];
            state.row_toggle_areas[i] = cols[0];
            let toggle_text = if enabled { "🗹" } else { "☐" };
            let toggle_style = if enabled {
                Style::default().fg(TOGGLE_ON)
            } else {
                Style::default().fg(TEXT_DIM)
            };
            let toggle_area = Rect::new(cols[0].x, cols[0].y + 1, cols[0].width, 1);
            f.render_widget(
                Paragraph::new(toggle_text)
                    .style(toggle_style)
                    .alignment(Alignment::Center),
                toggle_area,
            );

            for j in 0..3 {
                let idx = i * 3 + j;
                let input = &mut state.inputs[idx];
                state.input_areas[idx] = cols[j + 1];
                let sub_title = if j == 0 {
                    format!("{} Min", title)
                } else if j == 1 {
                    format!("{} Max", title)
                } else {
                    format!("{} Step", title)
                };

                if idx == state.focused_idx {
                    input.set_style(Style::default().fg(if enabled { ACCENT } else { TEXT_DIM }));
                    input.set_block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(sub_title)
                            .border_style(Style::default().fg(ACCENT)),
                    );
                } else {
                    input.set_style(Style::default().fg(if enabled { TEXT } else { TEXT_DIM }));
                    input.set_block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(sub_title)
                            .border_style(Style::default().fg(if enabled { BORDER } else { TEXT_DIM })),
                    );
                }
                f.render_widget(&*input, cols[j + 1]);
            }
        }

        // Render checkboxes side-by-side
        let cb_area = rows[8];
        let cb_cols = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(cb_area);
        state.shortest_line_area = cb_cols[0];
        state.randomize_area = cb_cols[1];

        // Checkbox 1
        let cb1_text = if state.optimize_shortest_longest_line {
            "🗹 Optimize for shortest longest line"
        } else {
            "☐ Optimize for shortest longest line"
        };
        let cb1_style = if state.focused_idx == 24 {
            Style::default().fg(ACCENT)
        } else {
            Style::default()
        };
        let cb1_paragraph = Paragraph::new(cb1_text)
            .style(cb1_style)
            .alignment(Alignment::Center);
        f.render_widget(cb1_paragraph, cb_cols[0]);

        // Checkbox 2
        let cb2_text = if state.randomize {
            "🗹 Randomize"
        } else {
            "☐ Randomize"
        };
        let cb2_style = if state.focused_idx == 25 {
            Style::default().fg(ACCENT)
        } else {
            Style::default()
        };
        let cb2_paragraph = Paragraph::new(cb2_text)
            .style(cb2_style)
            .alignment(Alignment::Center);
        f.render_widget(cb2_paragraph, cb_cols[1]);

        // Render batch-size control. It is mouse-wheel adjustable anywhere in
        // this row, as well as through the existing keyboard controls.
        let slider_area = rows[9];
        state.batch_size_area = slider_area;
        let slider_style = if state.focused_idx == 26 {
            Style::default().fg(ACCENT)
        } else {
            Style::default().fg(Color::Gray)
        };
        let slider_text = format!("Batch Size: {}  (wheel or ←/→)", state.batch_size);
        let slider_p = Paragraph::new(slider_text)
            .style(slider_style)
            .alignment(Alignment::Center);
        f.render_widget(slider_p, slider_area);

        // Explicit mouse actions; Enter and Escape remain available.
        let action_cols = Layout::horizontal([
            Constraint::Percentage(50),
            Constraint::Percentage(50),
        ])
        .split(rows[10]);
        state.start_area = action_cols[0];
        state.cancel_area = action_cols[1];
        f.render_widget(
            Paragraph::new("Start (Enter)")
                .block(Block::default().borders(Borders::ALL))
                .style(Style::default().fg(TOGGLE_ON))
                .alignment(Alignment::Center),
            action_cols[0],
        );
        f.render_widget(
            Paragraph::new("Cancel (Esc)")
                .block(Block::default().borders(Borders::ALL))
                .style(Style::default().fg(Color::Red))
                .alignment(Alignment::Center),
            action_cols[1],
        );
    }
}
