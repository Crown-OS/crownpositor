use smithay::{
    delegate_cursor_shape, delegate_seat,
    input::{Seat, SeatHandler, SeatState, keyboard::LedState, pointer::CursorImageStatus},
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    wayland::tablet_manager::TabletSeatHandler,
};

use crate::state::State;

impl SeatHandler for State {
    // TODO: replace with focus targets covering popups, layer surfaces and X11 windows.
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.wayland.seat_state
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, _image: CursorImageStatus) {}

    fn focus_changed(&mut self, _seat: &Seat<Self>, _focused: Option<&Self::KeyboardFocus>) {}

    fn led_state_changed(&mut self, _seat: &Seat<Self>, _led_state: LedState) {}
}

impl TabletSeatHandler for State {}

delegate_seat!(State);
delegate_cursor_shape!(State);
