pub mod rules;
pub mod system;

pub use crownos_config::schema::{
    AnimationProfile, Binding, Compositor, LayoutMode, OutputSetting, OutputTransform, WindowRule,
};
pub use rules::{ResolvedRule, WindowRules};
pub use system::System;

#[derive(Debug)]
pub struct Config {
    pub compositor: Compositor,
    pub system: System,
    pub rules: WindowRules,

    pub gaps_inner: i32,
    pub gaps_outer: i32,
    pub border_width: i32,
    pub border_radius: i32,
    pub focus_follows_mouse: bool,
    pub default_layout: LayoutMode,
    pub animation: AnimationProfile,
}

impl Default for Config {
    fn default() -> Self {
        Self::compile(Compositor::default(), System::default())
    }
}

impl Config {
    /// Reads every section the compositor cares about.
    pub fn load() -> Self {
        Self::compile(crownos_config::load(Compositor::SECTION), System::load())
    }

    /// Re-reads only the shared settings, keeping the compositor section.
    pub fn reload_system(&self) -> Self {
        Self::compile(self.compositor.clone(), System::load())
    }

    /// Re-reads only the compositor section, keeping the shared settings.
    pub fn reload_compositor(&self) -> Self {
        Self::compile(
            crownos_config::load(Compositor::SECTION),
            self.system.clone(),
        )
    }

    pub fn compile(compositor: Compositor, system: System) -> Self {
        let rules = WindowRules::compile(&compositor.window_rules);
        Self {
            gaps_inner: system.gaps_inner as i32,
            gaps_outer: system.gaps_outer as i32,
            border_width: system.border_width as i32,
            border_radius: system.border_radius as i32,
            animation: system.animations,
            focus_follows_mouse: compositor.focus_follows_mouse,
            default_layout: compositor.layout,
            rules,
            compositor,
            system,
        }
    }

    /// Matched on connector name first, `"MAKE MODEL SERIAL"` identity second.
    pub fn output_setting(&self, connector: &str, identity: &str) -> Option<&OutputSetting> {
        self.compositor
            .outputs
            .iter()
            .find(|setting| setting.name == connector)
            .or_else(|| self.compositor.outputs.iter().find(|s| s.name == identity))
    }

    /// The scale for an output: its own override, else the system default.
    pub fn scale_for(&self, connector: &str, identity: &str) -> f64 {
        self.output_setting(connector, identity)
            .and_then(|setting| setting.scale)
            .unwrap_or_else(|| self.system.scale_factor())
    }

    /// Default window opacity, unless a rule says otherwise.
    pub fn opacity_for(&self, rule: &ResolvedRule) -> f32 {
        rule.opacity.unwrap_or_else(|| self.system.opacity())
    }

    /// Corner radius for a window, unless a rule says otherwise.
    pub fn corner_radius_for(&self, rule: &ResolvedRule) -> i32 {
        rule.corner_radius
            .map(i32::from)
            .unwrap_or(self.border_radius)
    }

    /// Every section a live-reload watcher should subscribe to.
    pub fn sections() -> Vec<&'static str> {
        let mut sections = vec![Compositor::SECTION];
        sections.extend(System::sections());
        sections
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_output_scale_overrides_the_system_default() {
        let config = Config::compile(
            Compositor {
                outputs: vec![OutputSetting {
                    name: "eDP-1".into(),
                    scale: Some(2.0),
                    ..Default::default()
                }],
                ..Default::default()
            },
            System::default(),
        );

        assert_eq!(config.scale_for("eDP-1", "Some Panel"), 2.0);
        assert_eq!(
            config.scale_for("HDMI-1", "Other"),
            config.system.scale_factor(),
            "an output with no override follows the system setting"
        );
    }

    #[test]
    fn an_output_matches_on_its_edid_identity_too() {
        let config = Config::compile(
            Compositor {
                outputs: vec![OutputSetting {
                    name: "DELL U2720Q ABC123".into(),
                    scale: Some(1.5),
                    ..Default::default()
                }],
                ..Default::default()
            },
            System::default(),
        );
        // Connector names are reassigned across a dock replug; the identity is not.
        assert_eq!(config.scale_for("DP-3", "DELL U2720Q ABC123"), 1.5);
    }

    #[test]
    fn a_rule_overrides_system_opacity() {
        let config = Config::compile(Compositor::default(), System::default());

        let default = ResolvedRule::default();
        assert_eq!(config.opacity_for(&default), config.system.opacity());

        let ruled = ResolvedRule {
            opacity: Some(0.5),
            ..Default::default()
        };
        assert_eq!(config.opacity_for(&ruled), 0.5);
    }

    #[test]
    fn corner_radius_falls_back_to_the_global_border_radius() {
        let config = Config::compile(Compositor::default(), System::default());
        assert_eq!(
            config.corner_radius_for(&ResolvedRule::default()),
            config.border_radius
        );
    }

    #[test]
    fn the_watch_list_covers_both_layers() {
        let sections = Config::sections();
        assert!(sections.contains(&Compositor::SECTION));
        assert!(sections.contains(&"appearance"));
        assert!(sections.contains(&"display"));
    }
}
