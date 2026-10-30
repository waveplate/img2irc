use super::*;
use crate::video::editor::{Clip, Request};
use std::sync::Mutex;

pub(super) struct State {
    pub clip: Clip,
    pub index: u64,
    pub waiting: bool,
    playing: bool,
    pub(super) focused: bool,
    selected: usize,
    editing: Option<String>,
    status: Arc<Mutex<String>>,
    output: String,
    areas: Vec<(Rect, usize)>,
    timeline: Rect,
    dragging_timeline: bool,
    next_at: Instant,
    export: Option<Export>,
}

struct Export {
    abort: Arc<AtomicBool>,
    progress: Arc<Mutex<Progress>>,
    join: Option<std::thread::JoinHandle<()>>,
    frames: u64,
    output: String,
}

#[derive(Default)]
struct Progress {
    frames: u64,
    frame_ms: u128,
    done: Option<Result<(), String>>,
}

impl Drop for Export {
    fn drop(&mut self) {
        self.abort.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl State {
    pub fn new(clip: Clip) -> Self {
        let output = clip
            .input
            .with_extension("i2v")
            .to_string_lossy()
            .into_owned();
        Self {
            clip,
            index: 0,
            waiting: true,
            playing: false,
            focused: false,
            selected: 0,
            editing: None,
            status: Arc::new(Mutex::new(String::new())),
            output,
            areas: Vec::new(),
            timeline: Rect::default(),
            dragging_timeline: false,
            next_at: Instant::now(),
            export: None,
        }
    }

    pub fn request(&self) -> Request {
        Request {
            clip: self.clip.clone(),
            index: self.index,
            status: self.status.clone(),
        }
    }

    fn value(&self, field: usize) -> String {
        match field {
            0 => format!("{:.3}", self.index as f64 / self.clip.fps),
            1 => format!("{:.3}", self.clip.start),
            2 => format!("{:.3}", self.clip.duration),
            3 => format!("{}", self.clip.fps),
            4 => self.clip.cache_mb.to_string(),
            5 => self.clip.reuse_tolerance.to_string(),
            6 => if self.clip.temporal { "on" } else { "off" }.into(),
            7 => self.output.clone(),
            _ => "Enter to encode / cancel".into(),
        }
    }

    fn set(&mut self, field: usize, text: &str) -> Result<(), String> {
        let number = || {
            text.parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .ok_or("enter a finite number".to_string())
        };
        match field {
            0 => {
                let seconds = crate::video::time_seconds(text)?;
                self.index = ((seconds * self.clip.fps).round() as u64).min(self.clip.frames() - 1);
            }
            1 => {
                let seconds = crate::video::time_seconds(text)?;
                if seconds >= self.clip.source_duration {
                    return Err("start must be inside the source video".into());
                }
                self.clip.start = seconds;
                self.clip.duration = self.clip.duration.min(self.clip.source_duration - seconds);
                self.index = 0;
            }
            2 => {
                self.clip.duration = crate::video::clip_duration(text)?
                    .min(self.clip.source_duration - self.clip.start);
                self.index = self.index.min(self.clip.frames() - 1);
            }
            3 => {
                let seconds = self.index as f64 / self.clip.fps;
                self.clip.fps = crate::video::frame_rate(text)?;
                self.index = ((seconds * self.clip.fps).round() as u64).min(self.clip.frames() - 1);
            }
            4 => self.clip.cache_mb = number()?.clamp(0.0, 1024.0) as u32,
            5 => self.clip.reuse_tolerance = number()?.clamp(0.0, 32.0) as u8,
            6 => self.clip.temporal = !self.clip.temporal,
            7 => {
                if text.is_empty() {
                    return Err("enter an output path".into());
                }
                self.output = text.into();
            }
            _ => {}
        }
        self.playing = false;
        self.status.lock().unwrap().clear();
        Ok(())
    }

    fn seek_at(&mut self, column: u16) -> bool {
        if self.timeline.width == 0 {
            return false;
        }
        let extent = self.timeline.width.saturating_sub(1);
        let position = column.saturating_sub(self.timeline.x).min(extent);
        let index = (position as f64 / extent.max(1) as f64 * (self.clip.frames() - 1) as f64)
            .round() as u64;
        self.playing = false;
        self.editing = None;
        let changed = self.index != index;
        self.index = index;
        changed
    }
}

pub(super) fn finish_edit(app: &mut App) {
    let Some(video) = app.video.as_mut() else {
        return;
    };
    video.dragging_timeline = false;
    let Some(text) = video.editing.take() else {
        return;
    };
    match video.set(video.selected, &text) {
        Ok(()) => app.mark_args_dirty(),
        Err(error) => *video.status.lock().unwrap() = error,
    }
}

pub(super) fn tick(app: &mut App) {
    let Some(video) = app.video.as_mut() else {
        return;
    };
    if !video.status.lock().unwrap().is_empty() && !app.is_rendering.load(Ordering::Relaxed) {
        video.waiting = false;
        video.playing = false;
    }
    if video.playing && !video.waiting && !app.args_dirty && Instant::now() >= video.next_at {
        video.index = (video.index + 1) % video.clip.frames();
        video.next_at = Instant::now() + Duration::from_secs_f64(1.0 / video.clip.fps);
        app.mark_args_dirty();
        app.flush_render_args();
    }
}

fn start_export(app: &mut App) {
    app.synchronize_render_args(false);
    let args = app.render_args.clone();
    let Some(video) = app.video.as_mut() else {
        return;
    };
    if let Some(export) = video.export.as_ref() {
        if export.progress.lock().unwrap().done.is_none() {
            export.abort.store(true, Ordering::Relaxed);
            return;
        }
    }
    video.export = None;
    video.playing = false;
    video.waiting = false;
    app.current_cancel_flag.store(true, Ordering::Relaxed);
    let abort = Arc::new(AtomicBool::new(false));
    let progress = Arc::new(Mutex::new(Progress::default()));
    let (worker_abort, worker_progress) = (abort.clone(), progress.clone());
    let (clip, output) = (video.clip.clone(), video.output.clone().into());
    let frames = clip.frames();
    let output_name = video.output.clone();
    let join = std::thread::spawn(move || {
        let result =
            crate::video::editor::export(clip, args, output, worker_abort, |frames, elapsed| {
                let mut p = worker_progress.lock().unwrap();
                p.frames = frames;
                p.frame_ms = elapsed.as_millis();
            })
            .map_err(|e| e.to_string());
        worker_progress.lock().unwrap().done = Some(result);
    });
    video.export = Some(Export {
        abort,
        progress,
        join: Some(join),
        frames,
        output: output_name,
    });
}

// Only video transport and source settings live here. Image controls and effect
// editing are handled by the existing panes, including mouse and numeric entry.
pub(super) fn handle_event(app: &mut App, event: &Event) -> bool {
    if app.video.is_none()
        || app.input_mode != InputMode::Normal
        || app.show_font_picker
        || app.show_unicode_explorer
        || app.explorer_dialog.is_some()
        || app.figlet_picker.is_some()
    {
        return false;
    }
    let video = app.video.as_mut().unwrap();
    if let Event::Mouse(mouse) = event {
        let seek = match mouse.kind {
            MouseEventKind::Down(MouseButton::Left)
                if video.timeline.contains((mouse.column, mouse.row).into()) =>
            {
                video.dragging_timeline = true;
                true
            }
            MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left) => {
                video.dragging_timeline
            }
            _ => false,
        };
        if seek {
            let changed = video.seek_at(mouse.column);
            let released = mouse.kind == MouseEventKind::Up(MouseButton::Left);
            if released {
                video.dragging_timeline = false;
            }
            if changed {
                app.mark_args_dirty();
            }
            if released && app.args_dirty {
                app.flush_render_args();
            }
            if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                app.focus_pane(Pane::Video);
            }
            return true;
        }
        if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
            if let Some(&(_, field)) = video
                .areas
                .iter()
                .find(|(area, _)| area.contains((mouse.column, mouse.row).into()))
            {
                video.selected = field;
                app.focus_pane(Pane::Video);
                let video = app.video.as_mut().unwrap();
                if field == 8 {
                    start_export(app);
                } else if field == 6 {
                    video.clip.temporal = !video.clip.temporal;
                    app.mark_args_dirty();
                } else {
                    video.editing = Some(video.value(field));
                }
                return true;
            }
            video.editing = None;
        }
        return false;
    }
    let Event::Key(key) = event else {
        return false;
    };
    if key.kind != KeyEventKind::Press {
        return false;
    }
    if key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return false;
    }
    if matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
        finish_edit(app);
        let video = app.video.as_mut().unwrap();
        let fraction = if key.modifiers.contains(KeyModifiers::SHIFT) {
            0.10
        } else {
            0.01
        };
        let step = (video.clip.frames() as f64 * fraction).round().max(1.0) as u64;
        video.index = if key.code == KeyCode::PageUp {
            video.index.saturating_sub(step)
        } else {
            video
                .index
                .saturating_add(step)
                .min(video.clip.frames() - 1)
        };
        video.playing = false;
        app.mark_args_dirty();
        return true;
    }
    if let Some(text) = video.editing.as_mut() {
        match key.code {
            KeyCode::Esc => video.editing = None,
            KeyCode::Enter => {
                let text = video.editing.take().unwrap();
                match video.set(video.selected, &text) {
                    Ok(()) => app.mark_args_dirty(),
                    Err(error) => *video.status.lock().unwrap() = error,
                }
            }
            KeyCode::Backspace => {
                text.pop();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => text.clear(),
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => text.push(c),
            _ => {}
        }
        return true;
    }
    match key.code {
        KeyCode::Enter
            if key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::SHIFT)
                && !app.effect_controls.is_editing() =>
        {
            video.playing = !video.playing;
            video.next_at = Instant::now();
        }
        KeyCode::F(6) => {
            let pane = if video.focused {
                app.primary_pane()
            } else {
                Pane::Video
            };
            app.focus_pane(pane);
        }
        KeyCode::F(7) => start_export(app),
        KeyCode::Esc if !video.focused => {
            video.playing = false;
            video.waiting = false;
            return false;
        }
        KeyCode::Char('[' | ']')
            if (video.focused || matches!(app.current_tab, 0 | 2))
                && !app.effect_controls.has_focus() =>
        {
            let step = if key.modifiers.contains(KeyModifiers::SHIFT) {
                video.clip.fps.round() as u64
            } else {
                1
            };
            video.index = if key.code == KeyCode::Char('[') {
                video.index.saturating_sub(step)
            } else {
                (video.index + step).min(video.clip.frames() - 1)
            };
            video.playing = false;
            app.mark_args_dirty();
        }
        KeyCode::Left | KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => {
            return false
        }
        _ if !video.focused => return false,
        KeyCode::Esc => app.focus_pane(app.primary_pane()),
        KeyCode::Down => video.selected = (video.selected + 1) % 9,
        KeyCode::Up => video.selected = (video.selected + 8) % 9,
        KeyCode::Char(' ') => {
            video.playing = !video.playing;
            video.next_at = Instant::now();
        }
        KeyCode::Enter => {
            if video.selected == 8 {
                start_export(app);
            } else if video.selected == 6 {
                video.clip.temporal = !video.clip.temporal;
                app.mark_args_dirty();
            } else {
                video.editing = Some(video.value(video.selected));
            }
        }
        KeyCode::Left | KeyCode::Right => {
            let delta = if key.code == KeyCode::Left { -1.0 } else { 1.0 };
            let field = video.selected;
            let step = match field {
                0 => 1.0 / video.clip.fps,
                1 | 2 => 0.1,
                4 => 16.0,
                _ => 1.0,
            };
            let value = video.value(field).parse::<f64>().unwrap_or(0.0) + delta * step;
            if field <= 6 {
                match video.set(
                    field,
                    &value
                        .max(if field == 2 || field == 3 { 0.1 } else { 0.0 })
                        .to_string(),
                ) {
                    Ok(()) => app.mark_args_dirty(),
                    Err(error) => *video.status.lock().unwrap() = error,
                }
            }
        }
        _ => {}
    }
    true
}

pub(super) fn render(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    let Some(video) = app.video.as_mut() else {
        return;
    };
    video.areas.clear();
    let block = Block::bordered()
        .title(" Video · Ctrl-Enter play/pause · PgUp/Dn 1% · Shift 10% · F7 encode ")
        .border_style(Style::default().fg(if video.focused { ACCENT } else { BORDER }));
    let inner = block.inner(area);
    video.timeline = Rect::default();
    f.render_widget(block, area);
    if inner.is_empty() {
        return;
    }
    let time = video.clip.start + video.index as f64 / video.clip.fps;
    let transport = format!(
        "{}  Frame {}/{} · source {:.3}s · {:.1} fps{}",
        if video.playing { "▶" } else { "Ⅱ" },
        video.index + 1,
        video.clip.frames(),
        time,
        video.clip.fps,
        if video.waiting {
            " · rendering…"
        } else {
            ""
        }
    );
    f.render_widget(
        Paragraph::new(transport).style(Style::default().fg(TEXT)),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
    if inner.height > 1 {
        let label = format!(
            " Seek {:.2}/{:.2}s ",
            video.index as f64 / video.clip.fps,
            video.clip.duration
        );
        let label_width = (label.chars().count() as u16).min(inner.width.saturating_sub(1));
        f.render_widget(
            Paragraph::new(label).style(Style::default().fg(TEXT_DIM)),
            Rect::new(inner.x, inner.y + 1, label_width, 1),
        );
        video.timeline = Rect::new(
            inner.x + label_width,
            inner.y + 1,
            inner.width - label_width,
            1,
        );
        let fraction = video.index as f64 / video.clip.frames().saturating_sub(1).max(1) as f64;
        let position = (fraction * video.timeline.width.saturating_sub(1) as f64).round() as usize;
        let track = Line::from(vec![
            Span::styled("━".repeat(position), Style::default().fg(ACCENT)),
            Span::styled(
                "●",
                Style::default().fg(if video.dragging_timeline {
                    Color::Yellow
                } else {
                    ACCENT
                }),
            ),
            Span::styled(
                "─".repeat(video.timeline.width as usize - position - 1),
                Style::default().fg(BORDER),
            ),
        ]);
        f.render_widget(Paragraph::new(track), video.timeline);
    }
    let labels = [
        "Position (s)",
        "Start (s)",
        "Duration (s)",
        "FPS",
        "Cache (MiB)",
        "Reuse tolerance",
        "Temporal reuse",
        "Output",
        "Encode",
    ];
    let half = inner.width / 2;
    for field in 0..9 {
        let row = field / 2 + 2;
        if row as u16 >= inner.height {
            break;
        }
        let area = if field == 8 {
            Rect::new(inner.x, inner.y + row as u16, inner.width, 1)
        } else {
            Rect::new(
                inner.x + (field % 2) as u16 * half,
                inner.y + row as u16,
                if field % 2 == 0 {
                    half
                } else {
                    inner.width - half
                },
                1,
            )
        };
        let selected = video.focused && video.selected == field;
        let value = if selected {
            video
                .editing
                .as_ref()
                .cloned()
                .unwrap_or_else(|| video.value(field))
        } else {
            video.value(field)
        };
        let mut text = format!(
            "{} {}: {}",
            if selected { "›" } else { " " },
            labels[field],
            value
        );
        if field == 8 {
            let status = video.status.lock().unwrap();
            if !status.is_empty() {
                text = status.clone();
            }
            if let Some(export) = &video.export {
                let p = export.progress.lock().unwrap();
                text = match &p.done {
                    None => format!(
                        "Encoding {}/{} · last {}ms · F7 cancel",
                        p.frames, export.frames, p.frame_ms
                    ),
                    Some(Ok(())) => format!("Saved {} frames → {}", p.frames, export.output),
                    Some(Err(error)) => format!("Encode: {error}"),
                };
            }
        }
        let editing = selected && video.editing.is_some();
        if editing {
            text.push('▏');
        }
        let scroll = if editing {
            text.chars().count().saturating_sub(area.width as usize) as u16
        } else {
            0
        };
        f.render_widget(
            Paragraph::new(text).scroll((0, scroll)).style(
                Style::default()
                    .fg(if selected { ACCENT } else { TEXT_DIM })
                    .bg(if selected { SELECTED_BG } else { BG }),
            ),
            area,
        );
        video.areas.push((area, field));
    }
    if inner.height > 7 {
        if let Some(result) = &app.base_render_result {
            let p = &result.profile;
            let text = format!(" Last frame: prepare {}ms · glyph fit {}ms · smoothing {}ms · shapes {}ms · Repair Edges {}ms · final score {}ms",
                p.prepare_canvas.as_millis(), p.glyph_match.as_millis(), p.smoothing_search.as_millis(),
                p.shape_refine.as_millis(), p.glyph_search.as_millis(), p.final_score.as_millis());
            f.render_widget(
                Paragraph::new(text).style(Style::default().fg(TEXT_DIM)),
                Rect::new(inner.x, inner.y + 7, inner.width, 1),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> State {
        State::new(Clip {
            input: "source.mp4".into(),
            start: 6.0,
            duration: 2.0,
            fps: 24.0,
            cache_mb: 64,
            reuse_tolerance: 0,
            temporal: false,
            aspect: 4.0 / 3.0,
            source_duration: 30.0,
        })
    }

    fn app() -> App {
        use clap::Parser;
        let args = crate::args_to_render_args(&crate::args::Args::parse_from(["img2irc"]));
        let flag = Arc::new(AtomicBool::new(false));
        let (tx, _rx) = watch::channel((args.clone(), flag.clone(), Instant::now(), None));
        let (_tx, rx) = mpsc::channel(1);
        let mut app = App::new(args, tx, rx, flag);
        app.video = Some(state());
        app
    }

    #[test]
    fn modified_enter_controls_playback_and_tab_traverses_video_as_a_pane() {
        let mut app = app();
        let key = |code, modifiers| Event::Key(event::KeyEvent::new(code, modifiers));
        assert!(!handle_event(
            &mut app,
            &key(KeyCode::F(5), KeyModifiers::NONE)
        ));
        assert!(handle_event(
            &mut app,
            &key(KeyCode::Enter, KeyModifiers::CONTROL)
        ));
        assert!(app.video.as_ref().unwrap().playing);
        assert!(handle_event(
            &mut app,
            &key(KeyCode::Enter, KeyModifiers::SHIFT)
        ));
        assert!(!app.video.as_ref().unwrap().playing);
        for expected in [Pane::Output, Pane::Video, Pane::Controls] {
            assert!(handle_pane_tab(
                &mut app,
                &key(KeyCode::Tab, KeyModifiers::NONE)
            ));
            assert_eq!(app.pane_focus, expected);
            assert_eq!(app.current_tab, 0);
        }
    }

    #[test]
    fn page_keys_seek_clip_percentages_on_any_page_and_clamp_to_the_clip() {
        let mut app = app();
        app.video.as_mut().unwrap().clip.duration = 20.0;
        let key = |code| Event::Key(event::KeyEvent::new(code, KeyModifiers::NONE));
        for page in 0..4 {
            app.toggle_tab(page);
            app.video.as_mut().unwrap().index = 0;
            assert!(handle_event(&mut app, &key(KeyCode::PageDown)));
            assert_eq!(app.video.as_ref().unwrap().index, 5);
            assert!(handle_event(&mut app, &key(KeyCode::PageUp)));
            assert_eq!(app.video.as_ref().unwrap().index, 0);
            handle_event(
                &mut app,
                &Event::Key(event::KeyEvent::new(KeyCode::PageDown, KeyModifiers::SHIFT)),
            );
            assert_eq!(app.video.as_ref().unwrap().index, 48);
            app.video.as_mut().unwrap().index = 478;
            handle_event(&mut app, &key(KeyCode::PageDown));
            assert_eq!(app.video.as_ref().unwrap().index, 479);
            app.video.as_mut().unwrap().index = 2;
            handle_event(&mut app, &key(KeyCode::PageUp));
            assert_eq!(app.video.as_ref().unwrap().index, 0);
            assert!(!app.video.as_ref().unwrap().playing);
            assert_eq!(app.current_tab, page);
        }
    }

    #[test]
    fn seek_bar_click_and_drag_include_both_endpoints_and_stop_on_release() {
        let mut app = app();
        app.video.as_mut().unwrap().playing = true;
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(120, 10)).unwrap();
        terminal
            .draw(|frame| render(frame, &mut app, Rect::new(0, 0, 120, 10)))
            .unwrap();
        let bar = app.video.as_ref().unwrap().timeline;
        assert!(bar.width > 90, "seek bar should fill the pane");
        let mouse = |kind, x| {
            Event::Mouse(event::MouseEvent {
                kind,
                column: x,
                row: bar.y,
                modifiers: KeyModifiers::NONE,
            })
        };
        handle_event(
            &mut app,
            &mouse(MouseEventKind::Down(MouseButton::Left), bar.x),
        );
        assert!(!app.video.as_ref().unwrap().playing);
        assert_eq!(app.video.as_ref().unwrap().index, 0);
        handle_event(
            &mut app,
            &mouse(MouseEventKind::Drag(MouseButton::Left), bar.right() - 1),
        );
        assert_eq!(app.video.as_ref().unwrap().index, 47);
        handle_event(
            &mut app,
            &mouse(MouseEventKind::Up(MouseButton::Left), bar.right() + 10),
        );
        assert!(!app.video.as_ref().unwrap().dragging_timeline);
        handle_event(
            &mut app,
            &mouse(MouseEventKind::Drag(MouseButton::Left), bar.x),
        );
        assert_eq!(app.video.as_ref().unwrap().index, 47);
        handle_event(
            &mut app,
            &mouse(MouseEventKind::Down(MouseButton::Left), bar.x),
        );
        handle_event(&mut app, &mouse(MouseEventKind::Up(MouseButton::Left), 0));
        assert_eq!(app.video.as_ref().unwrap().index, 0);
    }

    #[test]
    fn clip_edits_preserve_playhead_time_and_keep_it_inside_the_clip() {
        let mut state = state();
        state.index = 24;
        state.set(3, "12").unwrap();
        assert_eq!(state.index, 12);
        state.set(2, "0.5").unwrap();
        assert_eq!(state.index, 5);
        state.set(1, "29.75").unwrap();
        assert_eq!(state.clip.duration, 0.25);
        assert_eq!(state.index, 0);
        state.set(0, "100").unwrap();
        assert_eq!(state.index, 2);
        assert!(state.set(1, "30").is_err());
        assert_eq!(state.clip.start, 29.75);
        assert!(state.set(3, "NaN").is_err());
        assert_eq!(state.clip.fps, 12.0);
    }

    #[test]
    fn seek_and_clip_edits_leave_render_cache_controls_and_output_intact() {
        let mut state = state();
        state.set(4, "1024").unwrap();
        state.set(5, "16").unwrap();
        state.set(6, "").unwrap();
        state.set(7, "/tmp/custom.i2v").unwrap();
        state.set(0, "1.5").unwrap();
        state.set(1, "00:00:08.25").unwrap();
        let request = state.request();
        assert_eq!(request.clip.start, 8.25);
        assert_eq!(request.index, 0);
        assert_eq!(request.clip.cache_mb, 1024);
        assert_eq!(request.clip.reuse_tolerance, 16);
        assert!(request.clip.temporal);
        assert_eq!(state.output, "/tmp/custom.i2v");
    }
}
