//! The perceptual level clients see, from 0 to [`FULL`], and the lowest level
//! each control lets an output reach.

use crate::{color::night_light::MIN_BRIGHTNESS, controls::Step};

pub const FULL: u32 = 10_000;

const BACKLIGHT_FLOOR: u32 = FULL / 100;
const SOFTWARE_FLOOR: u32 = (MIN_BRIGHTNESS * FULL as f64) as u32;

/// How the compositor changes the brightness of one output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    None,
    Backlight,
    Ddc,
    Software,
}

impl Control {
    /// The lowest level, so the control that brings the screen back stays
    /// visible. A monitor's own DDC/CI scale already stops short of black.
    pub fn floor(self) -> u32 {
        match self {
            Self::None => FULL,
            Self::Backlight => BACKLIGHT_FLOOR,
            Self::Ddc => 0,
            Self::Software => SOFTWARE_FLOOR,
        }
    }

    /// `level` raised to the floor, or `None` when the level cannot change.
    pub fn clamp(self, level: u32) -> Option<u32> {
        match self {
            Self::None => None,
            _ => Some(level.clamp(self.floor(), FULL)),
        }
    }
}

/// What one output reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reading {
    pub control: Control,
    pub level: u32,
}

impl Reading {
    pub const UNCONTROLLED: Self = Self {
        control: Control::None,
        level: FULL,
    };
}

/// `level` moved by `step` percent of the full range.
pub fn stepped(level: u32, step: Step) -> u32 {
    let change = i64::from(step.0) * i64::from(FULL) / 100;
    (i64::from(level) + change).clamp(0, i64::from(FULL)) as u32
}

/// The `software_brightness` entry for a software level: unset at full, so a
/// monitor brought back to full loses its old dim.
pub fn software_share(level: u32) -> Option<f64> {
    (level < FULL).then(|| f64::from(level) / f64::from(FULL))
}

/// The software level a saved `software_brightness` asks for.
pub fn software_level(share: Option<f64>) -> u32 {
    let share = share.unwrap_or(1.0);
    if !share.is_finite() {
        return FULL;
    }
    Control::Software
        .clamp((share * f64::from(FULL)).round() as u32)
        .unwrap_or(FULL)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_control_raises_a_level_to_its_own_floor() {
        assert_eq!(Control::Backlight.clamp(0), Some(100));
        assert_eq!(Control::Software.clamp(500), Some(1500));
        assert_eq!(Control::Ddc.clamp(0), Some(0));
        assert_eq!(Control::Software.clamp(7000), Some(7000));
        assert_eq!(Control::Backlight.clamp(20_000), Some(FULL));
    }

    #[test]
    fn an_output_with_no_control_takes_no_level() {
        assert_eq!(Control::None.clamp(5000), None);
        assert_eq!(Reading::UNCONTROLLED.level, FULL);
    }

    #[test]
    fn a_step_is_a_share_of_the_full_range_and_stays_in_it() {
        assert_eq!(stepped(5000, Step(5)), 5500);
        assert_eq!(stepped(5000, Step(-5)), 4500);
        assert_eq!(stepped(9800, Step(5)), FULL);
        assert_eq!(stepped(200, Step(-5)), 0);
    }

    #[test]
    fn a_software_level_survives_a_save_and_a_load() {
        for level in [1500, 4321, 9999] {
            assert_eq!(software_level(software_share(level)), level);
        }
        assert_eq!(software_share(FULL), None);
        assert_eq!(software_level(None), FULL);
    }

    #[test]
    fn a_saved_share_out_of_range_is_brought_back_into_it() {
        assert_eq!(software_level(Some(0.01)), 1500);
        assert_eq!(software_level(Some(3.0)), FULL);
        assert_eq!(software_level(Some(f64::NAN)), FULL);
    }
}
