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
    if args.blend_colours {
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
