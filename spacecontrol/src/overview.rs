//! How far open the overview is, and everything that follows from it.
//!
//! One spring owns a single number: 0 is the live desktop, 1 is the overview.
//! Every other quantity on screen — how far a window has flown towards its
//! thumbnail, how blurred the wallpaper is, how far the workspace bar has
//! climbed — is read off that number, so they cannot drift out of step with
//! each other.
//!
//! Three fingers going up drive the spring's *target* through the stiff
//! [`SpringProfile::TRACK`], which makes the overview a smoothed copy of the
//! hand rather than a raw one. Letting go is only a retarget, so the speed
//! already on screen carries into the settle and there is no seam between
//! following the fingers and arriving. The same object is therefore both the
//! animation and the gesture; there is no second code path for "animate open"
//! that could look different from swiping open.

use crate::animations::{
    rubber_band::RubberBand,
    spring::{Spring, SpringProfile},
};

/// Seconds of coasting the release speed is projected over to guess where the
/// fingers were heading. Shorter than a workspace flick: this is a commit to
/// one of two states, not a choice among many pages.
const PROJECTION: f64 = 0.35;

/// How far past fully open or fully closed the fingers may pull, and how hard
/// the ends resist.
const BAND: RubberBand = RubberBand::new(0.1, 0.5);

/// Below this the overview is not worth drawing.
const EPSILON: f64 = 1e-3;

/// How dark the wallpaper goes behind the overview, at full open.
const MAX_DIM: f32 = 0.45;

/// How far the wallpaper zooms in behind the overview. Slight: enough to read
/// as depth, not enough to notice as motion.
const BACKGROUND_ZOOM: f64 = 1.08;

/// Progress before the workspace bar starts climbing into place, so the
/// windows are already moving when it appears rather than everything arriving
/// at once.
const BAR_DELAY: f64 = 0.15;

/// The overview's open/closed state, and the gesture that drives it.
#[derive(Debug, Clone, Copy)]
pub struct Overview {
    progress: Spring,
    /// Where the overview has committed to going. The spring is only ever
    /// catching up to this, so input routing never has to read intent out of a
    /// float mid-flight.
    open: bool,
    /// Progress the fingers went down on. Grabbing an overview that is still
    /// flying anchors here, so it does not jump on contact.
    drag: Option<f64>,
    /// `None` disables motion entirely: every change lands on the same frame.
    profile: Option<SpringProfile>,
}

impl Default for Overview {
    fn default() -> Self {
        Self::new()
    }
}

impl Overview {
    pub fn new() -> Self {
        Self {
            progress: Spring::with_profile(0.0, SpringProfile::GESTURE),
            open: false,
            drag: None,
            profile: Some(SpringProfile::GESTURE),
        }
    }

    pub fn set_profile(&mut self, profile: Option<SpringProfile>) {
        self.profile = profile;
        match profile {
            // Mid-drag the spring is on TRACK; the new profile takes over on
            // release.
            Some(profile) if self.drag.is_none() => self.progress.set_profile(profile),
            Some(_) => {}
            // Turning animations off mid-flight lands the overview now rather
            // than leaving it stranded half open.
            None => self.progress.snap_to_target(),
        }
    }

    /// How far open, 0 to 1 — and a little outside that while the fingers pull
    /// against an end.
    pub fn progress(&self) -> f64 {
        f64::from(self.progress.position)
    }

    /// [`Overview::progress`] with the overshoot taken off, for the quantities
    /// that would look wrong past their ends: an alpha above 1, a blur beyond
    /// full.
    pub fn eased(&self) -> f64 {
        self.progress().clamp(0.0, 1.0)
    }

    /// Whether the overview has committed to being open. This is what decides
    /// where a click or a keystroke goes, and it flips once — on the release —
    /// rather than sliding across the halfway mark mid-gesture.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Whether there is anything of the overview on screen to draw.
    pub fn is_visible(&self) -> bool {
        self.open || self.drag.is_some() || self.progress() > EPSILON
    }

    /// Whether the overview still needs frames: fingers are on it, or the
    /// spring has not arrived yet.
    pub fn is_active(&self) -> bool {
        self.drag.is_some() || !self.progress.at_rest()
    }

    pub fn open(&mut self) {
        self.animate_to(true);
    }

    pub fn close(&mut self) {
        self.animate_to(false);
    }

    pub fn toggle(&mut self) {
        self.animate_to(!self.open);
    }

    /// Retargets without disturbing whatever velocity the spring already has,
    /// so a second trigger mid-flight reverses rather than restarts.
    pub fn animate_to(&mut self, open: bool) {
        self.drag = None;
        self.open = open;

        let Some(profile) = self.profile else {
            self.snap_to(open);
            return;
        };
        self.progress.set_profile(profile);
        self.progress.set_target(if open { 1.0 } else { 0.0 });
    }

    pub fn snap_to(&mut self, open: bool) {
        self.drag = None;
        self.open = open;
        self.progress.set_target(if open { 1.0 } else { 0.0 });
        self.progress.snap_to_target();
    }

    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    pub fn begin_gesture(&mut self) {
        if self.profile.is_some() {
            self.progress.set_profile(SpringProfile::TRACK);
            // Whatever the spring was flying towards, the fingers own it now.
            self.progress.set_target(self.progress.position);
        }
        self.drag = Some(self.progress());
    }

    /// Points the overview at the fingers. `travelled` is cumulative and in
    /// units of the whole open-to-closed distance, positive as the fingers go
    /// up — so the compositor divides finger travel by however far it decides a
    /// full swipe is, and nothing here needs to know about touchpads.
    pub fn update_gesture(&mut self, travelled: f64) {
        let Some(origin) = self.drag else {
            return;
        };
        let pinned = BAND.clamp(origin + travelled, 0.0, 1.0) as f32;
        match self.profile {
            Some(_) => self.progress.set_target(pinned),
            None => self.progress.hold(pinned),
        }
    }

    /// Lets go. `velocity` is the fingers' speed in the same units per second,
    /// positive upward. Returns whether the overview is now headed open, which
    /// the caller should treat as the state immediately.
    ///
    /// The velocity only picks the end. The spring is retargeted, not
    /// restarted, so it keeps the speed it already had on screen.
    pub fn end_gesture(&mut self, velocity: f64) -> bool {
        let Some(_) = self.drag.take() else {
            return self.open;
        };

        // Project from where the fingers pinned it, not from the smoothed
        // position still catching up.
        let pinned = f64::from(self.progress.target);
        let open = pinned + velocity * PROJECTION >= 0.5;

        self.animate_to(open);
        open
    }

    /// Abandons a gesture, returning to whichever end the overview was already
    /// committed to.
    pub fn cancel_gesture(&mut self) {
        if self.drag.take().is_some() {
            self.animate_to(self.open);
        }
    }

    pub fn step(&mut self, dt: f32) {
        self.progress.step(dt);
    }

    pub fn settle(&mut self) {
        if self.drag.is_none() {
            self.progress.snap_to_target();
        }
    }

    /// How dark to wash the wallpaper.
    pub fn dim(&self) -> f32 {
        self.eased() as f32 * MAX_DIM
    }

    /// How much of the blur to apply, 0 to 1. The compositor scales its own
    /// radius by this, so the wallpaper comes into and out of focus with the
    /// gesture rather than snapping.
    pub fn blur(&self) -> f32 {
        self.eased() as f32
    }

    /// What to scale the wallpaper by. It creeps towards the viewer as the
    /// windows lift away from it.
    pub fn background_scale(&self) -> f64 {
        1.0 + (BACKGROUND_ZOOM - 1.0) * self.eased()
    }

    /// How far the workspace bar has arrived, 0 to 1 — used for both its climb
    /// from the bottom edge and its fade. It trails the windows so the two
    /// movements read as one thing following another.
    pub fn bar(&self) -> f64 {
        ((self.eased() - BAR_DELAY) / (1.0 - BAR_DELAY)).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Steps at 60 Hz until the overview stops needing frames, then lands it —
    /// the same two-step the render loop performs every frame.
    fn settle(overview: &mut Overview) {
        for _ in 0..600 {
            if !overview.is_active() {
                overview.settle();
                return;
            }
            overview.step(1.0 / 60.0);
        }
        panic!("the overview never came to rest");
    }

    /// Steps while the fingers hold still, so the tracking spring catches up.
    fn follow(overview: &mut Overview, frames: usize) {
        for _ in 0..frames {
            overview.step(1.0 / 60.0);
        }
    }

    #[test]
    fn a_fresh_overview_is_closed_invisible_and_still() {
        let overview = Overview::new();
        assert!(!overview.is_open());
        assert!(!overview.is_visible());
        assert!(!overview.is_active());
        assert_eq!(overview.progress(), 0.0);
    }

    #[test]
    fn opening_animates_rather_than_jumping() {
        let mut overview = Overview::new();
        overview.open();

        assert!(overview.is_open(), "the commit is immediate");
        assert!(overview.progress() < 1.0, "the motion is not");
        assert!(overview.is_active());

        settle(&mut overview);
        assert_eq!(overview.progress(), 1.0);
        assert!(overview.is_visible());
    }

    #[test]
    fn closing_returns_all_the_way_to_the_desktop() {
        let mut overview = Overview::new();
        overview.snap_to(true);
        overview.close();
        settle(&mut overview);

        assert!(!overview.is_open());
        assert!(!overview.is_visible());
        assert_eq!(overview.progress(), 0.0);
    }

    #[test]
    fn toggling_flips_whichever_way_it_was_facing() {
        let mut overview = Overview::new();
        overview.toggle();
        assert!(overview.is_open());
        overview.toggle();
        assert!(!overview.is_open());
    }

    #[test]
    fn a_swipe_up_opens_it_progressively() {
        let mut overview = Overview::new();
        overview.begin_gesture();

        overview.update_gesture(0.4);
        follow(&mut overview, 60);
        assert!(
            (overview.progress() - 0.4).abs() < 0.01,
            "{}",
            overview.progress()
        );
        assert!(overview.is_visible(), "a half-open overview is on screen");
    }

    #[test]
    fn the_overview_smooths_the_fingers_rather_than_copying_them() {
        let mut overview = Overview::new();
        overview.begin_gesture();
        overview.update_gesture(0.5);

        overview.step(1.0 / 60.0);
        let progress = overview.progress();
        assert!(progress > 0.0 && progress < 0.5, "{progress}");
    }

    #[test]
    fn a_swipe_past_halfway_commits_to_opening() {
        let mut overview = Overview::new();
        overview.begin_gesture();
        overview.update_gesture(0.6);
        assert!(overview.end_gesture(0.0));

        settle(&mut overview);
        assert_eq!(overview.progress(), 1.0);
    }

    #[test]
    fn a_short_swipe_with_no_speed_falls_back_closed() {
        let mut overview = Overview::new();
        overview.begin_gesture();
        overview.update_gesture(0.2);
        assert!(!overview.end_gesture(0.0));

        settle(&mut overview);
        assert_eq!(overview.progress(), 0.0);
    }

    #[test]
    fn a_short_but_fast_flick_still_opens_it() {
        // The reason release speed is projected rather than compared against a
        // distance: this swipe is nowhere near halfway.
        let mut overview = Overview::new();
        overview.begin_gesture();
        overview.update_gesture(0.2);
        assert!(overview.end_gesture(1.5));
    }

    #[test]
    fn a_flick_back_down_closes_an_almost_open_overview() {
        let mut overview = Overview::new();
        overview.begin_gesture();
        overview.update_gesture(0.8);
        assert!(!overview.end_gesture(-2.0));
    }

    #[test]
    fn swiping_down_from_open_closes_it() {
        let mut overview = Overview::new();
        overview.snap_to(true);

        overview.begin_gesture();
        overview.update_gesture(-0.7);
        assert!(!overview.end_gesture(0.0));

        settle(&mut overview);
        assert_eq!(overview.progress(), 0.0);
    }

    #[test]
    fn the_ends_resist_instead_of_stopping_dead() {
        let mut overview = Overview::new();
        overview.snap_to(true);
        overview.begin_gesture();
        overview.update_gesture(1.0);
        follow(&mut overview, 60);

        let progress = overview.progress();
        assert!(progress > 1.0, "the end should still give a little");
        assert!(
            progress < 1.0 + BAND.limit,
            "and never past the limit: {progress}"
        );
    }

    #[test]
    fn overshoot_never_reaches_the_look_of_the_thing() {
        let mut overview = Overview::new();
        overview.snap_to(true);
        overview.begin_gesture();
        overview.update_gesture(1.0);
        follow(&mut overview, 60);

        assert_eq!(overview.eased(), 1.0);
        assert_eq!(overview.blur(), 1.0);
        assert_eq!(overview.dim(), MAX_DIM);
        assert_eq!(overview.bar(), 1.0);
    }

    #[test]
    fn a_grab_mid_flight_does_not_jump() {
        let mut overview = Overview::new();
        overview.open();
        follow(&mut overview, 4);

        let caught = overview.progress();
        overview.begin_gesture();
        overview.update_gesture(0.0);
        assert!((overview.progress() - caught).abs() < 1e-6);
    }

    #[test]
    fn the_speed_on_screen_carries_through_the_release() {
        let mut overview = Overview::new();
        overview.begin_gesture();

        let dt = 1.0 / 60.0;
        let (mut travelled, mut during) = (0.0, 0.0);
        for _ in 0..24 {
            travelled += 1.2 * f64::from(dt);
            overview.update_gesture(travelled);
            let before = overview.progress();
            overview.step(dt);
            during = (overview.progress() - before) / f64::from(dt);
        }

        let released_at = overview.progress();
        overview.end_gesture(1.2);
        overview.step(dt);
        let after = (overview.progress() - released_at) / f64::from(dt);

        assert!(during > 0.5, "the swipe never got up to speed: {during}");
        assert!(
            after > during * 0.7,
            "the release jerked: {after} against {during}"
        );
    }

    #[test]
    fn cancelling_returns_to_the_state_it_was_committed_to() {
        let mut overview = Overview::new();
        overview.snap_to(true);
        overview.begin_gesture();
        overview.update_gesture(-0.8);
        overview.cancel_gesture();

        assert!(!overview.is_dragging());
        assert!(overview.is_open());
        settle(&mut overview);
        assert_eq!(overview.progress(), 1.0);
    }

    #[test]
    fn a_dragged_overview_keeps_asking_for_frames_even_while_still() {
        let mut overview = Overview::new();
        overview.begin_gesture();
        overview.update_gesture(0.0);
        assert!(overview.is_active());
    }

    #[test]
    fn the_background_zooms_in_as_it_opens() {
        let mut overview = Overview::new();
        assert_eq!(overview.background_scale(), 1.0);

        overview.snap_to(true);
        assert_eq!(overview.background_scale(), BACKGROUND_ZOOM);
    }

    #[test]
    fn the_bar_trails_the_windows_and_still_arrives() {
        let mut overview = Overview::new();
        overview.begin_gesture();
        overview.update_gesture(BAR_DELAY / 2.0);
        follow(&mut overview, 60);
        assert_eq!(overview.bar(), 0.0, "the bar waits its turn");

        overview.snap_to(true);
        assert_eq!(overview.bar(), 1.0);
    }

    #[test]
    fn every_look_of_the_thing_moves_together() {
        let mut overview = Overview::new();
        overview.begin_gesture();
        overview.update_gesture(0.5);
        follow(&mut overview, 60);

        let progress = overview.eased();
        assert!((f64::from(overview.blur()) - progress).abs() < 1e-6);
        assert!((f64::from(overview.dim()) - progress * f64::from(MAX_DIM)).abs() < 1e-6);
        assert!(overview.background_scale() > 1.0 && overview.background_scale() < BACKGROUND_ZOOM);
    }

    #[test]
    fn disabled_animations_land_immediately() {
        let mut overview = Overview::new();
        overview.set_profile(None);

        overview.open();
        assert_eq!(overview.progress(), 1.0);
        assert!(!overview.is_active());

        overview.begin_gesture();
        overview.update_gesture(-0.6);
        assert!(!overview.end_gesture(0.0));
        assert_eq!(overview.progress(), 0.0, "no frames to animate over");
    }

    #[test]
    fn turning_animations_off_mid_flight_lands_the_overview() {
        let mut overview = Overview::new();
        overview.open();
        overview.step(1.0 / 60.0);
        overview.set_profile(None);

        assert_eq!(overview.progress(), 1.0);
        assert!(!overview.is_active());
    }
}
