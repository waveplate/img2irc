use super::*;

pub(crate) fn render_text_tab(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
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
        Constraint::Length(25), // Color picker
        Constraint::Length(3),  // FG/BG color swatches
        Constraint::Length(3),  // Options (Wrap toggle / Bold / Italic / Underline)
        Constraint::Length(3),  // Delete button
        Constraint::Length(8),  // Mini list of overlays
        Constraint::Min(0),     // Instructions / Status
    ])
    .split(inner);

    // Color Picker always visible
    let cp_rect = Rect::new(chunks[0].x, chunks[0].y, chunks[0].width, 27);

    // Ensure CP is instantiated with correct mode
    let mode = match app.render_mode_idx {
        0 => crate::colorpicker::PaletteMode::Irc,
        1 => crate::colorpicker::PaletteMode::Ansi,
        2 => crate::colorpicker::PaletteMode::Ansi24,
        _ => crate::colorpicker::PaletteMode::Irc,
    };
    if app.color_picker_state.is_none() {
        app.color_picker_state = Some(crate::colorpicker::ColorPickerState::new(mode, true));
    } else {
        let cp = app.color_picker_state.as_mut().unwrap();
        cp.mode = mode;
        cp.is_open = true; // Force open
    }

    // Sync CP with selection
    if let Some(cp_state) = &mut app.color_picker_state {
        if let Some(sel) = app.text_selected {
            if sel > 0 && sel <= app.render_args.overlays.len() {
                let ov = &app.render_args.overlays[sel - 1];
                if cp_state.editing_fg {
                    if let Some(fg) = &ov.fg {
                        let (r, g, b) = match fg {
                            crate::args::ColorSpec::Rgb(rgb) => (rgb[0], rgb[1], rgb[2]),
                            crate::args::ColorSpec::Index(i) => cp_state.rgb_from_idx(*i as usize),
                        };
                        cp_state.fg_rgb = (r, g, b);
                        cp_state.set_rgb(r, g, b);
                    } else {
                        cp_state.fg_idx = None;
                    }
                } else {
                    if let Some(bg) = &ov.bg {
                        let (r, g, b) = match bg {
                            crate::args::ColorSpec::Rgb(rgb) => (rgb[0], rgb[1], rgb[2]),
                            crate::args::ColorSpec::Index(i) => cp_state.rgb_from_idx(*i as usize),
                        };
                        cp_state.bg_rgb = Some((r, g, b));
                        cp_state.set_rgb(r, g, b);
                    } else {
                        cp_state.bg_idx = None;
                        cp_state.bg_rgb = None;
                    }
                }
            }
        }
        f.render_widget(
            crate::colorpicker::ColorPickerWidget {
                state: cp_state,
                show_details: false,
            },
            cp_rect,
        );
    }

    // FG/BG color swatch display
    {
        let swatch_area = chunks[1];
        let swatch_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(BORDER))
            .title(" Color ");
        let swatch_inner = swatch_block.inner(swatch_area);
        f.render_widget(swatch_block, swatch_area);

        let swatch_halves =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(swatch_inner);

        // FG swatch
        let fg_editing = app
            .color_picker_state
            .as_ref()
            .map_or(true, |cp| cp.editing_fg);
        let fg_label = if fg_editing { "▶Fg " } else { " Fg " };
        if let Some(sel) = app.text_selected {
            if sel > 0 && sel <= app.render_args.overlays.len() {
                let ov = &app.render_args.overlays[sel - 1];
                let fg_style = if let Some(fg) = &ov.fg {
                    let (r, g, b) = match fg {
                        crate::args::ColorSpec::Rgb(rgb) => (rgb[0], rgb[1], rgb[2]),
                        crate::args::ColorSpec::Index(i) => {
                            let cp = app.color_picker_state.as_ref().unwrap();
                            cp.rgb_from_idx(*i as usize)
                        }
                    };
                    Style::default().bg(Color::Rgb(r, g, b)).fg(
                        if (r as u16 + g as u16 + b as u16) > 380 {
                            Color::Black
                        } else {
                            Color::White
                        },
                    )
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                let fg_text = if ov.fg.is_some() {
                    format!("{}████", fg_label)
                } else {
                    format!("{}[None]", fg_label)
                };
                f.render_widget(Paragraph::new(fg_text).style(fg_style), swatch_halves[0]);

                // BG swatch
                let bg_label = if !fg_editing { "▶Bg " } else { " Bg " };
                let bg_style = if let Some(bg) = &ov.bg {
                    let (r, g, b) = match bg {
                        crate::args::ColorSpec::Rgb(rgb) => (rgb[0], rgb[1], rgb[2]),
                        crate::args::ColorSpec::Index(i) => {
                            let cp = app.color_picker_state.as_ref().unwrap();
                            cp.rgb_from_idx(*i as usize)
                        }
                    };
                    Style::default().bg(Color::Rgb(r, g, b)).fg(
                        if (r as u16 + g as u16 + b as u16) > 380 {
                            Color::Black
                        } else {
                            Color::White
                        },
                    )
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                let bg_text = if ov.bg.is_some() {
                    format!("{}████", bg_label)
                } else {
                    format!("{}[None]", bg_label)
                };
                f.render_widget(Paragraph::new(bg_text).style(bg_style), swatch_halves[1]);
            }
        }
    }

    app.text_btn_areas.clear();

    let has_selection = app
        .text_selected
        .map_or(false, |s| s > 0 && s <= app.render_args.overlays.len());
    if has_selection {
        let sel_idx = app.text_selected.unwrap() - 1;
        let ov = &app.render_args.overlays[sel_idx];

        // Row 1: Wrap, Bold, Italic, Underline
        let options_chunks = Layout::horizontal([
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .split(chunks[2]);

        // Wrap
        let wrap_style = if ov.wrap {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::Gray)
        };
        let wrap_p = Paragraph::new(if ov.wrap { "[x] Wrap" } else { "[ ] Wrap" })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(wrap_style),
            )
            .alignment(Alignment::Center);
        f.render_widget(wrap_p, options_chunks[0]);
        app.text_btn_areas.push(options_chunks[0]); // Index 0

        // Bold
        let bold_style = if ov.bold {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::Gray)
        };
        let bold_p = Paragraph::new(" B ")
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(bold_style),
            )
            .alignment(Alignment::Center);
        f.render_widget(bold_p, options_chunks[1]);
        app.text_btn_areas.push(options_chunks[1]); // Index 1

        // Italic
        let italic_style = if ov.italic {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::Gray)
        };
        let italic_p = Paragraph::new(" I ")
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(italic_style),
            )
            .alignment(Alignment::Center);
        f.render_widget(italic_p, options_chunks[2]);
        app.text_btn_areas.push(options_chunks[2]); // Index 2

        // Underline
        let underline_style = if ov.underline {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::Gray)
        };
        let underline_p = Paragraph::new(" U ")
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(underline_style),
            )
            .alignment(Alignment::Center);
        f.render_widget(underline_p, options_chunks[3]);
        app.text_btn_areas.push(options_chunks[3]); // Index 3

        // Row 2: Delete
        let del_btn = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red));
        let del_p = Paragraph::new("Delete Overlay")
            .block(del_btn)
            .alignment(Alignment::Center);
        f.render_widget(del_p, chunks[3]);
        app.text_btn_areas.push(chunks[3]); // Index 4
    } else {
        for _ in 0..5 {
            app.text_btn_areas.push(Rect::default());
        }
    }

    // Mini-list of overlays
    let mut items: Vec<ListItem> = Vec::new();
    for (i, ov) in app.render_args.overlays.iter().enumerate() {
        let is_selected = app.text_selected == Some(i + 1);
        let text_trunc: String = if ov.text.chars().count() > 20 {
            format!("{}…", ov.text.chars().take(20).collect::<String>())
        } else {
            ov.text.clone()
        };
        let label = format!(
            " #{} {} ({},{}) {}x{}",
            i + 1,
            text_trunc,
            ov.x,
            ov.y,
            ov.w,
            ov.h
        );
        let style = if is_selected {
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(TEXT)
        };
        items.push(ListItem::new(label).style(style));
    }
    let list_block = Block::default().borders(Borders::ALL).title(" Overlays ");
    let list = List::new(items).block(list_block);
    f.render_stateful_widget(list, chunks[4], &mut app.text_list_state);
    // Note: We don't update text_list_area here because we want mouse to work on image

    let mut status_text = String::from(
        "Instructions:\n\
    - Double-click canvas: Create Text / Start Editing\n\
    - Left Click: Select FG / Right Click: Select BG\n\
    - Drag Box: Move / Drag Corner: Resize\n\
    - Type: Enter Text / Esc: Deselect",
    );

    if let Some(sel) = app.text_selected {
        if sel > 0 && sel <= app.render_args.overlays.len() {
            let ov = &app.render_args.overlays[sel - 1];
            status_text = format!(
                "Selected #{} | Pos: {},{} | Size: {}x{}\n",
                sel, ov.x, ov.y, ov.w, ov.h
            ) + &status_text;
        }
    }

    let inst_p = Paragraph::new(status_text).style(Style::default().fg(Color::DarkGray));
    f.render_widget(inst_p, chunks[5]);

    // Store rects for mouse events later
    app.color_picker_area = cp_rect;
    app.text_list_area = chunks[4];
}
