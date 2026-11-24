use super::*;

/// Focus within the current page. Page selection stays in `current_tab`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Pane {
    Controls,
    EffectControls,
    Options,
    GlyphTree,
    GlyphList,
    GlyphPreview,
    TextPalette,
    TextInput,
    TextList,
    Output,
    Video,
}

impl App {
    pub(super) fn primary_pane(&self) -> Pane {
        match self.current_tab {
            2 => Pane::GlyphTree,
            3 => Pane::TextList,
            _ => Pane::Controls,
        }
    }

    pub(super) fn register_pane(&mut self, pane: Pane, area: Rect) {
        if !area.is_empty() {
            self.pane_areas.push((pane, area));
        }
    }

    pub(super) fn visible_panes(&self) -> Vec<Pane> {
        const ORDER: &[Pane] = &[
            Pane::Controls,
            Pane::EffectControls,
            Pane::Options,
            Pane::GlyphTree,
            Pane::GlyphList,
            Pane::GlyphPreview,
            Pane::TextPalette,
            Pane::TextInput,
            Pane::TextList,
            Pane::Output,
            Pane::Video,
        ];
        let panes: Vec<_> = ORDER
            .iter()
            .copied()
            .filter(|pane| self.pane_areas.iter().any(|(visible, _)| visible == pane))
            .collect();
        if panes.is_empty() {
            let mut panes = vec![self.primary_pane(), Pane::Output];
            if self.video.is_some() {
                panes.push(Pane::Video);
            }
            panes
        } else {
            panes
        }
    }

    pub(super) fn focus_pane(&mut self, pane: Pane) {
        if pane != Pane::EffectControls {
            leave_effect_controls(self);
        }
        match pane {
            Pane::EffectControls => focus_first_effect_control(self),
            Pane::GlyphTree => self.selected_control = 0,
            Pane::GlyphList => self.selected_control = 1,
            Pane::TextPalette => self.text_field_focus = TextFieldFocus::FgButton,
            Pane::TextInput => self.text_field_focus = TextFieldFocus::TextInput,
            Pane::TextList => self.text_field_focus = TextFieldFocus::List,
            _ => {}
        }
        self.pane_focus = pane;
        if let Some(video) = self.video.as_mut() {
            video.focused = pane == Pane::Video;
        }
    }

    pub(super) fn cycle_pane_from(&mut self, current: Pane, direction: i32) {
        let panes = self.visible_panes();
        let index = panes.iter().position(|pane| *pane == current).unwrap_or(0) as i32;
        let next = (index + direction).rem_euclid(panes.len() as i32) as usize;
        self.focus_pane(panes[next]);
    }

    pub(super) fn focus_pane_at(&mut self, x: u16, y: u16) {
        // Later registrations cover overlays such as the General options pane.
        if let Some((pane, _)) = self
            .pane_areas
            .iter()
            .rev()
            .find(|(_, area)| area.contains((x, y).into()))
        {
            let pane = *pane;
            if pane != self.pane_focus {
                self.focus_pane(pane);
            }
        }
    }

    pub(super) fn begin_pane_frame(&mut self) {
        self.pane_areas.clear();
        self.control_areas.clear();
        self.font_scan_area = Rect::default();
        self.general_area = Rect::default();
        self.pipeline_column_area = Rect::default();
        self.pipeline_active_area = Rect::default();
        self.glyphs_area = Rect::default();
        self.tree_area = Rect::default();
        self.glyph_list_area = Rect::default();
        self.preview_area = Rect::default();
        self.glyph_select_all_area = Rect::default();
        self.glyph_select_none_area = Rect::default();
        self.glyph_add_area = Rect::default();
        self.glyph_delete_area = Rect::default();
        self.options_pane_area = Rect::default();
        self.options_areas.clear();
        self.text_area = Rect::default();
        self.text_list_area = Rect::default();
        self.text_btn_areas.clear();
        self.text_order_areas.clear();
        self.text_field_areas.clear();
        self.splitter_areas.clear();
    }
}

pub(super) fn handle_key(app: &mut App, key: &event::KeyEvent) -> bool {
    if app.input_mode != InputMode::Normal
        || key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return false;
    }
    if app.pane_focus == Pane::Output && app.current_tab == 3 && app.text_cursor_anchor.is_some() {
        // Text-page canvas clicks deliberately arm typing at the clicked cell.
        return false;
    }
    match app.pane_focus {
        Pane::Output | Pane::GlyphPreview => {
            match key.code {
                KeyCode::Up => app.scroll = app.scroll.saturating_sub(1),
                KeyCode::Down => app.scroll = app.scroll.saturating_add(1),
                KeyCode::PageUp => app.scroll = app.scroll.saturating_sub(10),
                KeyCode::PageDown => app.scroll = app.scroll.saturating_add(10),
                KeyCode::Esc => app.focus_pane(app.primary_pane()),
                KeyCode::Char('g' | 'p' | 'y') => return false,
                _ => {}
            }
            true
        }
        Pane::Options => {
            match key.code {
                KeyCode::Left | KeyCode::Up => adjust_control(app, -1),
                KeyCode::Right | KeyCode::Down => adjust_control(app, 1),
                KeyCode::Enter | KeyCode::Esc => app.focus_pane(Pane::Controls),
                KeyCode::Char('g' | 'p' | 'y') => return false,
                _ => {}
            }
            true
        }
        _ => false,
    }
}
