//! Delegation glue for `crownos_app_icons_v1`. The bookkeeping lives in
//! [`protocols::crownos_app_icons`]; what an activation does lives here.

use wayland_server::{Client, delegate_dispatch, delegate_global_dispatch};

use crownos_protocols::app_icons::v1::server::{
    crownos_app_icon_manager_v1::CrownosAppIconManagerV1, crownos_app_icon_v1::CrownosAppIconV1,
};
use protocols::crownos_app_icons::{
    AppIconData, AppIconsGlobalData, AppIconsHandler, AppIconsState, PlacedIcon, same_app,
};

use crate::{
    shell::tile::Tile,
    state::State,
    utils::{
        client_identity::{self, KnownExecutable},
        id::WindowId,
        surface,
    },
};

/// How an icon activation treats an application that is already in front.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActivationStyle {
    /// The dock's: pressing the icon of the app in front minimizes it, or
    /// moves on to its next window.
    Toggle,
    /// A notification's: the app comes forward and nothing else happens.
    /// Clicking a notification leaves focus where it was, so a toggle would
    /// minimize the very app the user is working in.
    RaiseOnly,
}

impl ActivationStyle {
    fn of(activated_by: Option<KnownExecutable>) -> Self {
        match activated_by {
            Some(KnownExecutable::Notifications) => Self::RaiseOnly,
            Some(KnownExecutable::AndroidDaemon) | None => Self::Toggle,
        }
    }
}

/// What an activation does to an application's windows, by index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IconAction {
    Minimize(usize),
    Raise(usize),
    RaiseAll,
}

/// The action for an app with `count` windows, `focused` being the index of
/// the one with focus, if one has it.
fn icon_action(style: ActivationStyle, count: usize, focused: Option<usize>) -> IconAction {
    match (style, focused) {
        (ActivationStyle::Toggle, Some(index)) if count == 1 => IconAction::Minimize(index),
        (ActivationStyle::Toggle, Some(index)) => IconAction::Raise((index + 1) % count),
        (ActivationStyle::RaiseOnly, _) | (ActivationStyle::Toggle, None) => IconAction::RaiseAll,
    }
}

impl AppIconsHandler for State {
    fn app_icons_state(&mut self) -> &mut AppIconsState {
        &mut self.wayland.app_icons_state
    }

    /// Acts as a taskbar button: an application in the background comes
    /// forward, every minimized window growing back out of the icon and the
    /// most recently used one taking focus; one already in front minimizes
    /// into the icon, or brings its next window forward when it has several.
    /// A notification only ever brings its app forward.
    fn activate_app(&mut self, client: &Client, app_id: &str, icon: Option<PlacedIcon>) -> bool {
        let windows: Vec<WindowId> = self
            .shell
            .tiles()
            .filter(|tile| !surface::has_parent(tile.surface()))
            .filter(|tile| {
                surface::app_id(tile.surface()).is_some_and(|own| same_app(app_id, &own))
            })
            .map(Tile::id)
            .collect();
        if windows.is_empty() {
            let from = icon.and_then(|icon| self.icon_at(&icon));
            self.wayland.launches.arm(app_id.to_owned(), from);
            return false;
        }
        let focused = self
            .shell
            .focused_window_id()
            .and_then(|focused| windows.iter().position(|&id| id == focused));
        let style = ActivationStyle::of(client_identity::executable(client));
        match icon_action(style, windows.len(), focused) {
            IconAction::Minimize(index) => {
                if let Some(&id) = windows.get(index) {
                    self.minimize_window(id);
                }
            }
            IconAction::Raise(index) => {
                self.bring_forward(windows.get(index..=index).unwrap_or_default())
            }
            IconAction::RaiseAll => self.bring_forward(&windows),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dock_toggles_the_app_in_front() {
        let dock = ActivationStyle::of(None);
        assert_eq!(icon_action(dock, 1, Some(0)), IconAction::Minimize(0));
        assert_eq!(icon_action(dock, 3, Some(1)), IconAction::Raise(2));
        assert_eq!(icon_action(dock, 3, Some(2)), IconAction::Raise(0));
        assert_eq!(icon_action(dock, 2, None), IconAction::RaiseAll);
    }

    #[test]
    fn a_notification_only_ever_raises() {
        let notification = ActivationStyle::of(Some(KnownExecutable::Notifications));
        assert_eq!(notification, ActivationStyle::RaiseOnly);
        for (count, focused) in [(1, Some(0)), (3, Some(1)), (2, None)] {
            assert_eq!(
                icon_action(notification, count, focused),
                IconAction::RaiseAll,
                "{count} windows, focused {focused:?}"
            );
        }
    }

    #[test]
    fn other_programs_keep_the_toggle() {
        assert_eq!(
            ActivationStyle::of(Some(KnownExecutable::AndroidDaemon)),
            ActivationStyle::Toggle
        );
    }
}
