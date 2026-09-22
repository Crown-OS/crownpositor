//! Driving the overview from input.
//!
//! The gesture side and the pointer side both land here so there is one place
//! that knows what clicking a thumbnail means. Everything acts on the *focused*
//! monitor for gestures — a touchpad has no position on screen — and on the
//! monitor under the pointer for clicks, which does.

use smithay::utils::{Logical, Point};

use spacecontrol::interaction::{Release, Target};

use crate::{
    shell::{Location, workspace::WorkspaceRef},
    state::State,
    utils::id::OutputId,
};

impl State {
    /// Whether any output is showing the overview, which is what makes the rest
    /// of the compositor treat input as belonging to it.
    pub fn overview_is_open(&self) -> bool {
        self.shell
            .monitors()
            .iter()
            .any(|monitor| monitor.spacecontrol().is_open())
    }

    /// Whether the overview owns input.
    ///
    /// It is a mode, not a region: while it is open no client hears the
    /// pointer or the keyboard, wherever on the desk the pointer happens to
    /// be. A thumbnail is a picture of a window, and a click, a scroll or a
    /// keystroke aimed at the picture is not aimed at the window.
    pub fn overview_owns_input(&self) -> bool {
        self.overview_is_open()
    }

    /// Points the focused monitor's overview at the fingers, opening it on the
    /// first update rather than waiting for the release.
    pub fn drive_overview(&mut self, travelled: f64) {
        let Some(monitor) = self.shell.focused_monitor_mut() else {
            return;
        };
        monitor.with_spacecontrol(|space, monitor| {
            space.begin_gesture(monitor);
            space.update_gesture(travelled);
        });
    }

    /// Lets a swipe go. Returns whether there was one to let go of, so the
    /// caller knows the release is spoken for.
    pub fn release_overview(&mut self, cancelled: bool, velocity: f64) -> bool {
        let Some(monitor) = self.shell.focused_monitor_mut() else {
            return false;
        };
        if !monitor.spacecontrol().is_dragging() {
            return false;
        }

        let space = monitor.spacecontrol_mut();
        if cancelled {
            space.cancel_gesture();
        } else {
            space.end_gesture(velocity);
        }
        true
    }

    /// The output whose overview should take this pointer position, if any.
    fn overview_under(&self, at: Point<f64, Logical>) -> Option<OutputId> {
        self.shell
            .monitor_at(at)
            .filter(|monitor| monitor.spacecontrol().is_open())
            .map(|monitor| monitor.id())
    }

    /// Moves the pointer over an open overview. Returns whether the overview
    /// took the motion, in which case it must not also reach a client.
    pub fn overview_motion(&mut self, at: Point<f64, Logical>) -> bool {
        let Some(id) = self.overview_under(at) else {
            return false;
        };
        let Some(monitor) = self.shell.monitor_by_id_mut(id) else {
            return false;
        };

        // Output-local, which is the space the overview laid itself out in.
        if monitor.spacecontrol_mut().motion(at) {
            self.queue_redraw();
        }
        true
    }

    /// Presses a button over an open overview.
    pub fn overview_press(&mut self, at: Point<f64, Logical>) -> bool {
        let Some(id) = self.overview_under(at) else {
            return false;
        };
        if let Some(monitor) = self.shell.monitor_by_id_mut(id) {
            monitor.spacecontrol_mut().press(at);
            self.queue_redraw();
        }
        true
    }

    /// Releases a button over an open overview, and does whatever that turned
    /// out to mean.
    pub fn overview_release(&mut self, at: Point<f64, Logical>) -> bool {
        let Some(id) = self.overview_under(at) else {
            return false;
        };
        let Some(monitor) = self.shell.monitor_by_id_mut(id) else {
            return false;
        };

        let release = monitor.spacecontrol_mut().release(at);
        match release {
            Release::Click(Target::Window(index)) => {
                let window = monitor.spacecontrol().window(index);
                monitor.spacecontrol_mut().close();
                if let Some(window) = window {
                    self.shell.focus_window(window);
                }
            }
            Release::Click(Target::Workspace(index)) => {
                monitor.switch_to(WorkspaceRef::Index(index));
                monitor.spacecontrol_mut().close();
            }
            Release::Dropped { window, workspace } => {
                self.drop_window(id, window, workspace);
            }
            // The carried window simply stops being carried, and the grid it
            // came from is still where it belongs.
            Release::Returned { .. } | Release::None => {}
        }

        self.shell.refresh();
        self.update_keyboard_focus();
        self.queue_redraw();
        true
    }

    /// Moves a window the pointer carried onto a workspace preview.
    ///
    /// The overview stays open: dropping a window somewhere is a tidying
    /// action, and the user almost always has another to move.
    fn drop_window(&mut self, output: OutputId, index: usize, workspace: usize) {
        let Some(monitor) = self.shell.monitor_by_id(output) else {
            return;
        };
        let Some(window) = monitor.spacecontrol().window(index) else {
            return;
        };
        let Some(target) = monitor
            .workspaces()
            .get(workspace)
            .map(|workspace| workspace.id())
        else {
            return;
        };

        self.shell.move_tile(
            window,
            Location {
                output,
                workspace: target,
            },
        );

        // The active workspace just lost a window, so the grid it was laid out
        // from no longer describes it.
        if let Some(monitor) = self.shell.monitor_by_id_mut(output) {
            monitor.with_spacecontrol(|space, monitor| space.resolve(monitor));
        }
    }
}
