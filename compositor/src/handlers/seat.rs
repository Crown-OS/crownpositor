use smithay::{
    delegate_cursor_shape, delegate_seat,
    desktop::LayerSurface,
    input::{
        keyboard::{KeyboardTarget, LedState},
        pointer::CursorImageStatus,
        Seat, SeatHandler, SeatState,
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::IsAlive,
    wayland::{
        seat::WaylandFocus, session_lock::LockSurface, shell::xdg::PopupSurface,
        tablet_manager::TabletSeatHandler,
    },
};

use crate::state::State;

#[derive(Debug, Clone, PartialEq)]
pub enum KeyboardFocusTarget {
    Window(WlSurface),
    Popup(PopupSurface),
    LayerShell(LayerSurface),
    LockScreen(LockSurface),
    None,
}

impl From<Option<WlSurface>> for KeyboardFocusTarget {
    fn from(surface: Option<WlSurface>) -> Self {
        match surface {
            Some(surface) => KeyboardFocusTarget::Window(surface),
            None => KeyboardFocusTarget::None,
        }
    }
}

impl IsAlive for KeyboardFocusTarget {
    fn alive(&self) -> bool {
        // TODO: Implement this
        true
    }
}

// TODO: Implement this
impl KeyboardTarget<State> for KeyboardFocusTarget {
    fn key(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        key: smithay::input::keyboard::KeysymHandle<'_>,
        state: smithay::backend::input::KeyState,
        serial: smithay::utils::Serial,
        time: u32,
    ) {
    }

    fn enter(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        keys: Vec<smithay::input::keyboard::KeysymHandle<'_>>,
        serial: smithay::utils::Serial,
    ) {
    }
    fn leave(&self, seat: &Seat<State>, data: &mut State, serial: smithay::utils::Serial) {}

    fn replace(
        &self,
        replaced: <State as SeatHandler>::KeyboardFocus,
        seat: &Seat<State>,
        data: &mut State,
        keys: Vec<smithay::input::keyboard::KeysymHandle<'_>>,
        modifiers: smithay::input::keyboard::ModifiersState,
        serial: smithay::utils::Serial,
    ) {
    }

    fn modifiers(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        modifiers: smithay::input::keyboard::ModifiersState,
        serial: smithay::utils::Serial,
    ) {
    }
}

impl WaylandFocus for KeyboardFocusTarget {
    fn wl_surface(
        &self,
    ) -> Option<std::borrow::Cow<'_, wayland_server::protocol::wl_surface::WlSurface>> {
        // TODO: Implement this
        None
    }

    fn same_client_as(&self, object_id: &wayland_server::backend::ObjectId) -> bool {
        // TODO: Implement this
        false
    }
}

impl SeatHandler for State {
    // TODO: replace with focus targets covering popups, layer surfaces and X11 windows.
    type KeyboardFocus = KeyboardFocusTarget;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.wayland.seat_state
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, _image: CursorImageStatus) {}

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&Self::KeyboardFocus>) {}

    fn led_state_changed(&mut self, _seat: &Seat<Self>, _led_state: LedState) {}
}

// TODO: Implement this
impl TabletSeatHandler for State {}

delegate_seat!(State);
delegate_cursor_shape!(State);
