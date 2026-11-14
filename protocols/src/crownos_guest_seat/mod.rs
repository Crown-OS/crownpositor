//! Server-side `crownos_guest_seat_v1`: a seat of a remote desktop guest's own.
//!
//! A guest seat buffers its input requests until `frame` and hands them to the compositor as
//! one [`CursorFrame`], the same frame an agent cursor produces, so one piece of the
//! compositor drives both. Which output confines the seat is the compositor's business through
//! [`GuestSeatHandler`]; this module only validates and routes.

mod dispatch;

use std::sync::Mutex;

use crownos_protocols::guest_seat::v1::server::{
    crownos_guest_seat_manager_v1::CrownosGuestSeatManagerV1,
    crownos_guest_seat_v1::CrownosGuestSeatV1,
};
use wayland_server::{Client, DisplayHandle, backend::GlobalId};

pub use crownos_protocols::guest_seat::v1::server::crownos_guest_seat_v1::RevokeReason;
pub use dispatch::GuestSeatDispatch;

use crate::crownos_agent_cursor::{CursorEvent, CursorFrame, is_valid_name};

/// The protocol version this module speaks.
pub const VERSION: u32 = 1;

/// What the compositor provides for this protocol to be delegated to [`GuestSeatState`].
pub trait GuestSeatHandler {
    fn guest_seat_state(&mut self) -> &mut GuestSeatState;
    /// Creates the seat behind `guest`, confined to `output`, answering with its `wl_seat`
    /// name, or `None` when it cannot be made.
    fn create_guest_seat(&mut self, guest: GuestSeat, name: &str, output: &str) -> Option<String>;
    fn guest_seat_frame(&mut self, guest: &GuestSeat, frame: CursorFrame);
    /// The client destroyed the seat or went away; the seat goes with it.
    fn guest_seat_destroyed(&mut self, guest: &GuestSeat);
}

/// Global data: which clients may see the manager.
pub struct GuestSeatGlobalData {
    filter: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

/// User data of a `crownos_guest_seat_v1`: the frame being built.
#[derive(Debug, Default)]
pub struct GuestSeatData {
    pending: Mutex<Vec<CursorEvent>>,
}

impl GuestSeatData {
    fn push(&self, event: CursorEvent) {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event);
    }

    fn take(&self) -> Vec<CursorEvent> {
        std::mem::take(
            &mut self
                .pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }
}

/// The compositor's handle on one guest seat, through which its events go out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestSeat(CrownosGuestSeatV1);

impl GuestSeat {
    pub fn created(&self, seat: &str) {
        self.0.created(seat.to_owned());
    }

    pub fn revoked(&self, reason: RevokeReason) {
        self.0.revoked(reason);
    }
}

/// Delegate type for the `crownos_guest_seat_manager_v1` global.
pub struct GuestSeatState {
    global: GlobalId,
    guests: Vec<GuestSeat>,
}

impl GuestSeatState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: GuestSeatDispatch,
        F: Fn(&Client) -> bool + Send + Sync + 'static,
    {
        let global = display.create_global::<D, CrownosGuestSeatManagerV1, _>(
            VERSION,
            GuestSeatGlobalData {
                filter: Box::new(filter),
            },
        );
        Self {
            global,
            guests: Vec::new(),
        }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }

    /// Tells one seat it is gone and forgets it; the compositor tears down the seat itself.
    pub fn revoke(&mut self, guest: &GuestSeat, reason: RevokeReason) {
        if self.forget(guest) {
            guest.revoked(reason);
        }
    }

    fn record(&mut self, guest: GuestSeat) {
        self.guests.push(guest);
    }

    fn forget(&mut self, guest: &GuestSeat) -> bool {
        let before = self.guests.len();
        self.guests.retain(|known| known != guest);
        self.guests.len() != before
    }
}
