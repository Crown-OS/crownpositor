//! Driving the overview from input.
//!
//! The gesture side and the pointer side both land here so there is one place
//! that knows what clicking a thumbnail means. Gestures act on the monitors in
//! scope — a touchpad has no position on screen — and clicks on the monitor
//! under the pointer, which does; a change to the shared workspaces then lays
//! out every overview linked to it.

use smithay::utils::{Logical, Point};

use spacecontrol::interaction::{Release, Target};

use crate::{shell::Location, state::State, utils::id::OutputId};

impl State {
    /// Whether any output is showing the overview, which is what makes the rest
    /// of the compositor treat input as belonging to it.
    pub fn overview_is_open(&self) -> bool {
        self.shell.any_overview_open()
    }

    /// Whether the overview or the Alt+Tab strip owns input.
    ///
    /// Each is a mode, not a region: while one is open no client hears the
    /// pointer or the keyboard, wherever on the desk the pointer happens to
    /// be. A thumbnail is a picture of a window, and a click, a scroll or a
    /// keystroke aimed at the picture is not aimed at the window.
    pub fn mode_owns_input(&self) -> bool {
        self.overview_is_open() || self.window_control_is_open()
    }

    /// Points the overviews in scope at the fingers.
    pub fn drive_overview(&mut self, travelled: f64) {
        self.shell.drive_overview(travelled);
    }

    /// Lets a swipe go. Returns whether there was one to let go of, so the
    /// caller knows the release is spoken for.
    pub fn release_overview(&mut self, cancelled: bool, velocity: f64) -> bool {
        self.shell.release_overview(cancelled, velocity)
    }

    /// Moves the pointer over an open overview. Returns whether the overview
    /// took the motion, in which case it must not also reach a client.
    pub fn overview_motion(&mut self, at: Point<f64, Logical>) -> bool {
        let Some((id, local)) = self.shell.overview_under(at) else {
            return false;
        };
        let Some(monitor) = self.shell.monitor_by_id_mut(id) else {
            return false;
        };

        if monitor.spacecontrol_mut().motion(local) {
            self.queue_redraw();
        }
        true
    }

    /// Presses a button over an open overview.
    pub fn overview_press(&mut self, at: Point<f64, Logical>) -> bool {
        let Some((id, local)) = self.shell.overview_under(at) else {
            return false;
        };
        if let Some(monitor) = self.shell.monitor_by_id_mut(id) {
            monitor.spacecontrol_mut().press(local);
            self.queue_redraw();
        }
        true
    }

    /// Releases a button over an open overview, and does whatever that turned
    /// out to mean.
    pub fn overview_release(&mut self, at: Point<f64, Logical>) -> bool {
        let Some((id, local)) = self.shell.overview_under(at) else {
            return false;
        };
        let Some(monitor) = self.shell.monitor_by_id_mut(id) else {
            return false;
        };

        let release = monitor.spacecontrol_mut().release(local);
        match release {
            Release::Click(Target::Window(index)) => {
                let window = monitor.spacecontrol().window(index);
                self.shell.close_linked_overviews(id);
                if let Some(window) = window {
                    self.focus_window_on_any_monitor(window);
                }
            }
            Release::Click(Target::Workspace(index)) => {
                self.shell.close_linked_overviews(id);
                self.shell.activate_index(id, index);
            }
            // A new workspace is added and shown in the bar, not switched to:
            // the user is arranging, and usually has windows to drop on it.
            Release::Click(Target::NewWorkspace) => {
                self.shell.create_workspace(id);
                self.shell.resolve_linked_overviews(id);
            }
            Release::Click(Target::Close(index)) => {
                if let Some(workspace) = monitor.workspaces().get(index).map(|it| it.id()) {
                    self.shell.remove_workspace(id, workspace);
                }
                self.shell.resolve_linked_overviews(id);
            }
            Release::Dropped { window, workspace } => {
                self.drop_window(id, window, workspace);
            }
            Release::DroppedOnNew { window } => {
                if let Some(workspace) = self.shell.create_workspace(id) {
                    self.drop_window(id, window, workspace);
                }
            }
            Release::Returned {
                window,
                from,
                velocity,
            } => monitor
                .spacecontrol_mut()
                .return_window(window, from, velocity),
            Release::Reordered { from, to, .. } => {
                self.shell.move_workspace(id, from, to);
                self.shell.resolve_linked_overviews(id);
            }
            Release::None => {}
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
        let location = Location {
            output,
            workspace: target,
        };
        // Dropped back where it lives: nothing moves in the model, and the
        // relayout glides it home from where the hand let go.
        if self.shell.location(window) == Some(location) {
            self.resolve_overview(output);
            return;
        }

        self.shell.move_tile(
            window,
            Location {
                output,
                workspace: target,
            },
        );
        // Arranged before the overview lays itself out again, so it solves the
        // sizes both workspaces are about to give their windows — once —
        // rather than the old ones and then again a frame later.
        self.shell.refresh();

        // The active workspace just lost a window, so the grid it was laid out
        // from no longer describes it.
        self.resolve_overview(output);
    }

    /// Lays the overview out again after the model changed under it.
    fn resolve_overview(&mut self, output: OutputId) {
        if let Some(monitor) = self.shell.monitor_by_id_mut(output) {
            monitor.with_spacecontrol(|space, monitor| space.resolve(monitor));
        }
    }
}
