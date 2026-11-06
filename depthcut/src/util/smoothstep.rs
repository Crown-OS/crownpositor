/// 0 up to `low`, 1 from `high`, and a Hermite curve between.
pub fn smoothstep(low: f32, high: f32, value: f32) -> f32 {
    let progress = ((value - low) / (high - low)).clamp(0.0, 1.0);
    progress * progress * (3.0 - 2.0 * progress)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_eases_from_zero_to_one() {
        assert_eq!(smoothstep(0.4, 0.6, 0.1), 0.0);
        assert!((smoothstep(0.4, 0.6, 0.5) - 0.5).abs() < 1e-6);
        assert_eq!(smoothstep(0.4, 0.6, 0.9), 1.0);
        assert!(smoothstep(0.4, 0.6, 0.45) < 0.25);
    }
}
