//! Colour edits applied after glyph matching and text overlays.
use std::collections::{BTreeMap, HashMap};

use crate::{args::Render, draw, font::GlyphStore, pipeline::ImageEffect};

type Rgb = [u8; 3];

/// Keep the most common original colours, mapping rarer ones to their nearest
/// retained neighbour in OKLab. RGB ordering makes equal-frequency ties stable.
pub(crate) fn reduction_map(counts: &BTreeMap<Rgb, u64>, limit: usize) -> BTreeMap<Rgb, Rgb> {
    if limit == 0 || counts.len() <= limit {
        return BTreeMap::new();
    }
    let mut ranked: Vec<_> = counts.iter().collect();
    ranked.sort_by(|(a, na), (b, nb)| nb.cmp(na).then_with(|| a.cmp(b)));
    let retained: Vec<_> = ranked[..limit]
        .iter()
        .map(|(rgb, _)| (**rgb, draw::rgb_to_oklab(**rgb)))
        .collect();
    ranked[limit..]
        .iter()
        .map(|(rgb, _)| {
            let lab = draw::rgb_to_oklab(**rgb);
            let nearest = retained
                .iter()
                .min_by(|a, b| {
                    draw::oklab_distance_squared(lab, a.1)
                        .total_cmp(&draw::oklab_distance_squared(lab, b.1))
                })
                .unwrap()
                .0;
            (**rgb, nearest)
        })
        .collect()
}

/// Pool similar shades before ranking them, so compression noise cannot occupy
/// multiple palette slots. Seeds are original colours in frequency order; they
/// never move, preventing a chain of nearby shades from swallowing an accent.
pub(crate) fn palette_map(
    counts: &BTreeMap<Rgb, u64>,
    limit: usize,
    threshold: f32,
    palette: &[Rgb],
) -> BTreeMap<Rgb, Rgb> {
    if !palette.is_empty() {
        let retained: Vec<_> = palette
            .iter()
            .map(|&rgb| (rgb, draw::rgb_to_oklab(rgb)))
            .collect();
        return counts
            .keys()
            .map(|&rgb| (rgb, nearest(rgb, &retained)))
            .collect();
    }
    let radius = threshold.clamp(0.0, 100.0) / 100.0;
    if radius < 1e-6 || !radius.is_finite() {
        let reduced = reduction_map(counts, limit);
        return counts
            .keys()
            .map(|&rgb| (rgb, reduced.get(&rgb).copied().unwrap_or(rgb)))
            .collect();
    }
    let mut ranked: Vec<_> = counts.iter().collect();
    ranked.sort_by(|(a, na), (b, nb)| nb.cmp(na).then_with(|| a.cmp(b)));
    let mut buckets: HashMap<(i32, i32, i32), Vec<usize>> = HashMap::new();
    let mut seeds = Vec::new();
    let mut pooled = BTreeMap::new();
    let mut assignments = BTreeMap::new();
    for (&rgb, &count) in ranked {
        let lab = draw::rgb_to_oklab(rgb);
        let bucket = (
            (lab.l / radius).floor() as i32,
            (lab.a / radius).floor() as i32,
            (lab.b / radius).floor() as i32,
        );
        let mut best = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(indices) =
                        buckets.get(&(bucket.0 + dx, bucket.1 + dy, bucket.2 + dz))
                    {
                        for &index in indices {
                            let (seed, seed_lab) = seeds[index];
                            let distance = draw::oklab_distance_squared(lab, seed_lab);
                            if distance <= radius * radius
                                && best.is_none_or(|(d, r)| {
                                    distance < d || (distance == d && seed < r)
                                })
                            {
                                best = Some((distance, seed));
                            }
                        }
                    }
                }
            }
        }
        let seed = best.map_or_else(
            || {
                let index = seeds.len();
                seeds.push((rgb, lab));
                buckets.entry(bucket).or_default().push(index);
                rgb
            },
            |(_, seed)| seed,
        );
        *pooled.entry(seed).or_default() += count;
        assignments.insert(rgb, seed);
    }
    let reduced = reduction_map(&pooled, limit);
    for seed in assignments.values_mut() {
        *seed = reduced.get(seed).copied().unwrap_or(*seed);
    }
    assignments
}

fn nearest(rgb: Rgb, retained: &[(Rgb, draw::Oklab)]) -> Rgb {
    let lab = draw::rgb_to_oklab(rgb);
    retained
        .iter()
        .min_by(|a, b| {
            draw::oklab_distance_squared(lab, a.1)
                .total_cmp(&draw::oklab_distance_squared(lab, b.1))
        })
        .unwrap()
        .0
}

pub fn apply(
    base: &draw::RenderResult,
    glyphs: &GlyphStore,
    pipeline: &[ImageEffect],
    render: Render,
    as_preview: bool,
    grayscale_tolerance: u8,
) -> draw::RenderResult {
    if base.grid.is_empty() || !pipeline.iter().any(ImageEffect::is_output_colour) {
        return base.clone();
    }
    let mut result = base.clone();
    let palette: &[u32] = match render {
        Render::Irc => &crate::palette::IRC99,
        Render::Ansi => &crate::palette::ANSI256,
        Render::Ansi24 => &[],
    };
    // Includes RGB text overlays: the histogram must count colours the output
    // format can actually show, and preview/export must use the same palette.
    let mut palette_cache = HashMap::new();
    let mut visible = |rgb| {
        *palette_cache
            .entry(rgb)
            .or_insert_with(|| crate::adjustments::quantize_rgb(rgb, palette))
    };
    for cell in result.grid.iter_mut().flatten() {
        cell.fg = visible(cell.fg);
        cell.bg = cell.bg.map(&mut visible);
    }
    let area = (glyphs.metrics.0 * glyphs.metrics.1) as u64;
    let mut ink_cache = HashMap::new();
    for effect in pipeline {
        match effect {
            ImageEffect::ReplaceColour {
                from,
                to,
                tolerance,
            } => {
                let from = visible(*from);
                let to = visible(*to);
                let limit = tolerance.clamp(0.0, 100.0) * 255.0 / 100.0;
                let replace = |rgb: Rgb| {
                    let distance = rgb
                        .iter()
                        .zip(from)
                        .map(|(&a, b)| (f32::from(a) - f32::from(b)).powi(2))
                        .sum::<f32>()
                        / 3.0;
                    if distance <= limit * limit {
                        to
                    } else {
                        rgb
                    }
                };
                for cell in result.grid.iter_mut().flatten() {
                    cell.fg = replace(cell.fg);
                    let bg = replace(cell.bg.unwrap_or([0; 3]));
                    if cell.bg.is_some() || bg != [0; 3] {
                        cell.bg = Some(bg);
                    }
                }
            }
            ImageEffect::QuantizeColours(_)
            | ImageEffect::QuantizePalette { source: false, .. } => {
                let (limit, threshold, chosen) = match effect {
                    ImageEffect::QuantizeColours(limit) => (*limit, 0.0, vec![]),
                    ImageEffect::QuantizePalette {
                        colours,
                        threshold,
                        palette,
                        ..
                    } => (
                        *colours,
                        *threshold,
                        palette.iter().map(|&rgb| visible(rgb)).collect(),
                    ),
                    _ => unreachable!(),
                };
                if limit == 0 && threshold == 0.0 && chosen.is_empty() {
                    continue;
                }
                let mut counts = BTreeMap::new();
                for cell in result.grid.iter().flatten() {
                    let ink = *ink_cache
                        .entry(cell.char)
                        .or_insert_with(|| draw::glyph_ink_pixels(glyphs, cell.char));
                    if ink > 0 {
                        *counts.entry(cell.fg).or_default() += ink;
                    }
                    if ink < area {
                        *counts.entry(cell.bg.unwrap_or([0; 3])).or_default() += area - ink;
                    }
                }
                let map = palette_map(&counts, limit as usize, threshold, &chosen);
                // Hidden foregrounds/backgrounds must not introduce extra
                // colours in the encoded output either.
                let Some(&fallback) = map.values().next() else {
                    continue;
                };
                let remap = |rgb: Rgb| map.get(&rgb).copied().unwrap_or_else(|| fallback);
                for cell in result.grid.iter_mut().flatten() {
                    cell.fg = remap(cell.fg);
                    cell.bg = Some(remap(cell.bg.unwrap_or([0; 3])));
                }
            }
            _ => {}
        }
    }
    draw::encode_output_colours(&result, render, as_preview, grayscale_tolerance)
}

#[cfg(test)]
mod tests {
    use super::*;
    use draw::{RenderResult, RenderedCell};

    #[test]
    fn compression_shades_share_a_slot_without_losing_a_rare_outline() {
        let red = [200, 40, 40];
        let noisy = [202, 41, 40];
        let black = [0; 3];
        let counts = [(red, 100), (noisy, 90), (black, 5)].into_iter().collect();
        assert_eq!(reduction_map(&counts, 2).get(&black), Some(&red));
        let map = palette_map(&counts, 2, 3.0, &[]);
        assert_eq!(map[&noisy], red);
        assert_eq!(map[&black], black);
        assert_eq!(
            map.values()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            2
        );
        assert_eq!(palette_map(&counts, 0, 3.0, &[]), map);
        assert_eq!(palette_map(&counts, 0, 0.0, &[])[&noisy], noisy);
    }

    #[test]
    fn explicit_palette_overrides_count_and_threshold_and_snaps_to_output_format() {
        let mut base = RenderResult::from("");
        base.grid = vec![vec![cell('▀', [240, 8, 8], [8, 8, 230])]];
        let chosen = vec![[255, 0, 0], [0, 0, 255]];
        let effect = ImageEffect::QuantizePalette {
            colours: 1,
            threshold: 100.0,
            palette: chosen.clone(),
            source: false,
        };
        for render in [Render::Ansi24, Render::Irc, Render::Ansi] {
            let export = apply(&base, &glyphs(), &[effect.clone()], render, false, 5);
            let preview = apply(&base, &glyphs(), &[effect.clone()], render, true, 5);
            assert_eq!(export.grid[0][0].fg, preview.grid[0][0].fg);
            assert_eq!(export.grid[0][0].bg, preview.grid[0][0].bg);
            assert_eq!(export.grid[0][0].char, '▀');
            assert!(chosen.contains(&export.grid[0][0].fg));
            assert!(chosen.contains(&export.grid[0][0].bg.unwrap()));
            assert_ne!(export.grid[0][0].fg, export.grid[0][0].bg.unwrap());
        }
    }

    #[test]
    fn nearby_shades_do_not_chain_across_a_gradient() {
        let shades = [[100; 3], [108; 3], [116; 3]];
        let counts = shades.into_iter().zip([100, 99, 98]).collect();
        let radius =
            draw::oklab_distance(draw::rgb_to_oklab(shades[0]), draw::rgb_to_oklab(shades[1]))
                * 101.0;
        let map = palette_map(&counts, 0, radius, &[]);
        assert_eq!(map[&shades[1]], shades[0]);
        assert_eq!(map[&shades[2]], shades[2]);
    }

    fn glyphs() -> GlyphStore {
        GlyphStore {
            glyphs: vec![],
            groups: Default::default(),
            selected: Default::default(),
            metrics: (2, 2),
            float_metrics: (2.0, 2.0),
        }
    }

    fn cell(ch: char, fg: Rgb, bg: Rgb) -> RenderedCell {
        RenderedCell {
            char: ch,
            fg,
            bg: Some(bg),
            inverted: false,
            bold: false,
            italic: false,
            underline: false,
        }
    }

    fn visible_colours(result: &RenderResult) -> std::collections::BTreeSet<Rgb> {
        let (_, _, rgba) = draw::render_result_rgba(result, &glyphs()).unwrap();
        rgba.chunks_exact(4).map(|p| [p[0], p[1], p[2]]).collect()
    }

    #[test]
    fn quantization_counts_visible_area_and_preserves_glyphs() {
        let red = [255, 0, 0];
        let white = [255; 3];
        let hidden = [0, 0, 255];
        let mut base = RenderResult::from("");
        base.grid = vec![vec![
            cell(' ', hidden, red),
            cell(' ', hidden, red),
            cell(' ', hidden, red),
            cell(' ', hidden, red),
            cell('█', white, hidden),
            cell('█', white, hidden),
            cell('▀', [250, 5, 5], [0; 3]),
        ]];
        let result = apply(
            &base,
            &glyphs(),
            &[ImageEffect::QuantizeColours(2)],
            Render::Ansi24,
            false,
            5,
        );
        assert_eq!(visible_colours(&result), [red, white].into_iter().collect());
        assert_eq!(result.grid[0][6].fg, red);
        for (before, after) in base.grid[0].iter().zip(&result.grid[0]) {
            assert_eq!(before.char, after.char);
            assert!(after.fg == red || after.fg == white);
            assert!(after.bg == Some(red) || after.bg == Some(white));
        }
        assert!(!result.content.contains("0;0;255"));
        let single = apply(
            &base,
            &glyphs(),
            &[ImageEffect::QuantizeColours(1)],
            Render::Ansi24,
            false,
            5,
        );
        assert_eq!(visible_colours(&single), [red].into_iter().collect());
    }

    #[test]
    fn unseen_colour_slots_cannot_exceed_limit_even_without_visible_reduction() {
        let mut base = RenderResult::from("");
        base.grid = vec![vec![cell(' ', [255; 3], [12, 34, 56])]];
        let result = apply(
            &base,
            &glyphs(),
            &[ImageEffect::QuantizeColours(1)],
            Render::Ansi24,
            false,
            5,
        );
        assert_eq!(result.grid[0][0].fg, [12, 34, 56]);
        assert_eq!(visible_colours(&result).len(), 1);
    }

    #[test]
    fn replacement_uses_rendered_colours_and_updates_encoded_output() {
        let mut base = RenderResult::from("original");
        base.grid = vec![vec![cell('▀', [123, 45, 67], [9, 8, 7])]];
        let effect = ImageEffect::ReplaceColour {
            from: [123, 45, 67],
            to: [1, 2, 3],
            tolerance: 0.0,
        };
        let result = apply(&base, &glyphs(), &[effect], Render::Ansi24, false, 5);
        assert_eq!(result.grid[0][0].fg, [1, 2, 3]);
        assert_eq!(result.grid[0][0].bg, Some([9, 8, 7]));
        assert!(result.content.contains("38;2;1;2;3"));
        assert_eq!(base.grid[0][0].fg, [123, 45, 67]);
    }

    #[test]
    fn indexed_previews_and_exports_use_the_same_retained_colours() {
        for render in [Render::Irc, Render::Ansi] {
            let mut base = RenderResult::from("");
            base.grid = vec![vec![
                cell('▀', [250, 10, 10], [0; 3]),
                cell('▀', [250; 3], [0; 3]),
            ]];
            let effect = ImageEffect::QuantizeColours(2);
            let preview = apply(&base, &glyphs(), &[effect.clone()], render, true, 5);
            let export = apply(&base, &glyphs(), &[effect], render, false, 5);
            assert_eq!(visible_colours(&preview), visible_colours(&export));
            assert_eq!(visible_colours(&export).len(), 2);
            if render == Render::Irc {
                assert_eq!(preview.save_content.as_ref(), Some(&export.content));
            } else {
                assert!(export.content.contains(";5;"));
            }
        }
    }

    #[test]
    fn pixel_processing_keeps_alpha_and_ties_are_deterministic() {
        let mut img =
            photon_rs::PhotonImage::new(vec![255, 0, 0, 255, 0, 0, 255, 255, 0, 255, 0, 0], 3, 1);
        crate::adjustments::quantize_colours(&mut img, 1);
        assert_eq!(
            img.get_raw_pixels(),
            vec![0, 0, 255, 255, 0, 0, 255, 255, 0, 255, 0, 0,]
        );
    }
}
