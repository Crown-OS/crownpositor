//! Between the perceptual level and the value a device takes.
//!
//! A backlight is linear in duty cycle and the eye is not, so a linear slider
//! spends its lower half on changes nobody sees. A monitor's DDC/CI scale is
//! a percentage the monitor has already made roughly perceptual, so it takes
//! the level as it is.

use super::level::FULL;

const BACKLIGHT_GAMMA: f64 = 2.2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Curve {
    Perceptual,
    Linear,
}

impl Curve {
    /// The device value for `level`. A backlight never gets 0, which on
    /// most panels switches the light off.
    pub fn raw(self, level: u32, max: u32) -> u32 {
        let share = f64::from(level.min(FULL)) / f64::from(FULL);
        let scaled = match self {
            Self::Perceptual => share.powf(BACKLIGHT_GAMMA),
            Self::Linear => share,
        };
        let raw = (scaled * f64::from(max)).round() as u32;
        match self {
            Self::Perceptual => raw.clamp(1.min(max), max),
            Self::Linear => raw.min(max),
        }
    }

    /// The level a device value stands for.
    pub fn level(self, raw: u32, max: u32) -> u32 {
        if max == 0 {
            return FULL;
        }
        let share = (f64::from(raw) / f64::from(max)).clamp(0.0, 1.0);
        let perceived = match self {
            Self::Perceptual => share.powf(BACKLIGHT_GAMMA.recip()),
            Self::Linear => share,
        };
        (perceived * f64::from(FULL)).round() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_backlight_value_comes_back_from_its_level() {
        for max in [7, 255, 1023, 4095] {
            for raw in 1..=max {
                let level = Curve::Perceptual.level(raw, max);
                assert_eq!(Curve::Perceptual.raw(level, max), raw, "{raw}/{max}");
            }
        }
    }

    #[test]
    fn the_lower_half_of_the_slider_is_the_dim_end_of_the_backlight() {
        assert!(Curve::Perceptual.raw(FULL / 2, 1000) < 250);
        assert_eq!(Curve::Perceptual.raw(FULL, 1000), 1000);
    }

    #[test]
    fn a_backlight_is_never_switched_off() {
        assert_eq!(Curve::Perceptual.raw(0, 255), 1);
        assert_eq!(Curve::Perceptual.raw(100, 255), 1);
    }

    #[test]
    fn a_ddc_scale_is_linear() {
        assert_eq!(Curve::Linear.raw(4300, 100), 43);
        assert_eq!(Curve::Linear.level(43, 100), 4300);
        assert_eq!(Curve::Linear.raw(0, 100), 0);
    }
}
