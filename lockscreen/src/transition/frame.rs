use super::LayerStyle;
use crate::clock::FROST_DARKENING;

/// How to draw everything at one instant, back to front: the desktop, the
/// lock's background over it, the background's darkening, the clock layer,
/// the background's subject when the clock has depth, and the prompt layer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    /// The clock screen — clock and date, plugin slots, a theme's
    /// decoration — scaled about the layer's centre.
    pub clock: LayerStyle,
    /// The password prompt — user, field, messages — scaled about the
    /// layer's centre.
    pub prompt: LayerStyle,
    /// The lock's own background. `opacity` is how much of it shows over the
    /// desktop, `scale` its zoom about the output's centre and `blur` the
    /// sigma, in logical pixels, of the gaussian it is blurred by behind the
    /// prompt: none at the clock. `offset` stays zero.
    pub wallpaper: LayerStyle,
    /// Extra darkening of the background, `0..=1`, on top of `lock.dim`.
    pub dim: f32,
    /// Horizontal offset of the password field, in logical pixels.
    pub shake: f32,
}

impl Frame {
    /// The alpha of the black wash over the whole background: `lock_dim`
    /// and this frame's darkening together, fading out with the background.
    pub fn darkening(&self, lock_dim: f32) -> f32 {
        let kept = (1.0 - lock_dim.clamp(0.0, 1.0)) * (1.0 - self.dim.clamp(0.0, 1.0));
        (1.0 - kept) * self.wallpaper.opacity
    }

    /// The darkening the frost behind glass digits carries at this instant:
    /// [`FROST_DARKENING`] of [`darkening`](Self::darkening).
    pub fn frost_darkening(&self, lock_dim: f32) -> f32 {
        FROST_DARKENING * self.darkening(lock_dim)
    }

    /// How the background's subject, cut out for depth, is drawn in front of
    /// the clock: moving with the background, and only while the clock shows.
    pub fn subject(&self) -> LayerStyle {
        LayerStyle {
            opacity: self.wallpaper.opacity * self.clock.opacity,
            ..self.wallpaper
        }
    }
}
