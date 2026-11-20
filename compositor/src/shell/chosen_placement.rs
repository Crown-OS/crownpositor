//! The scale and position output management set on a monitor.
//!
//! A monitor with a config entry has them saved there. A monitor with none
//! (a virtual output, a nested one that is not persisted) has nowhere else to
//! keep them, so they hold until the config names the monitor.

use smithay::utils::{Logical, Point};

use config::OutputSetting;

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct ChosenPlacement {
    pub scale: Option<f64>,
    pub position: Option<Point<i32, Logical>>,
}

impl ChosenPlacement {
    /// The scale to apply: the config's, else the chosen one for a monitor
    /// the config does not name. `None` asks for a guess.
    pub fn scale_for(&self, setting: Option<&OutputSetting>) -> Option<f64> {
        setting.map_or(self.scale, |setting| setting.scale)
    }

    /// Where to pin the monitor: where the config places it, else the chosen
    /// position for a monitor the config does not name. `None` packs it.
    pub fn position_for(
        &self,
        setting: Option<&OutputSetting>,
        configured: Option<Point<i32, Logical>>,
    ) -> Option<Point<i32, Logical>> {
        configured.or(setting.map_or(self.position, |_| None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHOSEN: ChosenPlacement = ChosenPlacement {
        scale: Some(2.0),
        position: Some(Point::new(0, 500)),
    };

    fn scaled(scale: Option<f64>) -> OutputSetting {
        OutputSetting {
            scale,
            ..OutputSetting::default()
        }
    }

    #[test]
    fn a_monitor_the_config_does_not_name_keeps_what_was_chosen() {
        assert_eq!(CHOSEN.scale_for(None), Some(2.0));
        assert_eq!(CHOSEN.position_for(None, None), Some(Point::new(0, 500)));
    }

    #[test]
    fn a_config_entry_wins_over_what_was_chosen() {
        let setting = scaled(Some(1.5));
        assert_eq!(CHOSEN.scale_for(Some(&setting)), Some(1.5));
        assert_eq!(CHOSEN.scale_for(Some(&scaled(None))), None);
        assert_eq!(CHOSEN.position_for(Some(&setting), None), None);
    }

    #[test]
    fn a_configured_position_wins_for_every_monitor() {
        let arranged = Some(Point::new(1920, 0));
        assert_eq!(CHOSEN.position_for(None, arranged), arranged);
    }
}
