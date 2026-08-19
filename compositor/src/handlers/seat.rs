use std::borrow::Cow;

use smithay::{
    backend::input::KeyState,
    delegate_cursor_shape, delegate_seat,
    desktop::{LayerSurface, PopupKind, Window},
    input::{
        keyboard::{KeyboardTarget, KeysymHandle, LedState, ModifiersState},
        pointer::CursorImageStatus,
        Seat, SeatHandler, SeatState,
    },
    reexports::wayland_server::{backend::ObjectId, protocol::wl_surface::WlSurface, Resource},
    utils::{IsAlive, Serial},
    wayland::{seat::WaylandFocus, session_lock::LockSurface, tablet_manager::TabletSeatHandler},
};

use crate::state::State;

/// Whatever can hold a seat's keyboard focus.
///
/// No `None` variant: `Option<KeyboardFocusTarget>` already says that, and a
/// `None` target would report itself alive forever and never be reaped.
#[derive(Debug, Clone, PartialEq)]
pub enum KeyboardFocusTarget {
    /// A `Window`, not a `WlSurface`, so focus changes can `set_activated`
    /// without a shell lookup on every keystroke.
    Window(Window),
    /// `PopupKind` also covers layer-shell popups, which need focus too.
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

impl SeatHandler for State {
    type KeyboardFocus = KeyboardFocusTarget;
    // TODO: needs its own target enum before pointer grabs and X11 surfaces land.
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.wayland.seat_state
    }

    /// A client set the cursor. Nothing else draws it, so this has to reach the
    /// screen: the shape only changes on a client's say-so, and if it does not
    /// force a frame the pointer keeps the previous image until something else
    /// happens to damage the output.
    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        if self.input.cursor.status == image {
            return;
        }
        self.input.cursor.status = image;
        self.queue_pointer_redraw();
    }

    /// Only records which window holds focus. The `Activated` state and its
    /// configure are `Shell::refresh`'s job, so exactly one pass decides what
    /// every window is told.
    fn focus_changed(&mut self, _seat: &Seat<Self>, focused: Option<&Self::KeyboardFocus>) {
        self.shell.activated = match focused {
            Some(KeyboardFocusTarget::Window(window)) => Some(window.clone()),
            _ => None,
        };
    }

    fn led_state_changed(&mut self, _seat: &Seat<Self>, _led_state: LedState) {}
}

// TODO: Implement this
impl TabletSeatHandler for State {}

delegate_seat!(State);
delegate_cursor_shape!(State);
