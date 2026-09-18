use super::*;

struct FontChoice {
    name: String,
    height: u32,
    fits: bool,
    overlay: crate::args::TextOverlay,
}

pub(crate) struct FigletPicker {
    overlay_index: usize,
    original: crate::args::TextOverlay,
    choices: Vec<FontChoice>,
    search: String,
    list: ListState,
    area: Rect,
}

pub(crate) fn open_figlet_picker(app: &mut App, index: usize) {
    let Some(original) = app.render_args.overlays.get(index).cloned() else {
        return;
    };
    if !cfg!(feature = "ocr") || !original.figlet_enabled() {
        return;
    }
    let font_lists = figlet_font_lists(app);
    let mut choices = Vec::new();
    for name in figlet_font_names(app) {
        let height = font_lists
            .iter()
            .find(|list| list.fonts.contains(&name))
            .map_or(0, |list| list.height);
        let mut overlay = original.clone();
        overlay.figlet_font = Some(name.clone());
        let fits = crate::ocr::refresh_figlet_overlay(&mut overlay, &font_lists);
        choices.push(FontChoice {
            name,
            height,
            fits,
            overlay,
        });
    }
    choices.sort_by(|a, b| {
        b.fits
            .cmp(&a.fits)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    let selected = choices
        .iter()
        .position(|choice| Some(choice.name.as_str()) == original.figlet_font.as_deref())
        .unwrap_or(0);
    app.figlet_picker = Some(FigletPicker {
        overlay_index: index,
        original,
        choices,
        search: String::new(),
        list: ListState::default().with_selected(Some(selected)),
        area: Rect::default(),
    });
}

impl FigletPicker {
    fn filtered(&self) -> Vec<usize> {
        let query = self.search.to_lowercase();
        self.choices
            .iter()
            .enumerate()
            .filter(|(_, choice)| choice.name.to_lowercase().contains(&query))
            .map(|(i, _)| i)
            .collect()
    }

    fn preview(&self, app: &mut App) -> bool {
        let Some(index) = self
            .list
            .selected()
            .and_then(|row| self.filtered().get(row).copied())
        else {
            return false;
        };
        let choice = &self.choices[index];
        if !choice.fits {
            return false;
        }
        if let Some(overlay) = app.render_args.overlays.get_mut(self.overlay_index) {
            *overlay = choice.overlay.clone();
            app.reapply_overlays();
            return true;
        }
        false
    }

    fn move_selection(&mut self, delta: i32) {
        let count = self.filtered().len();
        if count == 0 {
            self.list.select(None);
            return;
        }
        let next = (self.list.selected().unwrap_or(0) as i32 + delta).clamp(0, count as i32 - 1);
        self.list.select(Some(next as usize));
    }
}

pub(crate) fn handle_figlet_picker(app: &mut App, event: Event) {
    let Some(mut picker) = app.figlet_picker.take() else {
        return;
    };
    match event {
        Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
            KeyCode::Esc => {
                if let Some(overlay) = app.render_args.overlays.get_mut(picker.overlay_index) {
                    *overlay = picker.original;
                    app.reapply_overlays();
                }
                return;
            }
            KeyCode::Enter => {
                if picker.preview(app) {
                    return;
                }
            }
            KeyCode::Up => picker.move_selection(-1),
            KeyCode::Down => picker.move_selection(1),
            KeyCode::PageUp => picker.move_selection(-(picker.area.height as i32).max(1)),
            KeyCode::PageDown => picker.move_selection((picker.area.height as i32).max(1)),
            KeyCode::Home => picker.move_selection(i32::MIN / 2),
            KeyCode::End => picker.move_selection(i32::MAX / 2),
            KeyCode::Backspace => {
                picker.search.pop();
                picker.list.select(Some(0));
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                picker.search.push(c);
                picker.list.select(Some(0));
            }
            _ => {}
        },
        Event::Mouse(mouse) => match mouse.kind {
            MouseEventKind::ScrollUp => picker.move_selection(-1),
            MouseEventKind::ScrollDown => picker.move_selection(1),
            MouseEventKind::Down(MouseButton::Left) => {
                if picker
                    .area
                    .contains(ratatui::layout::Position::new(mouse.column, mouse.row))
                {
                    let row = (mouse.row - picker.area.y) as usize + picker.list.offset();
                    if row < picker.filtered().len() {
                        picker.list.select(Some(row));
                        if picker.preview(app) {
                            return;
                        }
                    }
                }
            }
            _ => {}
        },
        _ => {}
    }
    picker.preview(app);
    app.figlet_picker = Some(picker);
}

pub(crate) fn render_figlet_picker(f: &mut ratatui::Frame, picker: &mut FigletPicker) {
    let screen = f.area();
    let area = Rect::new(
        screen.x + 1,
        screen.y + 2,
        screen.width.saturating_sub(2).min(62),
        screen.height.saturating_sub(4).min(26),
    );
    f.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" FIGlet fonts — preview entire text block ")
        .border_style(Style::default().fg(ACCENT));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let parts = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .split(inner);
    let name_width = (parts[1].width as usize).saturating_sub(17).max(1);
    f.render_widget(
        Paragraph::new(format!(
            " Search: {}\n {:<name_width$} Rows",
            picker.search, "Font"
        )),
        parts[0],
    );
    let filtered = picker.filtered();
    let items: Vec<_> = filtered
        .iter()
        .map(|&index| {
            let choice = &picker.choices[index];
            let name = if choice.name.chars().count() > name_width {
                format!(
                    "{}…",
                    choice.name.chars().take(name_width - 1).collect::<String>()
                )
            } else {
                choice.name.clone()
            };
            ListItem::new(format!(
                " {name:<name_width$} {:>2}  {}",
                choice.height,
                if choice.fits { "" } else { "cannot fit" }
            ))
            .style(Style::default().fg(if choice.fits { TEXT } else { TEXT_DIM }))
        })
        .collect();
    picker.area = parts[1];
    f.render_stateful_widget(
        List::new(items).highlight_style(
            Style::default()
                .bg(SELECTED_BG)
                .add_modifier(Modifier::BOLD),
        ),
        parts[1],
        &mut picker.list,
    );
    let hint = if filtered.is_empty() {
        "No matching fonts. Backspace to change the search."
    } else {
        "Wheel / ↑↓: preview · PgUp/PgDn: jump · type to search\nClick / Enter: choose · Esc: restore previous font"
    };
    f.render_widget(Paragraph::new(hint), parts[2]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn font_list_supports_search_preview_cancel_and_click_selection() {
        let parsed = crate::args::Args::parse_from(["img2irc"]);
        let mut args = crate::args_to_render_args(&parsed);
        let original = crate::args::TextOverlay {
            text: "old art".into(),
            source_text: Some("ONE\nTWO".into()),
            figlet_font: Some("original".into()),
            w: 40,
            h: 12,
            x: 0,
            y: 0,
            fg: None,
            bg: None,
            wrap: false,
            auto_grow: false,
            transparent_spaces: true,
            bold: false,
            italic: false,
            underline: false,
        };
        args.overlays.push(original.clone());
        let flag = Arc::new(AtomicBool::new(false));
        let (tx, _rx) = watch::channel((args.clone(), flag.clone(), Instant::now()));
        let (_result_tx, result_rx) = mpsc::channel(1);
        let mut app = App::new(args, tx, result_rx, flag);
        let picker = || FigletPicker {
            overlay_index: 0,
            original: original.clone(),
            search: String::new(),
            list: ListState::default().with_selected(Some(0)),
            area: Rect::default(),
            choices: ["original", "new-font"]
                .into_iter()
                .map(|name| {
                    let mut overlay = original.clone();
                    overlay.text = name.into();
                    overlay.figlet_font = Some(name.into());
                    FontChoice {
                        name: name.into(),
                        height: 3,
                        fits: true,
                        overlay,
                    }
                })
                .collect(),
        };
        app.figlet_picker = Some(picker());
        handle_figlet_picker(
            &mut app,
            Event::Key(event::KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE)),
        );
        // Both names contain n; make the search specific to the alternate font.
        handle_figlet_picker(
            &mut app,
            Event::Key(event::KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE)),
        );
        assert_eq!(
            app.render_args.overlays[0].figlet_font.as_deref(),
            Some("new-font")
        );
        assert_eq!(
            app.render_args.overlays[0].source_text.as_deref(),
            Some("ONE\nTWO")
        );
        handle_figlet_picker(
            &mut app,
            Event::Key(event::KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert!(app.figlet_picker.is_none());
        assert_eq!(app.render_args.overlays[0].text, "old art");

        app.figlet_picker = Some(picker());
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(100, 35)).unwrap();
        terminal
            .draw(|frame| render_figlet_picker(frame, app.figlet_picker.as_mut().unwrap()))
            .unwrap();
        let area = app.figlet_picker.as_ref().unwrap().area;
        handle_figlet_picker(
            &mut app,
            Event::Mouse(event::MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: area.x + 1,
                row: area.y + 1,
                modifiers: KeyModifiers::NONE,
            }),
        );
        assert!(app.figlet_picker.is_none());
        assert_eq!(
            app.render_args.overlays[0].figlet_font.as_deref(),
            Some("new-font")
        );
    }
}
