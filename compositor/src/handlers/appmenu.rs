//! Delegation glue for `org_kde_kwin_appmenu`.
//!
//! The protocol itself lives in [`protocols::appmenu`]; this routes its
//! interfaces there and turns an address change into a fetch on the async pool.
//! Reaching D-Bus from the compositor thread would put another process's
//! responsiveness on the critical path of a frame, so nothing here awaits.

use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use wayland_protocols_plasma::appmenu::server::{
    org_kde_kwin_appmenu::OrgKdeKwinAppmenu, org_kde_kwin_appmenu_manager::OrgKdeKwinAppmenuManager,
};
use wayland_server::{delegate_dispatch, delegate_global_dispatch};

use protocols::appmenu::{AppmenuData, AppmenuHandler, AppmenuState, MenuAddress};

use crate::{menu::dbusmenu, state::State, utils::id::WindowId};

impl AppmenuHandler for State {
    fn appmenu_state(&mut self) -> &mut AppmenuState {
        &mut self.wayland.appmenu_state
    }

    fn appmenu_address_changed(&mut self, surface: &WlSurface, address: Option<MenuAddress>) {
        let Some(id) = self.shell.window_id(surface) else {
            // The window has not mapped yet. `map_pending` reads the address
            // off the surface once it does, so nothing is lost by ignoring it.
            return;
        };
        self.fetch_menu(id, address);
    }
}

impl State {
    /// Records a window's menu address and, if it gained one, goes and reads it.
    pub fn fetch_menu(&mut self, id: WindowId, address: Option<MenuAddress>) {
        if !self.config.current.appearance.appmenu {
            return;
        }
        let Some(address) = self.shell.menus.set_address(id, address) else {
            // Either nothing changed, or the menu went away — in which case
            // `set_address` has already dropped it.
            self.queue_redraw();
            return;
        };

        let (service, path) = (address.service.clone(), address.path.clone());
        self.common.tasks.run(
            async move { dbusmenu::fetch_bar(service, path).await },
            move |state, bar| {
                let Some(bar) = bar else {
                    return;
                };
                if state.shell.menus.set_bar(id, &address, bar) {
                    state.queue_redraw();
                }
            },
        );
    }

    /// Reads one submenu, for when the user opens it.
    pub fn fetch_submenu(&mut self, id: WindowId, item: i32) {
        let Some(address) = self.shell.menus.address(id).cloned() else {
            return;
        };
        let (service, path) = (address.service.clone(), address.path.clone());

        self.common.tasks.run(
            async move { dbusmenu::fetch_submenu(service, path, item).await },
            move |state, children| {
                let Some(children) = children else {
                    return;
                };
                if state.shell.menus.graft(id, item, children) {
                    state.queue_redraw();
                }
            },
        );
    }

    /// Tells the client an item was chosen, and closes the menu.
    ///
    /// The menu closes on the click rather than on the reply: a client that
    /// never answers must not leave one open on screen.
    pub fn activate_menu_item(&mut self, id: WindowId, item: i32) {
        let Some(address) = self.shell.menus.address(id).cloned() else {
            return;
        };
        let timestamp = self.wayland.clock.now().as_millis();

        self.shell.menus.close();
        self.queue_redraw();

        self.common.tasks.spawn(dbusmenu::activate(
            address.service,
            address.path,
            item,
            timestamp,
        ));
    }
}

delegate_global_dispatch!(State: [OrgKdeKwinAppmenuManager: ()] => AppmenuState);
delegate_dispatch!(State: [OrgKdeKwinAppmenuManager: ()] => AppmenuState);
delegate_dispatch!(State: [OrgKdeKwinAppmenu: AppmenuData] => AppmenuState);
