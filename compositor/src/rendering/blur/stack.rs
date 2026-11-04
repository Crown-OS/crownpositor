//! Which glass on a frame stands in front of which.
//!
//! Glass draws opaque, so where two pieces overlap only the upper one is ever
//! seen — and the lower one painting there anyway is what made the overlap
//! blur twice: it left its own blurred output in the frame for the upper piece
//! to blur again. So the lower piece skips it. What the upper piece then lifts
//! out of the frame is the desktop and whatever the lower surface drew on top
//! of its own glass, unblurred, which it blurs once.
//!
//! Elements are built front to back, so a backdrop is occluded by exactly the
//! ones built before it.

use smithay::utils::{Physical, Rectangle};

use crate::rendering::{decorate::Backdrop, opaque};

/// The opaque glass built so far for one frame.
#[derive(Debug, Default)]
pub struct GlassStack {
    opaque: Vec<Rectangle<i32, Physical>>,
}

impl GlassStack {
    pub(super) fn clear(&mut self) {
        self.opaque.clear();
    }

    /// The glass already standing in front of `backdrop`, clipped to the part
    /// of the output it draws on; and, on the way out, the part of `backdrop`
    /// that in turn stands in front of everything built after it.
    pub(super) fn occlude(
        &mut self,
        backdrop: &Backdrop,
        geometry: Rectangle<i32, Physical>,
    ) -> Vec<Rectangle<i32, Physical>> {
        let occluders = self
            .opaque
            .iter()
            .filter_map(|rect| rect.intersection(geometry))
            .collect();
        self.opaque.extend(opaque_core(backdrop, geometry));
        occluders
    }
}

/// Where a backdrop is opaque enough to hide what is under it. Nothing at all
/// while the glass is fading, because glass that is not opaque is glass
/// whatever is under it still shows through.
pub(super) fn opaque_core(
    backdrop: &Backdrop,
    geometry: Rectangle<i32, Physical>,
) -> impl Iterator<Item = Rectangle<i32, Physical>> {
    let opaque = backdrop.alpha >= 1.0;
    opaque::bands(backdrop.mask, [backdrop.radius; 4])
        .filter(move |_| opaque)
        .filter_map(move |band| band.intersection(geometry))
}

#[cfg(test)]
mod tests {
    use smithay::backend::renderer::{element::Id, utils::CommitCounter};

    use super::*;
    use crate::rendering::blur::Glass;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Physical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    fn backdrop(geometry: Rectangle<i32, Physical>, radius: f32, alpha: f32) -> Backdrop {
        Backdrop {
            id: Id::new(),
            commit: CommitCounter::default(),
            geometry,
            mask: geometry,
            radius,
            glass: Glass::default(),
            alpha,
            strength: 1.0,
        }
    }

    #[test]
    fn the_first_piece_of_glass_is_occluded_by_nothing() {
        let mut stack = GlassStack::default();
        let bar = backdrop(rect(0, 0, 200, 40), 0.0, 1.0);
        assert!(stack.occlude(&bar, bar.geometry).is_empty());
    }

    /// The fix: the bar does not paint glass where the popup's own glass will
    /// stand, so what the popup lifts out of the frame there is the desktop and
    /// the bar's contents, not the blur the bar left behind.
    #[test]
    fn a_piece_under_opaque_glass_gives_up_the_overlap() {
        let mut stack = GlassStack::default();
        let popup = backdrop(rect(50, 20, 60, 100), 0.0, 1.0);
        stack.occlude(&popup, popup.geometry);

        let bar = backdrop(rect(0, 0, 200, 40), 0.0, 1.0);
        assert_eq!(
            stack.occlude(&bar, bar.geometry),
            vec![rect(51, 21, 58, 19)]
        );
    }

    /// Glass mid-fade still shows what is under it, so the piece below has to
    /// keep drawing.
    #[test]
    fn a_piece_under_fading_glass_keeps_the_overlap() {
        let mut stack = GlassStack::default();
        let preview = backdrop(rect(50, 20, 60, 100), 0.0, 0.85);
        stack.occlude(&preview, preview.geometry);

        let bar = backdrop(rect(0, 0, 200, 40), 0.0, 1.0);
        assert!(stack.occlude(&bar, bar.geometry).is_empty());
    }

    /// A rounded piece claims its bands and leaves its corners to whatever
    /// is under it, which is the one place the arcs actually show.
    #[test]
    fn a_rounded_piece_does_not_claim_its_corners() {
        let mut stack = GlassStack::default();
        let popup = backdrop(rect(0, 0, 100, 100), 20.0, 1.0);
        stack.occlude(&popup, popup.geometry);

        let under = backdrop(rect(0, 0, 100, 100), 0.0, 1.0);
        let occluders = stack.occlude(&under, under.geometry);
        // A squircle corner of radius 20 starts curving 30 pixels in.
        assert_eq!(
            occluders,
            vec![
                rect(31, 1, 38, 98),
                rect(1, 31, 30, 38),
                rect(69, 31, 30, 38)
            ]
        );

        // The corner, and the start of its curve, are claimed by neither band.
        assert!(!occluders.iter().any(|rect| rect.contains((4, 4))));
        assert!(!occluders.iter().any(|rect| rect.contains((25, 2))));
    }

    #[test]
    fn a_piece_smaller_than_its_own_curve_claims_nothing() {
        let mut stack = GlassStack::default();
        let dot = backdrop(rect(0, 0, 4, 4), 2.0, 1.0);
        stack.occlude(&dot, dot.geometry);

        let under = backdrop(rect(0, 0, 100, 100), 0.0, 1.0);
        assert!(stack.occlude(&under, under.geometry).is_empty());
    }

    #[test]
    fn a_frame_starts_with_a_clear_stack() {
        let mut stack = GlassStack::default();
        let popup = backdrop(rect(0, 0, 200, 200), 0.0, 1.0);
        stack.occlude(&popup, popup.geometry);
        stack.clear();

        let bar = backdrop(rect(0, 0, 200, 40), 0.0, 1.0);
        assert!(stack.occlude(&bar, bar.geometry).is_empty());
    }
}
