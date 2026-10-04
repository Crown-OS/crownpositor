//! How each surface is shown, in the terms of `crownos_surface_visibility_v1`:
//! drawn, covered by a fullscreen window, or not drawn at all, and the scale
//! the overview settles it at.
//!
//! Everything is read off where the scene is headed rather than where its
//! springs are this frame — window targets, the committed overview state — so
//! a client hears about a change once, when it starts, and not again as it
//! animates.

use protocols::crownos_surface_visibility::Visibility;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

use crate::{
    shell::{Shell, monitor::Monitor, workspace::Workspace},
    utils::id::WindowId,
};

impl Shell {
    /// `root` is a toplevel's or a layer surface's root; anything else is
    /// hidden.
    pub fn surface_visibility(&self, root: &WlSurface) -> Visibility {
        if self.session_lock.is_active() {
            return Visibility::HIDDEN;
        }
        match self.window_id(root) {
            Some(id) => self.window_visibility(id),
            None => self.layer_visibility(root),
        }
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
