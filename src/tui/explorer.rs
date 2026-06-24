use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use educe::Educe;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, Borders, HighlightSpacing, List, ListState, StatefulWidget, Widget, WidgetRef,
    },
};
use std::sync::Arc;
use std::{fs::FileType, io::Result, path::PathBuf};

pub type FileFilter = Arc<dyn Fn(&File) -> bool + Send + Sync>;

#[derive(Clone, Educe)]
#[educe(Debug, PartialEq, Eq, Hash)]
pub struct FileExplorer {
    cwd: PathBuf,
    files: Vec<File>,
    selected: usize,
    theme: Theme,
    filter: Option<FileFilter>,
    #[educe(Debug(ignore), PartialEq(ignore), Hash(ignore))]
    pub(crate) list_offset: std::cell::Cell<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct File {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub file_type: Option<FileType>,
}

impl File {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub fn is_dir(&self) -> bool {
        self.is_dir
    }

    pub fn is_file(&self) -> bool {
        self.file_type
            .as_ref()
            .map(|f| f.is_file())
            .unwrap_or(false)
    }

    fn text(&self, theme: &Theme) -> Text<'static> {
        let style = if self.is_dir {
            theme.dir_style
        } else {
            theme.item_style
        };
        Span::styled(self.name.clone(), style).into()
    }
}

impl FileExplorer {
    pub fn new() -> Result<FileExplorer> {
        let cwd = std::env::current_dir()?;

        let mut file_explorer = Self {
            cwd,
            files: vec![],
            selected: 0,
            theme: Theme::default(),
            filter: None,
            list_offset: std::cell::Cell::new(0),
        };

        file_explorer.get_and_set_files()?;

        Ok(file_explorer)
    }

    pub fn with_theme(theme: Theme) -> Result<FileExplorer> {
        let mut file_explorer = Self::new()?;
        file_explorer.theme = theme;
        Ok(file_explorer)
    }

    pub fn with_filter(mut self, filter: FileFilter) -> Result<Self> {
        self.filter = Some(filter);
        self.get_and_set_files()?;
        self.selected = 0;
        Ok(self)
    }

    pub const fn widget(&self) -> Renderer<'_> {
        Renderer(self)
    }

    pub fn handle<I: Into<Input>>(&mut self, input: I) -> Result<()> {
        const SCROLL_COUNT: usize = 12;

        match input.into() {
            Input::Up => {
                if !self.files.is_empty() {
                    self.selected = self.selected.saturating_sub(1);
                }
            }
            Input::Down => {
                if !self.files.is_empty() {
                    self.selected = (self.selected + 1).min(self.files.len() - 1);
                }
            }
            Input::Home => {
                self.selected = 0;
            }
            Input::End => {
                if !self.files.is_empty() {
                    self.selected = self.files.len() - 1;
                }
            }
            Input::PageUp => {
                self.selected = self.selected.saturating_sub(SCROLL_COUNT);
            }
            Input::PageDown => {
                if !self.files.is_empty() {
                    self.selected = (self.selected + SCROLL_COUNT).min(self.files.len() - 1);
                }
            }
            Input::Left => {
                let parent = self.cwd.parent();

                if let Some(parent) = parent {
                    self.cwd = parent.to_path_buf();
                    self.get_and_set_files()?;
                    self.selected = 0;
                }
            }
            Input::Right => {
                if !self.files.is_empty() && self.files[self.selected].is_dir {
                    let path = self.files[self.selected].path.clone();
                    self.cwd = path;
                    self.get_and_set_files()?;
                    self.selected = 0;
                }
            }
            _ => {}
        }

        Ok(())
    }

    pub fn set_cwd<P: Into<PathBuf>>(&mut self, cwd: P) -> Result<()> {
        self.cwd = cwd.into();
        self.get_and_set_files()?;
        self.selected = 0;

        Ok(())
    }

    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }

    pub fn set_selected_idx(&mut self, selected: usize) {
        if selected < self.files.len() {
            self.selected = selected;
        }
    }

    pub fn current(&self) -> &File {
        &self.files[self.selected]
    }

    pub fn cwd(&self) -> &PathBuf {
        &self.cwd
    }

    pub fn files(&self) -> &Vec<File> {
        &self.files
    }

    pub fn selected_idx(&self) -> usize {
        self.selected
    }

    pub fn list_offset(&self) -> usize {
        self.list_offset.get()
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    fn get_and_set_files(&mut self) -> Result<()> {
        let entries = std::fs::read_dir(&self.cwd)?;
        let mut dirs = Vec::new();
        let mut files = Vec::new();

        for entry in entries {
            if let Ok(e) = entry {
                let path = e.path();
                let file_type = path.metadata().map(|m| m.file_type()).ok();
                let is_dir = file_type.as_ref().map(|f| f.is_dir()).unwrap_or(false);
                let name = if is_dir {
                    format!("{}/", e.file_name().to_string_lossy())
                } else {
                    e.file_name().to_string_lossy().into_owned()
                };

                let file = File {
                    name,
                    path,
                    is_dir,
                    file_type,
                };

                if is_dir {
                    dirs.push(file);
                } else {
                    let keep = if let Some(ref filter) = self.filter {
                        filter(&file)
                    } else {
                        true
                    };
                    if keep {
                        files.push(file);
                    }
                }
            }
        }

        dirs.sort_unstable_by(|f1, f2| f1.name.cmp(&f2.name));
        files.sort_unstable_by(|f1, f2| f1.name.cmp(&f2.name));

        let mut all_files = Vec::new();
        if let Some(parent) = self.cwd.parent() {
            all_files.push(File {
                name: "../".to_owned(),
                path: parent.to_path_buf(),
                is_dir: true,
                file_type: None,
            });
        }

        all_files.extend(dirs);
        all_files.extend(files);

        self.files = all_files;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Input {
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Left,
    Right,
    ToggleShowHidden,
    None,
}

impl From<&Event> for Input {
    fn from(value: &Event) -> Self {
        if let Event::Key(key) = value {
            if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                return match key.code {
                    KeyCode::Char('j') | KeyCode::Down => Input::Down,
                    KeyCode::Char('k') | KeyCode::Up => Input::Up,
                    KeyCode::Left | KeyCode::Backspace => Input::Left,
                    KeyCode::Char('h') => {
                        if key.modifiers.contains(KeyModifiers::CONTROL) {
                            Input::ToggleShowHidden
                        } else {
                            Input::Left
                        }
                    }
                    KeyCode::Char('l') | KeyCode::Right | KeyCode::Enter => Input::Right,
                    KeyCode::Home => Input::Home,
                    KeyCode::End => Input::End,
                    KeyCode::PageUp => Input::PageUp,
                    KeyCode::PageDown => Input::PageDown,
                    _ => Input::None,
                };
            }
        }
        Input::None
    }
}

pub struct Renderer<'a>(&'a FileExplorer);

impl WidgetRef for Renderer<'_> {
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        let mut state = ListState::default()
            .with_selected(Some(self.0.selected))
            .with_offset(self.0.list_offset.get());

        let highlight_style = if !self.0.files.is_empty() && self.0.current().is_dir {
            self.0.theme.highlight_dir_style
        } else {
            self.0.theme.highlight_item_style
        };

        let mut list = List::new(self.0.files.iter().map(|file| file.text(&self.0.theme)))
            .style(self.0.theme.style)
            .highlight_spacing(self.0.theme.highlight_spacing.clone())
            .highlight_style(highlight_style)
            .scroll_padding(self.0.theme.scroll_padding);

        if let Some(symbol) = self.0.theme.highlight_symbol.as_deref() {
            list = list.highlight_symbol(symbol);
        }

        if let Some(block) = self.0.theme.block.as_ref() {
            let mut block = block.clone();
            for title_top in self.0.theme.title_top(self.0) {
                block = block.title_top(title_top);
            }
            for title_bottom in self.0.theme.title_bottom(self.0) {
                block = block.title_bottom(title_bottom);
            }
            list = list.block(block);
        }

        StatefulWidget::render(list, area, buf, &mut state);
        self.0.list_offset.set(state.offset());
    }
}

impl Widget for Renderer<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_ref(area, buf);
    }
}

// Add Widget implementation for &Renderer to allow f.render_widget(&renderer, area)
impl Widget for &Renderer<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_ref(area, buf);
    }
}

type LineFactory = Arc<dyn Fn(&FileExplorer) -> Line<'static> + Send + Sync>;

#[derive(Clone, Educe)]
#[educe(Debug, PartialEq, Eq, Hash)]
pub struct Theme {
    pub block: Option<Block<'static>>,
    #[educe(Debug(ignore), PartialEq(ignore), Hash(ignore))]
    pub title_top: Vec<LineFactory>,
    #[educe(Debug(ignore), PartialEq(ignore), Hash(ignore))]
    pub title_bottom: Vec<LineFactory>,
    pub style: Style,
    pub item_style: Style,
    pub dir_style: Style,
    pub highlight_spacing: HighlightSpacing,
    pub highlight_item_style: Style,
    pub highlight_dir_style: Style,
    pub highlight_symbol: Option<String>,
    pub scroll_padding: usize,
}

impl Theme {
    pub fn new() -> Self {
        Self {
            block: None,
            title_top: Vec::new(),
            title_bottom: Vec::new(),
            style: Style::new(),
            item_style: Style::new(),
            dir_style: Style::new(),
            highlight_spacing: HighlightSpacing::WhenSelected,
            highlight_item_style: Style::new(),
            highlight_dir_style: Style::new(),
            highlight_symbol: None,
            scroll_padding: 0,
        }
    }

    pub fn add_default_title(self) -> Self {
        let title_top = Arc::new(|file_explorer: &FileExplorer| {
            Line::from(file_explorer.cwd().display().to_string())
        });
        let mut title_top_vec = self.title_top;
        title_top_vec.push(title_top);
        Self {
            title_top: title_top_vec,
            ..self
        }
    }

    pub fn with_block(mut self, block: Block<'static>) -> Self {
        self.block = Some(block);
        self
    }

    pub fn with_highlight_item_style<S: Into<Style>>(mut self, style: S) -> Self {
        self.highlight_item_style = style.into();
        self
    }

    pub fn with_highlight_dir_style<S: Into<Style>>(mut self, style: S) -> Self {
        self.highlight_dir_style = style.into();
        self
    }

    pub fn with_highlight_symbol(mut self, symbol: String) -> Self {
        self.highlight_symbol = Some(symbol);
        self
    }

    pub fn title_top(&self, explorer: &FileExplorer) -> Vec<Line<'static>> {
        self.title_top.iter().map(|f| f(explorer)).collect()
    }

    pub fn title_bottom(&self, explorer: &FileExplorer) -> Vec<Line<'static>> {
        self.title_bottom.iter().map(|f| f(explorer)).collect()
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            block: Some(Block::default().borders(Borders::ALL)),
            title_top: Vec::new(),
            title_bottom: Vec::new(),
            style: Style::default(),
            item_style: Style::default().fg(Color::White),
            dir_style: Style::default().fg(Color::LightBlue),
            highlight_spacing: HighlightSpacing::Always,
            highlight_item_style: Style::default().fg(Color::White).bg(Color::DarkGray),
            highlight_dir_style: Style::default().fg(Color::LightBlue).bg(Color::DarkGray),
            highlight_symbol: None,
            scroll_padding: 0,
        }
    }
}
