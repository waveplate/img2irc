//! Conservative magic-wand filling for compressed flat-shaded artwork.
//! Fixed-seed growth prevents chaining similar colours across a gradient.
use crate::draw::{oklab_distance_squared as distance, rgb_to_oklab};
use photon_rs::PhotonImage;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

mod temporal;
pub(crate) use temporal::{with_history, History};
type Rgb = [u8; 3];

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, clap::Args)]
#[serde(default)]
pub struct Options {
    /// Maximum OKLab distance from the region seed, in percent; higher joins more shades.
    #[arg(long = "flat-fill-tolerance", default_value_t = 2.5, value_parser = crate::args::parse_percentage)]
    pub tolerance: f32,
    /// Maximum within-region colour standard deviation, in percent; higher accepts more texture.
    #[arg(long = "flat-fill-flatness", default_value_t = 1.2, value_parser = crate::args::parse_percentage)]
    pub flatness: f32,
    /// Smallest region to fill, in source raster pixels (not terminal cells).
    #[arg(long = "flat-fill-min-area", default_value_t = 16, value_parser = clap::value_parser!(u32).range(1..=100_000_000))]
    pub min_area: u32,
    /// Gradient-protection threshold in percent; 0 disables gradient protection.
    #[arg(long = "flat-fill-gradient", default_value_t = 0.6, value_parser = crate::args::parse_percentage)]
    pub gradient: f32,
    /// Blend toward the chosen region colour, 0–100 percent.
    #[arg(long = "flat-fill-strength", default_value_t = 100.0, value_parser = crate::args::parse_percentage)]
    pub strength: f32,
    /// Hold a matching video's region colour within this OKLab tolerance; 0 disables history.
    #[arg(long = "flat-fill-temporal-tolerance", default_value_t = 1.0, value_parser = crate::args::parse_percentage)]
    pub temporal_tolerance: f32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            tolerance: 2.5,
            flatness: 1.2,
            min_area: 16,
            gradient: 0.6,
            strength: 100.0,
            temporal_tolerance: 1.0,
        }
    }
}

pub fn apply(image: &mut PhotonImage) {
    apply_with_options(image, &Options::default());
}

pub fn apply_with_options(image: &mut PhotonImage, options: &Options) {
    temporal::with_layer(|previous, record| apply_region_fill(image, options, previous, record));
}

fn apply_region_fill(
    image: &mut PhotonImage,
    options: &Options,
    previous: Option<&temporal::Layer>,
    record: bool,
) -> Option<temporal::Layer> {
    let (w, h) = (image.get_width() as usize, image.get_height() as usize);
    if w == 0 || h == 0 || options.strength <= 0.0 {
        return None;
    }
    let radius_squared = (options.tolerance / 100.0).powi(2);
    let flatness_squared = f64::from(options.flatness / 100.0).powi(2);
    let gradient_squared = (options.gradient / 100.0).powi(2);
    let mut rgba = image.get_raw_pixels();
    let labs: Vec<_> = rgba
        .chunks_exact(4)
        .map(|p| rgb_to_oklab([p[0], p[1], p[2]]))
        .collect();
    let mut labels = vec![0; labs.len()];
    let recording = record && options.temporal_tolerance > 0.0 && temporal::can_record(labs.len());
    let previous = previous.filter(|layer| {
        recording && layer.width == w && layer.height == h && layer.options == *options
    });
    let mut colours = HashMap::new();
    let mut region = Vec::new();
    for seed in 0..labs.len() {
        if labels[seed] != 0 {
            continue;
        }
        let label = seed + 1;
        labels[seed] = label;
        if rgba[seed * 4 + 3] != 255 {
            continue;
        }
        region.clear();
        region.push(seed);
        let mut cursor = 0;
        let mut ramp = false;
        while cursor < region.len() {
            let p = region[cursor];
            cursor += 1;
            let (x, y) = (p % w, p / w);
            for q in [
                x.checked_sub(1).map(|_| p - 1),
                (x + 1 < w).then_some(p + 1),
                y.checked_sub(1).map(|_| p - w),
                (y + 1 < h).then_some(p + w),
            ]
            .into_iter()
            .flatten()
            {
                if rgba[q * 4 + 3] != 255 {
                    continue;
                }
                // A fragment at the end of a ramp can lie wholly within the
                // seed radius. Its smooth boundary with a previous component
                // must also prevent filling, including at image edges.
                ramp |= labels[q] != 0
                    && labels[q] != label
                    && distance(labs[q], labs[p]) < gradient_squared;
                if !crate::colour_quantization::chroma_compatible(labs[q], labs[seed])
                    || distance(labs[q], labs[seed]) > radius_squared
                {
                    // A smooth continuation beyond the seed range is a ramp.
                    ramp |= distance(labs[q], labs[p]) < gradient_squared;
                } else if labels[q] == 0 {
                    labels[q] = label;
                    region.push(q);
                }
            }
        }
        if region.len() < options.min_area as usize {
            continue;
        }
        let mut sum = [0.0_f64; 3];
        for &p in &region {
            for (s, v) in sum.iter_mut().zip([labs[p].l, labs[p].a, labs[p].b]) {
                *s += f64::from(v);
            }
        }
        let mean = sum.map(|s| s / region.len() as f64);
        let error = |p: usize| {
            [labs[p].l, labs[p].a, labs[p].b]
                .into_iter()
                .zip(mean)
                .map(|(v, m)| (f64::from(v) - m).powi(2))
                .sum::<f64>()
        };
        if region.iter().map(|&p| error(p)).sum::<f64>() / region.len() as f64 > flatness_squared {
            continue;
        }
        if ramp && options.gradient > 0.0 {
            // A few antialiased boundary pixels must not reject a whole flat
            // interior. A ramp has a consistent colour trend across its area.
            let bounds = region.iter().fold([w, h, 0, 0], |b, &p| {
                [
                    b[0].min(p % w),
                    b[1].min(p / w),
                    b[2].max(p % w),
                    b[3].max(p / w),
                ]
            });
            let mut sums = [[0.0_f64; 3]; 4];
            let mut counts = [0; 4];
            for &p in &region {
                for half in [
                    usize::from((p % w) * 2 > bounds[0] + bounds[2]),
                    2 + usize::from((p / w) * 2 > bounds[1] + bounds[3]),
                ] {
                    counts[half] += 1;
                    for (s, c) in sums[half].iter_mut().zip([labs[p].l, labs[p].a, labs[p].b]) {
                        *s += f64::from(c);
                    }
                }
            }
            let trend = |a: usize, b: usize| {
                counts[a] > 0
                    && counts[b] > 0
                    && (0..3)
                        .map(|c| {
                            (sums[a][c] / counts[a] as f64 - sums[b][c] / counts[b] as f64).powi(2)
                        })
                        .sum::<f64>()
                        > f64::from(options.gradient / 100.0).powi(2)
            };
            if trend(0, 1) || trend(2, 3) {
                continue;
            }
        }
        // The RGB-bin winner jumped at bin boundaries. Choose an observed
        // colour nearest the region's perceptual mean instead.
        let representative = region
            .iter()
            .copied()
            .min_by(|&a, &b| error(a).total_cmp(&error(b)))
            .unwrap();
        let fresh: Rgb = rgba[representative * 4..representative * 4 + 3]
            .try_into()
            .unwrap();
        let colour = previous
            .and_then(|layer| layer.stable_colour(&region, &labs, mean))
            .unwrap_or(fresh);
        if recording {
            colours.insert(label, colour);
        }
        let chosen_lab = rgb_to_oklab(colour);
        for &p in &region {
            if !crate::colour_quantization::chroma_compatible(labs[p], chosen_lab) {
                continue;
            }
            if options.strength >= 100.0 {
                rgba[p * 4..p * 4 + 3].copy_from_slice(&colour);
            } else {
                for c in 0..3 {
                    let old = f32::from(rgba[p * 4 + c]);
                    rgba[p * 4 + c] = (old
                        + (f32::from(colour[c]) - old) * options.strength / 100.0)
                        .round()
                        .clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
    *image = PhotonImage::new(rgba, w as u32, h as u32);
    recording.then_some(temporal::Layer {
        options: *options,
        width: w,
        height: h,
        source: labs,
        labels,
        colours,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fills_noise_without_crossing_outlines_or_alpha() {
        let pixels: Vec<u8> = (0..16 * 16)
            .flat_map(|p| {
                let x = p % 16;
                let c = if x == 8 {
                    [0, 0, 0]
                } else if x < 8 {
                    [180 + (p % 3) as u8, 40, 40]
                } else {
                    [40, 50, 180 + (p % 3) as u8]
                };
                [c[0], c[1], c[2], if p == 0 { 17 } else { 255 }]
            })
            .collect();
        let mut image = PhotonImage::new(pixels.clone(), 16, 16);
        apply(&mut image);
        let result = image.get_raw_pixels();
        assert_eq!(&result[..4], &pixels[..4]);
        assert_eq!(&result[4..7], &result[16 * 4..16 * 4 + 3]);
        assert_eq!(&result[8 * 4..8 * 4 + 3], &[0, 0, 0]);
        assert_ne!(&result[4..7], &result[15 * 4..15 * 4 + 3]);
    }
    #[test]
    fn preserves_gradients() {
        let pixels: Vec<u8> = (0..128 * 8)
            .flat_map(|p| {
                let c = 50 + (p % 128) as u8;
                [c, c, c, 255]
            })
            .collect();
        let mut image = PhotonImage::new(pixels.clone(), 128, 8);
        apply(&mut image);
        assert_eq!(image.get_raw_pixels(), pixels);
    }

    #[test]
    fn controls_change_growth_flatness_area_gradient_protection_and_blend() {
        let pixels: Vec<u8> = (0..64)
            .flat_map(|p| {
                let c = if p % 8 < 4 { 100 } else { 110 };
                [c, c, c, 255]
            })
            .collect();
        let render = |options: Options| {
            let mut image = PhotonImage::new(pixels.clone(), 8, 8);
            apply_with_options(&mut image, &options);
            image.get_raw_pixels()
        };
        let mut options = Options {
            tolerance: 10.0,
            flatness: 10.0,
            gradient: 0.0,
            ..Options::default()
        };
        let filled = render(options);
        assert_ne!(filled, pixels);
        assert!(filled.chunks_exact(4).all(|p| p == &filled[..4]));
        options.tolerance = 0.0;
        assert_eq!(render(options), pixels);
        options.tolerance = 10.0;
        options.flatness = 0.0;
        assert_eq!(render(options), pixels);
        options.flatness = 10.0;
        options.min_area = 65;
        assert_eq!(render(options), pixels);
        options.min_area = 16;
        options.strength = 0.0;
        assert_eq!(render(options), pixels);
        options.strength = 50.0;
        let blended = render(options);
        assert_ne!(blended, pixels);
        assert_ne!(blended, filled);
        let ramp: Vec<u8> = (0..128 * 8)
            .flat_map(|p| {
                let c = 50 + (p % 128) as u8;
                [c, c, c, 255]
            })
            .collect();
        let mut image = PhotonImage::new(ramp.clone(), 128, 8);
        apply_with_options(
            &mut image,
            &Options {
                gradient: 0.0,
                ..Options::default()
            },
        );
        assert_ne!(image.get_raw_pixels(), ramp);
    }
}
