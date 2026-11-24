//! Infer transparent areas from source colours, then match glyph opacity.
//! The matching/refinement raster encodes coverage (0 = clear, 255 = ink),
//! never a terminal background colour. Fit actual ink colours after refinement.
use super::*;

/// Prefer a common source colour, with extra evidence from the image perimeter.
/// Coarse buckets pool small compression/antialiasing variations. No particular
/// hue or lightness is preferred; the inferred colour is only a segmentation cue.
fn transparent_colour(image: &AnsiImage) -> [u8; 3] {
    let height = image.bitmap.len();
    let width = image.bitmap.first().map_or(0, Vec::len);
    let mut counts = HashMap::<u16, (u64, u64, [u64; 3])>::new();
    let mut perimeter = 0u64;
    for (y, row) in image.bitmap.iter().enumerate() {
        for (x, &pixel) in row.iter().enumerate() {
            let rgb = unpack_rgb(pixel);
            let key = (u16::from(rgb[0] >> 4) << 8)
                | (u16::from(rgb[1] >> 4) << 4)
                | u16::from(rgb[2] >> 4);
            let entry = counts.entry(key).or_default();
            entry.0 += 1;
            if x == 0 || y == 0 || x + 1 == width || y + 1 == height {
                entry.1 += 1;
                perimeter += 1;
            }
            for (sum, channel) in entry.2.iter_mut().zip(rgb) {
                *sum += u64::from(channel);
            }
        }
    }
    counts
        .into_iter()
        .max_by_key(|&(key, (count, edge, _))| {
            (
                count * (perimeter + 4 * edge),
                count,
                std::cmp::Reverse(key),
            )
        })
        .map_or([0; 3], |(_, (count, _, sum))| {
            sum.map(|s| (s / count) as u8)
        })
}

pub(super) fn coverage_canvas(image: &AnsiImage, args: &RenderArgs) -> AnsiImage {
    let clear = transparent_colour(image);
    let height = image.bitmap.len();
    let width = image.bitmap[0].len();
    let pixels = image
        .bitmap
        .iter()
        .flatten()
        .flat_map(|&pixel| {
            let rgb = unpack_rgb(pixel);
            // Only source pixels close to the inferred clear region become clear.
            // Keeping opacity separate prevents dark ink from being lost on a
            // dark terminal, or light ink from being lost on a light terminal.
            let opaque = rgb.into_iter().zip(clear).any(|(a, b)| a.abs_diff(b) > 12);
            let alpha = if opaque { 255 } else { 0 };
            [alpha, alpha, alpha, 255]
        })
        .collect();
    AnsiImage::new(
        PhotonImage::new(pixels, width as u32, height as u32),
        image.glyphs.clone(),
        args.grayscale_tolerance,
        false,
    )
}

pub(super) fn render(
    image: &AnsiImage,
    args: &RenderArgs,
    render: Render,
    glyphs: &[FastGlyph],
    abort: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> RenderResult {
    if glyphs.is_empty() {
        return "Error: no glyphs available".into();
    }
    if image.bitmap.is_empty() || image.bitmap[0].is_empty() {
        return "".into();
    }
    if abort
        .as_ref()
        .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Relaxed))
    {
        return "Cancelled".into();
    }
    let started = RenderInstant::now();
    let coverage = coverage_canvas(image, args);
    let prepare_time = started.elapsed();
    let mut shape_args = args.clone();
    shape_args.overlays.clear();
    let mut result =
        super::render_blocks_smooth_inner(&coverage, &shape_args, render, glyphs, abort);
    if result.grid.is_empty() {
        return result;
    }
    result.profile.glyph_match += prepare_time;
    let recolour_started = RenderInstant::now();
    let indices: HashMap<_, _> = glyphs.iter().map(|g| (g.ch, g)).collect();
    let (gw, gh) = image.glyphs.metrics;
    for (y, row) in result.grid.iter_mut().enumerate() {
        for (x, cell) in row.iter_mut().enumerate() {
            let Some(glyph) = indices.get(&cell.char) else {
                continue;
            };
            let source = &image.block[y][x];
            let mask = &coverage.block[y][x];
            let mut counts = HashMap::<[u8; 3], usize>::new();
            let mut sum = [0u64; 3];
            let mut count = 0u64;
            for &pixel in &glyph.ones {
                let (px, py) = (pixel % gw, pixel / gw);
                if y * gh + py >= image.bitmap.len()
                    || x * gw + px >= image.bitmap[0].len()
                    || mask.pixels[py][px].orig == 0
                {
                    continue;
                }
                let rgb = unpack_rgb(source.pixels[py][px].orig);
                *counts.entry(rgb).or_default() += 1;
                for (s, channel) in sum.iter_mut().zip(rgb) {
                    *s += u64::from(channel);
                }
                count += 1;
            }
            let rgb = if args.blend_colours && count > 0 {
                sum.map(|s| (s as f64 / count as f64).round() as u8)
            } else {
                counts
                    .into_iter()
                    .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
                    .map_or_else(|| unpack_rgb(source.pixels[0][0].orig), |(rgb, _)| rgb)
            };
            let colour = if render == Render::Ansi24 {
                Colour::RGB(rgb)
            } else {
                let palette: &[u32] = if render == Render::Irc {
                    &IRC99
                } else {
                    &ANSI256
                };
                Colour::Index(nearest_strict_partitioned_hex_colour(
                    make_rgb_u32(&rgb),
                    palette,
                    args.grayscale_tolerance,
                ))
            };
            cell.fg = colour_to_rgb(&colour, render);
            cell.bg = None;
        }
    }
    result = encode_output_colours(&result, render, args.as_preview, args.grayscale_tolerance);
    result = apply_overlays_to_result(
        &result,
        &args.overlays,
        render,
        args.as_preview,
        args.grayscale_tolerance,
    );
    result.profile.encode += recolour_started.elapsed();
    result
}

/// Match opacity alone. The two numeric endpoints below are coverage levels,
/// not black/white display colours; actual RGB never enters this comparison.
pub(super) fn match_block(
    pixels: &[PrePixel],
    glyphs: &[FastGlyph],
    _args: &RenderArgs,
    _render: Render,
    abort: Option<&std::sync::atomic::AtomicBool>,
) -> Option<(BlockData, (usize, bool))> {
    let base: i64 = pixels.iter().map(|p| (p.r * p.r) as i64).sum();
    let mut best = None;
    let mut best_rank = (i64::MAX, usize::MAX);
    for (gi, glyph) in glyphs.iter().enumerate() {
        if gi & 63 == 0 && abort.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Relaxed))
        {
            return None;
        }
        let dot: i64 = glyph.ones.iter().map(|&i| pixels[i].r as i64).sum();
        let cost = base + glyph.ones.len() as i64 * 255 * 255 - 2 * 255 * dot;
        // Prefer less ink when two glyphs render equally well, including blanks.
        let rank = (cost, glyph.ones.len());
        if rank < best_rank {
            best_rank = rank;
            best = Some((
                BlockData {
                    fg: [255; 3],
                    bg: [0; 3],
                    fg_col: Some(Colour::RGB([255; 3])),
                    bg_col: None,
                    alt_fg_col: None,
                    alt_bg_col: None,
                },
                (gi, false),
            ));
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn options() -> RenderArgs {
        crate::args_to_render_args(&crate::args::Args::parse_from([
            "img2irc",
            "--foreground-only",
            "--disc-threshold",
            "0",
        ]))
    }

    fn canvas() -> AnsiImage {
        let glyphs = GlyphStore {
            glyphs: vec![
                (
                    '▀',
                    vec![vec![1; 2], vec![1; 2], vec![0; 2], vec![0; 2]],
                    "Test".into(),
                ),
                (
                    '▄',
                    vec![vec![0; 2], vec![0; 2], vec![1; 2], vec![1; 2]],
                    "Test".into(),
                ),
                (' ', vec![vec![0; 2]; 4], "Test".into()),
                ('█', vec![vec![1; 2]; 4], "Test".into()),
            ],
            groups: HashMap::new(),
            selected: std::collections::HashSet::from(['▀', '▄', ' ', '█']),
            metrics: (2, 4),
            float_metrics: (2.0, 4.0),
        };
        let rgba = (0..4)
            .flat_map(|y| {
                (0..8).flat_map(move |x| {
                    let ink = match x / 2 {
                        0 => y < 2,
                        1 => y >= 2,
                        2 => false,
                        _ => true,
                    };
                    let value = if ink { 255 } else { 0 };
                    [value, value, value, 255]
                })
            })
            .collect();
        AnsiImage::new(PhotonImage::new(rgba, 8, 4), glyphs, 32, false)
    }

    fn assert_foreground_only(result: &RenderResult) {
        assert!(result.foreground_only);
        assert!(result
            .grid
            .iter()
            .flatten()
            .all(|c| c.bg.is_none() && !c.inverted));
        for content in std::iter::once(&result.content).chain(result.save_content.iter()) {
            assert!(!content.contains("48;"), "{content:?}");
            assert!(!content.contains("[7m"), "{content:?}");
            for codes in content.split('\x03').skip(1) {
                let text = codes.trim_start_matches(|c: char| c.is_ascii_digit());
                assert!(
                    !text.starts_with(','),
                    "IRC background argument in {content:?}"
                );
            }
        }
    }

    #[test]
    fn foreground_only_uniform_white_can_choose_space_from_large_glyphs() {
        let mut args = options();
        let store = crate::font::glyph_store(
            &["large".into(), "space".into()],
            &[],
            &[],
            &[],
            None,
            &[],
            16.0,
            false,
            None,
        );
        let glyphs = prepare_glyphs(&store, &args);
        assert!(glyphs.iter().any(|glyph| glyph.ch == ' '));
        assert!(glyphs.iter().any(|glyph| glyph.ch == '𜸺'));
        let (gw, gh) = store.metrics;
        for rgb in [[255; 3], [0; 3], [30, 160, 210]] {
            let pixels = [rgb[0], rgb[1], rgb[2], 255].repeat(gw * gh);
            let image = PhotonImage::new(pixels, gw as u32, gh as u32);
            let canvas = AnsiImage::new(image, store.clone(), 32, false);
            for render in [Render::Irc, Render::Ansi, Render::Ansi24] {
                args.render = render;
                for refine in [false, true] {
                    args.smooth = refine;
                    args.smooth_shapes = refine;
                    args.smooth_orders = 1;
                    args.glyph_search = refine;
                    args.glyph_search_passes = 1;
                    args.region_guidance = refine;
                    let result = render_blocks(&canvas, &args, render, &glyphs, None);
                    assert_foreground_only(&result);
                    assert_eq!(result.grid[0][0].char, ' ');
                    let (_, _, rgba) = render_result_rgba(&result, &canvas.glyphs).unwrap();
                    assert!(rgba.chunks_exact(4).all(|pixel| pixel[3] == 0));
                }
                let braille = render_braille(&canvas, &canvas, &args, render);
                assert_foreground_only(&braille);
                assert!(braille.grid.iter().flatten().all(|c| c.char == '\u{2800}'));
            }
        }
    }

    #[test]
    fn foreground_only_infers_coloured_clear_regions_and_preserves_ink() {
        let store = canvas().glyphs;
        let mut args = options();
        args.render = Render::Ansi24;
        let glyphs = prepare_glyphs(&store, &args);
        let mut expected = [None, None];
        for (clear, ink) in [
            ([255; 3], [0; 3]),
            ([0; 3], [255; 3]),
            ([30, 160, 210], [220, 40, 80]),
            ([220, 40, 80], [30, 160, 210]),
        ] {
            let mut pixels = Vec::new();
            for y in 0..20 {
                for x in 0..10 {
                    let solid = y / 4 == 2
                        && match x / 2 {
                            1 => y % 4 < 2,
                            2 => y % 4 >= 2,
                            3 => true,
                            _ => false,
                        };
                    let rgb = if solid { ink } else { clear };
                    pixels.extend([rgb[0], rgb[1], rgb[2], 255]);
                }
            }
            let source = AnsiImage::new(PhotonImage::new(pixels, 10, 20), store.clone(), 32, false);
            for refine in [false, true] {
                args.smooth = refine;
                args.smooth_shapes = refine;
                args.smooth_orders = 1;
                args.glyph_search = refine;
                args.glyph_search_passes = 1;
                args.region_guidance = refine;
                let result = render_blocks(&source, &args, Render::Ansi24, &glyphs, None);
                assert_foreground_only(&result);
                if !refine {
                    assert_eq!(
                        result.grid[2].iter().map(|c| c.char).collect::<String>(),
                        " ▀▄█ "
                    );
                }
                assert!(result.grid[0].iter().all(|c| c.char == ' '));
                assert!(result
                    .grid
                    .iter()
                    .flatten()
                    .filter(|c| c.char != ' ')
                    .all(|c| c.fg == ink));
                let (_, _, rgba) = render_result_rgba(&result, &store).unwrap();
                assert!(rgba.chunks_exact(4).any(|p| p[..3] == ink && p[3] == 255));
                assert!(rgba.chunks_exact(4).any(|p| p[3] == 0));
                let geometry = (
                    result
                        .grid
                        .iter()
                        .flatten()
                        .map(|c| c.char)
                        .collect::<String>(),
                    result.score,
                    rgba.chunks_exact(4).map(|p| p[3]).collect::<Vec<_>>(),
                );
                let previous = expected[refine as usize].get_or_insert_with(|| geometry.clone());
                assert_eq!(
                    &geometry, previous,
                    "RGB must not change glyph coverage or its score"
                );
            }
        }
    }

    #[test]
    fn foreground_only_png_export_preserves_transparency() {
        let canvas = canvas();
        let mut args = options();
        args.render = Render::Ansi24;
        let glyphs = prepare_glyphs(&canvas.glyphs, &args);
        let result = render_blocks(&canvas, &args, Render::Ansi24, &glyphs, None);
        let path =
            std::env::temp_dir().join(format!("img2irc-transparent-{}.png", std::process::id()));
        save_as_png(&result, &canvas.glyphs, path.to_str().unwrap(), "{}").unwrap();
        let png = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        let image = image::load_from_memory(&png).unwrap();
        assert!(image.color().has_alpha());
        let (_, _, expected) = render_result_rgba(&result, &canvas.glyphs).unwrap();
        assert_eq!(image.to_rgba8().into_raw(), expected);
    }

    #[test]
    fn foreground_only_matches_ink_with_every_palette_and_preview() {
        let canvas = canvas();
        let mut args = options();
        let glyphs = prepare_glyphs(&canvas.glyphs, &args);
        for render in [Render::Ansi, Render::Ansi24, Render::Irc] {
            args.render = render;
            for preview in [false, true] {
                args.as_preview = preview;
                let result = render_blocks(&canvas, &args, render, &glyphs, None);
                assert_foreground_only(&result);
                assert_eq!(
                    result.grid[0].iter().map(|c| c.char).collect::<String>(),
                    "▄▀█ "
                );
                assert_eq!(
                    result.longest_line_bytes,
                    result
                        .save_content
                        .as_ref()
                        .unwrap_or(&result.content)
                        .lines()
                        .map(str::len)
                        .max()
                        .unwrap()
                );
            }
        }
        args.foreground_only = false;
        let result = render_blocks(&canvas, &args, Render::Irc, &glyphs, None);
        assert!(!result.foreground_only);
        assert!(result.grid.iter().flatten().all(|c| c.bg.is_some()));
    }

    #[test]
    fn foreground_only_refinement_never_paints_the_complement() {
        let canvas = canvas();
        let mut args = options();
        args.render = Render::Ansi24;
        args.smooth = true;
        args.smooth_shapes = true;
        args.smooth_orders = 1;
        args.glyph_search = true;
        args.glyph_search_candidates = 4;
        args.glyph_search_passes = 1;
        args.region_guidance = true;
        // Only the upper-half glyph is available. For the lower-half source,
        // painting its complement would be an exact but forbidden solution.
        let glyphs = prepare_glyphs(&canvas.glyphs, &args)
            .into_iter()
            .take(1)
            .collect::<Vec<_>>();
        let result = render_blocks(&canvas, &args, Render::Ansi24, &glyphs, None);
        assert_foreground_only(&result);
        assert!(result.grid.iter().flatten().all(|c| c.char == '▀'));
        let image = render_result_image(&result, &canvas.glyphs)
            .unwrap()
            .to_rgb8();
        assert!((2..4).all(|y| (0..8).all(|x| image.get_pixel(x, y).0 == [0; 3])));
    }

    #[test]
    fn foreground_only_survives_overlays_output_effects_and_braille() {
        let canvas = canvas();
        let mut args = options();
        let glyphs = prepare_glyphs(&canvas.glyphs, &args);
        let overlay = crate::args::TextOverlay {
            text: ",12".into(),
            source_text: None,
            figlet_font: None,
            x: 0,
            y: 0,
            w: 4,
            h: 1,
            fg: Some(crate::args::ColorSpec::Index(4)),
            bg: Some(crate::args::ColorSpec::Rgb([20, 30, 40])),
            wrap: false,
            auto_grow: false,
            transparent_spaces: false,
            bold: true,
            italic: true,
            underline: true,
        };
        let effects = [
            crate::pipeline::ImageEffect::ReplaceColour {
                from: [0; 3],
                to: [255; 3],
                tolerance: 0.0,
            },
            crate::pipeline::ImageEffect::QuantizePalette {
                colours: 1,
                threshold: 0.0,
                palette: vec![[240, 40, 40]],
                source: false,
            },
        ];
        for render in [Render::Ansi, Render::Ansi24, Render::Irc] {
            args.render = render;
            args.as_preview = render == Render::Irc;
            args.overlays = vec![overlay.clone()];
            for result in [
                render_blocks(&canvas, &args, render, &glyphs, None),
                render_braille(&canvas, &canvas, &args, render),
            ] {
                assert_foreground_only(&result);
                let overlaid = apply_overlays_to_result(
                    &result,
                    &[overlay.clone()],
                    render,
                    args.as_preview,
                    32,
                );
                assert_foreground_only(&overlaid);
                assert_eq!(overlaid.grid[0][0].char, ',');
                assert!(
                    overlaid.grid[0][0].bold
                        && overlaid.grid[0][0].italic
                        && overlaid.grid[0][0].underline
                );
                let recoloured = crate::output_colours::apply(
                    &overlaid,
                    &canvas.glyphs,
                    &effects,
                    render,
                    args.as_preview,
                    32,
                );
                assert_foreground_only(&recoloured);
            }
        }
    }

    #[test]
    fn foreground_only_settings_round_trip_and_default_off_for_old_settings() {
        let args = options();
        assert!(args.foreground_only);
        let mut saved = serde_json::to_value(&args).unwrap();
        assert!(
            serde_json::from_value::<RenderArgs>(saved.clone())
                .unwrap()
                .foreground_only
        );
        saved.as_object_mut().unwrap().remove("foreground_only");
        assert!(
            !serde_json::from_value::<RenderArgs>(saved)
                .unwrap()
                .foreground_only
        );
    }
}
