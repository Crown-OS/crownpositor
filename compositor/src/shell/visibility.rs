//! How each surface is shown, in the terms of `crownos_surface_visibility_v1`:
//! drawn, covered by a fullscreen window, or not drawn at all, and the scale
//! the overview settles it at.
//!
//! Everything is read off where the scene is headed rather than where its
//! springs are this frame — window targets, the committed overview state — so
//! a client hears about a change once, when it starts, and not again as it
//! animates.

use protocols::crownos_surface_visibility::{Visibility, VisibilityState};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

use crate::{
    shell::{Shell, monitor::Monitor, tile::Tile, workspace::Workspace},
    utils::id::WindowId,
};

/// Whether a window shown as `visibility` should be told it is suspended:
/// anything not drawn at all, or drawn only under a fullscreen window.
fn suspends(visibility: Visibility) -> bool {
    visibility.state != VisibilityState::Visible
}

/// The suspended state a window last told `suspended` has to be told now, if
/// it changed.
fn suspension_change(suspended: bool, visibility: Visibility) -> Option<bool> {
    let now = suspends(visibility);
    (now != suspended).then_some(now)
}

impl Shell {
    /// `root` is a toplevel's or a layer surface's root; anything else is
    /// hidden.
    pub fn surface_visibility(&self, root: &WlSurface) -> Visibility {
        if self.is_locked() {
            return Visibility::HIDDEN;
        }
        match self.window_id(root) {
            Some(id) => self.window_visibility(id),
            None => self.layer_visibility(root),
        }
    }

    /// Tells every window that went out of sight or came back into it, with
    /// the `suspended` toplevel state. Windows whose visibility did not change
    /// are left alone, and nothing is sent when no window's did.
    pub fn refresh_suspended(&mut self) {
        let changes: Vec<(WindowId, bool)> = self
            .monitors
            .iter()
            .flat_map(Monitor::workspaces)
            .flat_map(Workspace::tiles)
            .filter_map(|tile| {
                let visibility = self.window_visibility(tile.id());
                suspension_change(tile.is_suspended(), visibility).map(|now| (tile.id(), now))
            })
            .collect();
        if changes.is_empty() {
            return;
        }
        for (id, suspended) in changes {
            if let Some(tile) = self.tile_mut(id) {
                tile.set_suspended(suspended);
            }
        }
        self.send_configures();
    }

    fn layer_visibility(&self, root: &WlSurface) -> Visibility {
        let monitor = self
            .layer_to_output
            .get(root)
            .and_then(|output| self.monitor_by_id(*output));
        match monitor {
            Some(monitor) if covered_by_fullscreen(monitor) => Visibility::OCCLUDED,
            Some(_) => Visibility::VISIBLE,
            None => Visibility::HIDDEN,
        }
    }

    fn window_visibility(&self, id: WindowId) -> Visibility {
        if self.is_locked() || self.tile(id).is_none_or(Tile::is_minimized) {
            return Visibility::HIDDEN;
        }
        let Some(location) = self.location(id) else {
            return Visibility::HIDDEN;
        };
        let Some(monitor) = self.monitor_by_id(location.output) else {
            return Visibility::HIDDEN;
        };
        let Some(index) = monitor
            .workspaces()
            .iter()
            .position(|workspace| workspace.id() == location.workspace)
        else {
            return Visibility::HIDDEN;
        };

        let overview = monitor.spacecontrol();
        if overview.is_open() {
            return overview
                .thumbnail_scale(index, id)
                .map_or(Visibility::VISIBLE, Visibility::thumbnail);
        }

        let shown = monitor
            .visible_workspaces()
            .find(|(workspace, _)| workspace.id() == location.workspace);
        match shown {
            None => Visibility::HIDDEN,
            Some((workspace, _)) if covering_window(workspace).is_some_and(|cover| cover != id) => {
                Visibility::OCCLUDED
            }
            Some(_) => Visibility::VISIBLE,
        }
    }
}

/// The opaque fullscreen window `workspace` is headed towards showing alone.
fn covering_window(workspace: &Workspace) -> Option<WindowId> {
    let tile = workspace.tile(workspace.fullscreen()?)?;
    (tile.opacity() >= 1.0 && tile.target() == workspace.output_area()).then_some(tile.id())
}

/// Whether a fullscreen window covers the whole of `monitor`, panels included.
fn covered_by_fullscreen(monitor: &Monitor) -> bool {
    if monitor.spacecontrol().is_open() {
        return false;
    }
    let mut shown = monitor.visible_workspaces();
    let covered = shown
        .next()
        .is_some_and(|(workspace, _)| covering_window(workspace).is_some());
    covered && shown.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_out_of_sight_is_suspended() {
        assert_eq!(suspension_change(false, Visibility::HIDDEN), Some(true));
        assert_eq!(suspension_change(false, Visibility::OCCLUDED), Some(true));
    }

    #[test]
    fn a_window_back_in_sight_is_resumed() {
        assert_eq!(suspension_change(true, Visibility::VISIBLE), Some(false));
        assert_eq!(
            suspension_change(true, Visibility::thumbnail(0.25)),
            Some(false),
            "a thumbnail in the overview is still drawn"
        );
    }

    #[test]
    fn an_unchanged_window_hears_nothing() {
        assert_eq!(suspension_change(false, Visibility::VISIBLE), None);
        assert_eq!(suspension_change(true, Visibility::HIDDEN), None);
        assert_eq!(suspension_change(true, Visibility::OCCLUDED), None);
    }
}
