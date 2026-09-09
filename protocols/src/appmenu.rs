//! Server-side `org_kde_kwin_appmenu`.
//!
//! A client that exports its menubar over D-Bus uses this to say *where*: a
//! bus name and an object path implementing `com.canonical.dbusmenu`. That is
//! the whole protocol — two strings per surface, and no menu content ever
//! travels over Wayland.
//!
//! Written the way smithay writes its own protocol modules (see
//! `smithay::wayland::alpha_modifier`): a handler trait the compositor
//! implements, `Dispatch` impls generic over the compositor data `D`, and the
//! compositor supplying the delegation glue. Nothing here talks to D-Bus — it
//! records an address and tells the compositor it changed, and the compositor's
//! own client is what connects.

use std::sync::{Arc, Mutex};

use smithay::wayland::compositor::{SurfaceData, with_states};
use wayland_protocols_plasma::appmenu::server::{
    org_kde_kwin_appmenu::{self, OrgKdeKwinAppmenu},
    org_kde_kwin_appmenu_manager::{self, OrgKdeKwinAppmenuManager},
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, Weak,
    backend::GlobalId, protocol::wl_surface::WlSurface,
};

/// Longest bus name or object path accepted.
///
/// Both are bounded by the D-Bus specification at 255 bytes; anything longer
/// cannot name a real service, and refusing it here keeps a hostile client from
/// parking megabytes of string on a surface.
const MAX_ADDRESS: usize = 255;

/// What the compositor has to provide for this protocol to be delegated to
/// [`AppmenuState`].
pub trait AppmenuHandler {
    fn appmenu_state(&mut self) -> &mut AppmenuState;

    /// A surface's menu address appeared, changed or went away.
    ///
    /// The compositor connects (or disconnects) its own D-Bus client from
    /// here; nothing in this module does any I/O.
    fn appmenu_address_changed(&mut self, surface: &WlSurface, address: Option<MenuAddress>);
}

/// Where a window's menu lives on the session bus.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MenuAddress {
    pub service: String,
    pub path: String,
}

/// Reads a surface's menu address. `None` when the client never set one, or
/// released the object it set it through.
pub fn menu_address(states: &SurfaceData) -> Option<MenuAddress> {
    let slot = states.data_map.get::<AddressSlot>()?;
    slot.0.lock().ok()?.clone()
}

/// Per-surface state, living in the surface's `data_map` because it has to
/// outlive the appmenu object that produced it — a client may release the
/// object while the address stays valid.
#[derive(Debug, Default)]
struct AddressSlot(Mutex<Option<MenuAddress>>);

/// User data of an [`OrgKdeKwinAppmenu`] object.
#[derive(Debug)]
pub struct AppmenuData {
    /// Held weakly: an appmenu object outliving its surface must not keep the
    /// surface alive, and the protocol has nothing to say about such an object
    /// beyond that it does nothing.
    surface: Weak<WlSurface>,
}

/// Delegate type for the [`OrgKdeKwinAppmenuManager`] global.
#[derive(Debug)]
pub struct AppmenuState {
    global: GlobalId,
    /// Surfaces with a live address, so the compositor can be told to drop them
    /// all at once when the feature is switched off.
    exported: Arc<Mutex<Vec<Weak<WlSurface>>>>,
}

impl AppmenuState {
    pub fn new<D>(display: &DisplayHandle) -> Self
    where
        D: GlobalDispatch<OrgKdeKwinAppmenuManager, ()>
            + Dispatch<OrgKdeKwinAppmenuManager, ()>
            + Dispatch<OrgKdeKwinAppmenu, AppmenuData>
            + AppmenuHandler
            + 'static,
    {
        Self {
            global: display.create_global::<D, OrgKdeKwinAppmenuManager, _>(2, ()),
            exported: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }

    /// Every surface that currently names a menu.
    pub fn exported(&self) -> Vec<WlSurface> {
        let mut surfaces = self.exported.lock().unwrap_or_else(|err| err.into_inner());
        surfaces.retain(|surface| surface.upgrade().is_ok());
        surfaces
            .iter()
            .filter_map(|surface| surface.upgrade().ok())
            .collect()
    }

    fn remember(&self, surface: &WlSurface) {
        let mut surfaces = self.exported.lock().unwrap_or_else(|err| err.into_inner());
        surfaces.retain(|existing| {
            existing
                .upgrade()
                .is_ok_and(|existing| existing != *surface)
        });
        surfaces.push(surface.downgrade());
    }

    fn forget(&self, surface: &WlSurface) {
        let mut surfaces = self.exported.lock().unwrap_or_else(|err| err.into_inner());
        surfaces.retain(|existing| {
            existing
                .upgrade()
                .is_ok_and(|existing| existing != *surface)
        });
    }
}

impl<D> GlobalDispatch<OrgKdeKwinAppmenuManager, (), D> for AppmenuState
where
    D: GlobalDispatch<OrgKdeKwinAppmenuManager, ()>
        + Dispatch<OrgKdeKwinAppmenuManager, ()>
        + Dispatch<OrgKdeKwinAppmenu, AppmenuData>
        + AppmenuHandler
        + 'static,
{
    fn bind(
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<OrgKdeKwinAppmenuManager>,
        _global_data: &(),
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, ());
    }
}

impl<D> Dispatch<OrgKdeKwinAppmenuManager, (), D> for AppmenuState
where
    D: Dispatch<OrgKdeKwinAppmenuManager, ()>
        + Dispatch<OrgKdeKwinAppmenu, AppmenuData>
        + AppmenuHandler
        + 'static,
{
    fn request(
        _state: &mut D,
        _client: &Client,
        _manager: &OrgKdeKwinAppmenuManager,
        request: org_kde_kwin_appmenu_manager::Request,
        _data: &(),
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            org_kde_kwin_appmenu_manager::Request::Create { id, surface } => {
                data_init.init(
                    id,
                    AppmenuData {
                        surface: surface.downgrade(),
                    },
                );
            }
            org_kde_kwin_appmenu_manager::Request::Release => {}
            _ => {}
        }
    }
}

impl<D> Dispatch<OrgKdeKwinAppmenu, AppmenuData, D> for AppmenuState
where
    D: Dispatch<OrgKdeKwinAppmenu, AppmenuData> + AppmenuHandler + 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        _resource: &OrgKdeKwinAppmenu,
        request: org_kde_kwin_appmenu::Request,
        data: &AppmenuData,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        let Ok(surface) = data.surface.upgrade() else {
            // The surface is gone; the object is inert.
            return;
        };

        match request {
            org_kde_kwin_appmenu::Request::SetAddress {
                service_name,
                object_path,
            } => {
                // An empty name is how a client says "I no longer have one",
                // and an over-long one cannot address a real service.
                let usable = !service_name.is_empty()
                    && !object_path.is_empty()
                    && service_name.len() <= MAX_ADDRESS
                    && object_path.len() <= MAX_ADDRESS;
                publish(
                    state,
                    &surface,
                    usable.then_some(MenuAddress {
                        service: service_name,
                        path: object_path,
                    }),
                );
            }

            org_kde_kwin_appmenu::Request::Release => publish(state, &surface, None),

            _ => {}
        }
    }

    fn destroyed(
        state: &mut D,
        _client: wayland_server::backend::ClientId,
        _resource: &OrgKdeKwinAppmenu,
        data: &AppmenuData,
    ) {
        // A client dropping the object without releasing it means the same
        // thing: there is no menu at that address any more.
        if let Ok(surface) = data.surface.upgrade() {
            publish(state, &surface, None);
        }
    }
}

/// Records an address and tells the compositor, if it actually changed.
///
/// The three ways an address can move — set, released, and the object being
/// destroyed — all end here, so none of them can forget half the work.
fn publish<D: AppmenuHandler>(state: &mut D, surface: &WlSurface, address: Option<MenuAddress>) {
    if !store(surface, address.clone()) {
        return;
    }

    let appmenu = state.appmenu_state();
    match &address {
        Some(_) => appmenu.remember(surface),
        None => appmenu.forget(surface),
    }
    state.appmenu_address_changed(surface, address);
}

/// Records an address on a surface. Returns whether it actually changed, so a
/// client re-setting the same address does not make the compositor reconnect.
fn store(surface: &WlSurface, address: Option<MenuAddress>) -> bool {
    with_states(surface, |states| {
        let slot = states.data_map.get_or_insert_threadsafe(AddressSlot::default);
        let mut current = slot.0.lock().unwrap_or_else(|err| err.into_inner());
        if *current == address {
            return false;
        }
        *current = address;
        true
    })
}
