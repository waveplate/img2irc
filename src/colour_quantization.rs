//! Perceptual palette fitting and bounded, edge-aware spatial assignment.
use crate::draw::{oklab_distance_squared as distance, rgb_to_oklab, Oklab};
use std::collections::{BTreeMap, BTreeSet, HashMap};
type Rgb = [u8; 3];

fn chroma_class(lab: Oklab) -> i8 {
    let chroma = lab.a * lab.a + lab.b * lab.b;
    if chroma <= 0.012f32.powi(2) {
        -1
    } else if chroma >= 0.025f32.powi(2) {
        1
    } else {
        0
    }
}

pub(crate) fn chroma_compatible(source: Oklab, target: Oklab) -> bool {
    chroma_class(source) * chroma_class(target) != -1
}

pub(crate) fn snap_to_palette(rgb: Rgb, palette: &[u32]) -> Rgb {
    if palette.is_empty() {
        return rgb;
    }
    static ANSI: std::sync::OnceLock<Vec<Oklab>> = std::sync::OnceLock::new();
    static IRC: std::sync::OnceLock<Vec<Oklab>> = std::sync::OnceLock::new();
    let unpack = |packed: u32| [(packed >> 16) as u8, (packed >> 8) as u8, packed as u8];
    let prepare = || {
        palette
            .iter()
            .map(|&packed| rgb_to_oklab(unpack(packed)))
            .collect::<Vec<_>>()
    };
    let is_irc = palette == crate::palette::IRC99;
    let custom;
    let labs = if palette == crate::palette::ANSI256 {
        ANSI.get_or_init(prepare)
    } else if is_irc {
        IRC.get_or_init(prepare)
    } else {
        custom = prepare();
        &custom
    };
    let source = rgb_to_oklab(rgb);
    let start = if is_irc { 16 } else { 0 };
    let compatible = labs[start..]
        .iter()
        .any(|&target| chroma_compatible(source, target));
    let index = (start..palette.len())
        .filter(|&index| !compatible || chroma_compatible(source, labs[index]))
        .min_by(|&a, &b| distance(source, labs[a]).total_cmp(&distance(source, labs[b])))
        .unwrap_or(0);
    unpack(palette[index])
}

/// Minimise weighted colour error rather than spending every slot on the most
/// frequent shades. Centres remain observed colours, including after refinement.
pub(crate) fn select_palette(counts: &BTreeMap<Rgb, u64>, limit: usize) -> Vec<Rgb> {
    let mut samples: Vec<_> = counts
        .iter()
        .map(|(&rgb, &weight)| (rgb, weight, rgb_to_oklab(rgb)))
        .collect();
    samples.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    if samples.len() <= limit {
        return samples.iter().map(|sample| sample.0).collect();
    }
    if limit == 0 || samples.is_empty() {
        return Vec::new();
    }
    let neutral = samples
        .iter()
        .position(|sample| chroma_class(sample.2) == -1);
    let coloured = samples
        .iter()
        .position(|sample| chroma_class(sample.2) == 1);
    let mut centres = match (neutral, coloured) {
        (Some(neutral), Some(coloured)) if limit >= 2 => vec![neutral, coloured],
        _ => vec![0],
    };
    let mut chosen = vec![false; samples.len()];
    for &index in &centres {
        chosen[index] = true;
    }
    let mut errors: Vec<_> = samples
        .iter()
        .map(|sample| {
            let compatible = centres
                .iter()
                .any(|&index| chroma_compatible(sample.2, samples[index].2));
            centres
                .iter()
                .filter(|&&index| !compatible || chroma_compatible(sample.2, samples[index].2))
                .map(|&index| distance(sample.2, samples[index].2))
                .fold(f32::INFINITY, f32::min)
        })
        .collect();
    while centres.len() < limit {
        let candidate = (0..samples.len())
            .filter(|&index| !chosen[index])
            .max_by(|&a, &b| {
                (f64::from(errors[a]) * samples[a].1 as f64)
                    .total_cmp(&(f64::from(errors[b]) * samples[b].1 as f64))
                    .then_with(|| samples[b].0.cmp(&samples[a].0))
            });
        let Some(candidate) = candidate else {
            break;
        };
        if errors[candidate] <= 0.0 {
            break;
        }
        centres.push(candidate);
        chosen[candidate] = true;
        for (error, sample) in errors.iter_mut().zip(&samples) {
            if chroma_compatible(sample.2, samples[candidate].2) {
                *error = error.min(distance(sample.2, samples[candidate].2));
            }
        }
    }
    for _ in 0..2 {
        let mut sums = vec![[0.0f64; 4]; centres.len()];
        let mut assignments = Vec::with_capacity(samples.len());
        for sample in &samples {
            let compatible = centres
                .iter()
                .any(|&index| chroma_compatible(sample.2, samples[index].2));
            let cluster = (0..centres.len())
                .filter(|&cluster| {
                    !compatible || chroma_compatible(sample.2, samples[centres[cluster]].2)
                })
                .min_by(|&a, &b| {
                    distance(sample.2, samples[centres[a]].2)
                        .total_cmp(&distance(sample.2, samples[centres[b]].2))
                        .then_with(|| samples[centres[a]].0.cmp(&samples[centres[b]].0))
                })
                .unwrap();
            let weight = sample.1 as f64;
            for (sum, value) in sums[cluster][..3]
                .iter_mut()
                .zip([sample.2.l, sample.2.a, sample.2.b])
            {
                *sum += weight * f64::from(value);
            }
            sums[cluster][3] += weight;
            assignments.push(cluster);
        }
        let mut best: Vec<Option<(usize, f64)>> = vec![None; centres.len()];
        for (index, (&cluster, sample)) in assignments.iter().zip(&samples).enumerate() {
            if chroma_class(sample.2) != chroma_class(samples[centres[cluster]].2) {
                continue;
            }
            if sums[cluster][3] == 0.0 {
                continue;
            }
            let error: f64 = [sample.2.l, sample.2.a, sample.2.b]
                .iter()
                .enumerate()
                .map(|(c, &value)| (f64::from(value) - sums[cluster][c] / sums[cluster][3]).powi(2))
                .sum();
            if best[cluster].is_none_or(|(old, previous)| {
                error < previous - 1e-12
                    || ((error - previous).abs() <= 1e-12 && sample.0 < samples[old].0)
            }) {
                best[cluster] = Some((index, error));
            }
        }
        let next: Vec<_> = best
            .into_iter()
            .enumerate()
            .map(|(cluster, best)| best.map_or(centres[cluster], |(index, _)| index))
            .collect();
        if next == centres {
            break;
        }
        centres = next;
    }
    centres.into_iter().map(|index| samples[index].0).collect()
}

/// Each site is a source pixel, or a terminal cell with foreground/background
/// samples weighted by visible ink. Equal RGB values can map differently in
/// separate regions. Spatial preference cannot add >2% OKLab error squared.
pub(crate) fn spatial_assign(
    colours: &[Rgb],
    weights: &[u64],
    width: usize,
    height: usize,
    channels: usize,
    map: &BTreeMap<Rgb, Rgb>,
) -> Vec<Rgb> {
    let remapped = || {
        colours
            .iter()
            .map(|rgb| {
                map.get(rgb)
                    .copied()
                    .or_else(|| map.values().next().copied())
                    .unwrap_or(*rgb)
            })
            .collect()
    };
    if colours.is_empty()
        || map.is_empty()
        || map.iter().all(|(a, b)| a == b)
        || channels == 0
        || channels > 2
        || weights.len() != colours.len()
        || width
            .checked_mul(height)
            .and_then(|sites| sites.checked_mul(channels))
            != Some(colours.len())
    {
        return remapped();
    }
    let palette: Vec<_> = map
        .values()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let ids: HashMap<_, _> = palette
        .iter()
        .enumerate()
        .map(|(index, &rgb)| (rgb, index))
        .collect();
    let palette_labs: Vec<_> = palette.iter().copied().map(rgb_to_oklab).collect();
    let mut fits = HashMap::<Rgb, (Oklab, usize, f32)>::new();
    let mut source = Vec::with_capacity(colours.len());
    let mut baseline = Vec::with_capacity(colours.len());
    let mut errors = Vec::with_capacity(colours.len());
    for &rgb in colours {
        let (lab, label, error) = *fits.entry(rgb).or_insert_with(|| {
            let lab = rgb_to_oklab(rgb);
            let label = map
                .get(&rgb)
                .and_then(|colour| ids.get(colour))
                .copied()
                .unwrap_or(0);
            (lab, label, distance(lab, palette_labs[label]))
        });
        source.push(lab);
        baseline.push(label);
        errors.push(error);
    }
    let mut labels = baseline.clone();
    let max_weight = weights.iter().copied().max().unwrap_or(1).max(1) as f32;
    const BIAS: f32 = 0.02 * 0.02;
    const EDGE_SCALE: f32 = 0.04 * 0.04;
    let degree = (4 * channels + channels - 1) as f32;
    for pass in 0..3 {
        let mut changed = false;
        for step in 0..colours.len() {
            let i = if pass % 2 == 0 {
                step
            } else {
                colours.len() - 1 - step
            };
            // Exact retained colours anchor regions; invisible samples do not vote.
            if weights[i] == 0 || errors[i] <= 1e-12 {
                labels[i] = baseline[i];
                continue;
            }
            let site = i / channels;
            let (x, y) = (site % width, site / width);
            let mut votes = [(usize::MAX, 0.0f32); 10];
            let mut used = 0;
            let mut add = |q: usize| {
                if weights[q] == 0 {
                    return;
                }
                let weight = weights[q].min(weights[i]) as f32 / max_weight * EDGE_SCALE
                    / (EDGE_SCALE + distance(source[i], source[q]));
                let label = labels[q];
                if let Some((_, vote)) = votes[..used].iter_mut().find(|(other, _)| *other == label)
                {
                    *vote += weight;
                } else {
                    votes[used] = (label, weight);
                    used += 1;
                }
            };
            if channels == 2 {
                add(site * channels + (1 - i % channels));
            }
            for neighbour in [
                x.checked_sub(1).map(|_| site - 1),
                (x + 1 < width).then_some(site + 1),
                y.checked_sub(1).map(|_| site - width),
                (y + 1 < height).then_some(site + width),
            ]
            .into_iter()
            .flatten()
            {
                for channel in 0..channels {
                    add(neighbour * channels + channel);
                }
            }
            let total: f32 = votes[..used].iter().map(|(_, weight)| weight).sum();
            if total <= 0.0 {
                continue;
            }
            let own_weight = weights[i] as f32 / max_weight;
            let cost = |label| {
                let vote = votes[..used]
                    .iter()
                    .find(|(other, _)| *other == label)
                    .map_or(0.0, |(_, weight)| *weight);
                own_weight * distance(source[i], palette_labs[label])
                    + BIAS / degree * (total - vote)
            };
            let mut best = (labels[i], cost(labels[i]));
            for label in
                std::iter::once(baseline[i]).chain(votes[..used].iter().map(|(label, _)| *label))
            {
                if !chroma_compatible(source[i], palette_labs[label]) {
                    continue;
                }
                let error = distance(source[i], palette_labs[label]);
                if error > errors[i] + BIAS {
                    continue;
                }
                let candidate = cost(label);
                if candidate < best.1 - 1e-12
                    || ((candidate - best.1).abs() <= 1e-12 && palette[label] < palette[best.0])
                {
                    best = (label, candidate);
                }
            }
            changed |= best.0 != labels[i];
            labels[i] = best.0;
        }
        if !changed {
            break;
        }
    }
    labels.into_iter().map(|label| palette[label]).collect()
}
