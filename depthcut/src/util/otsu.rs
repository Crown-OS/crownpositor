const LEVELS: usize = 256;

/// The value that best splits `values`, all in `0..=1`, into a darker and a
/// brighter class: Otsu's maximum between-class variance over a 256-level
/// histogram, taken midway across a run of equally good splits. `None` when
/// every value falls on one level and there is nothing to split.
pub fn threshold(values: &[f32]) -> Option<f32> {
    let top = (LEVELS - 1) as f32;
    let mut histogram = [0u64; LEVELS];
    for value in values {
        histogram[(value.clamp(0.0, 1.0) * top).round() as usize] += 1;
    }
    let total = values.len() as f64;
    let total_mass: f64 = histogram
        .iter()
        .enumerate()
        .map(|(level, &count)| level as f64 * count as f64)
        .sum();
    let (mut below, mut below_mass) = (0.0, 0.0);
    let mut best: Option<(f64, usize, usize)> = None;
    for (level, &count) in histogram.iter().enumerate() {
        below += count as f64;
        below_mass += level as f64 * count as f64;
        let above = total - below;
        if below == 0.0 || above == 0.0 {
            continue;
        }
        let mean_gap = below_mass / below - (total_mass - below_mass) / above;
        let variance = below * above * mean_gap * mean_gap;
        best = match best {
            Some((widest, first, _)) if variance == widest => Some((widest, first, level)),
            Some((widest, ..)) if variance < widest => best,
            _ => Some((variance, level, level)),
        };
    }
    best.map(|(_, first, last)| ((first + last) as f32 / 2.0 + 0.5) / top)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_spikes_split_midway() {
        let values = [&[0.2; 300][..], &[0.8; 100]].concat();
        let threshold = threshold(&values).unwrap_or_default();
        assert!((threshold - 0.5).abs() < 1.0 / 255.0, "{threshold}");
    }

    #[test]
    fn two_spread_classes_split_between_them() {
        let spread = |centre: f32, count: usize| {
            (0..count).map(move |index| centre + 0.1 * (index as f32 / count as f32 - 0.5))
        };
        let values: Vec<f32> = spread(0.25, 900).chain(spread(0.75, 300)).collect();
        let threshold = threshold(&values).unwrap_or_default();
        assert!((0.3..0.7).contains(&threshold), "{threshold}");
    }

    #[test]
    fn one_level_has_nothing_to_split() {
        assert_eq!(threshold(&[0.4; 64]), None);
        assert_eq!(threshold(&[]), None);
    }
}
