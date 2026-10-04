//! `p ↦ scale·p + offset`: the transforms a rescale-and-relocate wrapper can
//! express, composable down a surface tree.

use smithay::utils::{Physical, Point};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UniformAffine {
    pub scale: f64,
    pub offset: Point<f64, Physical>,
}

impl UniformAffine {
    pub const IDENTITY: Self = Self {
        scale: 1.0,
        offset: Point::new(0.0, 0.0),
    };

    /// Scales about `center`, then translates.
    pub fn about(
        center: Point<f64, Physical>,
        scale: f64,
        translation: Point<f64, Physical>,
    ) -> Self {
        Self {
            scale,
            offset: center.upscale(1.0 - scale) + translation,
        }
    }

    /// `inner` first, then `self`: a child's own transform under its parent's.
    pub fn compose(self, inner: Self) -> Self {
        Self {
            scale: self.scale * inner.scale,
            offset: inner.offset.upscale(self.scale) + self.offset,
        }
    }

    pub fn apply(self, point: Point<f64, Physical>) -> Point<f64, Physical> {
        point.upscale(self.scale) + self.offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f64, y: f64) -> Point<f64, Physical> {
        Point::new(x, y)
    }

    #[test]
    fn scaling_about_the_center_keeps_the_center_in_place() {
        let center = point(50.0, 30.0);
        let affine = UniformAffine::about(center, 0.5, point(0.0, 0.0));
        assert_eq!(affine.apply(center), center);
        assert_eq!(affine.apply(point(0.0, 0.0)), point(25.0, 15.0));
    }

    #[test]
    fn translation_applies_after_scaling() {
        let affine = UniformAffine::about(point(10.0, 10.0), 2.0, point(5.0, -5.0));
        assert_eq!(affine.apply(point(10.0, 10.0)), point(15.0, 5.0));
        assert_eq!(affine.apply(point(20.0, 10.0)), point(35.0, 5.0));
    }

    #[test]
    fn composition_applies_the_inner_transform_first() {
        let parent = UniformAffine::about(point(0.0, 0.0), 2.0, point(100.0, 0.0));
        let child = UniformAffine::about(point(10.0, 10.0), 0.5, point(0.0, 20.0));
        let probe = point(4.0, 6.0);
        assert_eq!(
            parent.compose(child).apply(probe),
            parent.apply(child.apply(probe))
        );
        assert_eq!(UniformAffine::IDENTITY.compose(child), child);
    }
}
