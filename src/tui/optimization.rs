use super::*;
use serde_json::{json, Value};
use std::io::Write;

pub(super) fn load_glyphs(
    args: &RenderArgs,
    font_size: f32,
    cancel: Arc<AtomicBool>,
) -> Arc<font::GlyphStore> {
    Arc::new(font::glyph_store(
        &args.blocks,
        &args.exclude_range,
        &args.exclude,
        &args.include_range,
        args.include.as_ref(),
        &args.font,
        font_size,
        false,
        Some(cancel),
    ))
}

pub(super) fn optimization_score(result: &draw::RenderResult, shortest_line: bool) -> Option<f64> {
    if result.grid.is_empty() {
        return None;
    }
    let score = if shortest_line {
        result.longest_line_bytes as f64
    } else {
        result.score
    };
    score.is_finite().then_some(score)
}

impl App {
    pub(super) fn perform_font_size_scan(&mut self) {
        if self.font_scan_running || self.is_optimizing {
            if let Some(flag) = &self.auto_optimize_cancel {
                flag.store(true, Ordering::Relaxed);
            }
            self.optimization_progress = "Cancelling...".to_string();
            return;
        }
        let Some(path) = self.render_args.image.clone().filter(|path| !path.is_empty()) else {
            return;
        };
        // Synchronize current controls, then stop the ordinary render so no
        // expensive refinement runs alongside this sweep.
        self.synchronize_render_args(false);
        self.current_cancel_flag.store(true, Ordering::Relaxed);
        let args = crate::font_scan::quick_args(self.render_args.clone());
        let cancel = Arc::new(AtomicBool::new(false));
        self.auto_optimize_cancel = Some(cancel.clone());
        self.font_scan_running = true;
        self.is_optimizing = true;
        self.optimization_best_score = None;
        self.optimization_progress = "Scanning font sizes 9–30...".to_string();
        let (tx, rx) = mpsc::channel(32);
        self.rx_optimization = Some(rx);

        tokio::spawn(async move {
            let result = async {
                let initial_args = args.clone();
                let initial_cancel = cancel.clone();
                let glyphs = tokio::task::spawn_blocking(move || {
                    load_glyphs(&initial_args, crate::font_scan::MAX_SIZE as f32, initial_cancel)
                }).await.map_err(|error| format!("Font preparation failed: {error}"))?;
                let image = crate::load_image_from_url_or_path(&path, &args, &glyphs).await
                    .map_err(|error| error.to_string())?;
                let progress_tx = tx.clone();
                tokio::task::spawn_blocking(move || {
                    let mut log = OptimizationLog::new(&args, false);
                    log.record("start", json!({
                        "objective": "perceptual", "total_trials": 22, "quick_font_scan": true,
                    }));
                    let mut best_score = f64::INFINITY;
                    let mut best_args = args.clone();
                    let mut completed = 0;
                    let mut valid = 0;
                    let scan = crate::font_scan::scan(|size| -> Result<f64, String> {
                        if cancel.load(Ordering::Relaxed) || progress_tx.is_closed() {
                            return Err("Cancelled".to_string());
                        }
                        let mut trial = args.clone();
                        trial.font_size = size;
                        let started = Instant::now();
                        let glyphs = load_glyphs(&trial, size, cancel.clone());
                        let fast_glyphs = draw::prepare_glyphs(&glyphs, &trial);
                        let output = crate::render_base_output(
                            trial.clone(), &glyphs, image.clone(), &fast_glyphs, Some(cancel.clone()),
                        );
                        if cancel.load(Ordering::Relaxed) {
                            return Err("Cancelled".to_string());
                        }
                        let score = output.perceptual_score.as_ref()
                            .filter(|_| !output.grid.is_empty())
                            .map_or(f64::INFINITY, |score| score.total);
                        completed += 1;
                        valid += usize::from(score.is_finite());
                        let new_best = score < best_score;
                        if new_best {
                            best_score = score;
                            best_args = trial.clone();
                        }
                        log.result("trial", completed, completed, &trial, &output,
                            started.elapsed().as_secs_f64(), best_score, new_best);
                        Ok(score)
                    }, |progress| {
                        let best = progress.best.map_or(String::new(), |best|
                            format!(" · best {} ({:.3})", best.font_size, best.score));
                        let _ = progress_tx.blocking_send(OptimizationUpdate::Progress(format!(
                            "Font {} · {}/{}{}", progress.font_size, progress.completed, progress.total, best,
                        )));
                    });
                    log.finish(&best_args, best_score, completed, valid, cancel.load(Ordering::Relaxed));
                    scan?.ok_or_else(|| "No font size produced a valid Perceptual score".to_string())
                }).await.unwrap_or_else(|error| Err(format!("Scan worker failed: {error}")))
            }.await;
            let _ = tx.send(OptimizationUpdate::FontScanFinished(result)).await;
        });
    }
}

fn parameters(args: &RenderArgs) -> Value {
    let (top, right, bottom, left) = args.trim.unwrap_or_default();
    let (scale_x, scale_y) = args.scale.unwrap_or((1.0, 1.0));
    json!({
        "width": args.width,
        "height": args.height,
        "trim_top": top,
        "trim_right": right,
        "trim_bottom": bottom,
        "trim_left": left,
        "font_size": args.font_size,
        "scale_x": scale_x,
        "scale_y": scale_y,
    })
}

pub(super) struct OptimizationLog {
    file: Option<std::fs::File>,
    run_id: String,
    started: Instant,
    shortest_line: bool,
}

impl OptimizationLog {
    pub(super) fn new(args: &RenderArgs, shortest_line: bool) -> Self {
        let file = args.optimize_log.as_ref().and_then(|path| {
            match OpenOptions::new().create(true).append(true).open(path) {
                Ok(file) => Some(file),
                Err(error) => {
                    log::error!("Cannot open optimization log {}: {error}", path.display());
                    None
                }
            }
        });
        Self {
            file,
            run_id: format!(
                "{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos(),
            ),
            started: Instant::now(),
            shortest_line,
        }
    }

    pub(super) fn record(&mut self, event: &str, mut data: Value) {
        let Some(file) = self.file.as_mut() else {
            return;
        };
        data["event"] = event.into();
        data["run_id"] = self.run_id.clone().into();
        data["elapsed_seconds"] = self.started.elapsed().as_secs_f64().into();
        let result = (|| -> Result<(), Box<dyn Error>> {
            serde_json::to_writer(&mut *file, &data)?;
            file.write_all(b"\n")?;
            file.flush()?;
            Ok(())
        })();
        if let Err(error) = result {
            log::error!("Cannot write optimization log: {error}");
            self.file = None;
        }
    }

    pub(super) fn result(
        &mut self,
        event: &str,
        trial: usize,
        completed: usize,
        args: &RenderArgs,
        result: &draw::RenderResult,
        render_seconds: f64,
        best_score: f64,
        new_best: bool,
    ) {
        if self.file.is_none() {
            return;
        }
        let score = optimization_score(result, self.shortest_line);
        let mut data = parameters(args);
        data["trial"] = trial.into();
        data["completed"] = completed.into();
        data["status"] = if result.content == "Cancelled" {
            "cancelled"
        } else if score.is_some() {
            "ok"
        } else {
            "invalid"
        }
        .into();
        data["score"] = json!(score);
        data["quality_score"] = json!(result.score);
        data["perceptual_score"] = json!(result.perceptual_score);
        data["longest_line_bytes"] = result.longest_line_bytes.into();
        data["grid_width"] = result.grid.first().map_or(0, Vec::len).into();
        data["grid_height"] = result.grid.len().into();
        data["render_seconds"] = render_seconds.into();
        data["best_score"] = json!(best_score);
        data["new_best"] = new_best.into();
        if score.is_none() {
            data["message"] = result.content.clone().into();
        }
        self.record(event, data);
    }

    pub(super) fn finish(
        &mut self,
        best_args: &RenderArgs,
        best_score: f64,
        completed: usize,
        valid: usize,
        cancelled: bool,
    ) {
        let mut data = parameters(best_args);
        data["best_score"] = json!(best_score);
        data["completed"] = completed.into();
        data["valid"] = valid.into();
        data["cancelled"] = cancelled.into();
        self.record("finish", data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn temporary_path(extension: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "img2irc-optimization-{}-{}.{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            extension,
        ))
    }

    #[test]
    fn optimization_logging_is_opt_in_and_old_settings_still_load() {
        let defaults = crate::args_to_render_args(&crate::args::Args::parse_from(["img2irc"]));
        assert!(defaults.optimize_log.is_none());
        let args = crate::args::Args::parse_from(["img2irc", "--optimize-log", "trials.jsonl"]);
        assert_eq!(
            crate::args_to_tui_render_args(&args).optimize_log,
            Some("trials.jsonl".into())
        );
        let mut saved = serde_json::to_value(defaults).unwrap();
        saved.as_object_mut().unwrap().remove("optimize_log");
        assert!(serde_json::from_value::<RenderArgs>(saved)
            .unwrap()
            .optimize_log
            .is_none());
    }

    #[test]
    fn failed_cancelled_and_nonfinite_results_cannot_win() {
        for message in ["Error: no glyphs available", "Cancelled", ""] {
            let result = draw::RenderResult::from(message);
            assert_eq!(optimization_score(&result, false), None);
            assert_eq!(optimization_score(&result, true), None);
        }
        let mut result = draw::RenderResult::from("");
        result.grid = vec![vec![draw::RenderedCell {
            char: ' ',
            fg: [0; 3],
            bg: None,
            inverted: false,
            bold: false,
            italic: false,
            underline: false,
        }]];
        for score in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            result.score = score;
            assert_eq!(optimization_score(&result, false), None);
        }
        result.score = 19.0;
        result.longest_line_bytes = 42;
        assert_eq!(optimization_score(&result, false), Some(19.0));
        assert_eq!(optimization_score(&result, true), Some(42.0));
    }

    #[tokio::test]
    async fn font_sweep_matches_manual_scores_and_logs_every_trial() {
        let image_path = temporary_path("png");
        let log_path = temporary_path("jsonl");
        let image = image::RgbaImage::from_fn(96, 64, |x, y| {
            let inside = (x as i32 - 48).pow(2) + (y as i32 - 32).pow(2) < 24 * 24;
            image::Rgba(if inside {
                [30 + (x % 13) as u8, 60, 170, 255]
            } else {
                [230, 240, 210, 255]
            })
        });
        image.save(&image_path).unwrap();
        let source = PhotonImage::new(image.into_raw(), 96, 64);
        let mut args = crate::generator::default_options();
        args.image = Some(image_path.to_string_lossy().into_owned());
        args.font = vec![concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/static/CascadiaCode-Regular.ttf"
        )
        .into()];
        args.blocks = vec!["smooth".into()];
        args.include = Some(" ▀▄█▌▐/\\_".into());
        args.font_size = 28.0;
        args.width = Some(10);
        args.trim = Some((3, 1, 2, 4));
        args.pipeline = vec![crate::pipeline::ImageEffect::Invert];
        args.as_preview = true;
        args.smooth = true;
        args.optimize_log = Some(log_path.clone());

        let mut manual_scores = std::collections::BTreeMap::new();
        for size in 9..=30 {
            let mut a = args.clone();
            a.font_size = size as f32;
            let gs = load_glyphs(&a, a.font_size, Arc::new(AtomicBool::new(false)));
            let fg = draw::prepare_glyphs(&gs, &a);
            let result = crate::render_output(a, &gs, source.clone(), &fg, None);
            manual_scores.insert(size, optimization_score(&result, false).unwrap());
        }
        let minimum = manual_scores
            .values()
            .copied()
            .fold(f64::INFINITY, f64::min);

        for randomize in [false, true] {
            let flag = Arc::new(AtomicBool::new(false));
            let (tx, _rx) = watch::channel((args.clone(), flag.clone(), Instant::now()));
            let (_result_tx, result_rx) = mpsc::channel(1);
            let mut app = App::new(args.clone(), tx, result_rx, flag);
            // A preview with stale glyphs must not affect the baseline or the
            // candidate matching the current font-size setting.
            app.arc_glyphs = Some(load_glyphs(&args, 9.0, Arc::new(AtomicBool::new(false))));
            app.perform_auto_optimize();
            let state = app.auto_optimize_state.as_mut().unwrap();
            state.enabled_rows = [false; 8];
            state.enabled_rows[5] = true;
            state.inputs[15] = TextArea::new(vec!["9".into()]);
            state.inputs[16] = TextArea::new(vec!["30".into()]);
            state.inputs[17] = TextArea::new(vec!["1".into()]);
            state.batch_size = 3;
            state.randomize = randomize;
            app.start_auto_optimization();
            let mut updates = app.rx_optimization.take().unwrap();
            let winner = tokio::time::timeout(Duration::from_secs(60), async {
                let mut winner = None;
                while let Some(update) = updates.recv().await {
                    match update {
                        OptimizationUpdate::BestResult(a, score) => winner = Some((a, score)),
                        OptimizationUpdate::Finished => break,
                        OptimizationUpdate::Progress(_) | OptimizationUpdate::FontScanFinished(_) => {}
                    }
                }
                winner.unwrap()
            })
            .await
            .unwrap();
            assert!((winner.1 - minimum).abs() < 1e-10);
            assert_eq!(manual_scores[&(winner.0.font_size as i32)], winner.1);
            assert!(winner.1 <= manual_scores[&16]);
        }

        let records: Vec<Value> = std::fs::read_to_string(&log_path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.len(), 2 * (22 + 3));
        for run in records.chunks(25) {
            assert_eq!(run[0]["event"], "start");
            assert_eq!(run[0]["objective"], "perceptual");
            assert_eq!(run[0]["total_trials"], 22);
            assert_eq!(run[1]["event"], "baseline");
            let mut sizes = Vec::new();
            let mut trial_ids = Vec::new();
            for trial in &run[2..24] {
                assert_eq!(trial["event"], "trial");
                assert_eq!(trial["status"], "ok");
                assert_eq!(trial["run_id"], run[0]["run_id"]);
                let size = trial["font_size"].as_f64().unwrap() as i32;
                assert!((trial["score"].as_f64().unwrap() - manual_scores[&size]).abs() < 1e-10);
                assert!(trial["perceptual_score"]["roughness"].is_number());
                assert!(trial["render_seconds"].as_f64().unwrap() >= 0.0);
                sizes.push(size);
                trial_ids.push(trial["trial"].as_u64().unwrap());
            }
            sizes.sort();
            trial_ids.sort();
            assert_eq!(sizes, (9..=30).collect::<Vec<_>>());
            assert_eq!(trial_ids, (1..=22).collect::<Vec<_>>());
            assert_eq!(run[24]["event"], "finish");
            assert_eq!(run[24]["completed"], 22);
            assert_eq!(run[24]["valid"], 22);
            assert!((run[24]["best_score"].as_f64().unwrap() - minimum).abs() < 1e-10);
            assert_eq!(run[24]["cancelled"], false);
        }
        assert_ne!(records[0]["run_id"], records[25]["run_id"]);
        std::fs::remove_file(image_path).unwrap();
        std::fs::remove_file(log_path).unwrap();
    }
}
