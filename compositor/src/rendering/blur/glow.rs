//! The lit rim of a piece of glass, per kind of surface.

use config::{EdgeGlow, GlassSettings};

/// Which of the configured rims a piece of glass wears.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlassKind {
    /// Window frames, a window's own blurred background, and its thumbnail.
    Window,
    /// Layer-shell surfaces.
    Panel,
    /// The compositor's menus and other small overlays.
    Menu,
}

/// How the rim is lit, as the finish shader takes it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shading {
    /// Brightness of the highlight.
    pub intensity: f32,
    /// How much of the highlight's colour comes from what is behind the glass.
    pub tint: f32,
    /// Depth of the shade inside the edge.
    pub inner_shadow: f32,
    /// Unit vector towards the light, in output-local axes.
    pub light: (f32, f32),
}

impl Default for Shading {
    fn default() -> Self {
        Shading::from(&EdgeGlow::WINDOW)
    }
}

impl From<&EdgeGlow> for Shading {
    fn from(glow: &EdgeGlow) -> Self {
        let angle = (glow.light_angle as f32).to_radians();
        Self {
            intensity: glow.intensity.clamp(0.0, 1.0) as f32,
            tint: glow.adaptive_tint.clamp(0.0, 1.0) as f32,
            inner_shadow: glow.inner_shadow.clamp(0.0, 1.0) as f32,
            // Clockwise from up, with y growing downwards.
            light: (angle.sin(), -angle.cos()),
        }
    }
}

/// One kind's rim, ready to scale onto an output.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rim {
    /// Logical pixels; zero when the glow is off, which flattens the edge.
    pub width: f32,
    pub shading: Shading,
}

impl From<&EdgeGlow> for Rim {
    fn from(glow: &EdgeGlow) -> Self {
        Self {
            width: if glow.enabled {
                glow.width.clamp(0.0, 64.0) as f32
            } else {
                0.0
            },
            shading: Shading::from(glow),
        }
    }
}

/// The three configured rims.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rims {
    window: Rim,
    panel: Rim,
    menu: Rim,
}

impl Rims {
    pub fn get(&self, kind: GlassKind) -> Rim {
        match kind {
            GlassKind::Window => self.window,
            GlassKind::Panel => self.panel,
            GlassKind::Menu => self.menu,
        }
    }

    /// Every value that changes pixels, for the blur fingerprint.
    pub fn bits(&self) -> impl Iterator<Item = u32> + '_ {
        [self.window, self.panel, self.menu]
            .into_iter()
            .flat_map(|rim| {
                let Shading {
                    intensity,
                    tint,
                    inner_shadow,
                    light,
                } = rim.shading;
                [rim.width, intensity, tint, inner_shadow, light.0, light.1]
            })
            .map(f32::to_bits)
    }
}

impl From<&GlassSettings> for Rims {
    fn from(settings: &GlassSettings) -> Self {
        Self {
            window: Rim::from(&settings.windows),
            panel: Rim::from(&settings.panels),
            menu: Rim::from(&settings.menus),
        }
    }
}

impl Default for Rims {
    fn default() -> Self {
        Self::from(&GlassSettings::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_disabled_glow_flattens_the_edge() {
        let rim = Rim::from(&EdgeGlow {
            enabled: false,
            ..EdgeGlow::WINDOW
        });
        assert_eq!(rim.width, 0.0);
    }

    #[test]
    fn the_light_angle_points_where_it_says() {
        let from_top = Shading::from(&EdgeGlow {
            light_angle: 0.0,
            ..EdgeGlow::WINDOW
        });
        assert!(from_top.light.0.abs() < 1e-6 && (from_top.light.1 + 1.0).abs() < 1e-6);

        let upper_left = Shading::from(&EdgeGlow::WINDOW).light;
        assert!(upper_left.0 < 0.0 && upper_left.1 < 0.0);
        assert!((upper_left.0 - upper_left.1).abs() < 1e-6);
    }

    #[test]
    fn each_kind_reads_its_own_settings() {
        let rims = Rims::default();
        assert_eq!(rims.get(GlassKind::Menu).width, EdgeGlow::MENU.width as f32);
        assert_eq!(
            rims.get(GlassKind::Panel).shading.intensity,
            EdgeGlow::PANEL.intensity as f32
        );
    }
}
