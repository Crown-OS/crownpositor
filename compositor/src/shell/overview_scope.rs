//! Which monitors show the overview together. In `Shared` scope every shown
//! monitor opens, closes and follows a swipe as one, each showing its own
//! column of the shared workspaces; in `PerMonitor` scope only the monitor
//! under the pointer does.

use smithay::utils::{Logical, Point};

use crate::{
    shell::{Shell, monitor::Monitor},
    utils::id::OutputId,
};

impl Shell {
    pub fn any_overview_open(&self) -> bool {
        self.monitors
            .iter()
            .any(|monitor| monitor.spacecontrol().is_open())
    }

    pub fn open_overview(&mut self) {
        for output in self.scope_monitors() {
            if let Some(monitor) = self.monitor_by_id_mut(output) {
                monitor.with_spacecontrol(|space, monitor| space.open(monitor));
            }
        }
    }

    /// Closes every overview. One open overview owns the keyboard, so a close
    /// that left another open would leave the desktop deaf.
    pub fn close_overviews(&mut self) {
        for monitor in &mut self.monitors {
            monitor.spacecontrol_mut().close();
        }
    }

    /// One decision for the whole scope, so the monitors cannot fall out of
    /// phase.
    pub fn toggle_overview(&mut self) {
        match self.any_overview_open() {
            true => self.close_overviews(),
            false => self.open_overview(),
        }
    }

    /// Closes the overview on `from` and on the monitors linked to it.
    pub fn close_linked_overviews(&mut self, from: OutputId) {
        for output in self.linked(from) {
            if let Some(monitor) = self.monitor_by_id_mut(output) {
                monitor.spacecontrol_mut().close();
            }
        }
    }

    /// Lays out again every open overview linked to `from`, after a change to
    /// the workspaces they all show.
    pub fn resolve_linked_overviews(&mut self, from: OutputId) {
        for output in self.linked(from) {
            if let Some(monitor) = self
                .monitor_by_id_mut(output)
                .filter(|monitor| monitor.spacecontrol().is_open())
            {
                monitor.with_spacecontrol(|space, monitor| space.resolve(monitor));
            }
        }
    }

    /// Points the overviews in scope at the fingers, opening them on the
    /// first update rather than waiting for the release.
    pub fn drive_overview(&mut self, travelled: f64) {
        for output in self.scope_monitors() {
            if let Some(monitor) = self.monitor_by_id_mut(output) {
                monitor.with_spacecontrol(|space, monitor| {
                    space.begin_gesture(monitor);
                    space.update_gesture(travelled);
                });
            }
        }
    }

    /// Lets a swipe go on every overview that follows one. Returns whether
    /// there was one to let go of.
    pub fn release_overview(&mut self, cancelled: bool, velocity: f64) -> bool {
        let mut released = false;
        for monitor in &mut self.monitors {
            let space = monitor.spacecontrol_mut();
            if !space.is_dragging() {
                continue;
            }
            released = true;
            match cancelled {
                true => space.cancel_gesture(),
                false => {
                    space.end_gesture(velocity);
                }
            }
        }
        released
    }

    /// The monitor whose overview takes the pointer at `at`, and `at` in that
    /// monitor's own space. A press or a carry in progress keeps the pointer
    /// on the overview it started on, wherever the pointer goes.
    pub fn overview_under(
        &self,
        at: Point<f64, Logical>,
    ) -> Option<(OutputId, Point<f64, Logical>)> {
        let monitor = self
            .overview_holder()
            .or_else(|| self.monitor_at(at))
            .filter(|monitor| monitor.spacecontrol().is_open())?;
        Some((monitor.id(), at - monitor.geometry().loc.to_f64()))
    }

    fn overview_holder(&self) -> Option<&Monitor> {
        self.monitors.iter().find(|monitor| {
            let space = monitor.spacecontrol();
            space.is_open() && space.is_holding()
        })
    }
}
