use crownos_config::schema::Easing;

const SPRING_OVERSHOOT: f32 = 1.70158;

pub trait Curve {
    /// Maps linear time in `0..=1` to progress, `0` and `1` fixed.
    fn ease(self, time: f32) -> f32;
}

impl Curve for Easing {
    fn ease(self, time: f32) -> f32 {
        let t = time.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            Self::EaseIn => t * t * t,
            Self::EaseOut => 1.0 - (1.0 - t).powi(3),
            Self::EaseInOut if t < 0.5 => 4.0 * t * t * t,
            Self::EaseInOut => 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0,
            Self::Spring => {
                let back = t - 1.0;
                1.0 + (SPRING_OVERSHOOT + 1.0) * back.powi(3) + SPRING_OVERSHOOT * back.powi(2)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_curve_starts_at_rest_and_lands() {
        for easing in [
            Easing::Linear,
            Easing::EaseIn,
            Easing::EaseOut,
            Easing::EaseInOut,
            Easing::Spring,
        ] {
            assert!(easing.ease(0.0).abs() < 1e-5, "{easing:?}");
            assert!((easing.ease(1.0) - 1.0).abs() < 1e-5, "{easing:?}");
        }
        assert!(Easing::Spring.ease(0.8) > 1.0);
    }
}
