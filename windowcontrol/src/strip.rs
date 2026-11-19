//! The row of windows and which one is selected.
//!
//! Entries never vanish on the spot: one whose window closed shrinks away
//! under its own spring and is only dropped once it has, and a new window
//! grows in from nothing. The selection skips anything on its way out, and the
//! row's scroll position is renumbered whenever an entry ahead of it comes or
//! goes, so nothing on screen jumps.

use spacecontrol::animations::spring::Spring;

use crate::motion;

/// From this many entries the row is a ring: stepping past either end is one
/// step like any other, not a slide back across every card.
const RING: usize = 3;

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
    focus: Spring,
}

impl<K> Entry<K> {
    fn new(key: K, presence: f32) -> Self {
        Self {
            key,
            presence: Spring::with_profile(presence, motion::ENTRY),
            lift: Spring::with_profile(0.0, motion::ENTRY),
            focus: Spring::with_profile(0.0, motion::FOCUS),
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

    /// How far it has come forward as the selection, 0 to 1.
    pub fn focus(&self) -> f64 {
        f64::from(self.focus.position.clamp(0.0, 1.0))
    }

    pub fn is_leaving(&self) -> bool {
        self.presence.target == 0.0
    }

    fn springs(&mut self) -> [&mut Spring; 3] {
        [&mut self.presence, &mut self.lift, &mut self.focus]
    }
}

#[derive(Debug, Clone)]
pub struct Strip<K> {
    entries: Vec<Entry<K>>,
    selected: usize,
    /// The index of the card on top, fractional while it glides.
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
        self.focus_selected();
        if let Some(entry) = self.entries.get_mut(self.selected) {
            entry.focus.hold(1.0);
        }
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

    /// Places `index` sits behind the top card, negative to its left. Round a
    /// ring that is between [`Self::ring_ends`].
    pub fn depth(&self, index: usize) -> f64 {
        let offset = index as f64 - self.position();
        match self.ring_ends() {
            Some((start, end)) => start + (offset - start).rem_euclid(end - start),
            None => offset,
        }
    }

    /// The depths where a card crosses from one side of the stack to the
    /// other: half a place clear of every card at rest, the odd one of an
    /// even ring going right. `None` while the row has two ends.
    pub fn ring_ends(&self) -> Option<(f64, f64)> {
        let len = self.entries.len();
        let start = -((len.saturating_sub(1) / 2) as f64 + 0.5);
        self.is_ring().then_some((start, start + len as f64))
    }

    /// The two entries the top of the stack is between, and how far it has
    /// come from the first towards the second.
    pub fn between(&self) -> (usize, usize, f64) {
        let len = self.entries.len();
        let position = self.position();
        if self.is_ring() {
            let below = (position.floor() as i64).rem_euclid(len as i64) as usize;
            return (below, (below + 1) % len, position - position.floor());
        }
        let last = len.saturating_sub(1);
        let below = (position.max(0.0).floor() as usize).min(last);
        let blend = (position - below as f64).clamp(0.0, 1.0);
        (below, (below + 1).min(last), blend)
    }

    /// Whether the stack is gliding towards earlier entries.
    pub fn is_moving_back(&self) -> bool {
        self.position.target < self.position.position
    }

    fn is_ring(&self) -> bool {
        self.entries.len() >= RING
    }

    fn shortest(&self, offset: f64) -> f64 {
        match self.is_ring() {
            true => {
                let len = self.entries.len() as f64;
                offset - len * (offset / len).round()
            }
            false => offset,
        }
    }

    /// Glides to `index` the short way round, renumbering the position so
    /// the target is `index` itself.
    fn aim(&mut self, index: usize) {
        let target = index as f32;
        let from = self.position.target;
        let arrival = from + self.shortest(f64::from(target - from)) as f32;
        self.position.set_target(arrival);
        self.position.shift(target - arrival);
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
            self.aim(index);
            self.focus_selected();
        }
    }

    pub fn select_key(&mut self, key: K) {
        if let Some(index) = self.entries.iter().position(|entry| entry.key == key) {
            self.select(index);
        }
    }

    fn focus_selected(&mut self) {
        let selected = self.selected;
        for (index, entry) in self.entries.iter_mut().enumerate() {
            entry
                .focus
                .set_target(f32::from(u8::from(index == selected)));
        }
    }

    /// Drags the row by `entries` with the fingers on it; the selection follows
    /// whichever entry ends up centred.
    pub fn scroll(&mut self, entries: f64) {
        let len = self.entries.len().max(1);
        let moved = self.position.position + entries as f32;
        self.position.hold(match self.is_ring() {
            true => moved.rem_euclid(len as f32),
            false => moved.clamp(0.0, (len - 1) as f32),
        });
        let centred = self.position.position.round() as usize % len;
        if let Some(nearest) = self.nearest_live(centred) {
            self.selected = nearest;
            self.focus_selected();
        }
    }

    /// Lets go of the row: it glides onto the selection.
    pub fn release(&mut self) {
        self.aim(self.selected);
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
            && self.entries.iter().all(|entry| {
                [&entry.presence, &entry.lift, &entry.focus]
                    .into_iter()
                    .all(Spring::at_rest)
            })
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
        strip.scroll(1.4);
        assert_eq!(strip.position(), 0.0, "a ring scrolls round");
        assert_eq!(strip.selected_key(), Some(1));
    }

    #[test]
    fn a_row_of_two_stops_scrolling_at_its_ends() {
        let mut strip = strip(&[1, 2], Direction::Forward);
        strip.scroll(-9.0);
        assert_eq!(strip.position(), 0.0);
        assert_eq!(strip.selected_key(), Some(1));
    }

    #[test]
    fn wrapping_round_the_ring_is_one_step() {
        let mut strip = strip(&[1, 2, 3, 4], Direction::Backward);
        strip.advance(Direction::Forward);
        assert_eq!(strip.selected_key(), Some(1));
        assert_eq!(strip.position(), -1.0, "slides back across the stack");
        assert_eq!(strip.depth(0), 1.0);

        settled(&mut strip);
        assert_eq!(strip.position(), 0.0);
        assert_eq!(strip.depth(3), -1.0);
    }

    #[test]
    fn stepping_back_past_the_start_comes_round_from_the_left() {
        let mut strip = strip(&[1, 2, 3, 4, 5], Direction::Forward);
        strip.advance(Direction::Backward);
        strip.advance(Direction::Backward);
        assert_eq!(strip.selected_key(), Some(5));
        assert_eq!(strip.depth(4), -2.0);
        assert!(strip.is_moving_back());
    }

    #[test]
    fn a_row_of_two_keeps_its_ends() {
        let mut strip = strip(&[1, 2], Direction::Forward);
        strip.advance(Direction::Forward);
        assert_eq!(strip.selected_key(), Some(1));
        assert_eq!(strip.depth(0), -1.0);
        assert_eq!(strip.ring_ends(), None);
    }

    #[test]
    fn only_the_selection_comes_forward() {
        let mut strip = strip(&[1, 2, 3], Direction::Forward);
        assert_eq!(strip.entries()[1].focus(), 1.0, "opened mid-glide");
        strip.advance(Direction::Forward);
        settled(&mut strip);
        let focus: Vec<_> = strip.entries().iter().map(Entry::focus).collect();
        assert_eq!(focus, [0.0, 0.0, 1.0]);
    }
}
