use super::*;
use std::sync::{atomic::AtomicBool, Arc};

// Store a few exact source frames, not just hashes: collisions cannot reuse the
// wrong image. Output colour edits deliberately do not invalidate glyph work.
#[derive(Default)]
pub(crate) struct FrameCache {
    entries: VecDeque<Entry>,
    bytes: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temporal_search_never_degrades_the_current_frames_initial_fit() {
        let options = match Cli::parse_from([
            "video",
            "preview",
            "unused.mp4",
            "--width",
            "8",
            "--height",
            "4",
            "--glyphs",
            "smooth,half",
            "--glyph-search",
            "--glyph-search-candidates",
            "2",
            "--glyph-search-passes",
            "1",
            "--glyph-search-temporal",
            "--glyph-search-reuse-tolerance",
            "8",
        ])
        .command
        {
            Action::Preview(options) => options,
            _ => unreachable!(),
        };
        let mut renderer = Renderer::new(&options, 1.0).unwrap();
        let mut initial = Renderer::new(&options, 1.0).unwrap();
        initial.args.glyph_search = false;
        initial.args.perceptual_score = true;
        let mut reused = false;
        for frame in 0..3 {
            let rgba: Vec<u8> = (0..renderer.frame_bytes() / 4)
                .flat_map(|p| {
                    let (x, y) = (p % renderer.width as usize, p / renderer.width as usize);
                    let c = if (x + y) % 23 < 11 { 220 } else { 30 };
                    [
                        c,
                        c,
                        c + if x > renderer.width as usize - 4 {
                            frame
                        } else {
                            0
                        },
                        255,
                    ]
                })
                .collect();
            // Initial matching must not touch the temporal search history.
            let (baseline, _) = initial
                .render(&rgba, &mut FrameCache::default(), None)
                .unwrap();
            let (repaired, _) = renderer
                .render(&rgba, &mut FrameCache::default(), None)
                .unwrap();
            reused |= draw::glyph_search_temporal_reused_cells() > 0;
            assert!(
                repaired.perceptual_score.unwrap().total
                    <= baseline.perceptual_score.unwrap().total + 1e-12
            );
        }
        assert!(reused, "the quality check must exercise a warm start");
        draw::set_glyph_search_temporal(false);
        draw::set_glyph_search_cache_budget(0);
    }
}

struct Entry {
    key: Vec<u8>,
    rgba: Vec<u8>,
    result: draw::RenderResult,
    bytes: usize,
}

impl FrameCache {
    fn get(&mut self, key: &[u8], rgba: &[u8]) -> Option<draw::RenderResult> {
        let index = self
            .entries
            .iter()
            .position(|e| e.key == key && e.rgba == rgba)?;
        let entry = self.entries.remove(index)?;
        let mut result = entry.result.clone();
        result.profile = draw::RenderProfile::default();
        result.initial_time = Duration::ZERO;
        self.entries.push_back(entry);
        Some(result)
    }

    fn insert(&mut self, key: &[u8], rgba: &[u8], result: &draw::RenderResult) {
        let bytes = key.len()
            + rgba.len()
            + result.content.len()
            + result.save_content.as_ref().map_or(0, String::len)
            + result.grid.iter().map(Vec::len).sum::<usize>()
                * std::mem::size_of::<draw::RenderedCell>();
        const LIMIT: usize = 32 * 1024 * 1024;
        if bytes > LIMIT {
            return;
        }
        while self.entries.len() >= 4 || self.bytes + bytes > LIMIT {
            if let Some(entry) = self.entries.pop_front() {
                self.bytes -= entry.bytes;
            }
        }
        self.entries.push_back(Entry {
            key: key.to_vec(),
            rgba: rgba.to_vec(),
            result: result.clone(),
            bytes,
        });
        self.bytes += bytes;
    }
}

pub(crate) struct Renderer {
    args: args::RenderArgs,
    pub(crate) store: Arc<font::GlyphStore>,
    glyphs: Vec<draw::FastGlyph>,
    colours: Vec<ImageEffect>,
    key: Vec<u8>,
    pub width: u32,
    pub height: u32,
    columns: u16,
    rows: u16,
    cache_bytes: usize,
    reuse_tolerance: u8,
    temporal: bool,
    fill_history: crate::flat_fill::History,
    frame_index: Option<u64>,
    last_completed_index: Option<u64>,
    history_epoch: u64,
    last_temporal_key: Option<Vec<u8>>,
    pub(crate) preview_base: bool,
}

impl Renderer {
    pub fn set_cache_budget(&mut self, megabytes: u32) {
        self.cache_bytes = megabytes as usize * 1024 * 1024;
    }
    pub fn new(options: &Settings, aspect: f64) -> Result<Self> {
        Self::from_args(options, aspect, Self::arguments(options))
    }

    pub(crate) fn arguments(options: &Settings) -> args::RenderArgs {
        let mut args = crate::args_to_render_args(&args::Args::parse_from(["video"]));
        args.font_size = options.font_size;
        args.blocks = options.glyphs.clone();
        args.render = match options.palette {
            Palette::Ansi24 => args::Render::Ansi24,
            Palette::Ansi256 => args::Render::Ansi,
        };
        args.contrast = options.contrast;
        args.blend_colours = options.blend_colours;
        args.smooth = options.smooth || options.smooth_shapes;
        args.smooth_shapes = options.smooth_shapes;
        args.glyph_search = options.glyph_search;
        args.glyph_search_candidates = options.glyph_search_candidates;
        args.glyph_search_passes = options.glyph_search_passes;
        args.glyph_search_finalists = options.glyph_search_finalists;
        args.discontinuity_threshold = options.discontinuity_threshold;
        args.region_guidance = options.region_guidance;
        args.smooth_orders = 1;
        args.smooth_neighbors = false;
        args.perceptual_score = false;
        args.pipeline = crate::effects::default_pipeline(&args);
        if options.line_thickness != 0.0 {
            args.pipeline
                .push(ImageEffect::DarkLines(options.line_thickness));
        }
        if options.flat_fill {
            args.pipeline
                .push(ImageEffect::FlatFillWithOptions(options.flat_fill_options));
        }
        if options.img2svg {
            args.pipeline.push(ImageEffect::Img2Svg(options.img2svg_options));
        }
        if options.darken_outlines {
            args.pipeline.push(ImageEffect::DarkenOutlines(options.outline_options));
        }
        args.image = Some(options.input.to_string_lossy().into_owned());
        args.width = Some(u32::from(options.width));
        args.height = Some(u32::from(options.height));
        args
    }

    pub(crate) fn from_args(
        options: &Settings,
        aspect: f64,
        mut args: args::RenderArgs,
    ) -> Result<Self> {
        if args.font_size <= 0.0 {
            args.font_size = 16.0;
        }
        let (catalog, _) = font::load_blocks_config();
        for name in &args.blocks {
            if !catalog.contains_key(name) {
                return Err(format!("unknown glyph set: {name}").into());
            }
        }
        let store = font::glyph_store(
            &args.blocks,
            &args.exclude_range,
            &args.exclude,
            &args.include_range,
            args.include.as_ref(),
            &args.font,
            args.font_size,
            false,
            None,
        );
        let glyphs = draw::prepare_glyphs(&store, &args);
        if glyphs.is_empty() {
            return Err("no usable glyphs in the selected sets".into());
        }
        // Font advances describe terminal-cell proportions; integer bitmap
        // dimensions only describe the raster used to match each glyph.
        let cell_aspect = f64::from(store.float_metrics.0 / store.float_metrics.1);
        let (columns, rows) = fit_size(aspect / cell_aspect, (options.width, options.height));
        args.width = Some(u32::from(columns));
        args.height = Some(u32::from(rows));
        let width = u32::from(columns) * store.metrics.0 as u32;
        let height = u32::from(rows) * store.metrics.1 as u32;
        if u64::from(width) * u64::from(height) > 100_000_000 {
            return Err("canvas is too large; reduce --width or --height".into());
        }
        let key = serde_json::to_vec(&(
            &args,
            options.glyph_search_reuse_tolerance,
            options.glyph_search_cache_mb,
            options.glyph_search_temporal,
        ))?;
        let mut renderer = Self {
            args,
            store: Arc::new(store),
            glyphs,
            colours: Vec::new(),
            key,
            width,
            height,
            columns,
            rows,
            cache_bytes: options.glyph_search_cache_mb as usize * 1024 * 1024,
            reuse_tolerance: options.glyph_search_reuse_tolerance,
            temporal: options.glyph_search_temporal,
            fill_history: crate::flat_fill::History::default(),
            frame_index: None,
            last_completed_index: None,
            history_epoch: 0,
            last_temporal_key: None,
            preview_base: false,
        };
        renderer.set_colours(options);
        Ok(renderer)
    }

    pub fn set_colours(&mut self, options: &Settings) {
        self.colours.clear();
        if options.quantize_colours.is_some() || options.quantize_threshold.is_some() {
            self.colours.push(ImageEffect::QuantizePalette {
                colours: options.quantize_colours.unwrap_or(16),
                threshold: options.quantize_threshold.unwrap_or(0.0),
                palette: Vec::new(),
                source: false,
            });
        }
    }

    pub fn header(&self, fps: f64) -> Header {
        Header {
            format: "img2irc-video".into(),
            version: 1,
            width: self.columns,
            height: self.rows,
            fps,
        }
    }

    pub fn frame_bytes(&self) -> usize {
        self.width as usize * self.height as usize * 4
    }

    pub(crate) fn set_frame_index(&mut self, index: u64) {
        if self.frame_index != Some(index)
            && self
                .last_completed_index
                .is_some_and(|previous| previous.checked_add(1) != Some(index))
        {
            self.fill_history = crate::flat_fill::History::default();
            self.history_epoch = self.history_epoch.wrapping_add(1);
            self.last_temporal_key = None;
        }
        self.frame_index = Some(index);
    }

    fn temporal_fill_enabled(&self) -> bool {
        self.frame_index.is_some()
            && self.args.pipeline.iter().any(|effect| match effect {
                ImageEffect::FlatFill => true,
                ImageEffect::FlatFillWithOptions(options) => {
                    options.temporal_tolerance > 0.0 && options.strength > 0.0
                }
                _ => false,
            })
    }

    pub fn decoder(&self, options: &Settings, index: u64, quiet: bool) -> Result<Decoder> {
        let offset = index as f64 / options.fps;
        let mut command = Command::new("ffmpeg");
        command.args(["-v", "error", "-nostdin"]);
        if options.start + offset > 0.0 {
            command.args(["-ss", &(options.start + offset).to_string()]);
        }
        command.arg("-i").arg(&options.input);
        if let Some(duration) = options.duration {
            command.args(["-t", &(duration - offset).max(0.0).to_string()]);
        }
        // Convert to RGB before nearest resizing: scaling subsampled YUV first
        // can mix chroma even when the terminal renderer uses exact colours.
        let filter = if self.args.blend_colours {
            format!("fps={},scale={}:{},setsar=1", options.fps, self.width, self.height)
        } else {
            format!("fps={},format=rgba,scale={}:{}:flags=neighbor,setsar=1",
                options.fps, self.width, self.height)
        };
        Ok(Decoder(
            command
                .args([
                    "-map", "0:v:0", "-an", "-vf", &filter, "-pix_fmt", "rgba", "-f", "rawvideo",
                    "pipe:1",
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(if quiet {
                    Stdio::null()
                } else {
                    Stdio::inherit()
                })
                .spawn()
                .map_err(|e| format!("could not start ffmpeg (install it on PATH): {e}"))?,
        ))
    }

    pub fn render(
        &mut self,
        rgba: &[u8],
        cache: &mut FrameCache,
        abort: Option<Arc<AtomicBool>>,
    ) -> Result<(draw::RenderResult, bool)> {
        draw::configure_glyph_search_cache(self.cache_bytes, self.reuse_tolerance);
        draw::set_glyph_search_temporal(self.temporal);
        let temporal_fill = self.temporal_fill_enabled();
        let mut key = self.key.clone();
        if temporal_fill {
            key.extend_from_slice(&self.history_epoch.to_le_bytes());
            key.extend_from_slice(&self.frame_index.unwrap().to_le_bytes());
        }
        let mut cached = cache.get(&key, rgba);
        if cached.is_none() && temporal_fill {
            // Identical consecutive inputs have the same tracked regions.
            // Keep their glyph-cache benefit without reusing unrelated history.
            cached = self
                .last_temporal_key
                .as_ref()
                .and_then(|previous| cache.get(previous, rgba));
            if let Some(base) = &cached {
                cache.insert(&key, rgba, base);
            }
        }
        let hit = cached.is_some();
        let base = match cached {
            Some(base) => base,
            None => {
                let image = PhotonImage::new(rgba.to_vec(), self.width, self.height);
                let render = || {
                    if self.preview_base {
                        crate::render_base_output(
                            self.args.clone(),
                            &self.store,
                            image,
                            &self.glyphs,
                            abort.clone(),
                        )
                    } else {
                        crate::render_output(
                            self.args.clone(),
                            &self.store,
                            image,
                            &self.glyphs,
                            abort.clone(),
                        )
                    }
                };
                let result = if temporal_fill {
                    // Borrow render inputs separately from the owned history.
                    let mut history = std::mem::take(&mut self.fill_history);
                    let result = crate::flat_fill::with_history(&mut history, || {
                        let result = render();
                        let commit = result.content != "Cancelled"
                            && abort.as_ref().is_none_or(|flag| {
                                !flag.load(std::sync::atomic::Ordering::Relaxed)
                            });
                        (result, commit)
                    });
                    self.fill_history = history;
                    result
                } else {
                    render()
                };
                if result.content == "Cancelled"
                    || abort
                        .as_ref()
                        .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Relaxed))
                {
                    return Err("cancelled".into());
                }
                cache.insert(&key, rgba, &result);
                result
            }
        };
        self.last_completed_index = self.frame_index;
        if temporal_fill {
            self.last_temporal_key = Some(key);
        }
        Ok((
            crate::output_colours::apply(
                &base,
                &self.store,
                &self.colours,
                self.args.render,
                self.args.as_preview,
                self.args.grayscale_tolerance,
            ),
            hit,
        ))
    }
}
