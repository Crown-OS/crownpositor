//! A touchpad swipe between workspaces. The focused monitor leads: it decides
//! whether the band tears and where the swipe lands, and each linked monitor
//! follows that decision so none of them can land elsewhere.

use crate::{
    shell::{Shell, monitor::Monitor},
    utils::id::OutputId,
};

impl Shell {
    pub fn is_swiping_workspaces(&self) -> bool {
        self.focused_monitor().is_some_and(Monitor::is_swiping)
    }

    /// Hands the viewports of the focused monitor, and of the monitors linked
    /// to it, to the fingers.
    pub fn begin_workspace_swipe(&mut self) {
        if let Some(focused) = self.focused {
            self.each_linked_monitor(focused, Monitor::begin_switch_gesture);
        }
    }

    /// `travelled` is cumulative horizontal travel in pages, positive
    /// rightward.
    pub fn update_workspace_swipe(&mut self, travelled: f64) {
        let Some(focused) = self.focused else {
            return;
        };
        let Some(torn) = self
            .monitor_by_id_mut(focused)
            .map(|monitor| monitor.update_switch_gesture(travelled))
        else {
            return;
        };
        for follower in self.followers_of(focused) {
            if let Some(monitor) = self.monitor_by_id_mut(follower) {
                monitor.follow_switch_gesture(travelled, torn);
            }
        }
    }

    /// Releases the swipe into its spring. `velocity` is in pages per second,
    /// positive rightward; returns whether the active workspace moved.
    pub fn end_workspace_swipe(&mut self, velocity: f64) -> bool {
        let Some(focused) = self.focused else {
            return false;
        };
        let Some((moved, landed)) = self.monitor_by_id_mut(focused).map(|monitor| {
            let moved = monitor.end_switch_gesture(velocity);
            (moved, monitor.active_index())
        }) else {
            return false;
        };
        for follower in self.followers_of(focused) {
            if let Some(monitor) = self.monitor_by_id_mut(follower) {
                monitor.activate(landed);
            }
        }
        moved
    }

    pub fn cancel_workspace_swipe(&mut self) {
        if let Some(focused) = self.focused {
            self.each_linked_monitor(focused, Monitor::cancel_switch_gesture);
        }
    }

    fn followers_of(&self, leader: OutputId) -> Vec<OutputId> {
        let mut linked = self.linked(leader);
        linked.retain(|id| *id != leader);
        linked
    }
}
