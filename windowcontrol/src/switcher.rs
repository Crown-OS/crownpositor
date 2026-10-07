//! The switcher's state machine: open, browsing, then either committed to a
//! window or dismissed. Both endings sink the strip on the same spring; a
//! commit also remembers which window to zoom forward while it does.

use spacecontrol::animations::spring::{Spring, SpringProfile};

use crate::{
    motion,
    strip::{Direction, Strip},
};

const VISIBLE: f32 = 1e-3;

#[derive(Debug, Clone)]
pub struct Switcher<K> {
    reveal: Spring,
    open: bool,
    strip: Strip<K>,
    committed: Option<K>,
    motion: bool,
}

impl<K> Default for Switcher<K> {
    fn default() -> Self {
        Self {
            reveal: Spring::with_profile(0.0, motion::RISE),
            open: false,
            strip: Strip::default(),
            committed: None,
            motion: true,
        }
    }
}

impl<K: Copy + Eq> Switcher<K> {
    /// Rises with `keys`, most recent first. Nothing to switch between keeps it
    /// closed.
    pub fn open(&mut self, keys: impl IntoIterator<Item = K>, direction: Direction) -> bool {
        self.strip.reset(keys, direction);
        if !self.strip.has_live() {
            return false;
        }
        self.open = true;
        self.committed = None;
        self.animate(motion::RISE, 1.0);
        true
    }

    pub fn advance(&mut self, direction: Direction) {
        if self.open {
            self.strip.advance(direction);
            self.land();
        }
    }

    /// Sinks the strip and hands back the selected window to focus.
    pub fn commit(&mut self) -> Option<K> {
        if !self.open {
            return None;
        }
        self.committed = self.strip.selected_key();
        self.close();
        self.committed
    }

    pub fn commit_at(&mut self, index: usize) -> Option<K> {
        self.strip.select(index);
        self.commit()
    }

    /// Sinks the strip and leaves focus where it was.
    pub fn dismiss(&mut self) {
        if self.open {
            self.committed = None;
            self.close();
        }
    }

    pub fn scroll(&mut self, entries: f64) {
        if self.open {
            self.strip.scroll(entries);
        }
    }

    pub fn release_scroll(&mut self) {
        self.strip.release();
        self.land();
    }

    pub fn hover(&mut self, hovered: Option<usize>) {
        self.strip.hover(hovered);
        self.land();
    }

    /// Follows windows opening and closing while the strip is up; the last one
    /// closing dismisses it.
    pub fn sync<I>(&mut self, keys: I)
    where
        I: IntoIterator<Item = K>,
        I::IntoIter: Clone,
    {
        self.strip.sync(keys);
        if self.open && !self.strip.has_live() {
            self.dismiss();
        }
        self.land();
    }

    fn close(&mut self) {
        self.open = false;
        self.strip.hover(None);
        self.animate(motion::SINK, 0.0);
    }

    fn animate(&mut self, profile: SpringProfile, target: f32) {
        self.reveal.set_profile(profile);
        self.reveal.set_target(target);
        self.land();
    }

    /// With motion off, every change lands on the frame it happens.
    fn land(&mut self) {
        if !self.motion {
            self.settle();
        }
    }

    pub fn strip(&self) -> &Strip<K> {
        &self.strip
    }

    /// The window zooming forward, until the strip is gone.
    pub fn committed(&self) -> Option<K> {
        self.committed
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn is_visible(&self) -> bool {
        self.open || self.reveal.position > VISIBLE
    }

    pub fn is_active(&self) -> bool {
        self.is_visible() && !(self.reveal.at_rest() && self.strip.at_rest())
    }

    /// How far the strip has risen, overshoot included.
    pub fn progress(&self) -> f64 {
        f64::from(self.reveal.position)
    }

    pub fn eased(&self) -> f64 {
        self.progress().clamp(0.0, 1.0)
    }

    pub fn step(&mut self, dt: f32) {
        self.reveal.step(dt);
        self.strip.step(dt);
        self.forget_when_gone();
    }

    pub fn settle(&mut self) {
        self.reveal.snap_to_target();
        self.strip.settle();
        self.forget_when_gone();
    }

    pub fn set_profile(&mut self, profile: Option<SpringProfile>) {
        self.motion = profile.is_some();
        self.land();
    }

    fn forget_when_gone(&mut self) {
        if !self.is_visible() {
            self.committed = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settled(switcher: &mut Switcher<u32>) {
        for _ in 0..600 {
            if !switcher.is_active() {
                break;
            }
            switcher.step(1.0 / 60.0);
        }
        switcher.settle();
    }

    #[test]
    fn nothing_to_switch_between_stays_closed() {
        let mut switcher = Switcher::<u32>::default();
        assert!(!switcher.open([], Direction::Forward));
        assert!(!switcher.is_visible());
    }

    #[test]
    fn the_strip_overshoots_on_the_way_up() {
        let mut switcher = Switcher::default();
        switcher.open([1, 2], Direction::Forward);
        let mut highest = 0.0f64;
        for _ in 0..120 {
            switcher.step(1.0 / 60.0);
            highest = highest.max(switcher.progress());
        }
        assert!(highest > 1.0, "no bounce: {highest}");
        assert!(highest < 1.05, "too much bounce: {highest}");
    }

    #[test]
    fn committing_hands_back_the_selection_and_zooms_it_until_gone() {
        let mut switcher = Switcher::default();
        switcher.open([1, 2, 3], Direction::Forward);
        settled(&mut switcher);
        switcher.advance(Direction::Forward);

        assert_eq!(switcher.commit(), Some(3));
        assert!(!switcher.is_open());
        assert_eq!(switcher.committed(), Some(3));

        settled(&mut switcher);
        assert!(!switcher.is_visible());
        assert_eq!(switcher.committed(), None);
    }

    #[test]
    fn dismissing_commits_nothing() {
        let mut switcher = Switcher::default();
        switcher.open([1, 2], Direction::Forward);
        switcher.dismiss();
        assert_eq!(switcher.committed(), None);
        assert_eq!(switcher.commit(), None);
    }

    #[test]
    fn the_last_window_closing_dismisses_the_strip() {
        let mut switcher = Switcher::default();
        switcher.open([1], Direction::Forward);
        switcher.sync([]);
        assert!(!switcher.is_open());
    }

    #[test]
    fn without_motion_it_appears_and_vanishes_at_once() {
        let mut switcher = Switcher::default();
        switcher.set_profile(None);
        switcher.open([1, 2], Direction::Forward);
        assert_eq!(switcher.progress(), 1.0);
        switcher.dismiss();
        assert!(!switcher.is_visible());
    }
}
