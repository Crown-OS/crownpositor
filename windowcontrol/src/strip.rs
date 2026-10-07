//! The row of windows and which one is selected.
//!
//! Entries never vanish on the spot: one whose window closed shrinks away
//! under its own spring and is only dropped once it has, and a new window
//! grows in from nothing. The selection skips anything on its way out, and the
//! row's scroll position is renumbered whenever an entry ahead of it comes or
//! goes, so nothing on screen jumps.

use spacecontrol::animations::spring::Spring;

use crate::motion;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Backward,
}

#[derive(Debug, Clone, Copy)]
pub struct Entry<K> {
    pub key: K,
    presence: Spring,
    lift: Spring,
}

impl<K> Entry<K> {
    fn new(key: K, presence: f32) -> Self {
        Self {
            key,
            presence: Spring::with_profile(presence, motion::ENTRY),
            lift: Spring::with_profile(0.0, motion::ENTRY),
        }
    }

    /// 0 while arriving or gone, 1 once fully in the row.
    pub fn presence(&self) -> f64 {
        f64::from(self.presence.position.clamp(0.0, 1.0))
    }

    /// How far the pointer has lifted it, 0 to 1.
    pub fn lift(&self) -> f64 {
        f64::from(self.lift.position.clamp(0.0, 1.0))
    }

    pub fn is_leaving(&self) -> bool {
        self.presence.target == 0.0
    }

    fn springs(&mut self) -> [&mut Spring; 2] {
        [&mut self.presence, &mut self.lift]
    }
}

#[derive(Debug, Clone)]
pub struct Strip<K> {
    entries: Vec<Entry<K>>,
    selected: usize,
    /// The index the row is centred on, fractional while it glides.
    position: Spring,
}

impl<K> Default for Strip<K> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            selected: 0,
            position: Spring::with_profile(0.0, motion::SELECT),
        }
    }
}

impl<K: Copy + Eq> Strip<K> {
    /// Starts over with `keys`, most recent first, already in place. Forward
    /// selects the window before the focused one, Backward the oldest.
    pub fn reset(&mut self, keys: impl IntoIterator<Item = K>, direction: Direction) {
        self.entries.clear();
        self.entries
            .extend(keys.into_iter().map(|key| Entry::new(key, 1.0)));
        let len = self.entries.len();
        self.selected = match direction {
            _ if len < 2 => 0,
            Direction::Forward => 1,
            Direction::Backward => len - 1,
        };
        self.position.hold(self.selected as f32);
    }

    pub fn entries(&self) -> &[Entry<K>] {
        &self.entries
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn selected_key(&self) -> Option<K> {
        self.entries
            .get(self.selected)
            .filter(|entry| !entry.is_leaving())
            .map(|entry| entry.key)
    }

    pub fn position(&self) -> f64 {
        f64::from(self.position.position)
    }

    pub fn has_live(&self) -> bool {
        self.entries.iter().any(|entry| !entry.is_leaving())
    }

    /// Moves the selection one live entry along, wrapping at either end.
    pub fn advance(&mut self, direction: Direction) {
        let len = self.entries.len();
        let step = match direction {
            Direction::Forward => 1,
            Direction::Backward => len.saturating_sub(1),
        };
        let mut index = self.selected;
        for _ in 0..len {
            index = (index + step) % len;
            if !self.entries[index].is_leaving() {
                self.select(index);
                return;
            }
        }
    }

    pub fn select(&mut self, index: usize) {
        if self
            .entries
            .get(index)
            .is_some_and(|entry| !entry.is_leaving())
        {
            self.selected = index;
            self.position.set_target(index as f32);
        }
    }

    /// Drags the row by `entries` with the fingers on it; the selection follows
    /// whichever entry ends up centred.
    pub fn scroll(&mut self, entries: f64) {
        let last = self.entries.len().saturating_sub(1) as f32;
        self.position
            .hold((self.position.position + entries as f32).clamp(0.0, last));
        if let Some(nearest) = self.nearest_live(self.position.position.round() as usize) {
            self.selected = nearest;
        }
    }

    /// Lets go of the row: it glides onto the selection.
    pub fn release(&mut self) {
        self.position.set_target(self.selected as f32);
    }

    pub fn hover(&mut self, hovered: Option<usize>) {
        for (index, entry) in self.entries.iter_mut().enumerate() {
            entry
                .lift
                .set_target(f32::from(u8::from(hovered == Some(index))));
        }
    }

    /// Brings the row in line with the windows that exist now, `keys` most
    /// recent first: new ones grow in at the front, missing ones start leaving.
    pub fn sync<I>(&mut self, keys: I)
    where
        I: IntoIterator<Item = K>,
        I::IntoIter: Clone,
    {
        let keys = keys.into_iter();
        for entry in &mut self.entries {
            let present = keys.clone().any(|key| key == entry.key);
            entry.presence.set_target(f32::from(u8::from(present)));
        }
        for key in keys {
            if self.entries.iter().all(|entry| entry.key != key) {
                self.entries.insert(0, Entry::new(key, 0.0));
                self.selected += 1;
                self.position.shift(1.0);
            }
        }
        if self
            .entries
            .get(self.selected)
            .is_some_and(Entry::is_leaving)
            && let Some(nearest) = self.nearest_live(self.selected)
        {
            self.select(nearest);
        }
    }

    /// The live entry closest to `index`, preferring the one after it.
    fn nearest_live(&self, index: usize) -> Option<usize> {
        let live = |at: usize| {
            self.entries
                .get(at)
                .is_some_and(|entry| !entry.is_leaving())
        };
        (0..self.entries.len()).find_map(|distance| {
            [index + distance, index.wrapping_sub(distance)]
                .into_iter()
                .find(|&at| live(at))
        })
    }

    pub fn step(&mut self, dt: f32) {
        self.position.step(dt);
        for entry in &mut self.entries {
            entry
                .springs()
                .into_iter()
                .for_each(|spring| spring.step(dt));
        }
        self.prune();
    }

    pub fn at_rest(&self) -> bool {
        self.position.at_rest()
            && self
                .entries
                .iter()
                .all(|entry| entry.presence.at_rest() && entry.lift.at_rest())
    }

    pub fn settle(&mut self) {
        self.position.snap_to_target();
        for entry in &mut self.entries {
            entry.springs().into_iter().for_each(Spring::snap_to_target);
        }
        self.prune();
    }

    /// Drops entries that have finished leaving, renumbering everything after.
    fn prune(&mut self) {
        let mut index = 0;
        while index < self.entries.len() {
            let entry = &self.entries[index];
            if entry.is_leaving() && entry.presence.at_rest() {
                self.entries.remove(index);
                if index < self.selected {
                    self.selected -= 1;
                    self.position.shift(-1.0);
                }
            } else {
                index += 1;
            }
        }
        self.selected = self.selected.min(self.entries.len().saturating_sub(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip(keys: &[u32], direction: Direction) -> Strip<u32> {
        let mut strip = Strip::default();
        strip.reset(keys.iter().copied(), direction);
        strip
    }

    fn settled(strip: &mut Strip<u32>) {
        for _ in 0..600 {
            if strip.at_rest() {
                break;
            }
            strip.step(1.0 / 60.0);
        }
        strip.settle();
    }

    #[test]
    fn forward_opens_on_the_previous_window() {
        assert_eq!(
            strip(&[1, 2, 3], Direction::Forward).selected_key(),
            Some(2)
        );
    }

    #[test]
    fn backward_opens_on_the_oldest_window() {
        assert_eq!(
            strip(&[1, 2, 3], Direction::Backward).selected_key(),
            Some(3)
        );
    }

    #[test]
    fn a_lone_window_is_selected_either_way() {
        assert_eq!(strip(&[7], Direction::Forward).selected_key(), Some(7));
    }

    #[test]
    fn the_selection_wraps_at_both_ends() {
        let mut strip = strip(&[1, 2, 3], Direction::Backward);
        strip.advance(Direction::Forward);
        assert_eq!(strip.selected_key(), Some(1));
        strip.advance(Direction::Backward);
        assert_eq!(strip.selected_key(), Some(3));
    }

    #[test]
    fn a_new_window_grows_in_at_the_front_without_moving_the_selection() {
        let mut strip = strip(&[1, 2], Direction::Forward);
        strip.sync([9, 1, 2]);

        assert_eq!(strip.entries()[0].key, 9);
        assert_eq!(strip.entries()[0].presence(), 0.0);
        assert_eq!(strip.selected_key(), Some(2));
        assert_eq!(strip.position(), 2.0, "the row jumped under the selection");

        settled(&mut strip);
        assert_eq!(strip.entries()[0].presence(), 1.0);
    }

    #[test]
    fn a_closed_window_collapses_before_it_is_dropped() {
        let mut strip = strip(&[1, 2, 3], Direction::Backward);
        strip.sync([2, 3]);

        assert_eq!(strip.entries().len(), 3, "dropped before collapsing");
        assert!(strip.entries()[0].is_leaving());

        settled(&mut strip);
        assert_eq!(strip.entries().len(), 2);
        assert_eq!(strip.selected_key(), Some(3));
        assert_eq!(strip.position(), 1.0);
    }

    #[test]
    fn closing_the_selected_window_moves_the_selection_on() {
        let mut strip = strip(&[1, 2, 3], Direction::Forward);
        strip.sync([1, 3]);
        assert_eq!(strip.selected_key(), Some(3));
    }

    #[test]
    fn the_selection_skips_a_leaving_entry() {
        let mut strip = strip(&[1, 2, 3], Direction::Backward);
        strip.sync([1, 3]);
        strip.advance(Direction::Backward);
        assert_eq!(strip.selected_key(), Some(1));
    }

    #[test]
    fn scrolling_selects_whatever_lands_in_the_middle() {
        let mut strip = strip(&[1, 2, 3, 4], Direction::Forward);
        strip.scroll(1.6);
        assert_eq!(strip.selected_key(), Some(4));
        strip.scroll(-9.0);
        assert_eq!(strip.position(), 0.0);
        assert_eq!(strip.selected_key(), Some(1));
    }
}
