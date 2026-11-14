//! The seats behind `crownos_guest_seat_v1`: one pointer and keyboard per remote desktop
//! guest, confined to the virtual output their session shows. The guest sees their own
//! pointer over the picture, so nothing is drawn for the seat here.

use protocols::{
    crownos_agent_cursor::CursorFrame,
    crownos_guest_seat::{GuestSeat, RevokeReason},
};

use crate::state::{
    State,
    driven_seat::{Confinement, DrivenSeat},
};

const AUDIT: &str = "crownpositor::audit::guest_seat";
/// One seat per guest, and a host has few guests at once.
const MAX_GUESTS: usize = 8;

pub struct GuestSeatEntry {
    guest: GuestSeat,
    driven: DrivenSeat,
    name: String,
    /// The `wl_output` name of the output the seat is confined to.
    output: String,
}

#[derive(Default)]
pub struct GuestSeats {
    seats: Vec<GuestSeatEntry>,
    created: u32,
}

impl GuestSeats {
    fn index_of(&self, guest: &GuestSeat) -> Option<usize> {
        self.seats.iter().position(|seat| seat.guest == *guest)
    }
}

impl State {
    pub fn create_guest_seat(
        &mut self,
        guest: GuestSeat,
        name: &str,
        output: &str,
    ) -> Option<String> {
        if self.guest_seats.seats.len() >= MAX_GUESTS || self.shell.is_locked() {
            return None;
        }
        let confinement = self.guest_confinement(output)?;
        self.guest_seats.created += 1;
        let seat_name = format!("guest-{}", self.guest_seats.created);
        let driven = self.new_driven_seat(&seat_name, confinement)?;
        self.guest_seats.seats.push(GuestSeatEntry {
            guest,
            driven,
            name: name.to_owned(),
            output: output.to_owned(),
        });
        tracing::info!(target: AUDIT, guest = name, output, seat = %seat_name, "guest seat created");
        Some(seat_name)
    }

    fn guest_confinement(&self, output: &str) -> Option<Confinement> {
        let monitor = self.shell.monitor_by_name(output)?;
        Some(Confinement {
            output: monitor.id(),
            bounds: monitor.geometry(),
        })
    }

    pub fn apply_guest_frame(&mut self, guest: &GuestSeat, frame: CursorFrame) {
        let Some(index) = self.guest_seats.index_of(guest) else {
            return;
        };
        let Some(confinement) = self.guest_confinement(&self.guest_seats.seats[index].output)
        else {
            return;
        };
        // A guest at work is someone at the desktop: the idle lock would end their session.
        if !frame.events.is_empty() && !self.shell.is_locked() {
            self.note_user_activity();
        }
        let mut entry = self.guest_seats.seats.remove(index);
        self.drive_seat(&mut entry.driven, frame, confinement);
        self.guest_seats.seats.insert(index, entry);
    }

    /// Releases whatever the guest held, drops its focus and takes its seat away.
    pub fn destroy_guest_seat(&mut self, guest: &GuestSeat) {
        let Some(index) = self.guest_seats.index_of(guest) else {
            return;
        };
        let mut removed = self.guest_seats.seats.remove(index);
        self.release_driven_seat(&mut removed.driven);
        tracing::info!(target: AUDIT, guest = removed.name, "guest seat destroyed");
    }

    /// A locked session has no guests at work in it, and a seat whose output went away has
    /// nowhere to act.
    pub fn revoke_lost_guests(&mut self) {
        let locked = self.shell.is_locked();
        let lost: Vec<(GuestSeat, RevokeReason)> = self
            .guest_seats
            .seats
            .iter()
            .filter_map(|entry| {
                if locked {
                    Some((entry.guest.clone(), RevokeReason::Locked))
                } else if self.shell.monitor_by_name(&entry.output).is_none() {
                    Some((entry.guest.clone(), RevokeReason::OutputGone))
                } else {
                    None
                }
            })
            .collect();
        for (guest, reason) in lost {
            self.wayland.guest_seat_state.revoke(&guest, reason);
            self.destroy_guest_seat(&guest);
        }
    }
}
