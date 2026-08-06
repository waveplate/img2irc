#![allow(deprecated)]

use crate::palette;
use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::widgets::*;
use std::f64;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaletteMode {
    Irc,
    Ansi,
    Ansi24,
}

fn main_area_dimensions(mode: PaletteMode) -> (u16, u16) {
    match mode {
        PaletteMode::Irc => (24, 7),
        // The ordered ANSI palette has 12 columns and 20 logical rows. Two
        // logical rows share each terminal row through upper-half blocks.
        PaletteMode::Ansi => (24, 10),
        PaletteMode::Ansi24 => (30, 16),
    }
}

fn ordered_ansi_indices() -> Vec<usize> {
    [0, 2, 4]
        .into_iter()
        .flat_map(|red_band| {
            let blue_levels: Vec<i32> = if red_band == 2 {
                (0..6).rev().collect()
            } else {
                (0..6).collect()
            };
            blue_levels.into_iter().flat_map(move |blue| {
                [0, 1, 2, 3, 4, 5, 11, 10, 9, 8, 7, 6]
                    .into_iter()
                    .map(move |red_green| {
                        let red = red_band + red_green / 6;
                        let green = red_green % 6;
                        (16 + 36 * red + 6 * green + blue) as usize
                    })
            })
        })
        .chain(232..244)
        .chain((244..=255).rev())
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    MainArea,
    HueSlider,
    InputR,
    InputG,
    InputB,
    InputHex,
    OkButton,
    CancelButton,
    FgSwatch,
    BgSwatch,
}

pub fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (f64, f64, f64) {
    let r = r as f64 / 255.0;
    let g = g as f64 / 255.0;
    let b = b as f64 / 255.0;

    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;

    let h = if delta == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / delta).rem_euclid(6.0))
    } else if max == g {
        60.0 * (((b - r) / delta) + 2.0)
    } else {
        60.0 * (((r - g) / delta) + 4.0)
    };

    let s = if max == 0.0 { 0.0 } else { delta / max };
    let v = max;

    (h, s, v)
}

pub fn hsv_to_rgb(h: f64, s: f64, v: f64) -> (u8, u8, u8) {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;

    let (r, g, b) = match (h / 60.0) as usize {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    (
        ((r + m) * 255.0).round() as u8,
        ((g + m) * 255.0).round() as u8,
        ((b + m) * 255.0).round() as u8,
    )
}

#[derive(Clone)]
pub struct ColorPickerState {
    pub mode: PaletteMode,
    pub focus: Focus,

    pub grid_idx: usize,

    pub hue: f64,        // 0-360
    pub saturation: f64, // 0-1
    pub value: f64,      // 0-1

    pub rgb: (u8, u8, u8),
    pub hex: String,

    pub is_dual: bool,
    pub editing_fg: bool, // true for fg, false for bg

    pub fg_idx: Option<usize>,
    pub bg_idx: Option<usize>,
    pub fg_rgb: (u8, u8, u8),
    pub bg_rgb: Option<(u8, u8, u8)>, // None means transparent/no-bg

    pub is_dragging_main: bool,
    pub is_dragging_hue: bool,
    pub is_open: bool,
    pub ok_trigger: bool,
}

impl Default for ColorPickerState {
    fn default() -> Self {
        Self::new(PaletteMode::Ansi24, true)
    }
}

impl ColorPickerState {
    pub fn new(mode: PaletteMode, is_dual: bool) -> Self {
        Self {
            mode,
            focus: Focus::MainArea,
            grid_idx: 0,
            hue: 0.0,
            saturation: 1.0,
            value: 1.0,
            rgb: (255, 0, 0),
            hex: "FF0000".to_string(),
            is_dual,
            editing_fg: true,
            fg_idx: None,
            bg_idx: None,
            fg_rgb: (255, 0, 0),
            bg_rgb: if is_dual { Some((0, 0, 0)) } else { None },
            is_dragging_main: false,
            is_dragging_hue: false,
            is_open: true,
            ok_trigger: false,
        }
    }

    pub fn set_rgb(&mut self, r: u8, g: u8, b: u8) {
        self.rgb = (r, g, b);
        let (h, s, v) = rgb_to_hsv(r, g, b);
        self.hue = h;
        self.saturation = s;
        self.value = v;
        self.hex = format!("{:02X}{:02X}{:02X}", r, g, b);
        self.update_current();
    }

    pub fn update_from_hsv(&mut self, h: f64, s: f64, v: f64) {
        self.hue = h;
        self.saturation = s;
        self.value = v;
        let (r, g, b) = hsv_to_rgb(h, s, v);
        self.rgb = (r, g, b);
        self.hex = format!("{:02X}{:02X}{:02X}", r, g, b);
        self.update_current();
    }

    pub fn set_grid_idx(&mut self, idx: usize) {
        self.grid_idx = idx;
        let (r, g, b) = self.rgb_from_idx(idx);
        self.rgb = (r, g, b);
        self.hex = format!("{:02X}{:02X}{:02X}", r, g, b);
        let (h, s, v) = rgb_to_hsv(r, g, b);
        self.hue = h;
        self.saturation = s;
        self.value = v;
        self.update_current();
    }

    /// Move the embedded picker's keyboard selection while staying within the
    /// visible palette/grid.
    pub fn move_selection(&mut self, dx: i32, dy: i32) {
        match self.mode {
            PaletteMode::Irc => {
                let idx = self.grid_idx.clamp(16, 98);
                let row = ((idx - 16) / 12) as i32;
                let col = ((idx - 16) % 12) as i32;
                let next_row = (row + dy).clamp(0, 6);
                let next_col = (col + dx).clamp(0, 11);
                self.set_grid_idx((16 + next_row * 12 + next_col).min(98) as usize);
            }
            PaletteMode::Ansi => {
                let indices = ordered_ansi_indices();
                let position = indices
                    .iter()
                    .position(|idx| *idx == self.grid_idx)
                    .unwrap_or(0);
                let row = (position / 12) as i32;
                let col = (position % 12) as i32;
                let next_row = (row + dy).clamp(0, 19);
                let next_col = (col + dx).clamp(0, 11);
                self.set_grid_idx(indices[(next_row * 12 + next_col) as usize]);
            }
            PaletteMode::Ansi24 => {
                let saturation =
                    (self.saturation + dx as f64 / 29.0).clamp(0.0, 1.0);
                let value = (self.value - dy as f64 / 15.0).clamp(0.0, 1.0);
                self.update_from_hsv(self.hue, saturation, value);
            }
        }
    }

    fn update_current(&mut self) {
        if self.editing_fg {
            self.fg_rgb = self.rgb;
            if self.mode == PaletteMode::Irc || self.mode == PaletteMode::Ansi {
                self.fg_idx = Some(self.grid_idx);
            } else {
                self.fg_idx = None;
            }
        } else {
            self.bg_rgb = Some(self.rgb);
            if self.mode == PaletteMode::Irc || self.mode == PaletteMode::Ansi {
                self.bg_idx = Some(self.grid_idx);
            } else {
                self.bg_idx = None;
            }
        }
    }

    pub fn rgb_from_idx(&self, idx: usize) -> (u8, u8, u8) {
        let val = if self.mode == PaletteMode::Irc {
            *palette::IRC99.get(idx).unwrap_or(&0)
        } else {
            *palette::ANSI256.get(idx).unwrap_or(&0)
        };
        (
            ((val >> 16) & 0xFF) as u8,
            ((val >> 8) & 0xFF) as u8,
            (val & 0xFF) as u8,
        )
    }

    /// Handle a press in the embedded picker. `target` overrides which channel
    /// receives the color (`Some(true)` = foreground, `Some(false)` =
    /// background); `None` preserves the channel selected in the surrounding
    /// text UI.
    pub fn handle_mouse_down(
        &mut self,
        x: u16,
        y: u16,
        area: Rect,
        target: Option<bool>,
    ) -> bool {
        if !area.contains(ratatui::layout::Position { x, y }) {
            return false;
        }
        let inner = Rect::new(
            area.x + 1,
            area.y + 1,
            area.width.saturating_sub(2),
            area.height.saturating_sub(2),
        );

        let (main_width, main_height) = main_area_dimensions(self.mode);
        let main_rect = Rect::new(inner.x + 1, inner.y + 1, main_width, main_height);
        let hue_rect = Rect::new(inner.x + main_width + 3, inner.y + 1, 3, main_height);

        if main_rect.contains(ratatui::layout::Position { x, y }) {
            self.focus = Focus::MainArea;
            if let Some(editing_fg) = target {
                self.editing_fg = editing_fg;
            }
            self.is_dragging_main = true;
            self.apply_main_area_click(x, y, main_rect);
            return true;
        }

        if self.mode == PaletteMode::Ansi24 && hue_rect.contains(ratatui::layout::Position { x, y })
        {
            self.focus = Focus::HueSlider;
            self.is_dragging_hue = true;
            self.apply_hue_click(y, hue_rect);
            return true;
        }

        let swatch_rect = Rect::new(inner.x + 40, inner.y + 1, 12, 6);
        if swatch_rect.contains(ratatui::layout::Position { x, y }) {
            self.focus = Focus::FgSwatch;
            if y < swatch_rect.y + 4 || !self.is_dual {
                self.editing_fg = true;
            } else {
                self.editing_fg = false;
                if self.bg_rgb.is_none() {
                    self.bg_rgb = Some((0, 0, 0));
                }
            }
            if self.editing_fg {
                self.set_rgb(self.fg_rgb.0, self.fg_rgb.1, self.fg_rgb.2);
            } else {
                if let Some(bg) = self.bg_rgb {
                    self.set_rgb(bg.0, bg.1, bg.2);
                }
            }
            return true;
        }

        let bg_none_btn = Rect::new(inner.x + 40, inner.y + 7, 10, 1);
        if self.is_dual
            && !self.editing_fg
            && bg_none_btn.contains(ratatui::layout::Position { x, y })
        {
            self.bg_rgb = None;
            self.bg_idx = None;
            return true;
        }

        // Buttons removed since it's an inline widget

        true
    }

    pub fn handle_mouse_drag(&mut self, x: u16, y: u16, area: Rect) -> bool {
        let inner = Rect::new(
            area.x + 1,
            area.y + 1,
            area.width.saturating_sub(2),
            area.height.saturating_sub(2),
        );
        let (main_width, main_height) = main_area_dimensions(self.mode);
        let main_rect = Rect::new(inner.x + 1, inner.y + 1, main_width, main_height);
        let hue_rect = Rect::new(inner.x + main_width + 3, inner.y + 1, 3, main_height);

        if self.is_dragging_main {
            self.apply_main_area_click(x, y, main_rect);
            return true;
        }
        if self.is_dragging_hue {
            self.apply_hue_click(y, hue_rect);
            return true;
        }
        false
    }

    fn apply_main_area_click(&mut self, x: u16, y: u16, rect: Rect) {
        let cx = x.clamp(rect.x, rect.x + rect.width.saturating_sub(1));
        let cy = y.clamp(rect.y, rect.y + rect.height.saturating_sub(1));

        let dx = cx - rect.x;
        let dy = cy - rect.y;

        if self.mode == PaletteMode::Irc {
            // IRC layout: 12 columns x 7 rows, starting at index 16. Full block (2 chars wide).
            if dx < 24 && dy < 7 {
                let col = dx / 2;
                let row = dy;
                let idx = 16 + row * 12 + col;
                if idx < 99 {
                    self.set_grid_idx(idx as usize);
                }
            }
        } else if self.mode == PaletteMode::Ansi {
            if dx < 24 && dy < 10 {
                let col = dx / 2;
                // Each two-character half-block represents an upper and lower
                // color. The left/right half selects the upper/lower color,
                // preserving mouse access despite the compact display.
                let logical_row = dy * 2 + dx % 2;
                if let Some(idx) = ordered_ansi_indices().get((logical_row * 12 + col) as usize) {
                    self.set_grid_idx(*idx);
                }
            }
        } else {
            let s = dx as f64 / rect.width.saturating_sub(1).max(1) as f64;
            let v = 1.0 - (dy as f64 / rect.height.saturating_sub(1).max(1) as f64);
            self.update_from_hsv(self.hue, s, v);
        }
    }

    fn apply_hue_click(&mut self, y: u16, rect: Rect) {
        let cy = y.clamp(rect.y, rect.y + rect.height.saturating_sub(1));
        let dy = cy - rect.y;
        let h = 360.0 * (1.0 - (dy as f64 / rect.height.saturating_sub(1).max(1) as f64));
        self.update_from_hsv(h, self.saturation, self.value);
    }
}

pub struct ColorPickerWidget<'a> {
    pub state: &'a mut ColorPickerState,
    pub show_details: bool,
}

impl<'a> Widget for ColorPickerWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let main_block = Block::default()
            .borders(Borders::ALL)
            .border_set(ratatui::symbols::border::THICK)
            .title(Span::styled(
                " Color Picker ",
                Style::default().add_modifier(Modifier::BOLD),
            ))
            .style(Style::default().bg(Color::Rgb(30, 30, 35)).fg(Color::Gray));
        let inner = main_block.inner(area);
        main_block.render(area, buf);

        let (main_width, main_height) = main_area_dimensions(self.state.mode);
        let main_rect = Rect::new(inner.x + 1, inner.y + 1, main_width, main_height);
        let hue_rect = Rect::new(inner.x + main_width + 3, inner.y + 1, 3, main_height);

        Block::default()
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::DarkGray))
            .render(
                Rect::new(
                    main_rect.x - 1,
                    main_rect.y - 1,
                    main_rect.width + 2,
                    main_rect.height + 2,
                ),
                buf,
            );

        if self.state.mode == PaletteMode::Irc {
            for row in 0..7 {
                for col in 0..12 {
                    let idx = 16 + row * 12 + col;
                    if idx >= 99 {
                        continue;
                    }
                    let (r, g, b) = self.state.rgb_from_idx(idx as usize);
                    let fg = Color::Rgb(r, g, b);

                    let x = main_rect.x + (col as u16 * 2);
                    let y = main_rect.y + row as u16;

                    buf.get_mut(x, y).set_char('█').set_fg(fg);
                    buf.get_mut(x + 1, y).set_char('█').set_fg(fg);

                    if idx as usize == self.state.grid_idx {
                        let contrast = if (r as u32 + g as u32 + b as u32) > 380 {
                            Color::Black
                        } else {
                            Color::White
                        };
                        buf.get_mut(x, y).set_char('X').set_fg(contrast).set_bg(fg);
                        buf.get_mut(x + 1, y)
                            .set_char('X')
                            .set_fg(contrast)
                            .set_bg(fg);
                    }
                }
            }
        } else if self.state.mode == PaletteMode::Ansi {
            let indices = ordered_ansi_indices();
            for row in 0..10 {
                for col in 0..12 {
                    let upper_idx = indices[(row * 2 * 12 + col) as usize];
                    let lower_idx = indices[((row * 2 + 1) * 12 + col) as usize];
                    let (upper_r, upper_g, upper_b) = self.state.rgb_from_idx(upper_idx);
                    let (lower_r, lower_g, lower_b) = self.state.rgb_from_idx(lower_idx);
                    let upper = Color::Rgb(upper_r, upper_g, upper_b);
                    let lower = Color::Rgb(lower_r, lower_g, lower_b);
                    let x = main_rect.x + col * 2;
                    let y = main_rect.y + row;

                    buf.get_mut(x, y).set_char('▀').set_fg(upper).set_bg(lower);
                    buf.get_mut(x + 1, y)
                        .set_char('▀')
                        .set_fg(upper)
                        .set_bg(lower);

                    if upper_idx == self.state.grid_idx {
                        buf.get_mut(x, y)
                            .set_char('▛')
                            .set_fg(Color::White)
                            .set_bg(upper);
                    }
                    if lower_idx == self.state.grid_idx {
                        buf.get_mut(x + 1, y)
                            .set_char('▙')
                            .set_fg(lower)
                            .set_bg(Color::White);
                    }
                }
            }
        } else {
            let h = self.state.hue;
            for row in 0..16 {
                let v = 1.0 - (row as f64 / 15.0);
                for col in 0..main_width {
                    let s = col as f64 / main_width.saturating_sub(1).max(1) as f64;
                    let (r, g, b) = hsv_to_rgb(h, s, v);
                    buf.get_mut(main_rect.x + col, main_rect.y + row)
                        .set_char('█')
                        .set_fg(Color::Rgb(r, g, b));
                }
            }
            let sel_x = main_rect.x
                + (self.state.saturation * main_width.saturating_sub(1) as f64).round() as u16;
            let sel_y = main_rect.y + (15.0 - self.state.value * 15.0).round() as u16;
            buf.get_mut(sel_x, sel_y)
                .set_char('◯')
                .set_fg(Color::White)
                .set_bg(Color::Rgb(
                    self.state.rgb.0,
                    self.state.rgb.1,
                    self.state.rgb.2,
                ));

            Block::default()
                .borders(Borders::ALL)
                .style(Style::default().fg(Color::DarkGray))
                .render(
                    Rect::new(
                        hue_rect.x - 1,
                        hue_rect.y - 1,
                        hue_rect.width + 2,
                        hue_rect.height + 2,
                    ),
                    buf,
                );
            for row in 0..16 {
                let node_h = 360.0 * (1.0 - (row as f64 / 15.0));
                let (r, g, b) = hsv_to_rgb(node_h, 1.0, 1.0);
                for col in 0..3 {
                    buf.get_mut(hue_rect.x + col, hue_rect.y + row)
                        .set_char('█')
                        .set_fg(Color::Rgb(r, g, b));
                }
            }
            let h_y = hue_rect.y
                + (15.0 - (self.state.hue / 360.0) * 15.0)
                    .clamp(0.0, 15.0)
                    .round() as u16;
            buf.get_mut(hue_rect.x, h_y)
                .set_char('▶')
                .set_fg(Color::White);
            buf.get_mut(hue_rect.x + 2, h_y)
                .set_char('◀')
                .set_fg(Color::White);
        }

        if self.show_details {
            let swatch_rect = Rect::new(inner.x + 40, inner.y + 1, 14, 6);
            Block::default()
                .borders(Borders::ALL)
                .title(" Color ")
                .style(Style::default().fg(Color::DarkGray))
                .render(swatch_rect, buf);
            let swatch_inner = Rect::new(
                swatch_rect.x + 1,
                swatch_rect.y + 1,
                swatch_rect.width.saturating_sub(2),
                swatch_rect.height.saturating_sub(2),
            );

            let mut render_swatch =
                |y: u16, title: &str, rgb: Option<(u8, u8, u8)>, editing: bool| {
                    buf.set_string(swatch_inner.x, y, title, Style::default().fg(Color::Gray));
                    if let Some(rgb) = rgb {
                        let color = Color::Rgb(rgb.0, rgb.1, rgb.2);
                        for x in 0..6 {
                            buf.get_mut(swatch_inner.x + 3 + x, y)
                                .set_char(' ')
                                .set_bg(color);
                        }
                    } else {
                        buf.set_string(
                            swatch_inner.x + 4,
                            y,
                            "[None]",
                            Style::default().fg(Color::DarkGray),
                        );
                    }
                    if editing {
                        buf.set_string(
                            swatch_inner.x - 1,
                            y,
                            "▶",
                            Style::default().fg(Color::White),
                        );
                    }
                };

            render_swatch(
                swatch_inner.y + 1,
                "Fg",
                Some(self.state.fg_rgb),
                self.state.editing_fg,
            );
            if self.state.is_dual {
                render_swatch(
                    swatch_inner.y + 3,
                    "Bg",
                    self.state.bg_rgb,
                    !self.state.editing_fg,
                );

                let bg_none_btn = Rect::new(swatch_inner.x, swatch_inner.y + 5, 12, 1);
                let hover = !self.state.editing_fg;
                buf.set_string(
                    bg_none_btn.x,
                    bg_none_btn.y,
                    " [Clear Bg] ",
                    Style::default().fg(if hover { Color::White } else { Color::DarkGray }),
                );
            }

            let val_y = inner.y + 9;
            let val_x = inner.x + 40;
            buf.set_string(
                val_x,
                val_y,
                format!("R: {:>3}", self.state.rgb.0),
                Style::default().fg(Color::Gray),
            );
            buf.set_string(
                val_x,
                val_y + 1,
                format!("G: {:>3}", self.state.rgb.1),
                Style::default().fg(Color::Gray),
            );
            buf.set_string(
                val_x,
                val_y + 2,
                format!("B: {:>3}", self.state.rgb.2),
                Style::default().fg(Color::Gray),
            );
            buf.set_string(
                val_x,
                val_y + 4,
                format!("#{}", self.state.hex),
                Style::default().fg(Color::White),
            );

            if self.state.mode != PaletteMode::Ansi24 {
                let label = if self.state.mode == PaletteMode::Irc {
                    "IRC"
                } else {
                    "ANSI"
                };
                buf.set_string(
                    val_x,
                    val_y + 6,
                    format!("{}: {}", label, self.state.grid_idx),
                    Style::default().fg(Color::Yellow),
                );
            }
        }

        // OK/Cancel buttons removed for embedded TUI use.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_ansi_pickers_fit_the_text_pane() {
        let area = Rect::new(0, 0, 39, 20);
        for mode in [PaletteMode::Ansi, PaletteMode::Ansi24] {
            let mut state = ColorPickerState::new(mode, true);
            let mut buffer = Buffer::empty(area);
            ColorPickerWidget {
                state: &mut state,
                show_details: false,
            }
            .render(area, &mut buffer);
        }
    }

    #[test]
    fn ansi_palette_keeps_the_grouped_hue_order_in_a_compact_grid() {
        let indices = ordered_ansi_indices();
        assert_eq!(indices.len(), 240);
        assert_eq!(&indices[..6], &[16, 22, 28, 34, 40, 46]);
        assert_eq!(main_area_dimensions(PaletteMode::Ansi), (24, 10));
    }

    #[test]
    fn left_click_preserves_the_selected_channel_and_middle_click_selects_fg() {
        let area = Rect::new(0, 0, 39, 20);
        let mut state = ColorPickerState::new(PaletteMode::Irc, true);
        state.editing_fg = false;

        assert!(state.handle_mouse_down(2, 2, area, None));
        assert!(!state.editing_fg);
        state.is_dragging_main = false;

        assert!(state.handle_mouse_down(2, 2, area, Some(true)));
        assert!(state.editing_fg);
    }

    #[test]
    fn keyboard_palette_navigation_clamps_at_the_edges() {
        let mut state = ColorPickerState::new(PaletteMode::Ansi, true);
        let indices = ordered_ansi_indices();
        state.set_grid_idx(indices[0]);
        state.move_selection(-1, -1);
        assert_eq!(state.grid_idx, indices[0]);

        state.set_grid_idx(*indices.last().unwrap());
        state.move_selection(1, 1);
        assert_eq!(state.grid_idx, *indices.last().unwrap());
    }
}
