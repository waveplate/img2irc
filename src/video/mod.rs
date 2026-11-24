//! Shared terminal-video encoder, editor, player, and servers.
use crate::{args, draw, font, pipeline::ImageEffect};
use ansi_to_tui::IntoText;
use clap::{Parser, Subcommand, ValueEnum};
use crossterm::{
    cursor::{Hide, Show},
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    style::ResetColor,
    terminal::{
        self, Clear, ClearType, DisableLineWrap, EnableLineWrap, EndSynchronizedUpdate,
        EnterAlternateScreen, LeaveAlternateScreen,
    },
};
use photon_rs::PhotonImage;
use ratatui::{
    backend::{Backend, CrosstermBackend},
    buffer::Buffer,
    layout::Rect,
    widgets::{Paragraph, Widget},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    error::Error,
    fs::{self, File},
    io::{self, BufRead, BufReader, BufWriter, IsTerminal, Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

pub(crate) mod editor;
mod present;
pub(crate) mod render;
#[cfg(test)]
mod render_tests;
#[path = "ssh.rs"]
mod ssh_server;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Parser)]
#[command(about = "Experimental img2irc video encoder, live preview, player, and TCP/SSH server")]
struct Cli {
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    /// Decode with FFmpeg and render every frame ahead of time (no audio).
    Encode(Encode),
    /// Open the source video in img2irc's editor, with timeline and encoding controls.
    Preview(Settings),
    /// Play a previously encoded file. Q, Escape, or Ctrl-C exits.
    Play {
        input: PathBuf,
        /// Keep the encoded cell size instead of fitting to the terminal.
        #[arg(long)]
        ignore_terminal_size: bool,
    },
    /// Stream the video from the beginning to each TCP client, then disconnect.
    Serve {
        input: PathBuf,
        #[arg(long, default_value = "0.0.0.0:2323", value_parser = bind_address)]
        bind: SocketAddr,
        /// Fit to this assumed viewer terminal size; nc does not report its size.
        #[arg(long, value_name = "COLSxROWS", value_parser = terminal_size)]
        terminal_size: Option<(u16, u16)>,
    },
    /// Play once to public SSH viewers; accepts any username without a password.
    ServeSsh {
        /// Fallback when no readable <username>.i2v exists in the working directory.
        input: PathBuf,
        /// Listen address or port. Use port 22 for a plain `ssh HOST` connection.
        #[arg(long, default_value = "[::]:2222", value_parser = ssh_server::bind_address)]
        bind: SocketAddr,
        /// Persistent OpenSSH host key; created with private permissions if missing.
        #[arg(long, default_value = "video_ssh_host_ed25519")]
        host_key: PathBuf,
    },
}

fn bind_address(value: &str) -> std::result::Result<SocketAddr, String> {
    if let Ok(port) = value.parse::<u16>() {
        return Ok(SocketAddr::from(([0, 0, 0, 0], port)));
    }
    value
        .parse()
        .map_err(|_| "expected a port or IP:PORT, e.g. 2222 or 0.0.0.0:2222".into())
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum Palette {
    Ansi24,
    Ansi256,
}

#[derive(clap::Args)]
struct Encode {
    #[command(flatten)]
    settings: Settings,
    output: PathBuf,
    /// Render independent frames concurrently; requires temporal reuse off and reuse tolerance 0.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u16).range(1..=32))]
    workers: u16,
}

#[derive(Clone, PartialEq, clap::Args)]
pub(crate) struct Settings {
    input: PathBuf,
    /// Start offset in seconds or [HH:]MM:SS[.fraction].
    #[arg(long, visible_alias = "start-time", default_value = "0", value_name = "TIME", value_parser = time_seconds, allow_hyphen_values = true)]
    start: f64,
    /// Length from the selected start; seconds or [HH:]MM:SS[.fraction].
    #[arg(long, value_name = "TIME", value_parser = clip_duration, allow_hyphen_values = true)]
    duration: Option<f64>,
    /// Maximum output width in terminal columns.
    #[arg(long, default_value_t = 80, value_parser = clap::value_parser!(u16).range(1..))]
    width: u16,
    /// Maximum output height in terminal rows; no padding is added.
    #[arg(long, default_value_t = 24, value_parser = clap::value_parser!(u16).range(1..))]
    height: u16,
    /// Constant output frame rate (source frames are dropped or duplicated).
    #[arg(long, default_value = "24", value_parser = frame_rate)]
    fps: f64,
    #[arg(long, value_enum, default_value = "ansi24")]
    palette: Palette,
    /// Comma-separated img2irc glyph sets, e.g. half or smooth,half.
    #[arg(long, value_delimiter = ',', default_value = "half")]
    glyphs: Vec<String>,
    /// Rendering font height in pixels (6–256); controls glyph-matching resolution.
    #[arg(long, default_value_t = 16.0, value_parser = font_size)]
    font_size: f32,
    /// Contrast adjustment, -255 to 255; zero leaves it unchanged.
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true, value_parser = contrast)]
    contrast: f32,
    /// Allow blended resize/SVG colours instead of exact local source samples.
    #[arg(long = "blend-colours", visible_alias = "blend-colors", default_value_t = true, num_args = 0..=1, default_missing_value = "true", action = clap::ArgAction::Set)]
    blend_colours: bool,
    /// Use foreground colours only, leaving the terminal background unset.
    #[arg(long)]
    foreground_only: bool,
    /// Grow dark lines with positive values, light lines with negative values (-10 to 10).
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true, value_parser = args::parse_line_thickness)]
    line_thickness: f32,
    /// Darken narrow dark outlines before fitting glyphs.
    #[arg(long)]
    darken_outlines: bool,
    #[command(flatten)]
    outline_options: crate::outline_darkening::Options,
    /// Limit finished-art colours per frame; 0 merges shades without a colour cap.
    #[arg(long, alias = "quantize-colors", value_parser = clap::value_parser!(u32).range(0..=256))]
    quantize_colours: Option<u32>,
    /// Merge nearby shades in OKLab (0–100%, default 0); enables quantization at 16 colours if no count is given.
    #[arg(long, visible_alias = "quantize-merge", value_parser = merge_threshold)]
    quantize_threshold: Option<f32>,
    /// Fast glyph smoothing: five candidates, one scan order, no refinement.
    #[arg(long)]
    smooth: bool,
    /// Slower shape refinement after fast smoothing (implies --smooth).
    #[arg(long)]
    smooth_shapes: bool,
    /// Repair edges using neighbouring glyph combinations and perceptual scoring.
    #[arg(long, visible_alias = "refine-edges")]
    glyph_search: bool,
    /// Source-fit and seam-fit alternatives per cell, plus its current glyph.
    #[arg(long, default_value_t = 6, value_parser = clap::value_parser!(u32).range(1..=64))]
    glyph_search_candidates: u32,
    /// Maximum passes over neighbouring glyph pairs.
    #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u32).range(1..=20))]
    glyph_search_passes: u32,
    /// Full-image checks per pair; 0 checks every combination (expensive).
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u32).range(0..=4096))]
    glyph_search_finalists: u32,
    /// Defect threshold; 0 searches every non-flat cell.
    #[arg(long = "disc-threshold", visible_alias = "discontinuity-threshold", default_value_t = 0.5, value_parser = defect_threshold)]
    discontinuity_threshold: f32,
    /// Memory budget for exact local glyph-search reuse between frames; 0 disables it.
    #[arg(long, default_value_t = 64, value_parser = clap::value_parser!(u32).range(0..=1024))]
    glyph_search_cache_mb: u32,
    /// Allow this much RGB-channel drift when reusing local rankings (0–32).
    /// 0 is exact; nonzero trades search precision for compression-noise reuse.
    #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u8).range(0..=32))]
    glyph_search_reuse_tolerance: u8,
    /// Warm-start stable regions from the previous frame and search changed areas.
    /// May choose different glyphs; whole-image quality checks still apply.
    #[arg(long)]
    glyph_search_temporal: bool,
    /// Guide glyph colours and boundaries using connected image regions.
    #[arg(long)]
    region_guidance: bool,
    /// Flatten nearly uniform connected colour regions before fitting glyphs.
    #[arg(long)]
    flat_fill: bool,
    #[command(flatten)]
    flat_fill_options: crate::flat_fill::Options,
    /// Trace filled regions with img2svg before glyph matching.
    #[arg(long)]
    img2svg: bool,
    #[command(flatten)]
    img2svg_options: crate::svg_effect::Options,
    /// Print per-frame render-stage timings while encoding.
    #[arg(long)]
    profile: bool,
}

fn defect_threshold(value: &str) -> std::result::Result<f32, String> {
    let value = value.parse::<f32>().map_err(|e| e.to_string())?;
    if !value.is_finite() || value < 0.0 {
        return Err("defect threshold must be finite and nonnegative".into());
    }
    Ok(value)
}

fn merge_threshold(value: &str) -> std::result::Result<f32, String> {
    let value = value.parse::<f32>().map_err(|e| e.to_string())?;
    if !value.is_finite() || !(0.0..=100.0).contains(&value) {
        return Err("quantization merge must be between 0 and 100 percent".into());
    }
    Ok(value)
}

pub(crate) fn frame_rate(value: &str) -> std::result::Result<f64, String> {
    let value = value.parse::<f64>().map_err(|e| e.to_string())?;
    if !value.is_finite() || !(0.1..=120.0).contains(&value) {
        return Err("fps must be between 0.1 and 120".into());
    }
    Ok(value)
}

pub(crate) fn time_seconds(value: &str) -> std::result::Result<f64, String> {
    let error = "expected a nonnegative time in seconds or [HH:]MM:SS[.fraction]";
    let fields: Vec<_> = value.split(':').collect();
    if fields.is_empty() || fields.len() > 3 {
        return Err(error.into());
    }
    let seconds = fields.last().unwrap().parse::<f64>().map_err(|_| error)?;
    if !seconds.is_finite() || seconds < 0.0 || (fields.len() > 1 && seconds >= 60.0) {
        return Err(error.into());
    }
    let mut total = seconds;
    let mut multiplier = 60.0;
    for (index, field) in fields[..fields.len() - 1].iter().rev().enumerate() {
        let amount = field.parse::<u64>().map_err(|_| error)?;
        if fields.len() == 3 && index == 0 && amount >= 60 {
            return Err(error.into());
        }
        total += amount as f64 * multiplier;
        multiplier *= 60.0;
    }
    Ok(total)
}

pub(crate) fn clip_duration(value: &str) -> std::result::Result<f64, String> {
    let seconds = time_seconds(value)?;
    if seconds <= 0.0 {
        return Err("duration must be greater than zero".into());
    }
    Ok(seconds)
}

fn contrast(value: &str) -> std::result::Result<f32, String> {
    let value = value.parse::<f32>().map_err(|e| e.to_string())?;
    if !value.is_finite() || !(-255.0..=255.0).contains(&value) {
        return Err("contrast must be between -255 and 255".into());
    }
    Ok(value)
}

fn font_size(value: &str) -> std::result::Result<f32, String> {
    let value = value.parse::<f32>().map_err(|e| e.to_string())?;
    if !value.is_finite() || !(6.0..=256.0).contains(&value) {
        return Err("font size must be between 6 and 256 pixels".into());
    }
    Ok(value)
}

fn terminal_size(value: &str) -> std::result::Result<(u16, u16), String> {
    let parse = || -> Option<(u16, u16)> {
        let (columns, rows) = value.split_once('x')?;
        let columns = columns.parse::<u16>().ok()?;
        let rows = rows.parse::<u16>().ok()?;
        (columns > 0 && rows > 0).then_some((columns, rows))
    };
    parse().ok_or_else(|| "expected positive terminal dimensions, e.g. 286x70".into())
}

// JSON Lines: header, one JSON string per complete ANSI frame, then null.
// The terminator distinguishes a finished encode from an interrupted one.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Header {
    format: String,
    version: u32,
    width: u16,
    height: u16,
    fps: f64,
    #[serde(default)]
    foreground_only: bool,
}

impl Header {
    fn validate(&self) -> Result<()> {
        if self.format != "img2irc-video" || self.version != 1 {
            return Err("unsupported img2irc video format/version".into());
        }
        if self.width == 0 || self.height == 0 || frame_rate(&self.fps.to_string()).is_err() {
            return Err("invalid video dimensions or frame rate".into());
        }
        Ok(())
    }
}

/// Fit a width/height ratio within terminal-cell bounds.
fn fit_size(aspect: f64, (width, height): (u16, u16)) -> (u16, u16) {
    let rows = (f64::from(width) / aspect).round().max(1.0);
    if rows <= f64::from(height) {
        (width, rows as u16)
    } else {
        (
            (f64::from(height) * aspect)
                .round()
                .clamp(1.0, f64::from(width)) as u16,
            height,
        )
    }
}

fn video_aspect(input: &Path) -> Result<f64> {
    let probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,sample_aspect_ratio:stream_side_data=rotation",
            "-of",
            "json",
        ])
        .arg(input)
        .output()
        .map_err(|e| format!("could not start ffprobe (install FFmpeg tools on PATH): {e}"))?;
    if !probe.status.success() {
        return Err(format!(
            "ffprobe failed: {}",
            String::from_utf8_lossy(&probe.stderr).trim()
        )
        .into());
    }
    let metadata: serde_json::Value = serde_json::from_slice(&probe.stdout)?;
    let stream = &metadata["streams"][0];
    let width = stream["width"]
        .as_u64()
        .filter(|n| *n > 0)
        .ok_or("video has no width")?;
    let height = stream["height"]
        .as_u64()
        .filter(|n| *n > 0)
        .ok_or("video has no height")?;
    let sar = stream["sample_aspect_ratio"]
        .as_str()
        .and_then(|s| s.split_once(':'))
        .and_then(|(w, h)| Some(w.parse::<f64>().ok()? / h.parse::<f64>().ok()?))
        .filter(|n| n.is_finite() && *n > 0.0)
        .unwrap_or(1.0);
    let mut aspect = width as f64 * sar / height as f64;
    if stream["side_data_list"].as_array().is_some_and(|data| {
        data.iter().any(|item| {
            item["rotation"]
                .as_f64()
                .is_some_and(|r| r.rem_euclid(180.0) == 90.0)
        })
    }) {
        aspect = 1.0 / aspect;
    }
    Ok(aspect)
}

// Reap FFmpeg on ordinary errors too, including a full disk or broken pipe.
pub(crate) struct Decoder(Child);
impl Drop for Decoder {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn encode(options: Encode) -> Result<()> {
    encode_with_args(options, None, None, None)
}

fn encode_with_args(
    options: Encode,
    render_args: Option<args::RenderArgs>,
    observer: Option<&dyn Fn(u64, Duration)>,
    abort: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Result<()> {
    let settings = &options.settings;
    if options.workers > 1
        && settings.flat_fill
        && settings.flat_fill_options.strength > 0.0
        && settings.flat_fill_options.temporal_tolerance > 0.0
    {
        return Err("temporal Flat Fill needs consecutive frames; use --workers 1 or --flat-fill-temporal-tolerance 0".into());
    }
    if options.workers > 1
        && (settings.glyph_search_temporal || settings.glyph_search_reuse_tolerance != 0)
    {
        return Err("--workers > 1 requires temporal search off and --glyph-search-reuse-tolerance 0; these depend on earlier frames".into());
    }
    if settings.profile {
        let config = simplelog::ConfigBuilder::new()
            .add_filter_allow_str("img2irc_rs::draw::glyph_search")
            .add_filter_allow_str("img2irc_rs::perceptual_score")
            .build();
        let _ = simplelog::SimpleLogger::init(log::LevelFilter::Info, config);
    }
    let aspect = video_aspect(&settings.input)?;
    let renderer = match render_args {
        Some(args) => render::Renderer::from_args(settings, aspect, args)?,
        None => render::Renderer::new(settings, aspect)?,
    };
    let mut decoder = renderer.decoder(settings, 0, observer.is_some())?;
    let mut header = renderer.header(settings.fps);
    let frame_bytes = renderer.frame_bytes();
    let mut worker_settings = settings.clone();
    worker_settings.glyph_search_cache_mb /= u32::from(options.workers);
    let mut workers = vec![(renderer, render::FrameCache::default())];
    for _ in 1..options.workers {
        workers.push((
            render::Renderer::new(&worker_settings, aspect)?,
            render::FrameCache::default(),
        ));
    }
    if options.workers > 1 {
        workers[0]
            .0
            .set_cache_budget(worker_settings.glyph_search_cache_mb);
    }
    // Never overwrite the input or an existing encode.
    let file = File::create_new(&options.output)?;
    let result = (|| -> Result<()> {
        let mut output = BufWriter::new(file);
        let mut source = decoder.0.stdout.take().ok_or("missing FFmpeg stdout")?;
        let mut count = 0u64;
        let mut reused = 0u64;
        let started = Instant::now();
        loop {
            if abort
                .as_ref()
                .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Relaxed))
            {
                return Err("encoding cancelled".into());
            }
            let mut frames = Vec::with_capacity(workers.len());
            for _ in 0..workers.len() {
                let mut rgba = vec![0; frame_bytes];
                // Separate clean EOF from a partial raw frame.
                if source.read(&mut rgba[..1])? == 0 {
                    break;
                }
                source.read_exact(&mut rgba[1..])?;
                frames.push(rgba);
            }
            if frames.is_empty() {
                break;
            }
            let frame_start = count;
            let mut write = |rendered: EncodedFrame| -> Result<()> {
                let EncodedFrame {
                    result,
                    elapsed,
                    ranks: (hits, misses),
                    temporal_cells,
                } = rendered;
                let (frame, cached) = result.map_err(io::Error::other)?;
                let rows = u16::try_from(frame.grid.len()).map_err(|_| "too many output rows")?;
                let columns = u16::try_from(frame.grid.first().map_or(0, Vec::len))
                    .map_err(|_| "too many output columns")?;
                if count == 0 {
                    header.width = columns;
                    header.height = rows;
                    header.validate()?;
                    serde_json::to_writer(&mut output, &header)?;
                    output.write_all(b"\n")?;
                }
                if (columns, rows) != (header.width, header.height)
                    || frame
                        .grid
                        .iter()
                        .any(|row| row.len() != usize::from(columns))
                {
                    return Err("rendered frame dimensions changed during encoding".into());
                }
                if settings.profile {
                    eprintln!(
                        "\nFrame {count}: {} local-rank-cache={hits}/{} temporal-cells={}",
                        frame.profile.summary(),
                        hits + misses,
                        temporal_cells
                    );
                }
                reused += u64::from(cached);
                // Store complete independent frames. Presentation replaces these
                // line separators with absolute row positions to prevent scrolling.
                let packet = format!(
                    "\x1b[?2026h\x1b[H{}\x1b[0m\x1b[?2026l",
                    frame.content.replace('\n', "\r\n")
                );
                serde_json::to_writer(&mut output, &packet)?;
                output.write_all(b"\n")?;
                count += 1;
                if let Some(observer) = observer {
                    observer(count, elapsed);
                } else {
                    eprint!(
                        "\rEncoded {count} frames ({:.2}s of video); last frame {:.0}ms",
                        count as f64 / settings.fps,
                        elapsed.as_secs_f64() * 1000.0
                    );
                }
                Ok(())
            };
            if workers.len() == 1 {
                write(render_encoded_frame(
                    &mut workers[0],
                    &frames[0],
                    abort.clone(),
                    frame_start,
                ))?;
            } else {
                // Bound decoded and completed frames to one batch. A slow
                // early frame cannot build an unbounded reorder queue.
                std::thread::scope(|scope| -> Result<()> {
                    let (sender, receiver) = mpsc::channel();
                    let abort = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                    for (index, (worker, rgba)) in workers.iter_mut().zip(&frames).enumerate() {
                        let sender = sender.clone();
                        let abort = abort.clone();
                        scope.spawn(move || {
                            let frame = render_encoded_frame(
                                worker,
                                rgba,
                                Some(abort),
                                frame_start + index as u64,
                            );
                            let _ = sender.send((index, frame));
                        });
                    }
                    drop(sender);
                    let mut ready = std::collections::BTreeMap::new();
                    let mut next = 0;
                    let mut completed = 0;
                    for (index, frame) in receiver {
                        completed += 1;
                        eprint!(
                            "\rRendered {completed}/{} frames in batch; last frame {:.0}ms",
                            frames.len(),
                            frame.elapsed.as_secs_f64() * 1000.0
                        );
                        ready.insert(index, frame);
                        while let Some(frame) = ready.remove(&next) {
                            if let Err(error) = write(frame) {
                                abort.store(true, std::sync::atomic::Ordering::Relaxed);
                                return Err(error);
                            }
                            next += 1;
                        }
                    }
                    Ok(())
                })?;
            }
        }
        if !decoder.0.wait()?.success() {
            return Err("FFmpeg failed to decode the video".into());
        }
        if count == 0 {
            return Err("FFmpeg produced no video frames".into());
        }
        output.write_all(b"null\n")?;
        output.flush()?;
        if observer.is_none() {
            eprintln!(
                "\rEncoded {count} frames in {:.1}s ({reused} reused) -> {}",
                started.elapsed().as_secs_f64(),
                options.output.display()
            );
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&options.output);
    }
    result
}

struct EncodedFrame {
    result: std::result::Result<(draw::RenderResult, bool), String>,
    elapsed: Duration,
    ranks: (u64, u64),
    temporal_cells: usize,
}

fn render_encoded_frame(
    worker: &mut (render::Renderer, render::FrameCache),
    rgba: &[u8],
    abort: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    index: u64,
) -> EncodedFrame {
    worker.0.set_frame_index(index);
    let started = Instant::now();
    let result = worker
        .0
        .render(rgba, &mut worker.1, abort)
        .map_err(|error| error.to_string());
    EncodedFrame {
        result,
        elapsed: started.elapsed(),
        ranks: draw::glyph_search_cache_stats(),
        temporal_cells: draw::glyph_search_temporal_reused_cells(),
    }
}

fn read_frame(reader: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "unfinished video (missing end marker)",
        ));
    }
    let frame: Option<String> =
        serde_json::from_str(&line).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(frame.map(String::into_bytes))
}

fn open_video(input: &Path) -> Result<(Header, BufReader<File>)> {
    let mut reader = BufReader::with_capacity(1024 * 1024, File::open(input)?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let header: Header = serde_json::from_str(&line)?;
    header.validate()?;
    Ok((header, reader))
}

fn sleep_until(started: Instant, seconds: f64) {
    std::thread::sleep(Duration::from_secs_f64(seconds).saturating_sub(started.elapsed()));
}

/// Build one screen update. Old files use CRLF, which scrolls at the bottom
/// edge; absolute row positions never advance the terminal's scroll position.
fn prepare_frame(
    frame: &[u8],
    header: &Header,
    viewport: Option<(u16, u16)>,
    fit: bool,
    clear: bool,
) -> Result<Vec<u8>> {
    let content = std::str::from_utf8(frame)?;
    let content = content.strip_prefix("\x1b[?2026h\x1b[H").unwrap_or(content);
    let content = content
        .strip_suffix("\x1b[0m\x1b[?2026l")
        .unwrap_or(content);
    let mut packet = Vec::with_capacity(frame.len());
    packet.write_all(b"\x1b[?2026h")?;
    if clear {
        packet.write_all(clear_screen(header.foreground_only))?;
    }
    let layout = present::layout(header, viewport, fit);
    if layout.columns != header.width || layout.rows != header.height {
        let (columns, rows) = (layout.columns, layout.rows);
        let mut source = Buffer::empty(Rect::new(0, 0, header.width, header.height));
        Paragraph::new(content.into_text()?).render(source.area, &mut source);
        let mut target = Buffer::empty(Rect::new(0, 0, columns, rows));
        for y in 0..rows {
            for x in 0..columns {
                // Include both edges, so the last row/column is preserved when
                // fitting. The ignore-size option deliberately clips instead.
                let sx = if fit {
                    sample_cell(x, columns, header.width)
                } else {
                    x
                };
                let sy = if fit {
                    sample_cell(y, rows, header.height)
                } else {
                    y
                };
                target[(x, y)] = source[(sx, sy)].clone();
            }
        }
        // These colours are image data, including when NO_COLOR is set for logs.
        crossterm::style::force_color_output(true);
        let mut backend = CrosstermBackend::new(&mut packet);
        backend.draw(target.content.iter().enumerate().map(|(i, cell)| {
            (
                (i % usize::from(columns)) as u16,
                (i / usize::from(columns)) as u16,
                cell,
            )
        }))?;
    } else {
        for (y, row) in content.lines().enumerate() {
            write!(packet, "\x1b[{};1H{row}", y + 1)?;
        }
    }
    if let Some(notice) = layout.notice { notice.write(&mut packet)?; }
    packet.write_all(b"\x1b[0m\x1b[?2026l")?;
    Ok(packet)
}

fn sample_cell(index: u16, output: u16, input: u16) -> u16 {
    if output == 1 {
        input / 2
    } else {
        (u32::from(index) * u32::from(input - 1) / u32::from(output - 1)) as u16
    }
}

fn clear_screen(foreground_only: bool) -> &'static [u8] {
    if foreground_only {
        b"\x1b[0m\x1b[2J"
    } else {
        b"\x1b[0m\x1b[40m\x1b[2J"
    }
}

fn stream_video(mut stream: TcpStream, input: &Path, viewport: Option<(u16, u16)>) -> Result<()> {
    let (header, mut reader) = open_video(input)?;
    stream.set_nodelay(true)?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let result = (|| -> Result<()> {
        // Remote ANSI terminal setup; no local TTY or size negotiation needed.
        stream.write_all(b"\x1b[?1049h\x1b[?6l\x1b[r\x1b[?25l\x1b[?7l")?;
        stream.write_all(clear_screen(header.foreground_only))?;
        let started = Instant::now();
        let mut index = 0u64;
        let mut presenter = present::Presenter::default();
        while let Some(frame) = read_frame(&mut reader)? {
            sleep_until(started, index as f64 / header.fps);
            if started.elapsed().as_secs_f64() < (index + 1) as f64 / header.fps {
                stream.write_all(&presenter.prepare(&frame, &header, viewport, true, false)?)?;
            }
            index += 1;
        }
        if index == 0 {
            return Err("video contains no frames".into());
        }
        sleep_until(started, index as f64 / header.fps);
        Ok(())
    })();
    // Restore the client's screen before sending EOF, including on file errors.
    let cleanup = stream.write_all(b"\x1b[?2026l\x1b[0m\x1b[?7h\x1b[?25h\x1b[?1049l");
    let _ = stream.shutdown(Shutdown::Write);
    result?;
    cleanup?;
    Ok(())
}

fn serve(input: PathBuf, bind: SocketAddr, viewport: Option<(u16, u16)>) -> Result<()> {
    let (header, _) = open_video(&input)?;
    let listener = TcpListener::bind(bind)?;
    eprintln!(
        "Serving {} ({}x{}, {} fps) on {}",
        input.display(),
        header.width,
        header.height,
        header.fps,
        listener.local_addr()?
    );
    loop {
        let (stream, peer) = listener.accept()?;
        let input = input.clone();
        // Each viewer has its own file reader and clock. A slow or disconnected
        // client does not hold up other viewers; socket writes time out in 5s.
        std::thread::spawn(move || {
            if let Err(error) = stream_video(stream, &input, viewport) {
                eprintln!("{peer}: {error}");
            }
        });
    }
}

struct Terminal;
impl Terminal {
    fn enter(foreground_only: bool) -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        let guard = Self;
        execute!(
            io::stdout(),
            EnterAlternateScreen,
            Hide,
            DisableLineWrap,
            Clear(ClearType::All)
        )?;
        io::stdout().write_all(b"\x1b[?6l\x1b[r")?;
        io::stdout().write_all(clear_screen(foreground_only))?;
        Ok(guard)
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = execute!(
            io::stdout(),
            EndSynchronizedUpdate,
            ResetColor,
            EnableLineWrap,
            Show,
            LeaveAlternateScreen
        );
        let _ = terminal::disable_raw_mode();
    }
}

fn wait_until(started: Instant, seconds: f64) -> Result<bool> {
    loop {
        let remaining = seconds - started.elapsed().as_secs_f64();
        // Poll even when late so skipped frames cannot starve keyboard input.
        let wait = Duration::from_secs_f64(remaining.clamp(0.0, 0.05));
        if event::poll(wait)? {
            match event::read()? {
                Event::Key(key)
                    if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
                        || (key.code == KeyCode::Char('c')
                            && key.modifiers.contains(KeyModifiers::CONTROL)) =>
                {
                    return Ok(false)
                }
                _ => {}
            }
        }
        if started.elapsed().as_secs_f64() >= seconds {
            return Ok(true);
        }
    }
}

fn play(input: PathBuf, ignore_terminal_size: bool) -> Result<()> {
    let (header, mut reader) = open_video(&input)?;
    if !io::stdout().is_terminal() || !io::stdin().is_terminal() {
        return Err("playback requires an interactive terminal".into());
    }

    // Bounded read-ahead separates disk/JSON work from presentation. Dropping
    // the receiver on quit also stops the reader without loading the whole file.
    let (sender, receiver) = mpsc::sync_channel(4);
    std::thread::spawn(move || loop {
        match read_frame(&mut reader) {
            Ok(Some(frame)) => {
                if sender.send(Ok(frame)).is_err() {
                    break;
                }
            }
            Ok(None) => break,
            Err(error) => {
                let _ = sender.send(Err(error));
                break;
            }
        }
    });
    let mut buffered = VecDeque::new();
    for frame in receiver.iter().take(4) {
        buffered.push_back(frame?);
    }
    if buffered.is_empty() {
        return Err("video contains no frames".into());
    }

    let terminal_guard = Terminal::enter(header.foreground_only)?;
    let mut output = io::stdout().lock();
    let started = Instant::now();
    let mut index = 0u64;
    let mut skipped = 0u64;
    let mut previous_size = None;
    let mut presenter = present::Presenter::default();
    loop {
        let frame = match buffered.pop_front() {
            Some(frame) => frame,
            None => match receiver.recv() {
                Ok(frame) => frame?,
                Err(_) => break,
            },
        };
        if !wait_until(started, index as f64 / header.fps)? {
            return Ok(());
        }
        if started.elapsed().as_secs_f64() < (index + 1) as f64 / header.fps {
            let size = terminal::size()?;
            let packet = presenter.prepare(
                &frame,
                &header,
                Some(size),
                !ignore_terminal_size,
                previous_size != Some(size),
            )?;
            output.write_all(&packet)?;
            output.flush()?;
            previous_size = Some(size);
        } else {
            skipped += 1;
        }
        index += 1;
    }
    // Keep the final frame on screen for its full interval too.
    wait_until(started, index as f64 / header.fps)?;
    drop(output);
    drop(terminal_guard);
    eprintln!("Played {index} frames; skipped {skipped} late frames.");
    Ok(())
}

pub fn run(
    arguments: impl IntoIterator<Item = impl Into<std::ffi::OsString> + Clone>,
) -> std::process::ExitCode {
    let result = match Cli::parse_from(arguments).command {
        Action::Encode(options) => encode(options),
        Action::Preview(options) => editor::run(options),
        Action::Play {
            input,
            ignore_terminal_size,
        } => play(input, ignore_terminal_size),
        Action::Serve {
            input,
            bind,
            terminal_size,
        } => serve(input, bind, terminal_size),
        Action::ServeSsh {
            input,
            bind,
            host_key,
        } => ssh_server::serve(input, bind, host_key),
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("video: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn header(width: u16, height: u16) -> Header {
        Header {
            format: "img2irc-video".into(),
            version: 1,
            width,
            height,
            fps: 24.0,
            foreground_only: false,
        }
    }

    #[test]
    fn eighty_rows_fit_seventy_with_notice_without_losing_any_corner() {
        let mut rows = vec![".".repeat(80); 80];
        rows[0] = format!("A{}B", ".".repeat(78));
        rows[79] = format!("C{}D", ".".repeat(78));
        let frame = format!(
            "\x1b[?2026h\x1b[H\x1b[38;2;12;34;56m{}\x1b[0m\x1b[?2026l",
            rows.join("\r\n")
        );
        let packet = prepare_frame(
            frame.as_bytes(),
            &header(80, 80),
            Some((286, 70)),
            true,
            true,
        )
        .unwrap();
        assert!(!packet.contains(&b'\n') && !packet.contains(&b'\r'));
        let text = packet.into_text().unwrap();
        let cells: String = text
            .lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect();
        let notice = "INCREASE WINDOW SIZE FOR BEST VIEWING EXPERIENCE";
        assert_eq!(cells.len(), 69 * 69 + notice.len());
        assert_eq!(cells.as_bytes()[0], b'A');
        assert_eq!(cells.as_bytes()[68], b'B');
        assert_eq!(cells.as_bytes()[68 * 69], b'C');
        assert_eq!(cells.as_bytes()[69 * 69 - 1], b'D');
        assert!(cells.ends_with(notice));
        assert!(text
            .lines
            .iter()
            .flat_map(|line| &line.spans)
            .any(|span| span.style.fg == Some(ratatui::style::Color::Rgb(12, 34, 56))));
    }

    #[test]
    fn exact_fit_positions_each_row_without_scrolling() {
        let frame = b"\x1b[?2026h\x1b[HABC\x1b[0m\r\nDEF\x1b[0m\x1b[?2026l";
        let packet = prepare_frame(frame, &header(3, 2), Some((3, 2)), true, false).unwrap();
        let packet = String::from_utf8(packet).unwrap();
        assert!(packet.contains("\x1b[1;1HABC"));
        assert!(packet.contains("\x1b[2;1HDEF"));
        assert!(!packet.contains(['\r', '\n']));
    }

    #[test]
    fn output_bounds_are_in_cells_and_do_not_add_padding() {
        assert_eq!(fit_size((16.0 / 9.0) / 0.5, (80, 80)), (80, 23));
        assert_eq!(fit_size((9.0 / 16.0) / 0.5, (80, 24)), (27, 24));
        assert_eq!(fit_size(1.0, (286, 70)), (70, 70));
    }

    #[test]
    fn frame_stream_preserves_ansi_unicode_and_raw_mode_line_endings() {
        let frame = "\x1b[?2026h\x1b[H\x1b[38;2;1;2;3m▀\r\n ▀\x1b[0m\x1b[?2026l";
        let mut encoded = serde_json::to_vec(frame).unwrap();
        encoded.extend_from_slice(b"\nnull\n");
        let mut reader = Cursor::new(encoded);
        assert_eq!(read_frame(&mut reader).unwrap().unwrap(), frame.as_bytes());
        assert!(read_frame(&mut reader).unwrap().is_none());
    }

    #[test]
    fn incomplete_encodes_are_errors_instead_of_clean_eof() {
        let mut reader = Cursor::new(b"\"complete frame\"\n");
        assert!(read_frame(&mut reader).unwrap().is_some());
        assert_eq!(
            read_frame(&mut reader).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
        let mut reader = Cursor::new(b"\"partial frame");
        assert_eq!(
            read_frame(&mut reader).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
