pub mod rules;
pub mod startup;
pub mod watch;

pub use crownos_config::schema::{
    AnimationProfile, Binding, Compositor, LayoutMode, OutputSetting, OutputTransform, WindowRule,
};
pub use crownos_config::{Appearance, Display, DisplayScale, Keybinds};
pub use rules::{ResolvedRule, WindowRules};
pub use startup::split_argv;
pub use watch::{Update, Watch};

use crownos_config::load;

#[derive(Debug, Default)]
pub struct Config {
    pub compositor: Compositor,
    pub keybinds: Keybinds,
    pub appearance: Appearance,
    pub display: Display,
    pub window_rules: WindowRules,
}

impl Config {
    /// Reads every section the compositor cares about.
    pub fn load() -> Self {
        let compositor: Compositor = load(Compositor::SECTION);

        Self {
            window_rules: WindowRules::compile(&compositor.window_rules),
            compositor,
            keybinds: load(Keybinds::SECTION),
            appearance: load(Appearance::SECTION),
            display: load(Display::SECTION),
        }
    }

    pub fn output_setting(&self, connector: &str, identity: &str) -> Option<&OutputSetting> {
        self.compositor
            .outputs
            .iter()
            .find(|setting| setting.name == connector)
            .or_else(|| self.compositor.outputs.iter().find(|s| s.name == identity))
    }

    /// Default window opacity, unless a rule says otherwise. `transparency` is
    /// how see-through the user wants windows, so opacity is its complement.
    pub fn opacity_for(&self, rule: &ResolvedRule) -> f32 {
        rule.opacity
            .unwrap_or_else(|| 1.0 - self.appearance.transparency.clamp(0.0, 1.0) as f32)
    }

    /// Corner radius for a window, unless a rule says otherwise.
    pub fn corner_radius_for(&self, rule: &ResolvedRule) -> i32 {
        rule.corner_radius
            .unwrap_or(self.appearance.border_radius)
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config {
            compositor: Compositor {
                outputs: vec![
                    OutputSetting {
                        name: "eDP-1".into(),
                        scale: Some(2.0),
                        ..Default::default()
                    },
                    OutputSetting {
                        name: "Dell U2720Q ABC123".into(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            ..Config::default()
        }
    }

    #[test]
    fn an_output_matches_on_its_edid_identity_too() {
        let config = config();

        assert_eq!(
            config
                .output_setting("eDP-1", "Some Panel XYZ")
                .unwrap()
                .scale,
            Some(2.0)
        );
        assert_eq!(
            config
                .output_setting("DP-3", "Dell U2720Q ABC123")
                .unwrap()
                .name,
            "Dell U2720Q ABC123"
        );
        assert!(config.output_setting("DP-3", "Unknown").is_none());
    }

    #[test]
    fn opacity_is_the_complement_of_transparency() {
        let config = Config {
            appearance: Appearance {
                transparency: 0.25,
                ..Default::default()
            },
            ..Config::default()
        };

        assert!((config.opacity_for(&ResolvedRule::default()) - 0.75).abs() < f32::EPSILON);
    }

    #[test]
    fn a_rule_overrides_the_global_defaults() {
        let config = config();
        let rule = ResolvedRule {
            opacity: Some(0.5),
            corner_radius: Some(16),
            ..Default::default()
        };

        assert_eq!(config.opacity_for(&rule), 0.5);
        assert_eq!(config.corner_radius_for(&rule), 16);
    }

    #[test]
    fn corner_radius_falls_back_to_the_global_border_radius() {
        let config = Config {
            appearance: Appearance {
                border_radius: 12,
                ..Default::default()
            },
            ..Config::default()
        };

        assert_eq!(config.corner_radius_for(&ResolvedRule::default()), 12);
    }
}
