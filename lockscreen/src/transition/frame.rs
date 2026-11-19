use super::LayerStyle;
use crate::clock::FROST_DARKENING;

/// How to draw everything at one instant, back to front: the desktop, the
/// lock's background flooding over it, the background's darkening, the
/// clock layer, the background's subject when the clock has depth, and the
/// prompt layer. A host places it on its output with [`Frame::risen`].
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
    /// The background's subject, cut out for depth, in front of the clock:
    /// framed like the background, and only while the clock shows.
    pub subject: LayerStyle,
    /// How far the clock and subject have risen in from below the bottom
    /// edge: 0 a whole output height below their place, 1 in it.
    pub rise: f32,
    /// How far the background has flooded out from behind its subject over
    /// the desktop: 0 before it wells up, 1 once it covers the output, a
    /// touch past while the spring overshoots. A host that cannot draw the
    /// flood shows the background whole.
    pub flood: f32,
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
        (1.0 - kept) * self.wallpaper.opacity * self.flood.clamp(0.0, 1.0)
    }

    /// The darkening the frost behind glass digits carries at this instant:
    /// [`FROST_DARKENING`] of [`darkening`](Self::darkening).
    pub fn frost_darkening(&self, lock_dim: f32) -> f32 {
        FROST_DARKENING * self.darkening(lock_dim)
    }

    /// Placed on an output `height` logical pixels tall: the clock and the
    /// subject lowered by as much as they still have to rise.
    #[must_use]
    pub fn risen(mut self, height: f32) -> Self {
        let drop = (1.0 - self.rise) * height;
        self.clock.offset[1] += drop;
        self.subject.offset[1] += drop;
        self
    }
}
