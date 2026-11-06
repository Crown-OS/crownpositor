/// Fast off the mark, gentle on arrival: `0..=1` onto `0..=1`.
pub fn ease_out(time: f32) -> f32 {
    1.0 - (1.0 - time.clamp(0.0, 1.0)).powi(3)
}

/// Gentle at both ends: `0..=1` onto `0..=1`.
pub fn ease_in_out(time: f32) -> f32 {
    let t = time.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (2.0 - 2.0 * t).powi(3) / 2.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_curves_start_at_rest_and_land() {
        for curve in [ease_out, ease_in_out] {
            assert_eq!(curve(-1.0), 0.0);
            assert_eq!(curve(0.0), 0.0);
            assert_eq!(curve(1.0), 1.0);
            assert_eq!(curve(2.0), 1.0);
        }
        assert!(ease_out(0.5) > ease_in_out(0.5));
    }
}
