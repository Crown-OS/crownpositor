//! What a surface has asked to be drawn with, pending and committed.

use std::sync::Mutex;

use smithay::{
    utils::{Logical, Point},
    wayland::compositor::{Cacheable, SurfaceData},
};
use wayland_server::DisplayHandle;

use crate::crownos_background_effects::{color::Argb, shape::Primitives};

/// The blurred, tinted, saturated backdrop under a shape.
#[derive(Debug, Clone, PartialEq)]
pub struct Blur {
    /// `None` is the whole surface, clipped by the corner radius.
    pub shape: Option<Primitives>,
    /// Surface-local pixels. Zero is off, and the client has already been told
    /// so by the protocol, so this is never zero here.
    pub radius: u32,
    pub tint: Argb,
    /// Chroma multiplier about the luma. Never negative — `set_blur` rejects
    /// that — but freely above one, which is the vibrancy.
    pub saturation: f64,
}

/// The blurred silhouette cast underneath a surface.
#[derive(Debug, Clone, PartialEq)]
pub struct Shadow {
    pub shape: Option<Primitives>,
    pub radius: u32,
    pub offset: Point<i32, Logical>,
    pub color: Argb,
}

/// Everything one surface has committed through this protocol.
///
/// `PartialEq` is what the commit hook compares to decide whether anything
/// actually changed; the shapes compare by contents, which is cheap because
/// they are bounded by [`MAX_PRIMITIVES`](super::shape::MAX_PRIMITIVES).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Effects {
    /// Surface-local pixels, clipped to half the surface's shorter side by the
    /// renderer. Zero leaves the surface square.
    pub corner_radius: u32,
    /// The refractive rim's width in surface-local pixels. Zero draws none.
    pub border_width: u32,
    pub blur: Option<Blur>,
    pub shadow: Option<Shadow>,
}

impl Effects {
    /// Whether there is anything at all to draw. A surface whose client set
    /// effects and then turned every one of them off costs the renderer
    /// nothing beyond this check.
    pub fn is_empty(&self) -> bool {
        self.corner_radius == 0
            && self.border_width == 0
            && self.blur.is_none()
            && self.shadow.is_none()
    }
}

impl Cacheable for Effects {
    fn commit(&mut self, _dh: &DisplayHandle) -> Self {
        self.clone()
    }

    fn merge_into(self, into: &mut Self, _dh: &DisplayHandle) {
        *into = self;
    }
}

/// A surface's committed effects, with the counter that says when they last
/// changed.
#[derive(Debug, Clone, Default)]
pub struct CommittedEffects {
    pub effects: Effects,
    /// Bumped whenever `effects` changes. The renderer folds this into its
    /// elements' commit counters, which is what makes a change repaint rather
    /// than sit there until something else damages the window.
    pub generation: u32,
}

/// Reads a surface's committed effects. `None` when the client never touched
/// this protocol, or set effects and then let the object go.
pub fn surface_effects(states: &SurfaceData) -> Option<CommittedEffects> {
    let committed = states.data_map.get::<CommittedCache>()?;
    let committed = committed.0.lock().ok()?;
    (!committed.effects.is_empty()).then(|| committed.clone())
}

/// Per-surface committed state, in the surface's `data_map` because it has to
/// outlive the effect object that produced it — the object can go away while
/// its effects stay committed until the next commit.
#[derive(Debug, Default)]
pub(super) struct CommittedCache(Mutex<CommittedEffects>);

/// Applies the double-buffered effects, which is the post-commit hook's job.
pub(super) fn apply_committed(states: &SurfaceData) {
    // The cached state only exists once a client has touched this protocol, so
    // an unrelated surface's commit never allocates one.
    if !states.cached_state.has::<Effects>() {
        return;
    }
    let pending = states.cached_state.get::<Effects>().current().clone();

    let cache = states
        .data_map
        .get_or_insert_threadsafe(CommittedCache::default);
    let Ok(mut committed) = cache.0.lock() else {
        return;
    };

    if committed.effects == pending {
        return;
    }
    committed.effects = pending;
    // Wrapping is fine: the renderer only compares for equality, and the
    // effects would have to change four billion times between two frames to
    // land back on the value they started at.
    committed.generation = committed.generation.wrapping_add(1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crownos_background_effects::shape::RoundedRect;
    use smithay::utils::Rectangle;

    fn shape(x: i32) -> Primitives {
        Primitives::from(
            &[RoundedRect::new(
                Rectangle::new((x, 0).into(), (10, 10).into()),
                2,
            )][..],
        )
    }

    fn blur(shape: Option<Primitives>) -> Blur {
        Blur {
            shape,
            radius: 16,
            tint: Argb(0x20_ff_ff_ff),
            saturation: 1.3,
        }
    }

    #[test]
    fn effects_compare_by_the_geometry_they_carry_not_by_identity() {
        // Two separately allocated shapes describing the same geometry are the
        // same effect: a client that re-sets its blur every frame must not
        // repaint the window every frame.
        let one = Effects {
            blur: Some(blur(Some(shape(0)))),
            ..Default::default()
        };
        let same = Effects {
            blur: Some(blur(Some(shape(0)))),
            ..Default::default()
        };
        let moved = Effects {
            blur: Some(blur(Some(shape(5)))),
            ..Default::default()
        };

        assert_eq!(one, same);
        assert_ne!(one, moved);
    }

    #[test]
    fn a_whole_surface_blur_differs_from_a_shaped_one() {
        // `None` is "the whole surface", which is not the same request as a
        // shape that happens to be empty.
        let whole = blur(None);
        let empty = blur(Some(Primitives::from(&[][..])));
        assert_ne!(whole, empty);
    }

    #[test]
    fn nothing_set_is_nothing_to_draw() {
        assert!(Effects::default().is_empty());
        assert!(
            !Effects {
                corner_radius: 8,
                ..Default::default()
            }
            .is_empty()
        );
    }
}
