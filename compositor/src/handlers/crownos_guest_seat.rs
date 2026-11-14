//! Delegation glue for `crownos_guest_seat_v1`: the protocol lives in
//! [`protocols::crownos_guest_seat`], the seats in [`crate::state::guest_seats`].

use crownos_protocols::guest_seat::v1::server::{
    crownos_guest_seat_manager_v1::CrownosGuestSeatManagerV1,
    crownos_guest_seat_v1::CrownosGuestSeatV1,
};
use protocols::{
    crownos_agent_cursor::CursorFrame,
    crownos_guest_seat::{
        GuestSeat, GuestSeatData, GuestSeatGlobalData, GuestSeatHandler, GuestSeatState,
    },
};
use wayland_server::{delegate_dispatch, delegate_global_dispatch};

use crate::state::State;

impl GuestSeatHandler for State {
    fn guest_seat_state(&mut self) -> &mut GuestSeatState {
        &mut self.wayland.guest_seat_state
    }

    fn create_guest_seat(&mut self, guest: GuestSeat, name: &str, output: &str) -> Option<String> {
        State::create_guest_seat(self, guest, name, output)
    }

    fn guest_seat_frame(&mut self, guest: &GuestSeat, frame: CursorFrame) {
        self.apply_guest_frame(guest, frame);
    }

    fn guest_seat_destroyed(&mut self, guest: &GuestSeat) {
        self.destroy_guest_seat(guest);
    }
}

delegate_global_dispatch!(State: [CrownosGuestSeatManagerV1: GuestSeatGlobalData] => GuestSeatState);
delegate_dispatch!(State: [CrownosGuestSeatManagerV1: ()] => GuestSeatState);
delegate_dispatch!(State: [CrownosGuestSeatV1: GuestSeatData] => GuestSeatState);
