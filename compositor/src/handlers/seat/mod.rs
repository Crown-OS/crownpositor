mod keyboard_target;
// mod pointer_target;

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
pub use keyboard_target::KeyboardFocusTarget;

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
