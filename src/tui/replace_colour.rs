use super::*;

pub(crate) struct ReplaceColourDialog {
    node_id: usize,
    image: Option<image::RgbaImage>,
    picker: crate::colorpicker::ColorPickerState,
    colours: [[u8; 3]; 2],
    eyedropper: bool,
    cursor: (u16, u16),
    picker_area: Rect,
    image_area: Rect,
    from_area: Rect,
    to_area: Rect,
    eyedropper_area: Rect,
    apply_area: Rect,
    cancel_area: Rect,
    message: String,
}

fn mode_for_render(render_mode_idx: usize) -> crate::colorpicker::PaletteMode {
    match render_mode_idx {
        0 => crate::colorpicker::PaletteMode::Irc,
        1 => crate::colorpicker::PaletteMode::Ansi,
        _ => crate::colorpicker::PaletteMode::Ansi24,
    }
}

fn palette_for_mode(mode: crate::colorpicker::PaletteMode) -> Option<&'static [u32]> {
    match mode {
        crate::colorpicker::PaletteMode::Irc => Some(&crate::palette::IRC99),
        crate::colorpicker::PaletteMode::Ansi => Some(&crate::palette::ANSI256),
        crate::colorpicker::PaletteMode::Ansi24 => None,
    }
}

fn nearest_picker_index(mode: crate::colorpicker::PaletteMode, rgb: [u8; 3]) -> usize {
    let palette = palette_for_mode(mode).unwrap_or(&[]);
    // The shared text-overlay picker presents the extended palette beginning
    // at 16; base ANSI/IRC colours have equivalent entries in that range.
    palette
        .iter()
        .enumerate()
        .skip(16)
        .min_by_key(|(_, colour)| {
            let r = ((**colour >> 16) & 0xff) as i32;
            let g = ((**colour >> 8) & 0xff) as i32;
            let b = (**colour & 0xff) as i32;
            let dr = i32::from(rgb[0]) - r;
            let dg = i32::from(rgb[1]) - g;
            let db = i32::from(rgb[2]) - b;
            dr * dr + dg * dg + db * db
        })
        .map_or(16, |(index, _)| index)
}

fn set_picker_target(picker: &mut crate::colorpicker::ColorPickerState, from: bool, rgb: [u8; 3]) {
    picker.editing_fg = from;
    if picker.mode == crate::colorpicker::PaletteMode::Ansi24 {
        picker.set_rgb(rgb[0], rgb[1], rgb[2]);
    } else {
        picker.set_grid_idx(nearest_picker_index(picker.mode, rgb));
    }
}

fn target_rgb(picker: &crate::colorpicker::ColorPickerState, from: bool) -> [u8; 3] {
    let rgb = if from {
        picker.fg_rgb
    } else {
        picker.bg_rgb.unwrap_or((0, 0, 0))
    };
    [rgb.0, rgb.1, rgb.2]
}

fn visible_rgb(mode: crate::colorpicker::PaletteMode, rgb: [u8; 3]) -> [u8; 3] {
    palette_for_mode(mode).map_or(rgb, |palette| {
        crate::adjustments::quantize_rgb(rgb, palette)
    })
}

fn hex(rgb: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

pub(crate) async fn open_replace_colour(app: &mut App) {
    let Some(node) = app.pipeline_nodes.get(app.selected_control) else {
        return;
    };
    let crate::pipeline::ImageEffect::ReplaceColour { from, to, .. } = node.effect else {
        return;
    };
    let mode = mode_for_render(app.render_mode_idx);
    let from = visible_rgb(mode, from);
    let to = visible_rgb(mode, to);
    let mut picker = crate::colorpicker::ColorPickerState::new(mode, true);
    set_picker_target(&mut picker, true, from);
    set_picker_target(&mut picker, false, to);
    set_picker_target(&mut picker, true, from);

    let mut dialog = ReplaceColourDialog {
        node_id: node.id,
        image: None,
        picker,
        colours: [from, to],
        eyedropper: false,
        cursor: (0, 0),
        picker_area: Rect::default(),
        image_area: Rect::default(),
        from_area: Rect::default(),
        to_area: Rect::default(),
        eyedropper_area: Rect::default(),
        apply_area: Rect::default(),
        cancel_area: Rect::default(),
        message: String::new(),
    };
    if let Some(glyphs) = &app.arc_glyphs {
        match crate::load_image_from_url_or_path(&app.image_path, &app.render_args, glyphs).await {
            Ok(image) => {
                dialog.image = image::RgbaImage::from_raw(
                    image.get_width(),
                    image.get_height(),
                    image.get_raw_pixels(),
                )
            }
            Err(_) => {
                dialog.message = "Image unavailable; choose both colours from the palette.".into()
            }
        }
    }
    app.replace_colour_dialog = Some(dialog);
}

impl ReplaceColourDialog {
    fn select_target(&mut self, from: bool) {
        let colour = self.colours[usize::from(!from)];
        set_picker_target(&mut self.picker, from, colour);
    }

    fn target_rgb(&self, from: bool) -> [u8; 3] {
        self.colours[usize::from(!from)]
    }

    fn sync_target_from_picker(&mut self) {
        let from = self.picker.editing_fg;
        self.colours[usize::from(!from)] = target_rgb(&self.picker, from);
    }

    fn pixel(&self, x: u16, y: u16) -> Option<[u8; 4]> {
        let image = self.image.as_ref()?;
        if self.image_area.width == 0 || self.image_area.height == 0 {
            return None;
        }
        let px = (x as u64 * image.width() as u64 / self.image_area.width as u64)
            .min(image.width().saturating_sub(1) as u64) as u32;
        let py = (y as u64 * image.height() as u64 / (self.image_area.height as u64 * 2))
            .min(image.height().saturating_sub(1) as u64) as u32;
        let pixel = image.get_pixel(px, py).0;
        let visible = palette_for_mode(self.picker.mode)
            .map_or([pixel[0], pixel[1], pixel[2]], |palette| {
                crate::adjustments::quantize_rgb([pixel[0], pixel[1], pixel[2]], palette)
            });
        Some([visible[0], visible[1], visible[2], pixel[3]])
    }

    fn sample(&mut self) {
        if let Some(pixel) = self.pixel(self.cursor.0, self.cursor.1) {
            if pixel[3] == 0 {
                self.message = "Transparent pixel; choose a visible colour.".into();
                return;
            }
            let from = self.picker.editing_fg;
            self.colours[usize::from(!from)] = [pixel[0], pixel[1], pixel[2]];
            set_picker_target(&mut self.picker, from, [pixel[0], pixel[1], pixel[2]]);
            self.message = format!(
                "Sampled the visible {} colour {}.",
                if from { "From" } else { "To" },
                hex([pixel[0], pixel[1], pixel[2]])
            );
        }
    }
}

fn apply_dialog(app: &mut App, dialog: &ReplaceColourDialog) {
    let from = dialog.target_rgb(true);
    let to = dialog.target_rgb(false);
    if let Some(node) = app
        .pipeline_nodes
        .iter_mut()
        .find(|node| node.id == dialog.node_id)
    {
        if let crate::pipeline::ImageEffect::ReplaceColour {
            from: source,
            to: destination,
            ..
        } = &mut node.effect
        {
            *source = from;
            *destination = to;
        }
    }
    app.mark_args_dirty();
}

pub(crate) fn handle_replace_colour(app: &mut App, event: Event) {
    let Some(mut dialog) = app.replace_colour_dialog.take() else {
        return;
    };
    let mut apply = false;
    let mut close = false;
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
            KeyCode::Esc => close = true,
            KeyCode::Enter => apply = true,
            KeyCode::Tab | KeyCode::BackTab => {
                dialog.select_target(!dialog.picker.editing_fg);
            }
            KeyCode::Char('e') | KeyCode::Char('E') => {
                dialog.eyedropper = !dialog.eyedropper;
                dialog.message = if dialog.eyedropper {
                    "Eyedropper active: click the preview or use arrows + Space.".into()
                } else {
                    "Palette active: arrows choose a colour.".into()
                };
            }
            KeyCode::Char(' ') => {
                dialog.eyedropper = true;
                dialog.sample();
            }
            KeyCode::Left if dialog.eyedropper => {
                dialog.cursor.0 = dialog.cursor.0.saturating_sub(1)
            }
            KeyCode::Right if dialog.eyedropper => {
                dialog.cursor.0 =
                    (dialog.cursor.0 + 1).min(dialog.image_area.width.saturating_sub(1))
            }
            KeyCode::Up if dialog.eyedropper => dialog.cursor.1 = dialog.cursor.1.saturating_sub(1),
            KeyCode::Down if dialog.eyedropper => {
                dialog.cursor.1 = (dialog.cursor.1 + 1)
                    .min(dialog.image_area.height.saturating_mul(2).saturating_sub(1))
            }
            KeyCode::Left => {
                dialog.picker.move_selection(-1, 0);
                dialog.sync_target_from_picker();
            }
            KeyCode::Right => {
                dialog.picker.move_selection(1, 0);
                dialog.sync_target_from_picker();
            }
            KeyCode::Up => {
                dialog.picker.move_selection(0, -1);
                dialog.sync_target_from_picker();
            }
            KeyCode::Down => {
                dialog.picker.move_selection(0, 1);
                dialog.sync_target_from_picker();
            }
            _ => {}
        },
        Event::Mouse(mouse) => {
            let position = ratatui::layout::Position::new(mouse.column, mouse.row);
            match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    if dialog.apply_area.contains(position) {
                        apply = true;
                    } else if dialog.cancel_area.contains(position) {
                        close = true;
                    } else if dialog.from_area.contains(position) {
                        dialog.select_target(true);
                    } else if dialog.to_area.contains(position) {
                        dialog.select_target(false);
                    } else if dialog.eyedropper_area.contains(position) {
                        dialog.eyedropper = !dialog.eyedropper;
                    } else if dialog.eyedropper && dialog.image_area.contains(position) {
                        dialog.cursor = (
                            mouse.column - dialog.image_area.x,
                            (mouse.row - dialog.image_area.y) * 2 + mouse.column % 2,
                        );
                        dialog.sample();
                    } else if dialog.picker.handle_mouse_down(
                        mouse.column,
                        mouse.row,
                        dialog.picker_area,
                        Some(dialog.picker.editing_fg),
                    ) {
                        dialog.eyedropper = false;
                        dialog.sync_target_from_picker();
                    }
                }
                MouseEventKind::Drag(MouseButton::Left) => {
                    if dialog
                        .picker
                        .handle_mouse_drag(mouse.column, mouse.row, dialog.picker_area)
                    {
                        dialog.eyedropper = false;
                        dialog.sync_target_from_picker();
                    }
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    dialog.picker.is_dragging_main = false;
                    dialog.picker.is_dragging_hue = false;
                }
                _ => {}
            }
        }
        _ => {}
    }
    if apply {
        apply_dialog(app, &dialog);
    } else if !close {
        app.replace_colour_dialog = Some(dialog);
    }
}

fn target_line(label: &str, rgb: [u8; 3], active: bool) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!(" {label} "),
            Style::default()
                .fg(if active { Color::White } else { TEXT_DIM })
                .bg(if active { ACCENT_DIM } else { SURFACE })
                .add_modifier(if active {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ),
        Span::styled(
            "    ",
            Style::default().bg(Color::Rgb(rgb[0], rgb[1], rgb[2])),
        ),
        Span::styled(format!(" {} ", hex(rgb)), Style::default().fg(TEXT)),
    ])
}

pub(crate) fn render_replace_colour(f: &mut ratatui::Frame, dialog: &mut ReplaceColourDialog) {
    let area = f.area().inner(ratatui::layout::Margin::new(1, 0));
    f.render_widget(Clear, area);
    let mode_name = match dialog.picker.mode {
        crate::colorpicker::PaletteMode::Irc => "IRC palette",
        crate::colorpicker::PaletteMode::Ansi => "ANSI 256 palette",
        crate::colorpicker::PaletteMode::Ansi24 => "24-bit RGB",
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Replace Colour — {mode_name} "));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .split(inner);

    let actions = Layout::horizontal([
        Constraint::Length(23),
        Constraint::Length(23),
        Constraint::Length(17),
        Constraint::Min(0),
    ])
    .split(rows[0]);
    dialog.from_area = actions[0];
    dialog.to_area = actions[1];
    dialog.eyedropper_area = actions[2];
    f.render_widget(
        Paragraph::new(target_line(
            "From",
            dialog.target_rgb(true),
            dialog.picker.editing_fg,
        )),
        actions[0],
    );
    f.render_widget(
        Paragraph::new(target_line(
            "To",
            dialog.target_rgb(false),
            !dialog.picker.editing_fg,
        )),
        actions[1],
    );
    f.render_widget(
        Paragraph::new(if dialog.eyedropper {
            " ◉ Eyedropper "
        } else {
            " ◯ Eyedropper "
        })
        .style(
            Style::default()
                .fg(if dialog.eyedropper {
                    Color::White
                } else {
                    TEXT_DIM
                })
                .bg(if dialog.eyedropper {
                    ACCENT_DIM
                } else {
                    SURFACE
                }),
        ),
        actions[2],
    );

    let body = Layout::horizontal([Constraint::Length(39), Constraint::Min(1)]).split(rows[1]);
    dialog.picker_area = body[0];
    let picker_min_height = match dialog.picker.mode {
        crate::colorpicker::PaletteMode::Irc => 10,
        crate::colorpicker::PaletteMode::Ansi => 13,
        crate::colorpicker::PaletteMode::Ansi24 => 19,
    };
    if body[0].width >= 38 && body[0].height >= picker_min_height {
        f.render_widget(
            crate::colorpicker::ColorPickerWidget {
                state: &mut dialog.picker,
                show_details: false,
            },
            body[0],
        );
    } else {
        f.render_widget(
            Paragraph::new("Terminal is too small for the palette."),
            body[0],
        );
    }

    let image_block = Block::default()
        .borders(Borders::ALL)
        .title(" Visible quantized image ");
    let image_bounds = image_block.inner(body[1]);
    f.render_widget(image_block, body[1]);
    dialog.image_area = image_bounds;
    if let Some(image) = &dialog.image {
        let scale = (image_bounds.width as f64 / image.width().max(1) as f64)
            .min(image_bounds.height as f64 * 2.0 / image.height().max(1) as f64);
        let width = (image.width() as f64 * scale).floor().max(1.0) as u16;
        let height = (image.height() as f64 * scale / 2.0).floor().max(1.0) as u16;
        dialog.image_area = Rect::new(
            image_bounds.x + image_bounds.width.saturating_sub(width) / 2,
            image_bounds.y + image_bounds.height.saturating_sub(height) / 2,
            width.min(image_bounds.width),
            height.min(image_bounds.height),
        );
    }
    dialog.cursor.0 = dialog
        .cursor
        .0
        .min(dialog.image_area.width.saturating_sub(1));
    dialog.cursor.1 = dialog
        .cursor
        .1
        .min(dialog.image_area.height.saturating_mul(2).saturating_sub(1));
    for y in 0..dialog.image_area.height {
        for x in 0..dialog.image_area.width {
            if let (Some(top), Some(bottom)) = (dialog.pixel(x, y * 2), dialog.pixel(x, y * 2 + 1))
            {
                let colour = |pixel: [u8; 4]| {
                    if pixel[3] == 0 {
                        Color::DarkGray
                    } else {
                        Color::Rgb(pixel[0], pixel[1], pixel[2])
                    }
                };
                f.buffer_mut().set_string(
                    dialog.image_area.x + x,
                    dialog.image_area.y + y,
                    "▀",
                    Style::default().fg(colour(top)).bg(colour(bottom)),
                );
            }
        }
    }

    let footer = Layout::horizontal([
        Constraint::Min(1),
        Constraint::Length(9),
        Constraint::Length(10),
    ])
    .split(rows[2]);
    let help = if dialog.message.is_empty() {
        "Tab: From/To · arrows: palette · E: eyedropper · Space: sample"
    } else {
        dialog.message.as_str()
    };
    f.render_widget(
        Paragraph::new(format!(
            "{help}\nReplacement tolerance remains in the pipeline control."
        ))
        .style(Style::default().fg(TEXT_DIM)),
        footer[0],
    );
    dialog.apply_area = footer[1];
    dialog.cancel_area = footer[2];
    f.render_widget(
        Paragraph::new(" Apply ")
            .alignment(Alignment::Center)
            .style(
                Style::default()
                    .fg(Color::White)
                    .bg(ACCENT_DIM)
                    .add_modifier(Modifier::BOLD),
            ),
        footer[1],
    );
    f.render_widget(
        Paragraph::new(" Cancel ")
            .alignment(Alignment::Center)
            .style(Style::default().fg(TEXT).bg(SURFACE)),
        footer[2],
    );

    if dialog.eyedropper && dialog.image_area.width > 0 && dialog.image_area.height > 0 {
        f.set_cursor_position((
            dialog.image_area.x + dialog.cursor.0,
            dialog.image_area.y + dialog.cursor.1 / 2,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn dialog(mode: crate::colorpicker::PaletteMode) -> ReplaceColourDialog {
        let mut picker = crate::colorpicker::ColorPickerState::new(mode, true);
        set_picker_target(&mut picker, true, [255, 255, 255]);
        set_picker_target(&mut picker, false, [0, 0, 0]);
        set_picker_target(&mut picker, true, [255, 255, 255]);
        ReplaceColourDialog {
            node_id: 1,
            image: None,
            picker,
            colours: [[255, 255, 255], [0, 0, 0]],
            eyedropper: false,
            cursor: (0, 0),
            picker_area: Rect::default(),
            image_area: Rect::default(),
            from_area: Rect::default(),
            to_area: Rect::default(),
            eyedropper_area: Rect::default(),
            apply_area: Rect::default(),
            cancel_area: Rect::default(),
            message: String::new(),
        }
    }

    #[test]
    fn ansi_base_colour_stays_exact_when_sampled() {
        assert_eq!(
            visible_rgb(crate::colorpicker::PaletteMode::Ansi, [128, 0, 0]),
            [128, 0, 0]
        );
    }

    #[test]
    fn dialog_renders_in_a_common_terminal_size() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut dialog = dialog(crate::colorpicker::PaletteMode::Ansi24);
        terminal
            .draw(|frame| render_replace_colour(frame, &mut dialog))
            .unwrap();
        assert!(dialog.picker_area.width >= 38);
        assert!(dialog.apply_area.width > 0);
    }
}
