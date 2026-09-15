//! Parametric geometry, as the compositor's fragment shader wants it.
//!
//! A shape is a union of primitives, and both primitives the protocol defines
//! collapse to the same one: a circle is a rounded rectangle whose radius is
//! half its side. So the renderer only ever has to evaluate one signed
//! distance field, and `add_circle` costs it nothing beyond `add_rounded_rect`.

use std::sync::{Arc, Mutex};

use smithay::utils::{Logical, Rectangle, Size};

/// How many primitives of one shape the compositor renders.
///
/// Each one is a draw call in every frame the surface is on screen. "The
/// compositor may limit how many primitives it renders for one shape" — this
/// is that limit, and real interfaces ask for a handful.
pub const MAX_PRIMITIVES: usize = 32;

/// One primitive of a shape, in surface-local coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoundedRect {
    pub rect: Rectangle<i32, Logical>,
    /// Clamped to half the shorter side by [`Self::clamped_radius`] rather than
    /// here, because the clamp depends on the rectangle *after* it has been
    /// intersected with the surface — which only the renderer knows.
    pub radius: u32,
}

impl RoundedRect {
    pub fn new(rect: Rectangle<i32, Logical>, radius: u32) -> Self {
        Self { rect, radius }
    }

    /// A circle, as the rounded rectangle that draws it.
    pub fn circle(center_x: i32, center_y: i32, radius: u32) -> Self {
        let diameter = radius.saturating_mul(2).min(i32::MAX as u32) as i32;
        Self {
            rect: Rectangle::new(
                (center_x - diameter / 2, center_y - diameter / 2).into(),
                Size::from((diameter, diameter)),
            ),
            radius,
        }
    }

    /// The radius that actually draws: a radius wider than the box folds the
    /// distance field inside out, so it is capped at half the shorter side.
    pub fn clamped_radius(&self) -> f32 {
        let limit = self.rect.size.w.min(self.rect.size.h).max(0) as f32 * 0.5;
        (self.radius as f32).min(limit)
    }
}

/// A shape as an effect holds it: shared, immutable, and already truncated to
/// what will be drawn.
///
/// `Arc` because the same shape is commonly set as both the blur and the shadow
/// of a surface, and because an effect survives being re-committed every frame
/// by a client that also draws video.
pub type Primitives = Arc<[RoundedRect]>;

/// A `crownos_shape_v1` object's contents, mutable until it is used.
#[derive(Debug, Default)]
pub struct ShapeData(Mutex<Vec<RoundedRect>>);

impl ShapeData {
    pub fn push(&self, primitive: RoundedRect) {
        let Ok(mut primitives) = self.0.lock() else {
            return;
        };
        if primitives.len() >= MAX_PRIMITIVES {
            tracing::debug!(
                limit = MAX_PRIMITIVES,
                "shape has more primitives than we draw; ignoring the rest"
            );
            return;
        }
        primitives.push(primitive);
    }

    /// Copy semantics, which is what lets a client destroy a shape the moment
    /// it has passed it to an effect, and keep adding to one it has already
    /// used without changing that effect.
    pub fn snapshot(&self) -> Primitives {
        self.0
            .lock()
            .map(|primitives| Primitives::from(primitives.as_slice()))
            .unwrap_or_else(|_| Primitives::from(&[][..]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn a_circle_is_a_rounded_rect_around_its_centre() {
        let circle = RoundedRect::circle(100, 50, 20);

        assert_eq!(circle.rect, rect(80, 30, 40, 40));
        assert_eq!(circle.clamped_radius(), 20.0);
    }

    #[test]
    fn a_radius_wider_than_the_box_becomes_a_stadium() {
        // Half the shorter side, not the radius asked for: anything larger
        // folds the distance field inside out.
        let pill = RoundedRect::new(rect(0, 0, 200, 60), 999);
        assert_eq!(pill.clamped_radius(), 30.0);
    }

    #[test]
    fn a_shape_stops_growing_at_the_limit() {
        let shape = ShapeData::default();
        for index in 0..MAX_PRIMITIVES as i32 + 10 {
            shape.push(RoundedRect::new(rect(index, 0, 10, 10), 0));
        }
        assert_eq!(shape.snapshot().len(), MAX_PRIMITIVES);
    }

    #[test]
    fn a_snapshot_does_not_follow_the_shape_it_came_from() {
        // The whole of the copy semantics: an effect set from a shape must not
        // change when the client reuses that shape for something else.
        let shape = ShapeData::default();
        shape.push(RoundedRect::new(rect(0, 0, 10, 10), 2));
        let taken = shape.snapshot();

        shape.push(RoundedRect::new(rect(50, 50, 10, 10), 2));

        assert_eq!(taken.len(), 1);
        assert_eq!(shape.snapshot().len(), 2);
    }
}
