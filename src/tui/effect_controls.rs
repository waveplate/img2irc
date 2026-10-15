use super::*;
use crate::pipeline::ImageEffect;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Control {
    Number(usize),
    EditNumber(usize),
    Select(usize),
    From,
    To,
    Palette,
    Colour(usize),
    Stage,
    Automatic,
    Current,
    PickFrom,
    PickTo,
    PickPalette,
    ResetCrop,
    SelectCrop,
}

#[derive(Default)]
pub(crate) struct EffectControls {
    node: Option<usize>,
    hits: Vec<(Rect, Control)>,
    pub(crate) parameter: usize,
    focused: Option<Control>,
    dragging: Option<usize>,
    editing: Option<(Control, String)>,
    sampling: Option<Control>,
    error: String,
    sample_image: Option<photon_rs::PhotonImage>,
    sample_area: Rect,
    sample_cursor: (u32, u32),
    crop_drag: Option<(u32, u32)>,
    crop_box: Option<[u32; 4]>,
}

impl EffectControls {
    pub(crate) fn has_focus(&self) -> bool {
        self.focused.is_some() || self.editing.is_some() || self.sampling.is_some()
    }
}

fn controls(effect: &ImageEffect) -> Vec<Control> {
    let mut controls: Vec<_> = (0..effect.parameters().len())
        .map(Control::Number)
        .collect();
    match effect {
        ImageEffect::QuantizePalette { palette, .. } => {
            controls.extend([Control::Stage, Control::PickPalette, Control::Palette]);
            controls.extend((0..palette.len()).map(Control::Colour));
            controls.extend([Control::Automatic, Control::Current]);
        }
        ImageEffect::ReplaceColour { .. } => controls.extend([
            Control::From,
            Control::PickFrom,
            Control::To,
            Control::PickTo,
        ]),
        ImageEffect::Crop { .. } => controls.extend([Control::SelectCrop, Control::ResetCrop]),
        _ => {}
    }
    controls
}

fn parameters(app: &App, effect: &ImageEffect) -> Vec<crate::pipeline::EffectParameter> {
    let mut parameters = effect.parameters();
    if !matches!(effect, ImageEffect::Crop { .. }) {
        return parameters;
    }
    let dimensions = app.source_preview.lock().ok().and_then(|slot| {
        slot.as_ref()
            .filter(|(path, _)| Some(path) == app.render_args.image.as_ref())
            .map(|(_, image)| (image.get_width(), image.get_height()))
    });
    if let Some((mut width, mut height)) = dimensions {
        for node in app
            .pipeline_nodes
            .iter()
            .take(app.selected_control)
            .filter(|node| !node.disabled)
        {
            match &node.effect {
                ImageEffect::ScaleX(value) => {
                    width = ((width as f32 * *value as f32 / 100.0) as u32).max(1)
                }
                ImageEffect::ScaleY(value) => {
                    height = ((height as f32 * *value as f32 / 100.0) as u32).max(1)
                }
                ImageEffect::Crop {
                    top,
                    right,
                    bottom,
                    left,
                } => {
                    width = width.saturating_sub(*left).saturating_sub(*right).max(1);
                    height = height.saturating_sub(*top).saturating_sub(*bottom).max(1);
                }
                ImageEffect::CropLeft(value) | ImageEffect::CropRight(value) => {
                    width = width.saturating_sub((*value).max(0) as u32).max(1)
                }
                ImageEffect::CropTop(value) | ImageEffect::CropBottom(value) => {
                    height = height.saturating_sub((*value).max(0) as u32).max(1)
                }
                _ => {}
            }
        }
        for (index, parameter) in parameters.iter_mut().enumerate() {
            parameter.max = if index % 2 == 0 {
                height.saturating_sub(1)
            } else {
                width.saturating_sub(1)
            } as f32;
        }
    }
    parameters
}

fn focus(app: &mut App, control: Option<Control>) {
    app.effect_controls.focused = control;
    if control.is_none() {
        app.effect_controls.parameter = 0;
    }
    if let Some(Control::Number(index) | Control::Select(index)) = control {
        app.effect_controls.parameter = index;
    }
}

fn move_focus(app: &mut App, direction: i32) {
    let Some(effect) = app
        .pipeline_nodes
        .get(app.selected_control)
        .map(|node| &node.effect)
    else {
        return;
    };
    let order = controls(effect);
    let next = match app
        .effect_controls
        .focused
        .and_then(|current| order.iter().position(|&c| c == current))
    {
        Some(index) if direction > 0 => order.get(index + 1).copied(),
        Some(index) => index
            .checked_sub(1)
            .and_then(|index| order.get(index).copied()),
        None if direction > 0 => order.first().copied(),
        None => order.last().copied(),
    };
    focus(app, next);
}

fn hex(rgb: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

fn parse_palette(input: &str) -> Result<Vec<[u8; 3]>, String> {
    let mut colours = Vec::new();
    for token in input
        .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
        .filter(|s| !s.is_empty())
    {
        let value = token.trim_start_matches('#');
        if value.len() != 6 || !value.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("Invalid colour {token}; use #RRGGBB"));
        }
        let packed = u32::from_str_radix(value, 16).unwrap();
        let rgb = [(packed >> 16) as u8, (packed >> 8) as u8, packed as u8];
        if !colours.contains(&rgb) {
            colours.push(rgb);
        }
    }
    if colours.len() > 256 {
        return Err("Use at most 256 colours".into());
    }
    Ok(colours)
}

fn current_palette(app: &App) -> Vec<[u8; 3]> {
    let mut counts = std::collections::BTreeMap::new();
    if let (Some(result), Some(glyphs)) = (&app.cached_result, &app.arc_glyphs) {
        let area = (glyphs.metrics.0 * glyphs.metrics.1) as u64;
        for cell in result.grid.iter().flatten() {
            let ink = draw::glyph_ink_pixels(glyphs, cell.char);
            if ink > 0 {
                *counts.entry(cell.fg).or_insert(0u64) += ink;
            }
            if ink < area {
                *counts.entry(cell.bg.unwrap_or([0; 3])).or_insert(0u64) += area - ink;
            }
        }
    }
    let mut ranked: Vec<_> = counts.into_iter().collect();
    ranked.sort_by(|(a, na), (b, nb)| nb.cmp(na).then_with(|| a.cmp(b)));
    ranked.into_iter().take(256).map(|(rgb, _)| rgb).collect()
}

fn row(
    f: &mut ratatui::Frame,
    app: &mut App,
    area: Rect,
    index: u16,
    text: Line<'static>,
    control: Control,
) {
    if index >= area.height {
        return;
    }
    let rect = Rect::new(area.x, area.y + index, area.width, 1);
    let scroll = if app
        .effect_controls
        .editing
        .as_ref()
        .is_some_and(|(c, _)| *c == control)
    {
        text.width().saturating_sub(rect.width as usize) as u16
    } else {
        0
    };
    f.render_widget(
        Paragraph::new(text)
            .scroll((0, scroll))
            .style(
                Style::default()
                    .fg(TEXT)
                    .bg(if app.effect_controls.focused == Some(control) {
                        SELECTED_BG
                    } else {
                        SURFACE
                    }),
            ),
        rect,
    );
    app.effect_controls.hits.push((rect, control));
}

pub(crate) fn render_effect_controls(f: &mut ratatui::Frame, app: &mut App, area: Rect) -> Rect {
    app.effect_controls.hits.clear();
    if app.pipeline_expanded.is_some() {
        return area;
    }
    let Some(node) = app.pipeline_nodes.get_mut(app.selected_control) else {
        app.effect_controls = EffectControls::default();
        return area;
    };
    node.effect.upgrade_quantizer();
    if app.effect_controls.node != Some(node.id) {
        app.effect_controls = EffectControls {
            node: Some(node.id),
            ..Default::default()
        };
    }
    let effect = node.effect.clone();
    let parameters = parameters(app, &effect);
    if parameters.is_empty() {
        return area;
    }
    let custom_rows = match effect {
        ImageEffect::QuantizePalette { .. } => 5,
        ImageEffect::ReplaceColour { .. } => 4,
        ImageEffect::Crop { .. } => 2,
        _ => 0,
    };
    let height = (parameters.len() as u16 + custom_rows + 3).min(area.height.saturating_sub(4));
    if height < 4 {
        return area;
    }
    let parts = Layout::vertical([Constraint::Min(4), Constraint::Length(height)]).split(area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if app.effect_controls.has_focus() {
            ACCENT
        } else {
            BORDER
        }))
        .title(format!(" {} controls ", effect.name()));
    let inner = block.inner(parts[1]);
    f.render_widget(block, parts[1]);
    app.effect_controls.parameter = app.effect_controls.parameter.min(parameters.len() - 1);
    for (index, parameter) in parameters.iter().enumerate() {
        let label_width = 22.min(inner.width.saturating_sub(8));
        let bar_width = inner.width.saturating_sub(label_width + 7) as usize;
        let units =
            quantized_slider_units_f32(parameter.value, parameter.min, parameter.max, bar_width);
        let selected = app.effect_controls.focused == Some(Control::Number(index));
        row(
            f,
            app,
            inner,
            index as u16,
            Line::from(vec![
                Span::styled(
                    format!(
                        "{}{:<width$}",
                        if selected { "›" } else { " " },
                        parameter
                            .label
                            .chars()
                            .take(label_width.saturating_sub(1) as usize)
                            .collect::<String>(),
                        width = label_width.saturating_sub(1) as usize
                    ),
                    Style::default().fg(if selected { Color::White } else { TEXT_DIM }),
                ),
                Span::styled(
                    render_eighth_block_bar(units, bar_width),
                    Style::default().fg(ACCENT).bg(SURFACE),
                ),
                Span::raw(
                    app.effect_controls
                        .editing
                        .as_ref()
                        .filter(|(control, _)| *control == Control::Number(index))
                        .map(|(_, value)| format!(" {value}▏"))
                        .unwrap_or_else(|| format!(" {:>5.1}", parameter.value)),
                ),
            ]),
            Control::Number(index),
        );
        if let Some((rect, _)) = app.effect_controls.hits.last_mut() {
            rect.x += label_width;
            rect.width = bar_width as u16;
        }
        if index < inner.height as usize {
            let value_x = inner.x.saturating_add(label_width + bar_width as u16);
            app.effect_controls.hits.push((
                Rect::new(
                    value_x,
                    inner.y + index as u16,
                    inner.right().saturating_sub(value_x),
                    1,
                ),
                Control::EditNumber(index),
            ));
            app.effect_controls.hits.push((
                Rect::new(inner.x, inner.y + index as u16, label_width, 1),
                Control::Select(index),
            ));
        }
    }
    let start = parameters.len() as u16;
    match effect {
        ImageEffect::QuantizePalette {
            source, palette, ..
        } => {
            row(
                f,
                app,
                inner,
                start,
                Line::from(format!(
                    " [s] Apply to: {}  (click to switch)",
                    if source {
                        "source image"
                    } else {
                        "finished art"
                    }
                )),
                Control::Stage,
            );
            let text = app
                .effect_controls
                .editing
                .as_ref()
                .filter(|(c, _)| *c == Control::Palette)
                .map(|(_, s)| format!(" [p] Palette: {s}▏"))
                .unwrap_or_else(|| {
                    if palette.is_empty() {
                        " [p] Palette: automatic (enter hex colours to choose)".into()
                    } else {
                        format!(
                            " [p] Palette: {}",
                            palette
                                .iter()
                                .map(|&rgb| hex(rgb))
                                .collect::<Vec<_>>()
                                .join(" ")
                        )
                    }
                });
            let visible_colours = inner.width.saturating_sub(13).div_ceil(10).max(1) as usize;
            let first_colour = match app.effect_controls.focused {
                Some(Control::Colour(index)) => index.saturating_sub(visible_colours - 1),
                _ => 0,
            };
            let swatches = if app
                .effect_controls
                .editing
                .as_ref()
                .is_some_and(|(c, _)| *c == Control::Palette)
                || palette.is_empty()
            {
                vec![Span::raw(text)]
            } else {
                let mut spans = vec![Span::raw(" [p] Palette: ")];
                for (index, rgb) in palette
                    .iter()
                    .enumerate()
                    .skip(first_colour)
                    .take(visible_colours)
                {
                    spans.push(Span::styled(
                        "██",
                        Style::default().fg(Color::Rgb(rgb[0], rgb[1], rgb[2])),
                    ));
                    let colour = app
                        .effect_controls
                        .editing
                        .as_ref()
                        .filter(|(c, _)| *c == Control::Colour(index))
                        .map(|(_, s)| format!("{s}▏ "))
                        .unwrap_or_else(|| format!("{} ", hex(*rgb)));
                    spans.push(Span::styled(
                        colour,
                        Style::default().bg(
                            if app.effect_controls.focused == Some(Control::Colour(index)) {
                                SELECTED_BG
                            } else {
                                SURFACE
                            },
                        ),
                    ));
                }
                spans
            };
            row(
                f,
                app,
                inner,
                start + 1,
                Line::from(swatches),
                Control::Palette,
            );
            let buttons = Rect::new(inner.x, inner.y + start + 2, inner.width, 1);
            if !palette.is_empty() && start + 1 < inner.height {
                for index in first_colour..palette.len().min(first_colour + visible_colours) {
                    let x = inner
                        .x
                        .saturating_add(13 + (index - first_colour) as u16 * 10);
                    if x >= inner.right() {
                        break;
                    }
                    app.effect_controls.hits.insert(
                        0,
                        (
                            Rect::new(x, inner.y + start + 1, 10.min(inner.right() - x), 1),
                            Control::Colour(index),
                        ),
                    );
                }
            }
            let split =
                Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .split(buttons);
            if start + 2 < inner.height {
                f.render_widget(
                    Paragraph::new(" [a] Automatic palette").style(Style::default().fg(ACCENT).bg(
                        if app.effect_controls.focused == Some(Control::Automatic) {
                            SELECTED_BG
                        } else {
                            SURFACE
                        },
                    )),
                    split[0],
                );
                f.render_widget(
                    Paragraph::new(" [u] Use current art").style(Style::default().fg(ACCENT).bg(
                        if app.effect_controls.focused == Some(Control::Current) {
                            SELECTED_BG
                        } else {
                            SURFACE
                        },
                    )),
                    split[1],
                );
                app.effect_controls
                    .hits
                    .extend([(split[0], Control::Automatic), (split[1], Control::Current)]);
            }
            row(
                f,
                app,
                inner,
                start + 3,
                Line::from(" [i] Pick palette colours from image (Esc finishes)"),
                Control::PickPalette,
            );
            row(
                f,
                app,
                inner,
                start + 4,
                Line::from(if palette.is_empty() {
                    " Nearby shades merge first; 0 colours keeps all merged shades."
                } else {
                    " Palette overrides sliders; click edits, right click removes."
                }),
                Control::Palette,
            );
        }
        ImageEffect::ReplaceColour { from, to, .. } => {
            for (offset, (label, rgb, control, pick)) in [
                ("[f] From", from, Control::From, Control::PickFrom),
                ("[t] To", to, Control::To, Control::PickTo),
            ]
            .into_iter()
            .enumerate()
            {
                let text = app
                    .effect_controls
                    .editing
                    .as_ref()
                    .filter(|(c, _)| *c == control)
                    .map(|(_, s)| format!(" {label}: {s}▏"))
                    .unwrap_or_else(|| format!(" {label}: {}", hex(rgb)));
                row(
                    f,
                    app,
                    inner,
                    start + offset as u16,
                    Line::from(vec![
                        Span::raw(text),
                        Span::styled(
                            " ██",
                            Style::default().fg(Color::Rgb(rgb[0], rgb[1], rgb[2])),
                        ),
                    ]),
                    control,
                );
                if let Some((rect, _)) = app.effect_controls.hits.last_mut() {
                    rect.width = inner.width.saturating_sub(14);
                }
                let pick_rect = Rect::new(
                    inner.right().saturating_sub(14),
                    inner.y + start + offset as u16,
                    14.min(inner.width),
                    1,
                );
                if start + (offset as u16) < inner.height {
                    f.render_widget(
                        Paragraph::new(" [ Pick art ]").style(Style::default().fg(ACCENT).bg(
                            if app.effect_controls.focused == Some(pick) {
                                SELECTED_BG
                            } else {
                                SURFACE
                            },
                        )),
                        pick_rect,
                    );
                    app.effect_controls.hits.push((pick_rect, pick));
                }
            }
            row(
                f,
                app,
                inner,
                start + 2,
                Line::from(" Click a hex colour to edit; Pick art activates the eyedropper."),
                Control::PickFrom,
            );
            row(
                f,
                app,
                inner,
                start + 3,
                Line::from(" Eyedropper: left click = foreground, right click = background."),
                Control::PickFrom,
            );
        }
        ImageEffect::Crop { .. } => {
            row(
                f,
                app,
                inner,
                start,
                Line::from(" [i] Select crop area in image…"),
                Control::SelectCrop,
            );
            row(
                f,
                app,
                inner,
                start + 1,
                Line::from(" [r] Reset crop · Enter edits pixels for each side"),
                Control::ResetCrop,
            );
        }
        _ => {}
    }
    let message = if !app.effect_controls.error.is_empty() {
        app.effect_controls.error.clone()
    } else if app.effect_controls.editing.is_some() {
        " Enter applies · Tab applies and moves · Esc cancels".into()
    } else if app.effect_controls.sampling.is_some() {
        " Click a colour in the art · Esc cancels".into()
    } else {
        " Tab / Shift-Tab select input · arrows adjust · Enter edits · Esc = list".into()
    };
    if inner.height > 0 {
        f.render_widget(
            Paragraph::new(message).style(Style::default().fg(ACCENT_DIM)),
            Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
        );
    }
    parts[0]
}

fn source_rgb(image: &photon_rs::PhotonImage, x: u32, y: u32) -> [u8; 3] {
    let pixels = image.get_raw_pixels();
    let index = ((y.min(image.get_height() - 1) * image.get_width() + x.min(image.get_width() - 1))
        * 4) as usize;
    [pixels[index], pixels[index + 1], pixels[index + 2]]
}

pub(crate) fn render_source_picker(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    if app.current_tab != 1
        || !matches!(
            app.effect_controls.sampling,
            Some(Control::PickPalette | Control::SelectCrop)
        )
    {
        return;
    }
    let Some(image) = &app.effect_controls.sample_image else {
        return;
    };
    f.render_widget(Clear, area);
    let rgb = source_rgb(
        image,
        app.effect_controls.sample_cursor.0,
        app.effect_controls.sample_cursor.1,
    );
    let cropping = app.effect_controls.sampling == Some(Control::SelectCrop);
    let title = if cropping {
        app.effect_controls
            .crop_box
            .map(|[x0, y0, x1, y1]| {
                format!(
                    " Crop · {}×{} px · Enter applies · Esc cancels ",
                    x1 - x0,
                    y1 - y0
                )
            })
            .unwrap_or_else(|| {
                " Crop: drag area to keep · Enter applies · Esc cancels ".to_string()
            })
    } else {
        format!(" Pick image colours · {} · Esc finishes ", hex(rgb))
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT))
        .title(title);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width == 0 || inner.height < 2 {
        return;
    }
    let scale = (inner.width as f32 / image.get_width() as f32)
        .min((inner.height.saturating_sub(1) * 2) as f32 / image.get_height() as f32);
    let width = (image.get_width() as f32 * scale).floor().max(1.0) as u16;
    let height = (image.get_height() as f32 * scale / 2.0).ceil().max(1.0) as u16;
    let rect = Rect::new(
        inner.x + (inner.width - width) / 2,
        inner.y,
        width,
        height.min(inner.height - 1),
    );
    app.effect_controls.sample_area = rect;
    let pixels = image.get_raw_pixels();
    let pixel = |x: u16, y: u16| {
        let sx =
            (u32::from(x) * image.get_width() / u32::from(rect.width)).min(image.get_width() - 1);
        let sy = (u32::from(y) * image.get_height() / (u32::from(rect.height) * 2))
            .min(image.get_height() - 1);
        let index = ((sy * image.get_width() + sx) * 4) as usize;
        let outside = cropping
            && app
                .effect_controls
                .crop_box
                .is_some_and(|[x0, y0, x1, y1]| sx < x0 || sy < y0 || sx >= x1 || sy >= y1);
        let divisor = if outside { 4 } else { 1 };
        Color::Rgb(
            pixels[index] / divisor,
            pixels[index + 1] / divisor,
            pixels[index + 2] / divisor,
        )
    };
    for y in 0..rect.height {
        let spans: Vec<_> = (0..rect.width)
            .map(|x| {
                let cursor_x =
                    app.effect_controls.sample_cursor.0 * u32::from(rect.width) / image.get_width();
                let cursor_y = app.effect_controls.sample_cursor.1 * u32::from(rect.height)
                    / image.get_height();
                let border = if cropping {
                    app.effect_controls.crop_box.and_then(|[x0, y0, x1, y1]| {
                        let x0 = x0 * u32::from(rect.width) / image.get_width();
                        let y0 = y0 * u32::from(rect.height) / image.get_height();
                        let x1 = (x1 * u32::from(rect.width))
                            .div_ceil(image.get_width())
                            .saturating_sub(1);
                        let y1 = (y1 * u32::from(rect.height))
                            .div_ceil(image.get_height())
                            .saturating_sub(1);
                        let (x, y) = (u32::from(x), u32::from(y));
                        if x < x0 || x > x1 || y < y0 || y > y1 {
                            None
                        } else if y == y0 || y == y1 {
                            Some("─")
                        } else if x == x0 || x == x1 {
                            Some("│")
                        } else {
                            None
                        }
                    })
                } else {
                    None
                };
                if let Some(border) = border {
                    Span::styled(
                        border,
                        Style::default().fg(Color::White).bg(pixel(x, y * 2 + 1)),
                    )
                } else if !cropping && u32::from(x) == cursor_x && u32::from(y) == cursor_y {
                    Span::styled(
                        "+",
                        Style::default().fg(Color::White).bg(pixel(x, y * 2 + 1)),
                    )
                } else {
                    Span::styled(
                        "▀",
                        Style::default().fg(pixel(x, y * 2)).bg(pixel(x, y * 2 + 1)),
                    )
                }
            })
            .collect();
        f.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect::new(rect.x, rect.y + y, rect.width, 1),
        );
    }
    let message = if cropping {
        "Drag selects · arrows move · Shift + arrows resize · Enter applies"
    } else {
        "Click to add · arrows move · Enter picks · Esc returns to controls"
    };
    f.render_widget(
        Paragraph::new(message).style(Style::default().fg(ACCENT)),
        Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
    );
}

fn add_picked_colour(app: &mut App) {
    let Some(image) = &app.effect_controls.sample_image else {
        return;
    };
    let rgb = source_rgb(
        image,
        app.effect_controls.sample_cursor.0,
        app.effect_controls.sample_cursor.1,
    );
    if let Some(node) = app.pipeline_nodes.get_mut(app.selected_control) {
        if let ImageEffect::QuantizePalette { palette, .. } = &mut node.effect {
            if !palette.contains(&rgb) {
                if palette.len() == 256 {
                    app.effect_controls.error = "Palette is full (256 colours)".into();
                    return;
                }
                palette.push(rgb);
                app.mark_args_dirty();
            }
        }
    }
}

fn handle_source_picker(app: &mut App, event: &Event) -> bool {
    let Some(image) = &app.effect_controls.sample_image else {
        return false;
    };
    let (width, height) = (image.get_width(), image.get_height());
    match event {
        Event::Key(key) if key.modifiers.contains(KeyModifiers::ALT) => {
            app.effect_controls.sampling = None;
            app.effect_controls.sample_image = None;
            focus(app, None);
            return false;
        }
        Event::Key(key) if key.kind == KeyEventKind::Press => {
            let amount = if key.modifiers.contains(KeyModifiers::SHIFT) {
                10
            } else {
                1
            };
            let (x, y) = &mut app.effect_controls.sample_cursor;
            match key.code {
                KeyCode::Left => *x = x.saturating_sub(amount),
                KeyCode::Right => *x = x.saturating_add(amount).min(width - 1),
                KeyCode::Up => *y = y.saturating_sub(amount),
                KeyCode::Down => *y = y.saturating_add(amount).min(height - 1),
                KeyCode::Enter | KeyCode::Char(' ') => add_picked_colour(app),
                _ => {}
            }
            true
        }
        Event::Mouse(mouse)
            if matches!(
                mouse.kind,
                MouseEventKind::Down(MouseButton::Left | MouseButton::Right)
            ) =>
        {
            let area = app.effect_controls.sample_area;
            if !area.contains(ratatui::layout::Position::new(mouse.column, mouse.row)) {
                app.effect_controls.sampling = None;
                app.effect_controls.sample_image = None;
                return false;
            }
            let x = u32::from(mouse.column - area.x) * width / u32::from(area.width);
            let lower = u32::from(mouse.kind == MouseEventKind::Down(MouseButton::Right));
            let y =
                (u32::from(mouse.row - area.y) * 2 + lower) * height / (u32::from(area.height) * 2);
            app.effect_controls.sample_cursor = (x.min(width - 1), y.min(height - 1));
            add_picked_colour(app);
            true
        }
        _ => false,
    }
}

fn handle_crop_picker(app: &mut App, event: &Event) -> bool {
    let Some(image) = &app.effect_controls.sample_image else {
        return false;
    };
    let (width, height) = (image.get_width(), image.get_height());
    let mut apply = false;
    match event {
        Event::Mouse(mouse)
            if matches!(
                mouse.kind,
                MouseEventKind::Down(MouseButton::Left)
                    | MouseEventKind::Drag(MouseButton::Left)
                    | MouseEventKind::Up(MouseButton::Left)
            ) =>
        {
            let area = app.effect_controls.sample_area;
            if area.width == 0 || area.height == 0 {
                return true;
            }
            if mouse.kind == MouseEventKind::Down(MouseButton::Left)
                && !area.contains(ratatui::layout::Position::new(mouse.column, mouse.row))
            {
                return false;
            }
            let x = (u32::from(mouse.column.saturating_sub(area.x)).min(u32::from(area.width))
                * width
                / u32::from(area.width))
            .min(width);
            let y = (u32::from(mouse.row.saturating_sub(area.y)).min(u32::from(area.height))
                * height
                / u32::from(area.height))
            .min(height);
            if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                app.effect_controls.crop_drag = Some((x.min(width - 1), y.min(height - 1)));
            }
            if let Some((x0, y0)) = app.effect_controls.crop_drag {
                app.effect_controls.crop_box = Some([
                    x.min(x0),
                    y.min(y0),
                    x.max(x0).max(x0 + 1),
                    y.max(y0).max(y0 + 1),
                ]);
            }
            if mouse.kind == MouseEventKind::Up(MouseButton::Left) {
                app.effect_controls.crop_drag = None;
            }
        }
        Event::Key(key) if key.kind == KeyEventKind::Press => {
            if key.modifiers.contains(KeyModifiers::ALT) {
                app.effect_controls.sampling = None;
                app.effect_controls.sample_image = None;
                return false;
            }
            if key.code == KeyCode::Enter {
                apply = true;
            }
            if let Some(rect) = &mut app.effect_controls.crop_box {
                let direction = match key.code {
                    KeyCode::Left => (-1, 0),
                    KeyCode::Right => (1, 0),
                    KeyCode::Up => (0, -1),
                    KeyCode::Down => (0, 1),
                    _ => (0, 0),
                };
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    rect[2] = (rect[2] as i64 + direction.0)
                        .clamp((rect[0] + 1) as i64, width as i64)
                        as u32;
                    rect[3] = (rect[3] as i64 + direction.1)
                        .clamp((rect[1] + 1) as i64, height as i64)
                        as u32;
                } else {
                    let dx = direction
                        .0
                        .clamp(-(rect[0] as i64), (width - rect[2]) as i64);
                    let dy = direction
                        .1
                        .clamp(-(rect[1] as i64), (height - rect[3]) as i64);
                    rect[0] = (rect[0] as i64 + dx) as u32;
                    rect[2] = (rect[2] as i64 + dx) as u32;
                    rect[1] = (rect[1] as i64 + dy) as u32;
                    rect[3] = (rect[3] as i64 + dy) as u32;
                }
            }
        }
        _ => {}
    }
    if apply {
        if let Some([x0, y0, x1, y1]) = app.effect_controls.crop_box {
            if let Some(node) = app.pipeline_nodes.get_mut(app.selected_control) {
                node.effect = ImageEffect::Crop {
                    top: y0,
                    right: width - x1,
                    bottom: height - y1,
                    left: x0,
                };
                app.mark_args_dirty();
            }
        }
        app.effect_controls.sampling = None;
        app.effect_controls.sample_image = None;
    }
    true
}

fn activate(app: &mut App, control: Control) {
    let control = match control {
        Control::EditNumber(index) => Control::Number(index),
        other => other,
    };
    if !app
        .pipeline_nodes
        .get(app.selected_control)
        .is_some_and(|node| controls(&node.effect).contains(&control))
    {
        return;
    }
    focus(app, Some(control));
    if matches!(control, Control::PickPalette | Control::SelectCrop) {
        let cached = app.source_preview.lock().ok().and_then(|slot| {
            slot.as_ref()
                .filter(|(path, _)| Some(path) == app.render_args.image.as_ref())
                .map(|(_, image)| (**image).clone())
        });
        let preview = cached.map(Ok).unwrap_or_else(|| {
            app.render_args
                .image
                .as_deref()
                .ok_or_else(|| "Open an image first".to_string())
                .and_then(|path| std::fs::read(path).map_err(|error| error.to_string()))
                .and_then(|bytes| crate::generator::decode_image(&bytes))
        });
        match preview {
            Ok(image) => {
                let pipeline: Vec<_> = app
                    .pipeline_nodes
                    .iter()
                    .take(app.selected_control)
                    .filter(|node| !node.disabled && !node.effect.is_output_colour())
                    .map(|node| node.effect.clone())
                    .collect();
                let image = crate::effects::apply_pipeline(&app.render_args, image, &pipeline);
                if let Some(ImageEffect::Crop {
                    top,
                    right,
                    bottom,
                    left,
                }) = app
                    .pipeline_nodes
                    .get(app.selected_control)
                    .map(|node| &node.effect)
                {
                    let x0 = (*left).min(image.get_width() - 1);
                    let y0 = (*top).min(image.get_height() - 1);
                    app.effect_controls.crop_box = Some([
                        x0,
                        y0,
                        image.get_width().saturating_sub(*right).max(x0 + 1),
                        image.get_height().saturating_sub(*bottom).max(y0 + 1),
                    ]);
                }
                app.effect_controls.sample_image = Some(image);
                app.effect_controls.sample_cursor = (0, 0);
                app.effect_controls.sampling = Some(control);
                app.effect_controls.error.clear();
            }
            Err(error) => app.effect_controls.error = format!("Cannot open source picker: {error}"),
        }
        return;
    }
    let Some(node) = app.pipeline_nodes.get_mut(app.selected_control) else {
        return;
    };
    node.effect.upgrade_quantizer();
    app.effect_controls.error.clear();
    let mut dirty = false;
    match (&mut node.effect, control) {
        (effect, Control::Number(index)) => {
            if let Some(parameter) = effect.parameters().get(index) {
                app.effect_controls.editing = Some((control, parameter.value.to_string()));
            }
        }
        (
            ImageEffect::Crop {
                top,
                right,
                bottom,
                left,
            },
            Control::ResetCrop,
        ) => {
            (*top, *right, *bottom, *left) = (0, 0, 0, 0);
            dirty = true;
        }
        (ImageEffect::ReplaceColour { from, .. }, Control::From) => {
            app.effect_controls.editing = Some((control, hex(*from)))
        }
        (ImageEffect::ReplaceColour { to, .. }, Control::To) => {
            app.effect_controls.editing = Some((control, hex(*to)))
        }
        (ImageEffect::ReplaceColour { .. }, Control::PickFrom | Control::PickTo) => {
            app.effect_controls.sampling = Some(control)
        }
        (ImageEffect::QuantizePalette { palette, .. }, Control::Palette) => {
            app.effect_controls.editing = Some((
                control,
                palette
                    .iter()
                    .map(|&rgb| hex(rgb))
                    .collect::<Vec<_>>()
                    .join(" "),
            ))
        }
        (ImageEffect::QuantizePalette { source, .. }, Control::Stage) => {
            *source = !*source;
            dirty = true;
        }
        (ImageEffect::QuantizePalette { palette, .. }, Control::Colour(index)) => {
            if let Some(rgb) = palette.get(index) {
                app.effect_controls.editing = Some((control, hex(*rgb)));
            }
        }
        (ImageEffect::QuantizePalette { palette, .. }, Control::Automatic) => {
            palette.clear();
            dirty = true;
        }
        (_, Control::Current) => {
            let colours = current_palette(app);
            if colours.is_empty() {
                app.effect_controls.error = "Render the art first to use its palette".into();
                return;
            }
            if let Some(node) = app.pipeline_nodes.get_mut(app.selected_control) {
                if let ImageEffect::QuantizePalette { palette, .. } = &mut node.effect {
                    *palette = colours;
                    dirty = true;
                }
            }
        }
        _ => {}
    }
    if dirty {
        app.mark_args_dirty();
    }
}

fn slide(app: &mut App, index: usize, column: u16) {
    let Some((area, _)) = app
        .effect_controls
        .hits
        .iter()
        .find(|(_, c)| *c == Control::Number(index))
    else {
        return;
    };
    let ratio =
        f32::from(column.saturating_sub(area.x)) / f32::from(area.width.saturating_sub(1).max(1));
    let Some(parameter) = app
        .pipeline_nodes
        .get(app.selected_control)
        .and_then(|node| parameters(app, &node.effect).get(index).cloned())
    else {
        return;
    };
    let Some(node) = app.pipeline_nodes.get_mut(app.selected_control) else {
        return;
    };
    node.effect.set_parameter(
        index,
        parameter.min + ratio.clamp(0.0, 1.0) * (parameter.max - parameter.min),
    );
    app.effect_controls.parameter = index;
    app.effect_controls.focused = Some(Control::Number(index));
    app.mark_args_dirty();
}

pub(crate) fn handle_effect_controls(app: &mut App, event: &Event) -> bool {
    if app.current_tab != 1 || app.pipeline_expanded.is_some() {
        return false;
    }
    if matches!(event, Event::Mouse(mouse) if matches!(mouse.kind, MouseEventKind::Down(_)))
        || matches!(event, Event::Key(key) if key.modifiers.contains(KeyModifiers::ALT))
    {
        app.effect_controls.editing = None;
        app.effect_controls.error.clear();
    }
    if let Some((control, buffer)) = app.effect_controls.editing.clone() {
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Char('u' | 'a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if let Some((_, value)) = &mut app.effect_controls.editing {
                        value.clear();
                    }
                }
                KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if let Ok(text) =
                        arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text())
                    {
                        if let Some((_, value)) = &mut app.effect_controls.editing {
                            value.push_str(&text);
                        }
                    }
                }
                KeyCode::Esc => {
                    app.effect_controls.editing = None;
                    app.effect_controls.error.clear();
                }
                KeyCode::Enter | KeyCode::Tab | KeyCode::BackTab
                    if matches!(control, Control::Number(_)) =>
                {
                    if let Some(value) =
                        buffer.parse::<f32>().ok().filter(|value| value.is_finite())
                    {
                        let Control::Number(index) = control else {
                            unreachable!()
                        };
                        if let Some(node) = app.pipeline_nodes.get_mut(app.selected_control) {
                            node.effect.set_parameter(index, value);
                        }
                        app.effect_controls.editing = None;
                        app.effect_controls.error.clear();
                        app.mark_args_dirty();
                        if key.code != KeyCode::Enter {
                            move_focus(app, if key.code == KeyCode::BackTab { -1 } else { 1 });
                        }
                    } else {
                        app.effect_controls.error =
                            "Enter a finite number, or Esc to cancel".into();
                    }
                }
                KeyCode::Enter | KeyCode::Tab | KeyCode::BackTab => match parse_palette(&buffer) {
                    Ok(colours) if control == Control::Palette || colours.len() == 1 => {
                        if let Some(node) = app.pipeline_nodes.get_mut(app.selected_control) {
                            match (&mut node.effect, control) {
                                (
                                    ImageEffect::QuantizePalette { palette, .. },
                                    Control::Palette,
                                ) => *palette = colours,
                                (
                                    ImageEffect::QuantizePalette { palette, .. },
                                    Control::Colour(index),
                                ) => {
                                    if let Some(rgb) = palette.get_mut(index) {
                                        *rgb = colours[0];
                                    }
                                }
                                (ImageEffect::ReplaceColour { from, .. }, Control::From) => {
                                    *from = colours[0]
                                }
                                (ImageEffect::ReplaceColour { to, .. }, Control::To) => {
                                    *to = colours[0]
                                }
                                _ => {}
                            }
                        }
                        app.effect_controls.editing = None;
                        app.effect_controls.error.clear();
                        app.mark_args_dirty();
                        if key.code != KeyCode::Enter {
                            move_focus(app, if key.code == KeyCode::BackTab { -1 } else { 1 });
                        }
                    }
                    Ok(_) => app.effect_controls.error = "Enter one #RRGGBB colour".into(),
                    Err(error) => app.effect_controls.error = error,
                },
                KeyCode::Backspace => {
                    if let Some((_, value)) = &mut app.effect_controls.editing {
                        value.pop();
                    }
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if let Some((_, value)) = &mut app.effect_controls.editing {
                        value.push(c);
                    }
                }
                _ => {}
            },
            Event::Paste(text) => {
                if let Some((_, value)) = &mut app.effect_controls.editing {
                    value.push_str(text);
                }
            }
            _ => {}
        }
        // Invalid input must not trap focus. Tab leaves its previous value
        // intact and advances; Enter keeps the editor open for correction.
        if let Event::Key(key) = event {
            if key.kind == KeyEventKind::Press
                && matches!(key.code, KeyCode::Tab | KeyCode::BackTab)
                && app.effect_controls.editing.is_some()
            {
                app.effect_controls.editing = None;
                app.effect_controls.error.push_str(" · previous value kept");
                move_focus(app, if key.code == KeyCode::BackTab { -1 } else { 1 });
            }
        }
        return true;
    }
    if app.effect_controls.sampling.is_some() {
        if matches!(event, Event::Key(key) if matches!(key.code, KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab))
        {
            app.effect_controls.sampling = None;
            app.effect_controls.sample_image = None;
            if let Event::Key(key) = event {
                if key.code != KeyCode::Esc {
                    move_focus(app, if key.code == KeyCode::BackTab { -1 } else { 1 });
                }
            }
            return true;
        }
        if app.effect_controls.sampling == Some(Control::SelectCrop) {
            return handle_crop_picker(app, event);
        }
        if app.effect_controls.sampling == Some(Control::PickPalette) {
            return handle_source_picker(app, event);
        }
        if let Event::Mouse(mouse) = event {
            if matches!(
                mouse.kind,
                MouseEventKind::Down(MouseButton::Left | MouseButton::Right)
            ) && app
                .output_area
                .contains(ratatui::layout::Position::new(mouse.column, mouse.row))
            {
                if let Some((x, y)) = output_grid_position(app, mouse.column, mouse.row) {
                    if let Some(cell) = app
                        .cached_result
                        .as_ref()
                        .and_then(|result| result.grid.get(y as usize))
                        .and_then(|row| row.get(x as usize))
                    {
                        let rgb = if mouse.kind == MouseEventKind::Down(MouseButton::Right) {
                            cell.bg.unwrap_or([0; 3])
                        } else {
                            cell.fg
                        };
                        let target = app.effect_controls.sampling.take();
                        if let Some(node) = app.pipeline_nodes.get_mut(app.selected_control) {
                            if let ImageEffect::ReplaceColour { from, to, .. } = &mut node.effect {
                                if target == Some(Control::PickFrom) {
                                    *from = rgb;
                                } else {
                                    *to = rgb;
                                }
                                app.mark_args_dirty();
                            }
                        }
                    }
                }
                return true;
            }
        }
    }
    match event {
        Event::Mouse(mouse) => match mouse.kind {
            MouseEventKind::Down(MouseButton::Right) => {
                let pos = ratatui::layout::Position::new(mouse.column, mouse.row);
                if let Some((_, Control::Colour(index))) = app
                    .effect_controls
                    .hits
                    .iter()
                    .find(|(area, _)| area.contains(pos))
                    .copied()
                {
                    if let Some(node) = app.pipeline_nodes.get_mut(app.selected_control) {
                        if let ImageEffect::QuantizePalette { palette, .. } = &mut node.effect {
                            palette.remove(index);
                            app.mark_args_dirty();
                        }
                    }
                    return true;
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                app.effect_controls.dragging = None;
            }
            MouseEventKind::Drag(MouseButton::Left) if app.effect_controls.dragging.is_some() => {
                slide(app, app.effect_controls.dragging.unwrap(), mouse.column);
                return true;
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let pos = ratatui::layout::Position::new(mouse.column, mouse.row);
                if let Some((_, control)) = app
                    .effect_controls
                    .hits
                    .iter()
                    .find(|(area, _)| area.contains(pos))
                    .copied()
                {
                    match control {
                        Control::Number(index) => {
                            app.effect_controls.dragging = Some(index);
                            slide(app, index, mouse.column);
                        }
                        Control::Select(index) => focus(app, Some(Control::Number(index))),
                        _ => activate(app, control),
                    }
                    return true;
                }
            }
            _ => {}
        },
        Event::Key(key) if key.kind == KeyEventKind::Press => {
            if key
                .modifiers
                .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL)
            {
                return false;
            }
            match key.code {
                KeyCode::Tab | KeyCode::BackTab => {
                    move_focus(app, if key.code == KeyCode::BackTab { -1 } else { 1 });
                    return true;
                }
                KeyCode::Esc if app.effect_controls.focused.is_some() => {
                    focus(app, None);
                    return true;
                }
                KeyCode::Up | KeyCode::Down if app.effect_controls.focused.is_some() => {
                    move_focus(app, if key.code == KeyCode::Up { -1 } else { 1 });
                    return true;
                }
                KeyCode::Left | KeyCode::Right if app.effect_controls.focused.is_some() => {
                    if let Some(Control::Number(index)) = app.effect_controls.focused {
                        app.effect_controls.parameter = index;
                        let amount = if key.modifiers.contains(KeyModifiers::SHIFT) {
                            10.0
                        } else {
                            1.0
                        };
                        adjust_selected_pipeline_param(
                            app,
                            if key.code == KeyCode::Left {
                                -amount
                            } else {
                                amount
                            },
                        );
                    } else if app.effect_controls.focused == Some(Control::Stage) {
                        activate(app, Control::Stage);
                    }
                    return true;
                }
                KeyCode::Enter | KeyCode::Char(' ') if app.effect_controls.focused.is_some() => {
                    activate(app, app.effect_controls.focused.unwrap());
                    return true;
                }
                KeyCode::Delete
                    if matches!(app.effect_controls.focused, Some(Control::Colour(_))) =>
                {
                    let Some(Control::Colour(index)) = app.effect_controls.focused else {
                        unreachable!()
                    };
                    if let Some(node) = app.pipeline_nodes.get_mut(app.selected_control) {
                        if let ImageEffect::QuantizePalette { palette, .. } = &mut node.effect {
                            if index < palette.len() {
                                palette.remove(index);
                            }
                        }
                    }
                    focus(app, Some(Control::Palette));
                    app.mark_args_dirty();
                    return true;
                }
                KeyCode::Char('[' | ']') => {
                    let count = app
                        .pipeline_nodes
                        .get(app.selected_control)
                        .map_or(0, |n| n.effect.parameters().len());
                    if count > 0 {
                        app.effect_controls.parameter = (app.effect_controls.parameter
                            + if key.code == KeyCode::Char(']') {
                                1
                            } else {
                                count - 1
                            })
                            % count;
                        app.effect_controls.focused =
                            Some(Control::Number(app.effect_controls.parameter));
                    }
                    return true;
                }
                KeyCode::Char('p') => activate(app, Control::Palette),
                KeyCode::Char('f' | 'c') => activate(app, Control::From),
                KeyCode::Char('t') => activate(app, Control::To),
                KeyCode::Char('s') => activate(app, Control::Stage),
                KeyCode::Char('a') => activate(app, Control::Automatic),
                KeyCode::Char('u') => activate(app, Control::Current),
                KeyCode::Char('i') => {
                    let crop = matches!(
                        app.pipeline_nodes
                            .get(app.selected_control)
                            .map(|node| &node.effect),
                        Some(ImageEffect::Crop { .. })
                    );
                    activate(
                        app,
                        if crop {
                            Control::SelectCrop
                        } else {
                            Control::PickPalette
                        },
                    );
                }
                KeyCode::Char('r')
                    if matches!(
                        app.pipeline_nodes
                            .get(app.selected_control)
                            .map(|n| &n.effect),
                        Some(ImageEffect::Crop { .. })
                    ) =>
                {
                    activate(app, Control::ResetCrop)
                }
                _ => return false,
            }
            return true;
        }
        _ => {}
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn app(effect: ImageEffect) -> App {
        let parsed = crate::args::Args::parse_from(["img2irc"]);
        let args = crate::args_to_render_args(&parsed);
        let flag = Arc::new(AtomicBool::new(false));
        let (tx, _rx) = watch::channel((args.clone(), flag.clone(), Instant::now()));
        let (_tx, rx) = mpsc::channel(1);
        let mut app = App::new(args, tx, rx, flag);
        app.pipeline_nodes = vec![crate::pipeline::ImageEffectNode {
            id: 1,
            effect,
            disabled: false,
        }];
        app.pipeline_selected = Some(1);
        app.selected_control = 0;
        app.current_tab = 1;
        app
    }

    fn render(app: &mut App) {
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| {
                render_active_pipeline_list(frame, app, Rect::new(0, 0, 54, 22));
            })
            .unwrap();
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(crossterm::event::KeyEvent::new(code, modifiers))
    }

    #[test]
    fn mouse_and_keyboard_adjust_the_second_slider_without_changing_the_first() {
        let mut app = app(ImageEffect::Oil(3, 55.0));
        render(&mut app);
        let (slider, _) = app
            .effect_controls
            .hits
            .iter()
            .find(|(_, c)| *c == Control::Number(1))
            .copied()
            .unwrap();
        assert!(handle_effect_controls(
            &mut app,
            &Event::Mouse(crossterm::event::MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: slider.right() - 1,
                row: slider.y,
                modifiers: KeyModifiers::NONE
            })
        ));
        assert_eq!(app.pipeline_nodes[0].effect, ImageEffect::Oil(3, 255.0));
        adjust_selected_pipeline_param(&mut app, -1.0);
        assert_eq!(app.pipeline_nodes[0].effect, ImageEffect::Oil(3, 254.5));
        assert!(app.pipeline_expanded.is_none());
    }

    #[test]
    fn palette_editing_applies_inline_and_escape_leaves_the_effect_unchanged() {
        let mut app = app(ImageEffect::QuantizeColours(8));
        render(&mut app);
        assert!(app.pipeline_nodes[0].effect.is_output_colour());
        handle_effect_controls(&mut app, &key(KeyCode::Char('p'), KeyModifiers::NONE));
        handle_effect_controls(&mut app, &Event::Paste("#ffffff #000000".into()));
        handle_effect_controls(&mut app, &key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            matches!(&app.pipeline_nodes[0].effect, ImageEffect::QuantizePalette { palette, .. } if palette == &vec![[255; 3], [0; 3]])
        );
        let saved = app.pipeline_nodes[0].effect.clone();
        handle_effect_controls(&mut app, &key(KeyCode::Char('p'), KeyModifiers::NONE));
        handle_effect_controls(&mut app, &key(KeyCode::Char('u'), KeyModifiers::CONTROL));
        handle_effect_controls(&mut app, &Event::Paste("#ff0000".into()));
        handle_effect_controls(&mut app, &key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.pipeline_nodes[0].effect, saved);
        handle_effect_controls(&mut app, &key(KeyCode::Char('s'), KeyModifiers::NONE));
        assert!(!app.pipeline_nodes[0].effect.is_output_colour());
        render(&mut app);
        let (swatch, _) = app
            .effect_controls
            .hits
            .iter()
            .find(|(_, c)| *c == Control::Colour(0))
            .copied()
            .unwrap();
        let click = |kind| {
            Event::Mouse(crossterm::event::MouseEvent {
                kind,
                column: swatch.x,
                row: swatch.y,
                modifiers: KeyModifiers::NONE,
            })
        };
        handle_effect_controls(&mut app, &click(MouseEventKind::Down(MouseButton::Left)));
        handle_effect_controls(&mut app, &key(KeyCode::Char('u'), KeyModifiers::CONTROL));
        handle_effect_controls(&mut app, &Event::Paste("#ff0000".into()));
        handle_effect_controls(&mut app, &key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            matches!(&app.pipeline_nodes[0].effect, ImageEffect::QuantizePalette { palette, .. } if palette == &vec![[255, 0, 0], [0; 3]])
        );
        render(&mut app);
        handle_effect_controls(&mut app, &click(MouseEventKind::Down(MouseButton::Right)));
        assert!(
            matches!(&app.pipeline_nodes[0].effect, ImageEffect::QuantizePalette { palette, .. } if palette == &vec![[0; 3]])
        );
    }

    #[test]
    fn replacement_hex_edits_and_background_sampling_stay_in_the_editor() {
        let mut app = app(ImageEffect::ReplaceColour {
            from: [255; 3],
            to: [0; 3],
            tolerance: 5.0,
        });
        render(&mut app);
        handle_effect_controls(&mut app, &key(KeyCode::Char('t'), KeyModifiers::NONE));
        handle_effect_controls(&mut app, &key(KeyCode::Char('u'), KeyModifiers::CONTROL));
        handle_effect_controls(&mut app, &Event::Paste("#123456".into()));
        handle_effect_controls(&mut app, &key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(
            app.pipeline_nodes[0].effect,
            ImageEffect::ReplaceColour {
                to: [18, 52, 86],
                ..
            }
        ));
        let mut result = draw::RenderResult::from("");
        result.grid = vec![vec![draw::RenderedCell {
            char: '▀',
            fg: [255, 0, 0],
            bg: Some([0, 255, 0]),
            inverted: false,
            bold: false,
            italic: false,
            underline: false,
        }]];
        app.cached_result = Some(result);
        app.output_area = Rect::new(60, 0, 3, 3);
        app.effect_controls.sampling = Some(Control::PickFrom);
        assert!(handle_effect_controls(
            &mut app,
            &Event::Mouse(crossterm::event::MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Right),
                column: 61,
                row: 1,
                modifiers: KeyModifiers::NONE
            })
        ));
        assert!(matches!(
            app.pipeline_nodes[0].effect,
            ImageEffect::ReplaceColour {
                from: [0, 255, 0],
                ..
            }
        ));
        assert!(app.effect_controls.sampling.is_none());
    }

    #[test]
    fn inline_palette_accepts_hex_deduplicates_and_rejects_bad_input() {
        assert_eq!(
            parse_palette("#ff0000, 00ff00; #FF0000").unwrap(),
            vec![[255, 0, 0], [0, 255, 0]]
        );
        assert!(parse_palette("#xyz000").is_err());
        assert!(parse_palette("12345").is_err());
        assert!(parse_palette("").unwrap().is_empty());
    }
}
