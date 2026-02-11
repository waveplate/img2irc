use std::{error::Error, io, time::Duration, sync::Arc};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Terminal,
};
use tui_slider::Slider;
use img2irc_rs::{args::{RenderArgs, Render}, font, draw};
use photon_rs::PhotonImage;
use clap::Parser;
use ansi_to_tui::IntoText;
use tokio::sync::{mpsc, watch};

#[derive(Parser)]
struct Cli {
    #[arg(required = true)]
    image: String,
}

enum InputMode {
    Normal,
    Editing,
}

struct App {
    render_args: RenderArgs,
    
    // UI State
    input_mode: InputMode,
    selected_control: usize,
    
    // Async Communication
    tx_args: watch::Sender<RenderArgs>,
    rx_result: mpsc::Receiver<String>,
    
    // Cache
    cached_render: String,
    
    // Control values (mirrored from RenderArgs for slider ease)
    brightness: i8, // -100 to 100
    contrast: i8,   // -100 to 100
    saturation: i8, // -100 to 100
    gamma: i16,     // 0 to 255
    hue: i16,       // 0 to 360
    width: u16,     // 1 to 200
    dither: u8,     // 0 to 8
    
    // Toggles
    invert: bool,
    grayscale: bool,
    
    render_mode_idx: usize, // 0=IRC, 1=ANSI, 2=ANSI24
}

impl App {
    fn new(render_args: RenderArgs, tx_args: watch::Sender<RenderArgs>, rx_result: mpsc::Receiver<String>) -> App {
        App {
            render_args: render_args.clone(),
            input_mode: InputMode::Normal,
            selected_control: 0,
            tx_args,
            rx_result,
            cached_render: String::new(),
            
            brightness: 0,
            contrast: 0,
            saturation: 0,
            gamma: render_args.gamma as i16,
            hue: render_args.hue as i16,
            width: render_args.width.unwrap_or(80) as u16,
            dither: render_args.dither as u8,
            invert: render_args.invert,
            grayscale: render_args.grayscale,
            render_mode_idx: match render_args.render {
                Render::Irc => 0,
                Render::Ansi => 1,
                Render::Ansi24 => 2,
            },
        }
    }

    fn update_render_args(&mut self) {
        self.render_args.brightness = self.brightness as f32;
        self.render_args.contrast = self.contrast as f32;
        self.render_args.saturation = self.saturation as f32;
        self.render_args.gamma = self.gamma as f32;
        self.render_args.hue = self.hue as f32;
        self.render_args.width = Some(self.width as u32);
        self.render_args.dither = self.dither as u32;
        self.render_args.invert = self.invert;
        self.render_args.grayscale = self.grayscale;
        self.render_args.render = match self.render_mode_idx {
            0 => Render::Irc,
            1 => Render::Ansi,
            _ => Render::Ansi24,
        };
        
        // Non-blocking send
        let _ = self.tx_args.send(self.render_args.clone());
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    
    // Setup initial args
    let mut render_args = img2irc_rs::args_to_render_args(&img2irc_rs::args::Args::parse_from(&["img2irc", &cli.image]));
    // Force a default width for TUI visibility if not set
    if render_args.width.is_none() { render_args.width = Some(80); }
    // Default to ANSI
     render_args.render = Render::Ansi;

    // Load resources
    // Font / Glyphs
    let glyph_store = font::glyph_store(
        &render_args.blocks, 
        &render_args.exclude_range, 
        &render_args.exclude, 
        &render_args.include_range,
        render_args.include.as_ref(),
        &render_args.font,
        render_args.y_offset,
        render_args.font_size
    );
    
    // Image
    let img_res = img2irc_rs::load_image_from_url_or_path(&cli.image, &render_args, &glyph_store).await;
    let image = match img_res {
        Ok(img) => img,
        Err(e) => {
            eprintln!("Failed to load image: {}", e);
            return Ok(());
        }
    };
    
    let arc_image = Arc::new(image);
    let arc_glyphs = Arc::new(glyph_store);

    // Setup async rendering
    let (tx_args, mut rx_args) = watch::channel(render_args.clone());
    let (tx_result, rx_result) = mpsc::channel(1);

    // Render Task
    tokio::spawn(async move {
        // Prepare initial fast_glyphs
        let mut fast_glyphs_store: Option<Vec<draw::FastGlyph>> = None;
        
        loop {
            // Get current args
            let current_args = {
                rx_args.borrow_and_update().clone()
            };

            if fast_glyphs_store.is_none() {
                 let glyphs = draw::prepare_glyphs(&arc_glyphs, &current_args); 
                 fast_glyphs_store = Some(glyphs);
            }
            
            let glyphs = fast_glyphs_store.as_ref().unwrap();
            
            // Render
            let result = img2irc_rs::render_output(&current_args, &arc_glyphs, (*arc_image).clone(), glyphs);
            
            // Send result 
            if tx_result.send(result.content).await.is_err() {
                break;
            }
            
            // Wait for next change
            if rx_args.changed().await.is_err() {
                break;
            }
        }
    });

    // Prepare App
    let mut app = App::new(render_args, tx_args, rx_result);
    // Initial trigger
    app.update_render_args();

    // Terminal setup
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_app(&mut terminal, app).await;

    // cleanup
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{:?}", err)
    }

    Ok(())
}

async fn run_app<B: Backend>(terminal: &mut Terminal<B>, mut app: App) -> io::Result<()> {
    loop {
        // Check for new render results (Drain all pending, take last)
        while let Ok(content) = app.rx_result.try_recv() {
            app.cached_render = content;
        }
    
        terminal.draw(|f| ui(f, &mut app))?;

        // Poll for events with timeout to allow checking rx_result frequently
        if crossterm::event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                match app.input_mode {
                    InputMode::Normal => {
                        match key.code {
                            KeyCode::Char('q') => return Ok(()),
                            KeyCode::Down | KeyCode::Char('j') => {
                                app.selected_control = (app.selected_control + 1) % 10; 
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                if app.selected_control == 0 {
                                    app.selected_control = 9;
                                } else {
                                    app.selected_control -= 1;
                                }
                            }
                            KeyCode::Left | KeyCode::Char('h') => {
                                adjust_control(&mut app, -1);
                            }
                            KeyCode::Right | KeyCode::Char('l') => {
                                adjust_control(&mut app, 1);
                            }
                            KeyCode::Enter => {
                                // Toggle boolean controls
                                toggle_control(&mut app);
                            }
                            _ => {}
                        }
                    }
                    InputMode::Editing => {}
                }
            }
        }
    }
}

fn adjust_control(app: &mut App, delta: i32) {
    match app.selected_control {
        0 => { // Width
            let new = app.width as i32 + delta * 5;
            app.width = new.clamp(10, 300) as u16;
        }
        1 => { // Brightness
             let new = app.brightness as i32 + delta * 5;
             app.brightness = new.clamp(-100, 100) as i8;
        }
        2 => { // Contrast
             let new = app.contrast as i32 + delta * 5;
             app.contrast = new.clamp(-100, 100) as i8;
        }
        3 => { // Saturation
             let new = app.saturation as i32 + delta * 5;
             app.saturation = new.clamp(-100, 100) as i8;
        }
        4 => { // Dither
             let new = app.dither as i32 + delta;
             app.dither = new.clamp(0, 8) as u8;
        }
        5 => { // Gamma
             let new = app.gamma as i32 + delta * 5;
             app.gamma = new.clamp(0, 255) as i16;
        }
        6 => { // Hue
             let new = app.hue as i32 + delta * 5;
             app.hue = new.clamp(0, 360) as i16;
        }
        9 => { // Render Mode
            if delta > 0 {
                 app.render_mode_idx = (app.render_mode_idx + 1) % 3;
             } else {
                 app.render_mode_idx = (app.render_mode_idx + 2) % 3;
             }
        }
        _ => {}
    }
    app.update_render_args();
}

fn toggle_control(app: &mut App) {
    match app.selected_control {
        7 => { // Invert
            app.invert = !app.invert;
        }
        8 => { // Grayscale
            app.grayscale = !app.grayscale;
        }
        _ => {}
    }
    app.update_render_args();
}

fn ui(f: &mut ratatui::Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)].as_ref())
        .split(f.area());

    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(100)].as_ref())
        .split(chunks[0]);

    // Controls
    let controls_block = Block::default().borders(Borders::ALL).title("Controls");
    f.render_widget(controls_block, left_chunks[0]);

    let control_layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3), // Width
            Constraint::Length(3), // Brightness
            Constraint::Length(3), // Contrast
            Constraint::Length(3), // Saturation
            Constraint::Length(3), // Dither
            Constraint::Length(3), // Gamma
            Constraint::Length(3), // Hue
            Constraint::Length(3), // Invert
            Constraint::Length(3), // Grayscale
            Constraint::Length(3), // Render Mode
            Constraint::Min(0),
        ].as_ref())
        .split(left_chunks[0]);

    // Helper to render slider
    let render_slider = |f: &mut ratatui::Frame, area, title, value: i32, min, max, selected| {
        let block = Block::default().borders(Borders::ALL).title(title);
        let style = if selected { Style::default().fg(Color::Yellow) } else { Style::default() };
        
        let slider = Slider::new(value as f64, min as f64, max as f64)
            .block(block.style(style));
            
        f.render_widget(slider, area);
    };

    render_slider(f, control_layout[0], "Width", app.width as i32, 10, 300, app.selected_control == 0);
    render_slider(f, control_layout[1], "Brightness", app.brightness as i32, -100, 100, app.selected_control == 1);
    render_slider(f, control_layout[2], "Contrast", app.contrast as i32, -100, 100, app.selected_control == 2);
    render_slider(f, control_layout[3], "Saturation", app.saturation as i32, -100, 100, app.selected_control == 3);
    render_slider(f, control_layout[4], "Dither", app.dither as i32, 0, 8, app.selected_control == 4);
    render_slider(f, control_layout[5], "Gamma (x/255)", app.gamma as i32, 0, 255, app.selected_control == 5);
    render_slider(f, control_layout[6], "Hue", app.hue as i32, 0, 360, app.selected_control == 6);

    let checkbox = |f: &mut ratatui::Frame, area, title, value: bool, selected| {
         let style = if selected { Style::default().fg(Color::Yellow) } else { Style::default() };
         let text = if value { "[x] " } else { "[ ] " };
         let p = Paragraph::new(format!("{}{}", text, title)).block(Block::default().borders(Borders::ALL).style(style));
         f.render_widget(p, area);
    };

    checkbox(f, control_layout[7], "Invert", app.invert, app.selected_control == 7);
    checkbox(f, control_layout[8], "Grayscale", app.grayscale, app.selected_control == 8);

    // Render Mode Selector index 9
    let render_modes = ["IRC", "ANSI", "ANSI24"];
    let mode_str = format!("Mode: < {} >", render_modes[app.render_mode_idx]);
    let style = if app.selected_control == 9 { Style::default().fg(Color::Yellow) } else { Style::default() };
    let mode_p = Paragraph::new(mode_str).block(Block::default().borders(Borders::ALL).style(style));
    f.render_widget(mode_p, control_layout[9]);

    // Output
    let output_block = Block::default().borders(Borders::ALL).title("Output");
    
    // Convert ANSI string to TUI text
    let text = app.cached_render.as_bytes().into_text().unwrap_or_else(|_| {
        ratatui::text::Text::from(app.cached_render.clone()) 
    });

    let output_text = Paragraph::new(text)
        .block(output_block)
        .wrap(Wrap { trim: false }); 
    
    f.render_widget(output_text, chunks[1]);
}
