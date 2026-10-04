//! What the renderer applies to an animated subsurface.

use smithay::utils::{Logical, Point};

/// Applied on top of the subsurface's own position: scaled about its center,
/// then translated, and faded by `opacity`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceTransform {
    /// Surface-local logical pixels.
    pub translation: Point<f64, Logical>,
    pub scale: f64,
    pub opacity: f32,
}

impl SurfaceTransform {
    pub const IDENTITY: Self = Self {
        translation: Point::new(0.0, 0.0),
        scale: 1.0,
        opacity: 1.0,
    };

    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }
}

impl Default for SurfaceTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}
