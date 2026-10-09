//! Delegation glue for `crownos_app_icons_v1`. The bookkeeping lives in
//! [`protocols::crownos_app_icons`]; what an activation does lives here.

use wayland_server::{delegate_dispatch, delegate_global_dispatch};

use crownos_protocols::app_icons::v1::server::{
    crownos_app_icon_manager_v1::CrownosAppIconManagerV1, crownos_app_icon_v1::CrownosAppIconV1,
};
use protocols::crownos_app_icons::{
    AppIconData, AppIconsGlobalData, AppIconsHandler, AppIconsState, same_app,
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

    /// Brings the application forward: every minimized window grows back out
    /// of the icon, and its most recently used window takes focus.
    fn activate_app(&mut self, app_id: &str) -> bool {
        let windows: Vec<WindowId> = self
            .shell
            .tiles()
            .filter(|tile| {
                surface::app_id(tile.surface()).is_some_and(|own| same_app(app_id, &own))
            })
            .map(Tile::id)
            .collect();
        let Some(&last) = windows.last() else {
            return false;
        };
        let recent = self.shell.focused_monitor().and_then(|monitor| {
            monitor
                .active()
                .recent_windows()
                .find(|id| windows.contains(id))
        });

        for &id in &windows {
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
        true
    }
}

delegate_global_dispatch!(State: [CrownosAppIconManagerV1: AppIconsGlobalData] => AppIconsState);
delegate_dispatch!(State: [CrownosAppIconManagerV1: ()] => AppIconsState);
delegate_dispatch!(State: [CrownosAppIconV1: AppIconData] => AppIconsState);
