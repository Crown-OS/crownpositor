//! Where the viewport sits between workspaces, as a fractional index.
//!
//! One spring owns the value, and it never stops running. While the fingers
//! are down they drive the spring's *target* through the stiff
//! [`SpringProfile::TRACK`], so the viewport is a lightly smoothed copy of the
//! hand rather than a raw one. Letting go is only a retarget onto a whole
//! page: the spring keeps whatever velocity was on screen, so there is no seam
//! between following the fingers and settling. The fingers' own release speed
//! is used once — projected forward to decide *which* page to settle on.
//!
//! Nothing here knows about outputs, tiles or pixels: the caller converts its
//! own geometry to *pages* and back. That is what makes the same object usable
//! for any paged viewport later.

use crate::animations::spring::{Spring, SpringProfile};

/// Seconds of coasting the release velocity is projected over to guess where
/// the fingers were heading, and so how far a flick is worth. Roughly the time
/// constant of a thrown object slowing to a stop.
const PROJECTION: f64 = 0.5;
/// How far past the first or last workspace the fingers may pull, in pages.
const RUBBER_BAND_LIMIT: f64 = 0.35;
/// Resistance at the edge. 1.0 would track the fingers exactly at first.
const RUBBER_BAND_STRENGTH: f64 = 0.5;

/// Logical pixels between two workspaces as they slide past each other, so the
/// pages read as separate surfaces rather than one continuous strip.
pub const PAGE_GAP: i32 = 32;

/// A swipe in progress.
#[derive(Debug, Clone, Copy)]
struct Drag {
    /// Fractional position the fingers went down on. Grabbing a switch that is
    /// still flying anchors here rather than on the nearest workspace, so the
    /// viewport does not jump on contact.
    origin: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct WorkspaceSwitch {
    position: Spring,
    drag: Option<Drag>,
    /// `None` disables motion entirely: every change lands on the same frame.
    profile: Option<SpringProfile>,
}

impl WorkspaceSwitch {
    pub fn new(index: usize) -> Self {
        Self {
            position: Spring::with_profile(index as f32, SpringProfile::GESTURE),
            drag: None,
            profile: Some(SpringProfile::GESTURE),
        }
    }

    pub fn set_profile(&mut self, profile: Option<SpringProfile>) {
        self.profile = profile;
        match profile {
            // Mid-drag the spring is on TRACK; the new profile takes over on
            // release.
            Some(profile) if self.drag.is_none() => self.position.set_profile(profile),
            Some(_) => {}
            // Turning animations off mid-flight lands the switch now rather
            // than leaving it stranded between two workspaces.
            None => self.position.snap_to_target(),
        }
    }

    pub fn position(&self) -> f64 {
        self.position.position as f64
    }

    /// Every workspace with part of itself on screen, and how far off centre,
    /// nearest first — the order both the renderer and hit-testing want.
    ///
    /// At most two can qualify, and `floor` names them, so this allocates
    /// nothing on a path that runs once per output per frame.
    pub fn visible(&self, count: usize) -> impl Iterator<Item = (usize, f64)> {
        let position = self.position();
        let last = count.saturating_sub(1) as f64;
        let lower = position.floor();

        let mut pair = [
            (lower, lower - position),
            (lower + 1.0, lower + 1.0 - position),
        ];
        pair.sort_by(|a, b| a.1.abs().total_cmp(&b.1.abs()));

        pair.into_iter()
            .filter(move |(index, offset)| *index >= 0.0 && *index <= last && offset.abs() < 1.0)
            .map(|(index, offset)| (index as usize, offset))
    }

    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Whether the viewport still needs frames: fingers are on it, or the
    /// spring has not arrived yet.
    pub fn is_active(&self) -> bool {
        self.drag.is_some() || !self.position.at_rest()
    }

    pub fn begin(&mut self) {
        if self.profile.is_some() {
            self.position.set_profile(SpringProfile::TRACK);
            // Whatever the spring was flying towards, the fingers own it now.
            self.position.set_target(self.position.position);
        }
        self.drag = Some(Drag {
            origin: self.position(),
        });
    }

    /// Points the viewport at the fingers. `travelled` is cumulative, in pages,
    /// positive when the fingers moved right — which pulls the *previous*
    /// workspace in, so the content follows the hand instead of opposing it.
    /// The spring does the following, so the motion on screen is a smoothed
    /// copy of the hand.
    pub fn drag_to(&mut self, travelled: f64, last: usize) {
        let Some(drag) = self.drag else {
            return;
        };
        let pinned = resist(drag.origin - travelled, last) as f32;
        match self.profile {
            Some(_) => self.position.set_target(pinned),
            None => self.position.hold(pinned),
        }
    }

    /// Lets go. `velocity` is the fingers' speed in pages per second, positive
    /// rightward; the returned index is the workspace the spring is now headed
    /// for, which the caller should make active immediately.
    ///
    /// The velocity only picks the page. The spring is retargeted, not
    /// restarted, so it carries the speed it already had on screen — which is
    /// what makes the release seamless.
    pub fn release(&mut self, velocity: f64, last: usize) -> usize {
        // The viewport moves against the fingers.
        let velocity = -velocity;
        let Some(drag) = self.drag.take() else {
            return self.nearest(last);
        };

        // Project from where the fingers pinned the viewport, not from the
        // smoothed position still catching up to it.
        let pinned = self.position.target as f64;
        let projected = pinned + velocity * PROJECTION;
        let page = drag.origin.round();
        // One workspace per swipe: a hard flick turns a page, it does not scrub
        // through the whole list.
        let target = projected
            .round()
            .clamp(page - 1.0, page + 1.0)
            .clamp(0.0, last as f64) as usize;

        self.animate_to(target);
        target
    }

    /// Abandons a swipe, returning the viewport to `index` under spring alone.
    pub fn cancel(&mut self, index: usize) {
        if self.drag.take().is_some() {
            self.animate_to(index);
        }
    }

    /// Retargets without disturbing whatever velocity the spring already has,
    /// so a second keystroke mid-flight redirects rather than restarts — and a
    /// released swipe coasts out of the fingers' own speed.
    pub fn animate_to(&mut self, index: usize) {
        self.drag = None;
        let Some(profile) = self.profile else {
            self.snap_to(index);
            return;
        };
        self.position.set_profile(profile);
        self.position.set_target(index as f32);
    }

    pub fn snap_to(&mut self, index: usize) {
        self.drag = None;
        self.position.set_target(index as f32);
        self.position.snap_to_target();
    }

    pub fn step(&mut self, dt: f32) {
        self.position.step(dt);
    }

    pub fn settle(&mut self) {
        if self.drag.is_none() {
            self.position.snap_to_target();
        }
    }

    fn nearest(&self, last: usize) -> usize {
        self.position().round().clamp(0.0, last as f64) as usize
    }
}

/// Squashes a position that has run off either end, so the first and last
/// workspaces resist instead of stopping dead.
fn resist(position: f64, last: usize) -> f64 {
    let last = last as f64;
    if position < 0.0 {
        -band(-position)
    } else if position > last {
        last + band(position - last)
    } else {
        position
    }
}

/// Diminishing returns: the first pixels nearly track the fingers, and no
/// amount of pulling gets past [`RUBBER_BAND_LIMIT`].
fn band(overshoot: f64) -> f64 {
    RUBBER_BAND_LIMIT * (1.0 - 1.0 / (overshoot * RUBBER_BAND_STRENGTH / RUBBER_BAND_LIMIT + 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Steps at 60 Hz until the switch stops needing frames, then lands it —
    /// the same two-step the render loop performs every frame.
    fn settle(switch: &mut WorkspaceSwitch) -> usize {
        for frame in 0..600 {
            if !switch.is_active() {
                switch.settle();
                return frame;
            }
            switch.step(1.0 / 60.0);
        }
        panic!("the switch never came to rest");
    }

    /// Steps at 60 Hz while the fingers hold still, so the tracking spring
    /// catches up to wherever they pinned the viewport.
    fn follow(switch: &mut WorkspaceSwitch, frames: usize) {
        for _ in 0..frames {
            switch.step(1.0 / 60.0);
        }
    }

    #[test]
    fn a_fresh_switch_needs_no_frames() {
        assert!(!WorkspaceSwitch::new(0).is_active());
    }

    #[test]
    fn dragging_moves_the_viewport_against_the_fingers() {
        let mut switch = WorkspaceSwitch::new(1);
        switch.begin();

        // Fingers to the left pull the next workspace in.
        switch.drag_to(-0.4, 3);
        follow(&mut switch, 60);
        assert!((switch.position() - 1.4).abs() < 0.01, "{}", switch.position());

        // And to the right, the previous one.
        switch.drag_to(0.4, 3);
        follow(&mut switch, 60);
        assert!((switch.position() - 0.6).abs() < 0.01, "{}", switch.position());
    }

    #[test]
    fn the_viewport_smooths_the_fingers_rather_than_copying_them() {
        let mut switch = WorkspaceSwitch::new(1);
        switch.begin();
        switch.drag_to(-0.4, 3);

        // The pin moved a full 0.4 pages; the viewport eases after it instead
        // of teleporting.
        switch.step(1.0 / 60.0);
        let position = switch.position();
        assert!(position > 1.0 && position < 1.4, "{position}");
    }

    #[test]
    fn a_half_page_drag_with_no_speed_commits() {
        let mut switch = WorkspaceSwitch::new(0);
        switch.begin();
        switch.drag_to(-0.6, 3);
        assert_eq!(switch.release(0.0, 3), 1);
        settle(&mut switch);
        assert_eq!(switch.position(), 1.0);
    }

    #[test]
    fn a_short_drag_with_no_speed_snaps_back() {
        let mut switch = WorkspaceSwitch::new(2);
        switch.begin();
        switch.drag_to(-0.2, 3);
        assert_eq!(switch.release(0.0, 3), 2);
        settle(&mut switch);
        assert_eq!(switch.position(), 2.0);
    }

    #[test]
    fn a_short_but_fast_flick_still_turns_the_page() {
        // The whole reason release velocity is projected rather than compared
        // against a distance: this drag is nowhere near half a page. One page a
        // second is about as fast as a touchpad flick gets.
        let mut switch = WorkspaceSwitch::new(0);
        switch.begin();
        switch.drag_to(-0.15, 3);
        assert_eq!(switch.release(-1.0, 3), 1);
    }

    #[test]
    fn a_gentle_nudge_is_not_a_flick() {
        let mut switch = WorkspaceSwitch::new(0);
        switch.begin();
        switch.drag_to(-0.15, 3);
        assert_eq!(
            switch.release(-0.3, 3),
            0,
            "drifting to a halt should not turn a page"
        );
    }

    #[test]
    fn a_flick_back_the_way_it_came_returns_home() {
        let mut switch = WorkspaceSwitch::new(1);
        switch.begin();
        switch.drag_to(-0.7, 3);
        // Dragged most of the way, then thrown back.
        assert_eq!(switch.release(1.5, 3), 1);
    }

    #[test]
    fn one_swipe_never_crosses_more_than_one_workspace() {
        let mut switch = WorkspaceSwitch::new(2);
        switch.begin();
        switch.drag_to(-0.9, 9);
        assert_eq!(
            switch.release(-40.0, 9),
            3,
            "an absurd fling is still one page"
        );
    }

    #[test]
    fn the_speed_on_screen_carries_through_the_release() {
        let mut switch = WorkspaceSwitch::new(0);
        switch.begin();

        // Fingers sweep left at a steady 1.8 pages a second; the tracking
        // spring settles onto that speed.
        let dt = 1.0 / 60.0;
        let mut travelled = 0.0;
        let mut during = 0.0;
        for _ in 0..30 {
            travelled -= 1.8 * dt as f64;
            switch.drag_to(travelled, 3);
            let before = switch.position();
            switch.step(dt);
            during = (switch.position() - before) / dt as f64;
        }

        let released_at = switch.position();
        switch.release(-1.8, 3);
        switch.step(dt);
        let after = (switch.position() - released_at) / dt as f64;

        assert!(during > 1.0, "the drag never got up to speed: {during}");
        assert!(
            after > during * 0.7,
            "the release jerked: {after} against {during}"
        );
    }

    #[test]
    fn the_ends_resist_instead_of_stopping_dead() {
        let mut switch = WorkspaceSwitch::new(0);
        switch.begin();
        switch.drag_to(1.0, 3);
        follow(&mut switch, 60);

        let position = switch.position();
        assert!(position < 0.0, "the edge should still give a little");
        assert!(
            position > -RUBBER_BAND_LIMIT,
            "and never past the limit: {position}"
        );

        // Pulling ten times as hard barely gets further.
        switch.drag_to(10.0, 3);
        follow(&mut switch, 60);
        assert!(switch.position() > -RUBBER_BAND_LIMIT);
    }

    #[test]
    fn a_swipe_off_the_end_snaps_back() {
        let mut switch = WorkspaceSwitch::new(3);
        switch.begin();
        switch.drag_to(-1.0, 3);
        assert_eq!(switch.release(-8.0, 3), 3);
        settle(&mut switch);
        assert_eq!(switch.position(), 3.0);
    }

    #[test]
    fn cancelling_returns_to_the_workspace_the_model_still_thinks_is_active() {
        let mut switch = WorkspaceSwitch::new(1);
        switch.begin();
        switch.drag_to(-0.8, 3);
        switch.cancel(1);

        assert!(!switch.is_dragging());
        settle(&mut switch);
        assert_eq!(switch.position(), 1.0);
    }

    #[test]
    fn only_the_two_workspaces_on_screen_are_visible() {
        let mut switch = WorkspaceSwitch::new(0);
        switch.begin();
        switch.drag_to(-1.5, 4);
        follow(&mut switch, 60);

        let visible: Vec<_> = switch.visible(5).collect();
        assert_eq!(visible.len(), 2);
        // Nearest first, so the renderer draws the dominant page on top.
        assert_eq!(visible[0].0, 1);
        assert_eq!(visible[1].0, 2);
        assert!(visible[0].1.abs() <= visible[1].1.abs());
    }

    #[test]
    fn a_settled_switch_shows_exactly_one_workspace() {
        let switch = WorkspaceSwitch::new(2);
        let visible: Vec<_> = switch.visible(5).collect();
        assert_eq!(visible, vec![(2, 0.0)]);
    }

    #[test]
    fn rubber_banding_past_the_first_workspace_shows_no_phantom_page() {
        let mut switch = WorkspaceSwitch::new(0);
        switch.begin();
        switch.drag_to(1.0, 3);
        follow(&mut switch, 30);
        // Position is negative, but there is nothing to the left of zero.
        assert!(switch.position() < 0.0);
        assert!(switch.visible(4).all(|(index, _)| index < 4));
        assert!(switch.visible(4).any(|(index, _)| index == 0));
    }

    #[test]
    fn a_grab_mid_flight_does_not_jump() {
        let mut switch = WorkspaceSwitch::new(0);
        switch.animate_to(1);
        for _ in 0..4 {
            switch.step(1.0 / 60.0);
        }

        let caught = switch.position();
        switch.begin();
        switch.drag_to(0.0, 3);
        assert!((switch.position() - caught).abs() < 1e-6);
    }

    #[test]
    fn disabled_animations_land_immediately() {
        let mut switch = WorkspaceSwitch::new(0);
        switch.set_profile(None);

        switch.animate_to(2);
        assert_eq!(switch.position(), 2.0);
        assert!(!switch.is_active());

        switch.begin();
        switch.drag_to(-0.6, 3);
        assert_eq!(switch.release(-5.0, 3), 3, "one page on from workspace 2");
        assert_eq!(switch.position(), 3.0, "no frames to animate over");
    }

    #[test]
    fn turning_animations_off_mid_flight_lands_the_switch() {
        let mut switch = WorkspaceSwitch::new(0);
        switch.animate_to(1);
        switch.step(1.0 / 60.0);
        switch.set_profile(None);

        assert_eq!(switch.position(), 1.0);
        assert!(!switch.is_active());
    }

    #[test]
    fn a_dragged_switch_keeps_asking_for_frames_even_while_still() {
        let mut switch = WorkspaceSwitch::new(1);
        switch.begin();
        switch.drag_to(0.0, 3);
        assert!(
            switch.is_active(),
            "fingers down means the viewport is live, however still they are"
        );
    }
}
