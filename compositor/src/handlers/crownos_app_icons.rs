//! Delegation glue for `crownos_app_icons_v1`. The bookkeeping lives in
//! [`protocols::crownos_app_icons`]; what an activation does lives here.

use wayland_server::{delegate_dispatch, delegate_global_dispatch};

use crownos_protocols::app_icons::v1::server::{
    crownos_app_icon_manager_v1::CrownosAppIconManagerV1, crownos_app_icon_v1::CrownosAppIconV1,
};
use protocols::crownos_app_icons::{
    AppIconData, AppIconsGlobalData, AppIconsHandler, AppIconsState, PlacedIcon, same_app,
};

use crate::{
    shell::tile::Tile,
    state::State,
    utils::{id::WindowId, surface},
};

impl AppIconsHandler for State {
    fn app_icons_state(&mut self) -> &mut AppIconsState {
        &mut self.wayland.app_icons_state
    }

    /// Acts as a taskbar button: an application in the background comes
    /// forward, every minimized window growing back out of the icon and the
    /// most recently used one taking focus; one already in front minimizes
    /// into the icon, or brings its next window forward when it has several.
    fn activate_app(&mut self, app_id: &str, icon: Option<PlacedIcon>) -> bool {
        let windows: Vec<WindowId> = self
            .shell
            .tiles()
            .filter(|tile| !surface::has_parent(tile.surface()))
            .filter(|tile| {
                surface::app_id(tile.surface()).is_some_and(|own| same_app(app_id, &own))
            })
            .map(Tile::id)
            .collect();
        let Some(&last) = windows.last() else {
            let from = icon.and_then(|icon| self.icon_at(&icon));
            self.wayland.launches.arm(app_id.to_owned(), from);
            return false;
        };
        let focused = self
            .shell
            .focused_window_id()
            .and_then(|focused| windows.iter().position(|&id| id == focused));
        match focused {
            Some(_) if windows.len() == 1 => self.minimize_window(last),
            Some(index) => self.bring_forward(&[windows[(index + 1) % windows.len()]]),
            None => self.bring_forward(&windows),
        }
        true
    }
}

impl State {
    /// Restores every minimized window of `windows` out of its icon and
    /// focuses the most recently used, the last of them if none was.
    fn bring_forward(&mut self, windows: &[WindowId]) {
        let Some(&last) = windows.last() else {
            return;
        };
        let recent = self.shell.focused_monitor().and_then(|monitor| {
            monitor
                .active()
                .recent_windows()
                .find(|id| windows.contains(id))
        });
        for &id in windows {
            if self.shell.tile(id).is_some_and(Tile::is_minimized)
                && let Some(origin) = self.icon_of(id)
            {
                self.shell.unminimize_window(id, origin);
            }
        }
        self.shell.focus_window(recent.unwrap_or(last));
        self.shell.refresh();
        self.update_keyboard_focus();
        self.queue_redraw();
    }
}

delegate_global_dispatch!(State: [CrownosAppIconManagerV1: AppIconsGlobalData] => AppIconsState);
delegate_dispatch!(State: [CrownosAppIconManagerV1: ()] => AppIconsState);
delegate_dispatch!(State: [CrownosAppIconV1: AppIconData] => AppIconsState);
