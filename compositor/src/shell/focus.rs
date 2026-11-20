//! The two monitors focus is about: the one under the pointer, which
//! monitor-scoped actions use, and the one whose active workspace holds the
//! keyboard, which window actions use. Both only ever name a shown monitor.

use smithay::{
    output::Output,
    utils::{Logical, Point},
};

use crate::{
    layout::Direction,
    shell::{Shell, monitor::Monitor},
    utils::id::{OutputId, WindowId},
};

impl Shell {
    /// The monitor under the pointer.
    pub fn focused_monitor(&self) -> Option<&Monitor> {
        self.monitor_by_id(self.focused?)
    }

    pub fn focused_output(&self) -> Option<&Output> {
        self.focused_monitor().map(Monitor::output)
    }

    /// The monitor whose active workspace holds the keyboard.
    pub fn keyboard_monitor(&self) -> Option<&Monitor> {
        self.monitor_by_id(self.keyboard?)
    }

    pub fn keyboard_output(&self) -> Option<&Output> {
        self.keyboard_monitor().map(Monitor::output)
    }

    /// The window the keyboard is on, as the model sees it.
    pub fn focused_window_id(&self) -> Option<WindowId> {
        self.keyboard_monitor()?.active().focus()
    }

    /// Follows the pointer. Returns whether the focused monitor changed.
    pub fn focus_at(&mut self, location: Point<f64, Logical>) -> bool {
        let Some(id) = self.monitor_at(location).map(Monitor::id) else {
            return false;
        };
        let changed = self.focused != Some(id);
        self.focused = Some(id);
        changed
    }

    /// Hands the keyboard to a monitor's active workspace. Refused for a
    /// hidden or unknown monitor.
    pub fn focus_keyboard_on(&mut self, output: OutputId) -> bool {
        if !self.is_shown(output) {
            return false;
        }
        self.keyboard = Some(output);
        true
    }

    /// Moves the keyboard to the monitor under the pointer, as an explicit
    /// workspace switch does.
    pub fn focus_keyboard_on_focused(&mut self) -> bool {
        self.focused
            .is_some_and(|focused| self.focus_keyboard_on(focused))
    }

    /// The shown monitor next to the keyboard's in `dir`.
    pub fn output_in_direction(&self, dir: Direction) -> Option<OutputId> {
        self.monitor_in_direction(self.keyboard.or(self.focused)?, dir)
    }

    /// Gives a first shown monitor both kinds of focus.
    pub(super) fn seed_focus(&mut self, output: OutputId) {
        if !self.is_shown(output) {
            return;
        }
        self.focused.get_or_insert(output);
        self.keyboard.get_or_insert(output);
    }

    /// Points focus that named a removed monitor at the survivor.
    pub(super) fn hand_focus_over(&mut self, removed: OutputId, survivor: Option<OutputId>) {
        for focus in [&mut self.focused, &mut self.keyboard] {
            if *focus == Some(removed) {
                *focus = survivor;
            }
        }
    }

    fn is_shown(&self, output: OutputId) -> bool {
        self.monitor_by_id(output)
            .is_some_and(|monitor| !monitor.is_hidden())
    }

    #[cfg(debug_assertions)]
    pub(super) fn assert_focus_invariants(&self) {
        for (what, focus) in [("focused", self.focused), ("keyboard", self.keyboard)] {
            assert!(
                focus.is_none_or(|output| self.is_shown(output)),
                "the {what} monitor {focus:?} is not a shown monitor"
            );
        }
    }
}
