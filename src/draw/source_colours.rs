//! Choose actual, local RGB values separately under a glyph's two masks.
use super::*;

pub(super) fn observed(
    pixels: impl IntoIterator<Item = [u8; 3]>,
    glyph: &FastGlyph,
    inverted: bool,
) -> ([u8; 3], [u8; 3]) {
    observed_indexed(pixels.into_iter().enumerate(), glyph, inverted)
}

fn observed_indexed(
    pixels: impl IntoIterator<Item = (usize, [u8; 3])>,
    glyph: &FastGlyph,
    inverted: bool,
) -> ([u8; 3], [u8; 3]) {
    let mut counts = HashMap::<[u8; 3], [usize; 2]>::new();
    for (index, rgb) in pixels {
        let foreground = (glyph.bitmap[index] != 0) ^ inverted;
        counts.entry(rgb).or_default()[foreground as usize] += 1;
    }
    let mode = |side: usize| {
        counts
            .iter()
            .filter(|(_, count)| count[side] > 0)
            .max_by(|(a, ac), (b, bc)| ac[side].cmp(&bc[side]).then_with(|| b.cmp(a)))
            .map(|(rgb, _)| *rgb)
    };
    let fg = mode(1);
    let bg = mode(0);
    // A full/empty glyph has only one visible colour. The invisible colour
    // takes the same observed value, rather than an unrelated palette fallback.
    (fg.or(bg).unwrap_or([0; 3]), bg.or(fg).unwrap_or([0; 3]))
}

/// Refinement can change the masks without changing the stored colours. Refit
/// after it, so a colour taken from an old mask cannot leak into the new one.
pub(super) fn refit(
    image: &AnsiImage,
    args: &RenderArgs,
    render: Render,
    data: &mut [BlockData],
    states: &[(usize, bool)],
    glyphs: &[FastGlyph],
    grid_width: usize,
) -> bool {
    // Foreground-only refinement operates on opacity, not display colours.
    // Its source ink colours are fitted once the final glyph masks are known.
    if args.blend_colours || args.foreground_only {
        return false;
    }
    let (gw, gh) = image.glyphs.metrics;
    let source_width = image.bitmap.first().map_or(0, Vec::len);
    let source_height = image.bitmap.len();
    data.par_iter_mut()
        .enumerate()
        .map(|(index, data)| {
            let block = &image.block[index / grid_width][index % grid_width];
            let (glyph, inverted) = states[index];
            let (fg, bg) = observed_indexed(
                block
                    .pixels
                    .iter()
                    .flatten()
                    .enumerate()
                    // Padding outside a partial edge cell is not source data.
                    .filter(|(pixel, _)| {
                        index % grid_width * gw + pixel % gw < source_width
                            && index / grid_width * gh + pixel / gw < source_height
                    })
                    .map(|(pixel, colour)| (pixel, unpack_rgb(colour.orig))),
                &glyphs[glyph],
                inverted,
            );
            let colour = |rgb| {
                if render == Render::Ansi24 {
                    return Colour::RGB(rgb);
                }
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
            let fg_col = colour(fg);
            let bg_col = colour(bg);
            let fg = colour_to_rgb(&fg_col, render);
            let bg = colour_to_rgb(&bg_col, render);
            if data.fg == fg && data.bg == bg {
                return false;
            }
            data.fg = fg;
            data.bg = bg;
            data.fg_col = Some(fg_col);
            data.bg_col = Some(bg_col);
            data.alt_fg_col = None;
            data.alt_bg_col = None;
            true
        })
        .reduce(|| false, |a, b| a | b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    #[test]
    fn ansi_preview_keeps_the_selected_palette_with_blending_on_or_off() {
        let store = GlyphStore {
            glyphs: vec![(
                '▀',
                vec![vec![1; 2], vec![1; 2], vec![0; 2], vec![0; 2]],
                "test".into(),
            )],
            groups: HashMap::new(),
            selected: ['▀'].into_iter().collect(),
            metrics: (2, 4),
            float_metrics: (2.0, 4.0),
        };
        // Both are exact ANSI-256 colours, with no resampling involved.
        let fg = [95, 135, 175];
        let bg = [215, 95, 135];
        let pixels = (0..8)
            .flat_map(|p| {
                let rgb = if p < 4 { fg } else { bg };
                [rgb[0], rgb[1], rgb[2], 255]
            })
            .collect();
        let canvas = AnsiImage::new(PhotonImage::new(pixels, 2, 4), store.clone(), 32, false);
        for render in [Render::Ansi, Render::Ansi24, Render::Irc] {
            for blend in [true, false] {
                for variant in 0..3 {
                    let mut args = crate::generator::default_options();
                    args.render = render;
                    args.blend_colours = blend;
                    args.region_guidance = variant == 1;
                    if variant == 2 {
                        args.overlays.push(crate::args::TextOverlay {
                            text: "X".into(),
                            source_text: None,
                            figlet_font: None,
                            x: 0,
                            y: 0,
                            w: 1,
                            h: 1,
                            fg: Some(crate::args::ColorSpec::Index(67)),
                            bg: Some(crate::args::ColorSpec::Index(16)),
                            wrap: false,
                            auto_grow: false,
                            transparent_spaces: false,
                            bold: false,
                            italic: false,
                            underline: false,
                        });
                    }
                    let glyphs = prepare_glyphs(&store, &args);
                    args.as_preview = false;
                    let normal = render_blocks(&canvas, &args, render, &glyphs, None);
                    args.as_preview = true;
                    let preview = render_blocks(&canvas, &args, render, &glyphs, None);
                    let cell = &preview.grid[0][0];
                    let expected = &normal.grid[0][0];
                    assert_eq!((cell.char, cell.fg, cell.bg), (expected.char, expected.fg, expected.bg),
                    "render={render:?}, blend={blend}, variant={variant}: preview encoding must not change the palette");
                    assert_eq!(preview.score.to_bits(), normal.score.to_bits());
                    if render == Render::Ansi {
                        let background = if variant == 2 { [0; 3] } else { bg };
                        assert_eq!((cell.fg, cell.bg), (fg, Some(background)));
                    }
                }
            }
        }
    }

    #[test]
    fn repair_finished_score_matches_the_final_source_colours() {
        let chars = ['▀', '▌', '╲', '╱', ' ', '█'];
        let store = GlyphStore {
            glyphs: chars
                .iter()
                .enumerate()
                .map(|(g, &ch)| {
                    let bitmap = (0..4)
                        .map(|y| {
                            (0..4)
                                .map(|x| {
                                    u8::from(match g {
                                        0 => y < 2,
                                        1 => x < 2,
                                        2 => x <= y,
                                        3 => x + y < 4,
                                        4 => false,
                                        _ => true,
                                    })
                                })
                                .collect()
                        })
                        .collect();
                    (ch, bitmap, "test".into())
                })
                .collect(),
            groups: HashMap::new(),
            selected: chars.into_iter().collect(),
            metrics: (4, 4),
            float_metrics: (4.0, 4.0),
        };
        let palette = [[235, 220, 180], [30, 45, 70], [220, 80, 40], [50, 160, 90]];
        for (render, blend) in [
            (Render::Ansi24, false),
            (Render::Ansi24, true),
            (Render::Ansi, false),
            (Render::Irc, false),
        ] {
            for pattern in 0..3 {
                let mut args = crate::generator::default_options();
                args.blend_colours = blend;
                args.render = render;
                // Exercise the editor's preview encoding as well as CLI output.
                args.as_preview = pattern != 0;
                args.glyph_search = true;
                args.glyph_search_candidates = 4;
                args.glyph_search_passes = 1;
                args.discontinuity_threshold = 0.0;
                args.perceptual_score = pattern != 1;
                args.show_glyph_defects = pattern == 1;
                let glyphs = prepare_glyphs(&store, &args);
                let (width, height) = if pattern == 2 { (13, 11) } else { (12, 12) };
                let pixels = (0..height)
                    .flat_map(|y| {
                        (0..width).flat_map(move |x| {
                            let rgb = palette[((x + pattern * y) / 3 + y / 5) % palette.len()];
                            [rgb[0], rgb[1], rgb[2], 255]
                        })
                    })
                    .collect();
                let image = AnsiImage::new(
                    PhotonImage::new(pixels, width as u32, height as u32),
                    store.clone(),
                    5,
                    false,
                );
                let progress = Rc::new(RefCell::new(Vec::new()));
                let sink = progress.clone();
                let mut uncached = GlyphSearchCache::new(0);
                let result = uncached.run(|| search_status::observe_glyph_search(
                    move |p| sink.borrow_mut().push(p.clone()),
                    || render_blocks(&image, &args, args.render, &glyphs, None),
                ));
                let mut cached = GlyphSearchCache::new(8 * 1024 * 1024);
                cached.run(|| {
                    for round in 0..2 {
                        let hits = glyph_search_cache_stats().0;
                        let reused = render_blocks(&image, &args, args.render, &glyphs, None);
                        assert_eq!(reused.content, result.content,
                            "cache round={round}, render={render:?}, blend={blend}, pattern={pattern}");
                        assert_eq!(reused.score.to_bits(), result.score.to_bits());
                        if round == 1 {
                            assert!(glyph_search_cache_stats().0 > hits, "warm search must reuse rankings");
                        }
                    }
                    if pattern == 0 {
                        let mut changed = args.clone();
                        changed.glyph_search_passes += 1;
                        let expected = uncached.run(|| render_blocks(&image, &changed, changed.render, &glyphs, None));
                        let hits = glyph_search_cache_stats().0;
                        let reused = render_blocks(&image, &changed, changed.render, &glyphs, None);
                        assert_eq!(reused.content, expected.content);
                        assert_eq!(reused.score.to_bits(), expected.score.to_bits());
                        assert!(glyph_search_cache_stats().0 > hits,
                            "changing the pass budget must still reuse unchanged contexts");
                    }
                });
                let updates = progress.borrow();
                let finished = updates.last().unwrap();
                assert_eq!(finished.stage, search_status::GlyphSearchStage::Finished);
                assert_eq!(
                    finished.score.to_bits(),
                    result.score.to_bits(),
                    "blend={blend}, pattern={pattern}: finished {} != final {}",
                    finished.score,
                    result.score
                );
                assert_eq!(
                    finished.score.to_bits(),
                    result.perceptual_score.unwrap().total.to_bits()
                );
                let initial = updates
                    .iter()
                    .find(|p| p.stage == search_status::GlyphSearchStage::Candidates)
                    .unwrap()
                    .score;
                assert!(
                    result.score <= initial,
                    "final source colours must pass the improvement check"
                );
                let preview = &finished.preview.as_ref().unwrap().result;
                assert_eq!(preview.score.to_bits(), result.score.to_bits());
                let colours = |r: &RenderResult| {
                    r.grid
                        .iter()
                        .flatten()
                        // The encoder can replace an invisible glyph with a
                        // cheaper blank when foreground equals background.
                        .map(|c| (if c.bg == Some(c.fg) { ' ' } else { c.char }, c.fg, c.bg))
                        .collect::<Vec<_>>()
                };
                assert_eq!(colours(preview), colours(&result));
                if !blend && render == Render::Ansi24 {
                    let states: Vec<_> = result
                        .grid
                        .iter()
                        .flatten()
                        .map(|c| (glyphs.iter().position(|g| g.ch == c.char).unwrap(), false))
                        .collect();
                    let mut data: Vec<_> = result
                        .grid
                        .iter()
                        .flatten()
                        .map(|c| BlockData {
                            fg: c.fg,
                            bg: c.bg.unwrap(),
                            fg_col: None,
                            bg_col: None,
                            alt_fg_col: None,
                            alt_bg_col: None,
                        })
                        .collect();
                    assert!(
                        !refit(
                            &image,
                            &args,
                            render,
                            &mut data,
                            &states,
                            &glyphs,
                            result.grid[0].len()
                        ),
                        "output colours must already match the final source masks"
                    );
                }
            }
        }
    }
}
