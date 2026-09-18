use super::*;

fn text_toggle_button(
    label: &'static str,
    enabled: bool,
    focused: bool,
) -> Paragraph<'static> {
    let fill = if focused {
        ACCENT
    } else if enabled {
        TOGGLE_ON
    } else {
        TOGGLE_OFF
    };
    let foreground = if focused || enabled {
        Color::Black
    } else {
        TEXT
    };
    Paragraph::new(format!(" {label} "))
        .style(
            Style::default()
                .fg(foreground)
                .bg(fill)
                .add_modifier(if enabled || focused {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        )
        .alignment(Alignment::Center)
}

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
        Constraint::Length(20), // Color picker
        Constraint::Length(1),  // FG/BG targets and clear actions
        Constraint::Length(5),  // Text editor
        Constraint::Length(1),  // Wrap / fit / formatting / delete
        Constraint::Min(6),     // Overlay list
        Constraint::Length(5),  // Instructions / status
    ])
    .split(inner);

    // Color Picker always visible
    let cp_rect = chunks[0];
    let has_selection = app
        .text_selected
        .is_some_and(|s| s > 0 && s <= app.render_args.overlays.len());
    app.text_btn_areas.clear();
    app.text_field_areas.clear();
    app.text_field_areas.push(chunks[2]);

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
                        match fg {
                            crate::args::ColorSpec::Rgb(rgb) => {
                                cp_state.fg_rgb = (rgb[0], rgb[1], rgb[2]);
                                cp_state.set_rgb(rgb[0], rgb[1], rgb[2]);
                            }
                            crate::args::ColorSpec::Index(i) => {
                                cp_state.set_grid_idx(*i as usize);
                            }
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
                            }
                            crate::args::ColorSpec::Index(i) => {
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
        f.render_widget(
            crate::colorpicker::ColorPickerWidget {
                state: cp_state,
                show_details: false,
            },
            cp_rect,
        );
    }

    // Keep each clear action immediately adjacent to the swatch it affects.
    let color_btn_areas = {
        let swatch_area = chunks[1];
        let parts = Layout::horizontal([
            Constraint::Length(10),
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Length(10),
            Constraint::Length(2),
            Constraint::Min(0),
        ])
        .split(swatch_area);
        let color_btn_areas = [parts[0], parts[3], parts[1], parts[4]];

        let fg_editing = app
            .color_picker_state
            .as_ref()
            .map_or(true, |cp| cp.editing_fg);
        if has_selection {
            let ov = &app.render_args.overlays[app.text_selected.unwrap() - 1];
            let color_rgb = |color: &Option<crate::args::ColorSpec>| {
                color.as_ref().map(|color| match color {
                    crate::args::ColorSpec::Rgb(rgb) => (rgb[0], rgb[1], rgb[2]),
                    crate::args::ColorSpec::Index(i) => {
                        let cp = app.color_picker_state.as_ref().unwrap();
                        cp.rgb_from_idx(*i as usize)
                    }
                })
            };
            let color_line =
                |label: &'static str, rgb: Option<(u8, u8, u8)>, active: bool| -> Line<'static> {
                    let mut spans = vec![Span::styled(
                        format!(" {label} "),
                        Style::default()
                            .fg(if active { Color::White } else { TEXT_DIM })
                            .bg(if active { ACCENT_DIM } else { SURFACE })
                            .add_modifier(if active {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            }),
                    )];
                    if let Some((r, g, b)) = rgb {
                        spans.push(Span::styled(
                            "    ",
                            Style::default().bg(Color::Rgb(r, g, b)),
                        ));
                    } else {
                        spans.push(Span::styled(" ∅ ", Style::default().fg(TEXT_DIM)));
                    }
                    Line::from(spans)
                };
            f.render_widget(
                Paragraph::new(color_line("FG", color_rgb(&ov.fg), fg_editing)),
                parts[0],
            );
            f.render_widget(
                Paragraph::new(color_line("BG", color_rgb(&ov.bg), !fg_editing)),
                parts[3],
            );

            let clear_style = |enabled| {
                Style::default()
                    .fg(if enabled { Color::LightRed } else { TEXT_DIM })
                    .add_modifier(Modifier::BOLD)
            };
            f.render_widget(
                Paragraph::new("×")
                    .style(
                        clear_style(ov.fg.is_some()).bg(
                            if app.text_field_focus == TextFieldFocus::ClearFgButton {
                                ACCENT_DIM
                            } else {
                                SURFACE
                            },
                        ),
                    )
                    .alignment(Alignment::Center),
                parts[1],
            );
            f.render_widget(
                Paragraph::new("×")
                    .style(
                        clear_style(ov.bg.is_some()).bg(
                            if app.text_field_focus == TextFieldFocus::ClearBgButton {
                                ACCENT_DIM
                            } else {
                                SURFACE
                            },
                        ),
                    )
                    .alignment(Alignment::Center),
                parts[4],
            );
        } else {
            f.render_widget(
                Paragraph::new(" Select an overlay to choose colors ")
                    .style(Style::default().fg(TEXT_DIM)),
                swatch_area,
            );
        }
        color_btn_areas
    };

    // The text box is both a live preview and the entry point for editing.
    if has_selection {
        let ov = &app.render_args.overlays[app.text_selected.unwrap() - 1];
        let editing = app.input_mode == InputMode::TextEdit;
        let mut visible_text = if editing {
            app.value_edit_buffer.clone()
        } else {
            ov.editable_text().to_string()
        };
        if editing {
            visible_text.push('▌');
        } else if visible_text.is_empty() {
            visible_text.push_str("Click here to type…");
        }
        let title = if editing {
            " Text · Enter = newline · Ctrl+Enter/Esc = done "
        } else {
            " Text · click to edit "
        };
        let editor = Paragraph::new(visible_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(
                        if editing || app.text_field_focus == TextFieldFocus::TextInput {
                            ACCENT
                        } else {
                            BORDER
                        },
                    ))
                    .title(title),
            )
            .style(Style::default().fg(if ov.editable_text().is_empty() && !editing {
                TEXT_DIM
            } else {
                TEXT
            }))
            .wrap(Wrap { trim: false });
        f.render_widget(editor, chunks[2]);
    } else {
        f.render_widget(
            Paragraph::new(
                "Single-click the canvas and type, or double-click and drag a fixed-size box.",
            )
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(BORDER))
                    .title(" Text "),
            )
            .style(Style::default().fg(TEXT_DIM))
            .wrap(Wrap { trim: true }),
            chunks[2],
        );
    }

    if has_selection {
        let sel_idx = app.text_selected.unwrap() - 1;
        let ov = &app.render_args.overlays[sel_idx];

        // Borderless, fixed-width chips keep the toolbar compact and readable.
        let options_chunks = Layout::horizontal([
            Constraint::Length(7),
            Constraint::Length(5),
            Constraint::Length(7),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(5),
            Constraint::Min(0),
        ])
        .split(chunks[3]);

        f.render_widget(
            text_toggle_button(
                "Wrap",
                ov.wrap,
                app.text_field_focus == TextFieldFocus::WrapButton,
            ),
            options_chunks[0],
        );
        app.text_btn_areas.push(options_chunks[0]); // Index 0

        // Grow the box in both directions as text is entered.
        f.render_widget(
            text_toggle_button(
                "Fit",
                ov.auto_grow,
                app.text_field_focus == TextFieldFocus::FitButton,
            ),
            options_chunks[1],
        );
        app.text_btn_areas.push(options_chunks[1]); // Index 1

        f.render_widget(
            text_toggle_button(
                "FIGlet",
                ov.figlet_enabled(),
                app.text_field_focus == TextFieldFocus::FigletButton,
            ),
            options_chunks[2],
        );
        app.text_btn_areas.push(options_chunks[2]); // Index 2

        f.render_widget(
            text_toggle_button(
                "B",
                ov.bold,
                app.text_field_focus == TextFieldFocus::BoldButton,
            ),
            options_chunks[3],
        );
        app.text_btn_areas.push(options_chunks[3]); // Index 3

        f.render_widget(
            text_toggle_button(
                "I",
                ov.italic,
                app.text_field_focus == TextFieldFocus::ItalicButton,
            ),
            options_chunks[4],
        );
        app.text_btn_areas.push(options_chunks[4]); // Index 4

        f.render_widget(
            text_toggle_button(
                "U",
                ov.underline,
                app.text_field_focus == TextFieldFocus::UnderlineButton,
            ),
            options_chunks[5],
        );
        app.text_btn_areas.push(options_chunks[5]); // Index 5

        let del_p = Paragraph::new("Del")
            .style(
                Style::default()
                    .fg(if app.text_field_focus == TextFieldFocus::DeleteButton {
                        Color::Black
                    } else {
                        Color::LightRed
                    })
                    .bg(if app.text_field_focus == TextFieldFocus::DeleteButton {
                        ACCENT
                    } else {
                        TOGGLE_OFF
                    })
                    .add_modifier(Modifier::BOLD),
            )
            .alignment(Alignment::Center);
        f.render_widget(del_p, options_chunks[6]);
        app.text_btn_areas.push(options_chunks[6]); // Index 6

        let font_name = ov.figlet_font.as_deref().unwrap_or("select font");
        let font_style = Style::default()
            .fg(if ov.figlet_enabled() { TEXT } else { TEXT_DIM })
            .bg(if app.text_field_focus == TextFieldFocus::FigletFontButton {
                ACCENT_DIM
            } else {
                SURFACE
            })
            .add_modifier(if app.text_field_focus == TextFieldFocus::FigletFontButton {
                Modifier::BOLD
            } else {
                Modifier::empty()
            });
        f.render_widget(
            Paragraph::new(format!(" Font: {font_name} ▸"))
                .style(font_style)
                .alignment(Alignment::Left),
            options_chunks[7],
        );
        app.text_btn_areas.push(options_chunks[7]); // Temporarily index 7; moved below colors.
    } else {
        for _ in 0..8 {
            app.text_btn_areas.push(Rect::default());
        }
    }
    // Preserve the established action indices (0..6) and colour indices
    // (7..10); the explicit FIGlet font selector is index 11.
    let figlet_font_area = app.text_btn_areas.pop().unwrap_or_default();
    app.text_btn_areas.extend(color_btn_areas);
    app.text_btn_areas.push(figlet_font_area);

    // Mini-list of overlays
    let mut items: Vec<ListItem> = Vec::new();
    let add_style = if app.text_selected == Some(0) {
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(TOGGLE_ON)
    };
    items.push(ListItem::new(" + Add text overlay").style(add_style));
    for (i, ov) in app.render_args.overlays.iter().enumerate() {
        let is_selected = app.text_selected == Some(i + 1);
        let single_line = ov.editable_text().replace('\n', " ↵ ");
        let text_trunc: String = if single_line.chars().count() > 22 {
            format!("{}…", single_line.chars().take(22).collect::<String>())
        } else if single_line.is_empty() {
            "(empty)".to_string()
        } else {
            single_line
        };
        let size = if ov.auto_grow {
            format!("fit {}×{}", ov.w, ov.h)
        } else if ov.w > 0 || ov.h > 0 {
            format!("{}×{}", ov.w, ov.h)
        } else {
            "auto".to_string()
        };
        let figlet = if ov.figlet_enabled() { " · FIGlet" } else { "" };
        let label = format!(
            " {}. {}  @{},{} · {}{}",
            i + 1,
            text_trunc,
            ov.x,
            ov.y,
            size,
            figlet
        );
        let style = if is_selected {
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(TEXT)
        };
        items.push(ListItem::new(label).style(style));
    }
    let list_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(
            if app.text_field_focus == TextFieldFocus::List {
                ACCENT
            } else {
                BORDER
            },
        ))
        .title(" Overlays ");
    let list = List::new(items).block(list_block);
    f.render_stateful_widget(list, chunks[4], &mut app.text_list_state);

    let mut status_text = String::from(
        "Click canvas + type = fit box · double-click + drag = fixed box\n\
Hover = outline · either lower-right edge = 🢆 resize · inside = move\n\
Palette: left = selected · middle = FG · right = BG\n\
Tab moves focus · Enter/Space activates · Esc finishes",
    );

    if !cfg!(feature = "ocr") {
        status_text.push_str("\nFIGlet text requires a build with OCR support.");
    }

    if let Some(sel) = app.text_selected {
        if sel > 0 && sel <= app.render_args.overlays.len() {
            let ov = &app.render_args.overlays[sel - 1];
            status_text = format!(
                "#{} · position {},{} · box {}×{}{}\n",
                sel,
                ov.x,
                ov.y,
                ov.w,
                ov.h,
                if ov.figlet_enabled() {
                    format!(" · FIGlet font {}", ov.figlet_font.as_deref().unwrap_or("unavailable"))
                } else {
                    String::new()
                }
            ) + &status_text;
        }
    }

    let inst_p = Paragraph::new(status_text).style(Style::default().fg(Color::DarkGray));
    f.render_widget(inst_p, chunks[5]);

    // Store rects for mouse events later
    app.color_picker_area = cp_rect;
    app.text_list_area = chunks[4];
}
