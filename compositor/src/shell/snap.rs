//! The preview that shows where a dragged window would land.
//!
//! Which zone the pointer is in is [`SnapZone`]'s job and pure geometry; this
//! is the part that has to be on screen and has to move smoothly. The springs
//! are the same ones a window uses ([`TileAnim`]), so the preview glides from
//! one zone to the next and fades away when the pointer leaves them all,
//! instead of blinking between rectangles.

use smithay::utils::{Logical, Point, Rectangle};

use crate::{
    layout::{SnapBounds, SnapZone},
    shell::{decoration::DecorationIds, tile::TileAnim},
    utils::id::{OutputId, WindowId},
};

/// A drag hovering over a snap zone.
#[derive(Debug)]
pub struct SnapPreview {
    /// The window being dragged. A second drag replaces the preview rather than
    /// adding one.
    window: WindowId,
    output: OutputId,
    zone: SnapZone,
    anim: TileAnim,
    /// Kept for the preview's lifetime so moving it between zones is one
    /// element travelling, not a new one appearing every time.
    ids: DecorationIds,
    /// The pointer has left every zone, so the preview is on its way out. It
    /// stays alive until the fade finishes, which is what makes leaving a zone
    /// look like the reverse of entering one.
    leaving: bool,
}

impl SnapPreview {
    fn new(
        window: WindowId,
        output: OutputId,
        zone: SnapZone,
        rect: Rectangle<i32, Logical>,
    ) -> Self {
        let mut anim = TileAnim::new(rect);
        anim.fade_in();
        Self {
            window,
            output,
            zone,
            anim,
            ids: DecorationIds::default(),
            leaving: false,
        }
    }

    pub fn window(&self) -> WindowId {
        self.window
    }

    pub fn output(&self) -> OutputId {
        self.output
    }

    pub fn ids(&self) -> &DecorationIds {
        &self.ids
    }

    /// `None` once the pointer has left every zone — releasing there should drop
    /// the window where it lies, not snap it to the last edge it passed.
    pub fn zone(&self) -> Option<SnapZone> {
        (!self.leaving).then_some(self.zone)
    }

    /// Where to draw it this frame: the interpolated rect, not the target.
    pub fn render_rect(&self) -> Rectangle<f64, Logical> {
        self.anim.rect()
    }

    pub fn alpha(&self) -> f32 {
        self.anim.alpha()
    }

    pub fn step(&mut self, dt: f32) {
        self.anim.step(dt);
    }

    /// Whether it still needs frames.
    pub fn is_animating(&self) -> bool {
        !self.anim.at_rest()
    }

    /// Whether it has finished fading out and can be dropped.
    pub fn is_spent(&self) -> bool {
        self.leaving && self.anim.at_rest()
    }

    fn retarget(&mut self, zone: SnapZone, rect: Rectangle<i32, Logical>) {
        self.zone = zone;
        self.leaving = false;
        self.anim.retarget(rect);
        self.anim.fade_in();
    }

    fn dismiss(&mut self) {
        self.leaving = true;
        self.anim.fade_out();
    }
}

/// Every preview the compositor has on screen, which is at most one.
///
/// A slot rather than an `Option` on its own so the "which window, which
/// output, has the zone changed" bookkeeping lives in one place instead of at
/// each call site in the grab.
#[derive(Debug, Default)]
pub struct SnapPreviews {
    current: Option<SnapPreview>,
}

impl SnapPreviews {
    pub fn get(&self) -> Option<&SnapPreview> {
        self.current.as_ref()
    }

    /// Points the preview at whatever zone the pointer is in, or starts it
    /// fading out if there is none. Returns whether anything changed, which is
    /// what the caller turns into a frame.
    pub fn track(
        &mut self,
        window: WindowId,
        output: OutputId,
        pointer: Point<f64, Logical>,
        bounds: SnapBounds,
    ) -> bool {
        let zone = bounds.zone_at(pointer);

        match (&mut self.current, zone) {
            (Some(preview), Some(zone)) if preview.window == window => {
                if preview.zone == zone && !preview.leaving {
                    return false;
                }
                preview.retarget(zone, bounds.rect(zone));
                true
            }
            (Some(preview), None) if preview.window == window => {
                if preview.leaving {
                    return false;
                }
                preview.dismiss();
                true
            }
            (_, Some(zone)) => {
                self.current = Some(SnapPreview::new(window, output, zone, bounds.rect(zone)));
                true
            }
            (_, None) => self.clear(),
        }
    }

    /// Ends the drag, naming the zone the window should land in.
    pub fn release(&mut self) -> Option<SnapZone> {
        self.current.take().and_then(|preview| preview.zone())
    }

    pub fn clear(&mut self) -> bool {
        self.current.take().is_some()
    }

    pub fn step(&mut self, dt: f32) {
        if let Some(preview) = &mut self.current {
            preview.step(dt);
            if preview.is_spent() {
                self.current = None;
            }
        }
    }

    pub fn is_animating(&self) -> bool {
        self.current.as_ref().is_some_and(SnapPreview::is_animating)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Gaps, testing::area};

    fn bounds() -> SnapBounds {
        SnapBounds {
            output: area(1000, 800),
            area: area(1000, 800),
            gaps: Gaps { inner: 8, outer: 8 },
        }
    }

    fn track(previews: &mut SnapPreviews, window: WindowId, x: f64, y: f64) -> bool {
        previews.track(window, OutputId::next(), (x, y).into(), bounds())
    }

    #[test]
    fn nothing_is_previewed_away_from_the_edges() {
        let mut previews = SnapPreviews::default();
        assert!(!track(&mut previews, WindowId::next(), 500.0, 400.0));
        assert!(previews.get().is_none());
    }

    #[test]
    fn dragging_onto_an_edge_starts_a_preview() {
        let mut previews = SnapPreviews::default();
        let id = WindowId::next();

        assert!(track(&mut previews, id, 2.0, 400.0));
        assert_eq!(previews.get().unwrap().zone(), Some(SnapZone::LeftHalf));
    }

    #[test]
    fn staying_in_one_zone_reports_no_change() {
        let mut previews = SnapPreviews::default();
        let id = WindowId::next();

        track(&mut previews, id, 2.0, 400.0);
        assert!(
            !track(&mut previews, id, 4.0, 380.0),
            "the same zone must not repaint on every motion event"
        );
    }

    /// The rect is retargeted rather than replaced, so the springs carry the
    /// preview across instead of it blinking from one half to the other.
    #[test]
    fn crossing_to_another_zone_moves_the_same_preview() {
        let mut previews = SnapPreviews::default();
        let id = WindowId::next();

        track(&mut previews, id, 2.0, 400.0);
        let before = previews.get().unwrap().render_rect();

        assert!(track(&mut previews, id, 998.0, 400.0));
        assert_eq!(previews.get().unwrap().zone(), Some(SnapZone::RightHalf));
        assert_eq!(
            previews.get().unwrap().render_rect(),
            before,
            "it has not moved yet — the spring has to carry it"
        );
    }

    #[test]
    fn leaving_every_zone_fades_the_preview_out_before_dropping_it() {
        let mut previews = SnapPreviews::default();
        let id = WindowId::next();

        track(&mut previews, id, 2.0, 400.0);
        for _ in 0..120 {
            previews.step(1.0 / 60.0);
        }

        assert!(track(&mut previews, id, 500.0, 400.0));
        assert!(previews.get().is_some(), "still on screen, on its way out");
        assert_eq!(previews.get().unwrap().zone(), None, "no longer snappable");

        for _ in 0..240 {
            previews.step(1.0 / 60.0);
        }
        assert!(previews.get().is_none(), "gone once it finished fading");
    }

    #[test]
    fn releasing_names_the_zone_and_clears_the_preview() {
        let mut previews = SnapPreviews::default();
        let id = WindowId::next();

        track(&mut previews, id, 999.0, 1.0);
        assert_eq!(previews.release(), Some(SnapZone::TopRight));
        assert!(previews.get().is_none());
    }

    #[test]
    fn releasing_outside_every_zone_snaps_nothing() {
        let mut previews = SnapPreviews::default();
        let id = WindowId::next();

        track(&mut previews, id, 2.0, 400.0);
        track(&mut previews, id, 500.0, 400.0);
        assert_eq!(previews.release(), None);
    }

    /// A second window picking up the drag takes the preview over rather than
    /// leaving the first one's on screen.
    #[test]
    fn another_window_replaces_the_preview() {
        let mut previews = SnapPreviews::default();
        let first = WindowId::next();
        let second = WindowId::next();

        track(&mut previews, first, 2.0, 400.0);
        track(&mut previews, second, 998.0, 400.0);

        assert_eq!(previews.get().unwrap().window(), second);
        assert_eq!(previews.get().unwrap().zone(), Some(SnapZone::RightHalf));
    }
}
