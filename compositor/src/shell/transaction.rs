//! Grouped configures.
//!
//! A relayout hands several windows a new size at once. Each client acks and
//! redraws whenever it can, so without a barrier a five-window reflow arrives as
//! five separate resizes. A transaction records who was asked and waits for all
//! of them — with a deadline, because a misbehaving client must never be able to
//! stall the compositor.

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use smithay::utils::Serial;

use crate::utils::id::WindowId;

#[derive(Debug)]
pub struct Transaction {
    /// Windows still to ack, and the serial each must reach.
    pending: HashMap<WindowId, Serial>,
    deadline: Instant,
}

impl Transaction {
    /// Long enough for a slow client to redraw, short enough that a hung one is
    /// not noticeable.
    pub const TIMEOUT: Duration = Duration::from_millis(300);

    pub fn new(pending: HashMap<WindowId, Serial>, now: Instant) -> Self {
        Self {
            pending,
            deadline: now + Self::TIMEOUT,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn expired(&self, now: Instant) -> bool {
        now >= self.deadline
    }

    /// Records an ack. A client may ack a *later* serial than the one we are
    /// waiting on — it configured again in between — so anything at or past the
    /// expected serial counts.
    pub fn ack(&mut self, id: WindowId, serial: Serial) {
        if self
            .pending
            .get(&id)
            .is_some_and(|expected| serial >= *expected)
        {
            self.pending.remove(&id);
        }
    }

    /// Stops waiting on a window that no longer exists.
    pub fn forget(&mut self, id: WindowId) {
        self.pending.remove(&id);
    }

    pub fn waiting_on(&self) -> impl Iterator<Item = WindowId> + '_ {
        self.pending.keys().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn serial(n: u32) -> Serial {
        Serial::from(n)
    }

    fn transaction(now: Instant) -> Transaction {
        let mut pending = HashMap::new();
        pending.insert(WindowId::next(), serial(5));
        Transaction::new(pending, now)
    }

    #[test]
    fn a_matching_ack_completes_it() {
        let now = Instant::now();
        let mut tx = transaction(now);
        let id = tx.waiting_on().next().unwrap();

        assert!(!tx.is_empty());
        tx.ack(id, serial(5));
        assert!(tx.is_empty());
    }

    #[test]
    fn a_later_serial_also_counts() {
        // The client was configured again before it got round to acking.
        let mut tx = transaction(Instant::now());
        let id = tx.waiting_on().next().unwrap();
        tx.ack(id, serial(9));
        assert!(tx.is_empty());
    }

    #[test]
    fn a_stale_serial_does_not() {
        let mut tx = transaction(Instant::now());
        let id = tx.waiting_on().next().unwrap();
        tx.ack(id, serial(2));
        assert!(!tx.is_empty(), "an older ack is not the one we asked for");
    }

    #[test]
    fn an_ack_from_an_unrelated_window_is_ignored() {
        let mut tx = transaction(Instant::now());
        tx.ack(WindowId::next(), serial(5));
        assert!(!tx.is_empty());
    }

    #[test]
    fn a_closed_window_stops_being_waited_on() {
        let mut tx = transaction(Instant::now());
        let id = tx.waiting_on().next().unwrap();
        tx.forget(id);
        assert!(tx.is_empty(), "a dead client can never ack");
    }

    #[test]
    fn it_expires_so_a_hung_client_cannot_stall_the_compositor() {
        let now = Instant::now();
        let tx = transaction(now);
        assert!(!tx.expired(now));
        assert!(tx.expired(now + Transaction::TIMEOUT));
    }
}
