//! Video sources for the shared editor. Rendering settings remain RenderArgs.
use super::*;
use std::sync::{atomic::AtomicBool, Arc, Mutex};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Clip {
    pub input: PathBuf,
    pub start: f64,
    pub duration: f64,
    pub fps: f64,
    pub cache_mb: u32,
    pub reuse_tolerance: u8,
    pub temporal: bool,
    pub aspect: f64,
    pub source_duration: f64,
}

impl Clip {
    pub fn open(input: PathBuf) -> Result<Self> {
        let aspect = video_aspect(&input)?;
        let probe = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration:stream=duration",
                "-of",
                "json",
            ])
            .arg(&input)
            .output()?;
        if !probe.status.success() {
            return Err("ffprobe could not read video duration".into());
        }
        let info: serde_json::Value = serde_json::from_slice(&probe.stdout)?;
        let duration = info["format"]["duration"]
            .as_str()
            .and_then(|v| v.parse::<f64>().ok())
            .or_else(|| {
                info["streams"]
                    .as_array()?
                    .iter()
                    .find_map(|s| s["duration"].as_str()?.parse::<f64>().ok())
            })
            .filter(|v| v.is_finite() && *v > 0.0)
            .ok_or("video has no finite duration")?;
        Ok(Self {
            input,
            start: 0.0,
            duration,
            fps: 24.0,
            cache_mb: 64,
            reuse_tolerance: 0,
            temporal: false,
            aspect,
            source_duration: duration,
        })
    }

    pub fn frames(&self) -> u64 {
        (self.duration * self.fps).ceil().max(1.0) as u64
    }

    fn settings(&self, args: &args::RenderArgs) -> Settings {
        static DEFAULTS: std::sync::OnceLock<Settings> = std::sync::OnceLock::new();
        let mut settings = DEFAULTS
            .get_or_init(|| {
                let Action::Preview(settings) =
                    Cli::parse_from(["video", "preview", "source"]).command
                else {
                    unreachable!()
                };
                settings
            })
            .clone();
        settings.input = self.input.clone();
        settings.start = self.start;
        settings.duration = Some(self.duration);
        settings.fps = self.fps;
        settings.width = args.width.unwrap_or(80).clamp(1, 65535) as u16;
        settings.height = args.height.unwrap_or(65535).clamp(1, 65535) as u16;
        settings.glyph_search_cache_mb = self.cache_mb;
        settings.glyph_search_reuse_tolerance = self.reuse_tolerance;
        settings.glyph_search_temporal = self.temporal;
        settings
    }
}

pub(crate) fn is_video(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" | "mpeg" | "mpg" | "ts"
            )
        })
}

#[derive(Clone)]
pub(crate) struct Request {
    pub clip: Clip,
    pub index: u64,
    pub status: Arc<Mutex<String>>,
}

#[derive(Default)]
pub(crate) struct Source {
    key: Vec<u8>,
    renderer: Option<render::Renderer>,
    cache: render::FrameCache,
    decoder: Option<Decoder>,
    decoded_clip: Option<Clip>,
    decoded_blending: bool,
    next: u64,
    raw_index: Option<u64>,
    rgba: Vec<u8>,
}

pub(crate) struct Rendered {
    pub result: draw::RenderResult,
    pub glyphs: Arc<font::GlyphStore>,
    pub image: Arc<PhotonImage>,
}

impl Source {
    pub fn render(
        &mut self,
        mut args: args::RenderArgs,
        request: &Request,
        abort: Arc<AtomicBool>,
        progress: impl Fn(Arc<font::GlyphStore>, &draw::search_status::GlyphSearchProgress) + 'static,
    ) -> Result<Rendered> {
        request.status.lock().unwrap().clear();
        if self.decoded_blending != args.blend_colours {
            self.decoder = None;
            self.raw_index = None;
            self.decoded_clip = None;
            self.decoded_blending = args.blend_colours;
        }
        args.pipeline.retain(|effect| !effect.is_output_colour());
        let settings = request.clip.settings(&args);
        let key = serde_json::to_vec(&(
            &args,
            request.clip.cache_mb,
            request.clip.reuse_tolerance,
            request.clip.temporal,
        ))?;
        if self.renderer.is_none() || self.key != key {
            let mut renderer = render::Renderer::from_args(&settings, request.clip.aspect, args)?;
            renderer.preview_base = true;
            if self
                .renderer
                .as_ref()
                .is_none_or(|old| (old.width, old.height) != (renderer.width, renderer.height))
            {
                self.decoder = None;
                self.raw_index = None;
            }
            self.renderer = Some(renderer);
            self.key = key;
        }
        if self.decoded_clip.as_ref() != Some(&request.clip) {
            self.decoder = None;
            self.raw_index = None;
            self.decoded_clip = Some(request.clip.clone());
        }
        let renderer = self.renderer.as_mut().unwrap();
        self.rgba.resize(renderer.frame_bytes(), 0);
        if self.raw_index != Some(request.index) {
            if self.decoder.is_none() || self.next != request.index {
                self.decoder = Some(renderer.decoder(&settings, request.index, true)?);
            }
            let child = &mut self.decoder.as_mut().unwrap().0;
            let stdout = child.stdout.as_mut().ok_or("missing FFmpeg stdout")?;
            if stdout.read(&mut self.rgba[..1])? == 0 {
                self.decoder = None;
                return Err("end of video (or FFmpeg decoding failed)".into());
            }
            stdout.read_exact(&mut self.rgba[1..])?;
            self.raw_index = Some(request.index);
            self.next = request.index + 1;
        }
        let glyphs = renderer.store.clone();
        renderer.set_frame_index(request.index);
        let (result, _) = draw::search_status::observe_glyph_search(
            move |update| progress(glyphs.clone(), update),
            || renderer.render(&self.rgba, &mut self.cache, Some(abort)),
        )?;
        Ok(Rendered {
            result,
            glyphs: renderer.store.clone(),
            image: Arc::new(PhotonImage::new(
                self.rgba.clone(),
                renderer.width,
                renderer.height,
            )),
        })
    }
}

pub(crate) fn export(
    clip: Clip,
    mut args: args::RenderArgs,
    output: PathBuf,
    abort: Arc<AtomicBool>,
    progress: impl Fn(u64, Duration),
) -> Result<()> {
    // The editor can use the IRC palette too. Store its ANSI preview so the
    // encoded picture keeps those same colours while remaining playable.
    args.as_preview = args.render == args::Render::Irc;
    let settings = clip.settings(&args);
    super::encode_with_args(
        Encode {
            settings,
            output,
            workers: 1,
        },
        Some(args),
        Some(&progress),
        Some(abort),
    )
}

pub(super) fn run(options: Settings) -> Result<()> {
    let mut clip = Clip::open(options.input.clone())?;
    clip.start = options.start;
    if clip.start >= clip.source_duration {
        return Err("start time is beyond the end of the video".into());
    }
    clip.duration = options
        .duration
        .unwrap_or(clip.source_duration - clip.start)
        .min(clip.source_duration - clip.start);
    clip.fps = options.fps;
    clip.cache_mb = options.glyph_search_cache_mb;
    clip.reuse_tolerance = options.glyph_search_reuse_tolerance;
    clip.temporal = options.glyph_search_temporal;
    let mut args = render::Renderer::arguments(&options);
    if options.quantize_colours.is_some() || options.quantize_threshold.is_some() {
        args.pipeline.push(ImageEffect::QuantizePalette {
            colours: options.quantize_colours.unwrap_or(16),
            threshold: options.quantize_threshold.unwrap_or(0.0),
            palette: vec![],
            source: false,
        });
    }
    tokio::runtime::Runtime::new()?.block_on(crate::tui::run_video(args, clip))
}
