use super::*;

fn settings(extra: &[&str]) -> Settings {
    let mut argv = vec![
        "video",
        "preview",
        "unused.mp4",
        "--width",
        "4",
        "--height",
        "2",
    ];
    argv.extend_from_slice(extra);
    match Cli::parse_from(argv).command {
        Action::Preview(settings) => settings,
        _ => unreachable!(),
    }
}

#[test]
fn quantization_reuses_glyphs_and_matches_a_fresh_render() {
    let mut options = settings(&["--smooth"]);
    let mut renderer = render::Renderer::new(&options, 2.0).unwrap();
    let mut cache = render::FrameCache::default();
    let mut rgba: Vec<u8> = (0..renderer.frame_bytes() / 4)
        .flat_map(|i| {
            [
                ((i * 17) % 256) as u8,
                ((i * 7) % 256) as u8,
                ((i / 9 * 53) % 256) as u8,
                255,
            ]
        })
        .collect();
    let (base, hit) = renderer.render(&rgba, &mut cache, None).unwrap();
    assert!(!hit);
    options.quantize_colours = Some(2);
    options.quantize_threshold = Some(5.5);
    renderer.set_colours(&options);
    let (quantized, hit) = renderer.render(&rgba, &mut cache, None).unwrap();
    assert!(
        hit,
        "output colour edits should reuse matching and smoothing"
    );
    let (fresh, hit) = render::Renderer::new(&options, 2.0)
        .unwrap()
        .render(&rgba, &mut render::FrameCache::default(), None)
        .unwrap();
    assert!(!hit);
    assert_eq!(quantized.content, fresh.content);
    let colours: std::collections::HashSet<_> = quantized
        .grid
        .iter()
        .flatten()
        .flat_map(|c| std::iter::once(c.fg).chain(c.bg))
        .collect();
    assert!(colours.len() <= 2);
    assert_eq!(
        base.grid
            .iter()
            .flatten()
            .map(|c| c.char)
            .collect::<String>(),
        quantized
            .grid
            .iter()
            .flatten()
            .map(|c| c.char)
            .collect::<String>()
    );
    rgba[0] ^= 1;
    assert!(
        !renderer.render(&rgba, &mut cache, None).unwrap().1,
        "even a one-channel pixel change must invalidate reuse"
    );
    options.contrast = 10.0;
    assert!(
        !render::Renderer::new(&options, 2.0)
            .unwrap()
            .render(&rgba, &mut cache, None)
            .unwrap()
            .1
    );
}

#[test]
fn quantization_cli_accepts_merge_only_and_rejects_bad_thresholds() {
    let options = settings(&["--quantize-colours", "0", "--quantize-merge", "3.25"]);
    assert_eq!(options.quantize_colours, Some(0));
    assert_eq!(options.quantize_threshold, Some(3.25));
    for threshold in ["NaN", "inf", "-1", "100.1"] {
        assert!(Cli::try_parse_from([
            "video",
            "encode",
            "in.mp4",
            "out.i2v",
            "--quantize-threshold",
            threshold
        ])
        .is_err());
    }
}

#[test]
fn temporal_rankings_preserve_results_and_invalidate_changed_inputs() {
    let mut options = settings(&[
        "--glyphs",
        "smooth,half",
        "--glyph-search",
        "--glyph-search-candidates",
        "2",
        "--glyph-search-passes",
        "1",
        "--disc-threshold",
        "0",
    ]);
    let mut renderer = render::Renderer::new(&options, 1.0).unwrap();
    let mut rgba: Vec<u8> = (0..renderer.frame_bytes() / 4)
        .flat_map(|p| {
            let (x, y) = (p % renderer.width as usize, p / renderer.width as usize);
            let c = if (x + y) % 23 < 11 { 220 } else { 30 };
            [c, c, c, 255]
        })
        .collect();
    let (first, _) = renderer
        .render(&rgba, &mut render::FrameCache::default(), None)
        .unwrap();
    let before = draw::glyph_search_cache_stats().0;
    let (repeated, _) = renderer
        .render(&rgba, &mut render::FrameCache::default(), None)
        .unwrap();
    assert_eq!(first.content, repeated.content);
    assert!(
        draw::glyph_search_cache_stats().0 > before,
        "local rankings should be reused without the whole-frame cache"
    );
    rgba[0] ^= 63;
    let (changed, _) = renderer
        .render(&rgba, &mut render::FrameCache::default(), None)
        .unwrap();
    options.glyph_search_cache_mb = 0;
    let (uncached, _) = render::Renderer::new(&options, 1.0)
        .unwrap()
        .render(&rgba, &mut render::FrameCache::default(), None)
        .unwrap();
    assert_eq!(changed.content, uncached.content);
    draw::set_glyph_search_cache_budget(0);
}

#[test]
fn video_settings_enter_the_shared_editor_without_losing_effect_parameters() {
    let options = settings(&[
        "--glyph-search-candidates",
        "9",
        "--glyph-search-passes",
        "4",
        "--glyph-search-finalists",
        "0",
        "--flat-fill",
        "--flat-fill-tolerance",
        "4.5",
        "--flat-fill-min-area",
        "40",
        "--line-thickness",
        "1.5",
    ]);
    let args = render::Renderer::arguments(&options);
    assert_eq!(args.glyph_search_candidates, 9);
    assert_eq!(args.glyph_search_passes, 4);
    assert_eq!(args.glyph_search_finalists, 0);
    assert!(args.pipeline.contains(&ImageEffect::DarkLines(1.5)));
    assert!(args.pipeline.iter().any(|effect| matches!(effect,
        ImageEffect::FlatFillWithOptions(options) if options.tolerance == 4.5 && options.min_area == 40)));
}
