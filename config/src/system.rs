use crownos_config::schema::{AccentColor, AnimationProfile, Appearance, Display, DisplayScale};

/// The slice of system settings that changes how the compositor draws or places
/// things.
#[derive(Debug, Clone, PartialEq)]
pub struct System {
    pub dark_mode: bool,
    pub accent: AccentColor,
    pub transparency: f64,
    pub scale: DisplayScale,
    pub night_light: bool,

    pub gaps_inner: u16,
    pub gaps_outer: u16,
    pub border_width: u16,
    pub border_radius: u16,
    pub animations: AnimationProfile,

    pub blur: bool,
    pub blur_passes: u16,
    pub blur_size: f64,
    pub blur_noise: f64,
}

impl Default for System {
    fn default() -> Self {
        Self::from_parts(Appearance::default(), Display::default())
    }
}

impl System {
    pub fn load() -> Self {
        Self::from_parts(
            crownos_config::load(Appearance::SECTION),
            crownos_config::load(Display::SECTION),
        )
    }

    pub fn from_parts(appearance: Appearance, display: Display) -> Self {
        Self {
            dark_mode: appearance.dark_mode,
            accent: appearance.accent,
            transparency: appearance.transparency.clamp(0.0, 1.0),
            scale: display.scale,
            night_light: display.night_light,
            gaps_inner: appearance.gaps_inner,
            gaps_outer: appearance.gaps_outer,
            border_width: appearance.border_width,
            border_radius: appearance.border_radius,
            animations: appearance.animations,
            blur: appearance.blur,
            blur_passes: appearance.blur_passes,
            blur_size: appearance.blur_size,
            blur_noise: appearance.blur_noise,
        }
    }

    /// Default window opacity. `transparency` is how see-through the user wants
    /// things, so opacity is its complement.
    pub fn opacity(&self) -> f32 {
        (1.0 - self.transparency) as f32
    }

    pub fn scale_factor(&self) -> f64 {
        self.scale.factor()
    }

    /// The sections the compositor should watch for live reloads.
    pub fn sections() -> [&'static str; 2] {
        [Appearance::SECTION, Display::SECTION]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opacity_is_the_complement_of_transparency() {
        let system = System {
            transparency: 0.25,
            ..System::default()
        };
        assert!((system.opacity() - 0.75).abs() < f32::EPSILON);
    }

    #[test]
    fn transparency_is_clamped_to_a_usable_range() {
        let opaque = System::from_parts(
            Appearance {
                transparency: -3.0,
                ..Default::default()
            },
            Display::default(),
        );
        assert_eq!(opaque.transparency, 0.0);

        // Fully invisible windows would be indistinguishable from a bug.
        let clear = System::from_parts(
            Appearance {
                transparency: 9.0,
                ..Default::default()
            },
            Display::default(),
        );
        assert_eq!(clear.transparency, 1.0);
    }

    #[test]
    fn display_scale_maps_to_a_factor() {
        let system = System {
            scale: DisplayScale::S200,
            ..System::default()
        };
        assert_eq!(system.scale_factor(), 2.0);
    }
}
