//! The keyboard across an output change. The window that held it before the
//! change keeps it when it lands on an active workspace of a shown monitor;
//! otherwise the keyboard goes to the monitor under the pointer.

use crate::{
    shell::{Location, Shell},
    utils::id::WindowId,
};

/// The window that held the keyboard when the outputs began to change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeldKeyboard {
    window: Option<WindowId>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum KeyboardAfterChange {
    /// No shown monitor is left to type on: keep the note for the next one.
    Wait,
    Window(WindowId),
    FocusedMonitor,
}

pub fn keyboard_after_change(
    held: HeldKeyboard,
    landed: Option<Location>,
    shown_active: &[Location],
) -> KeyboardAfterChange {
    if shown_active.is_empty() {
        return KeyboardAfterChange::Wait;
    }
    match held.window {
        Some(window) if landed.is_some_and(|landed| shown_active.contains(&landed)) => {
            KeyboardAfterChange::Window(window)
        }
        _ => KeyboardAfterChange::FocusedMonitor,
    }
}

impl Shell {
    /// Notes the keyboard window before an output change. A note still
    /// waiting from an earlier change is kept: it is older and still true.
    pub(super) fn note_keyboard_before_change(&mut self) {
        if self.keyboard_before_change.is_none() {
            self.keyboard_before_change = Some(HeldKeyboard {
                window: self.focused_window_id(),
            });
        }
    }

    /// Gives the keyboard back after an output change, by the rule above.
    pub fn hand_keyboard_back(&mut self) {
        let Some(held) = self.keyboard_before_change else {
            return;
        };
        let landed = held.window.and_then(|window| self.location(window));
        let shown_active: Vec<Location> = self
            .shown_monitors()
            .map(|monitor| Location {
                output: monitor.id(),
                workspace: monitor.active().id(),
            })
            .collect();
        match keyboard_after_change(held, landed, &shown_active) {
            KeyboardAfterChange::Wait => return,
            KeyboardAfterChange::Window(window) => {
                self.focus_window(window);
            }
            KeyboardAfterChange::FocusedMonitor => {
                self.focus_keyboard_on_focused();
            }
        }
        self.keyboard_before_change = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::id::{OutputId, WorkspaceId};

    fn location(output: u64) -> Location {
        Location {
            output: OutputId::from_raw(output),
            workspace: WorkspaceId::next(),
        }
    }

    fn held(window: WindowId) -> HeldKeyboard {
        HeldKeyboard {
            window: Some(window),
        }
    }

    #[test]
    fn a_window_on_a_shown_active_workspace_keeps_the_keyboard() {
        let window = WindowId::next();
        let (survivor, returned) = (location(1), location(2));
        assert_eq!(
            keyboard_after_change(held(window), Some(returned), &[survivor, returned]),
            KeyboardAfterChange::Window(window)
        );
    }

    #[test]
    fn a_window_parked_on_an_inactive_workspace_gives_way_to_the_focused_monitor() {
        let window = WindowId::next();
        assert_eq!(
            keyboard_after_change(held(window), Some(location(1)), &[location(1)]),
            KeyboardAfterChange::FocusedMonitor
        );
    }

    #[test]
    fn no_keyboard_window_falls_back_to_the_focused_monitor() {
        assert_eq!(
            keyboard_after_change(HeldKeyboard { window: None }, None, &[location(1)]),
            KeyboardAfterChange::FocusedMonitor
        );
    }

    #[test]
    fn with_no_shown_monitor_left_the_note_waits() {
        let window = WindowId::next();
        assert_eq!(
            keyboard_after_change(held(window), Some(location(0)), &[]),
            KeyboardAfterChange::Wait
        );
    }
}
