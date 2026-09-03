use std::borrow::Cow;

use smithay::{
    backend::input::KeyState,
    desktop::{LayerSurface, PopupKind, Window},
    input::{
        Seat,
        keyboard::{KeyboardTarget, KeysymHandle, ModifiersState},
    },
    reexports::wayland_server::{Resource, backend::ObjectId, protocol::wl_surface::WlSurface},
    utils::{IsAlive, Serial},
    wayland::{seat::WaylandFocus, session_lock::LockSurface},
};

use crate::state::State;

#[derive(Debug, Clone, PartialEq)]
pub enum KeyboardFocusTarget {
    Window(Window),
    Popup(PopupKind),
    LayerShell(LayerSurface),
    LockScreen(LockSurface),
}

impl From<Window> for KeyboardFocusTarget {
    fn from(window: Window) -> Self {
        Self::Window(window)
    }
}

impl From<PopupKind> for KeyboardFocusTarget {
    fn from(popup: PopupKind) -> Self {
        Self::Popup(popup)
    }
}

impl From<LayerSurface> for KeyboardFocusTarget {
    fn from(layer: LayerSurface) -> Self {
        Self::LayerShell(layer)
    }
}

impl From<LockSurface> for KeyboardFocusTarget {
    fn from(lock: LockSurface) -> Self {
        Self::LockScreen(lock)
    }
}

impl IsAlive for KeyboardFocusTarget {
    fn alive(&self) -> bool {
        match self {
            Self::Window(window) => window.alive(),
            Self::Popup(popup) => popup.alive(),
            Self::LayerShell(layer) => layer.alive(),
            Self::LockScreen(lock) => lock.alive(),
        }
    }
}

impl WaylandFocus for KeyboardFocusTarget {
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        match self {
            Self::Window(window) => window.wl_surface(),
            Self::Popup(popup) => Some(Cow::Borrowed(popup.wl_surface())),
            Self::LayerShell(layer) => Some(Cow::Borrowed(layer.wl_surface())),
            Self::LockScreen(lock) => Some(Cow::Borrowed(lock.wl_surface())),
        }
    }

    fn same_client_as(&self, object_id: &ObjectId) -> bool {
        self.wl_surface()
            .is_some_and(|surface| surface.id().same_client_as(object_id))
    }
}

/// Delegates to smithay's `impl KeyboardTarget for WlSurface`, which is what
/// actually emits `wl_keyboard.enter/key/leave/modifiers`.
///
/// `replace` is left to the trait default, which already does leave-old,
/// enter-new, modifiers in that order.
impl KeyboardTarget<State> for KeyboardFocusTarget {
    fn enter(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        keys: Vec<KeysymHandle<'_>>,
        serial: Serial,
    ) {
        if let Some(surface) = self.wl_surface() {
            KeyboardTarget::enter(surface.as_ref(), seat, data, keys, serial);
        }
    }

    fn leave(&self, seat: &Seat<State>, data: &mut State, serial: Serial) {
        if let Some(surface) = self.wl_surface() {
            KeyboardTarget::leave(surface.as_ref(), seat, data, serial);
        }
    }

    fn key(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        key: KeysymHandle<'_>,
        state: KeyState,
        serial: Serial,
        time: u32,
    ) {
        if let Some(surface) = self.wl_surface() {
            KeyboardTarget::key(surface.as_ref(), seat, data, key, state, serial, time);
        }
    }

    fn modifiers(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        modifiers: ModifiersState,
        serial: Serial,
    ) {
        if let Some(surface) = self.wl_surface() {
            KeyboardTarget::modifiers(surface.as_ref(), seat, data, modifiers, serial);
        }
    }
}
