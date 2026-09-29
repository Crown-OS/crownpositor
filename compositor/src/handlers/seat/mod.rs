mod keyboard_target;
mod pointer_target;

use smithay::{
    backend::input::TabletToolDescriptor,
    input::{
        Seat, SeatHandler, SeatState, keyboard::LedState, pointer::CursorImageStatus,
        tablet::TabletSeatHandler,
    },
    reexports::wayland_server::{Resource, protocol::wl_surface::WlSurface},
    wayland::{
        seat::WaylandFocus,
        selection::{data_device::set_data_device_focus, primary_selection::set_primary_focus},
    },
};

use crate::state::State;
pub use keyboard_target::KeyboardFocusTarget;
pub use pointer_target::PointerFocusTarget;

impl SeatHandler for State {
    type KeyboardFocus = KeyboardFocusTarget;
    type PointerFocus = PointerFocusTarget;
    // TODO: nothing feeds touch yet. When it does it wants the pointer's target,
    // not a bare surface: a tap has the same "which window did that land in?"
    // question to answer as a click.
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

    /// Records which window holds focus and hands the clipboard and primary
    /// selection to its client: `wl_data_device` and primary-selection offers
    /// only ever go to the keyboard-focused client. The `Activated` state and
    /// its configure are `Shell::refresh`'s job, so exactly one pass decides
    /// what every window is told.
    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&Self::KeyboardFocus>) {
        self.shell.activated = match focused {
            Some(KeyboardFocusTarget::Window(window)) => Some(window.clone()),
            _ => None,
        };

        let display_handle = &self.common.display_handle;
        let client = focused
            .and_then(WaylandFocus::wl_surface)
            .and_then(|surface| display_handle.get_client(surface.id()).ok());
        set_data_device_focus(display_handle, seat, client.clone());
        set_primary_focus(display_handle, seat, client);
    }

    fn led_state_changed(&mut self, _seat: &Seat<Self>, _led_state: LedState) {}
}

impl TabletSeatHandler for State {
    type ToolFocus = WlSurface;

    /// The pen moves the one cursor there is, so its image is the cursor's.
    fn tablet_tool_image(&mut self, _tool: &TabletToolDescriptor, image: CursorImageStatus) {
        if self.input.cursor.status == image {
            return;
        }
        self.input.cursor.status = image;
        self.queue_pointer_redraw();
    }
}
