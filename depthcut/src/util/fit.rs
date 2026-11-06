/// `dimensions` shrunk so the longer side is at most `long_side`, keeping the
/// aspect ratio; a picture already within it keeps its size.
pub fn within_long_side((width, height): (u32, u32), long_side: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= long_side {
        return (width, height);
    }
    let scale = f64::from(long_side) / f64::from(longest);
    let fit = |side: u32| ((f64::from(side) * scale).round() as u32).max(1);
    (fit(width), fit(height))
}

/// The size with `dimensions`' aspect ratio, as near as whole `patch` squares
/// allow, that holds about `patches` of them: a vision transformer then costs
/// the same whatever the picture's shape.
pub fn patch_grid((width, height): (u32, u32), patch: u32, patches: u32) -> (u32, u32) {
    let aspect = f64::from(width.max(height)) / f64::from(width.min(height).max(1));
    let short = (f64::from(patches) / aspect).sqrt().round().max(1.0);
    let long = (short * aspect).round();
    let (short, long) = (short as u32 * patch, long as u32 * patch);
    if width >= height {
        (long, short)
    } else {
        (short, long)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATCH: u32 = 14;
    const BUDGET: u32 = 37 * 37;

    #[test]
    fn a_large_picture_shrinks_to_the_long_side() {
        assert_eq!(within_long_side((5120, 2880), 1280), (1280, 720));
        assert_eq!(within_long_side((2880, 5120), 1280), (720, 1280));
        assert_eq!(within_long_side((4000, 1), 1280), (1280, 1));
    }

    #[test]
    fn a_small_picture_keeps_its_size() {
        assert_eq!(within_long_side((800, 600), 1280), (800, 600));
        assert_eq!(within_long_side((1280, 1280), 1280), (1280, 1280));
    }

    #[test]
    fn a_square_fills_the_training_square() {
        assert_eq!(patch_grid((1000, 1000), PATCH, BUDGET), (518, 518));
    }

    #[test]
    fn a_widescreen_keeps_its_aspect_on_whole_patches() {
        assert_eq!(patch_grid((5120, 2880), PATCH, BUDGET), (700, 392));
        assert_eq!(patch_grid((2880, 5120), PATCH, BUDGET), (392, 700));
    }

    #[test]
    fn every_shape_stays_near_the_budget() {
        for dimensions in [(3440, 1440), (5120, 1440), (1080, 2400), (64, 48), (7, 3)] {
            let (width, height) = patch_grid(dimensions, PATCH, BUDGET);
            assert_eq!((width % PATCH, height % PATCH), (0, 0));
            let patches = (width / PATCH) * (height / PATCH);
            assert!(
                patches.abs_diff(BUDGET) * 20 < BUDGET,
                "{dimensions:?} -> {patches}"
            );
            let aspect = |(w, h): (u32, u32)| f64::from(w) / f64::from(h);
            assert!((aspect((width, height)) / aspect(dimensions) - 1.0).abs() < 0.05);
        }
    }
}
