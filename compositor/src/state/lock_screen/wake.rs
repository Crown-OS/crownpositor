//! One timer for everything time alone changes on the lock screen: the clock
//! turning over, the prompt timing out, a backoff ending.

use std::time::Instant;

use calloop::timer::{TimeoutAction, Timer};
use lockscreen::clock::{ClockFace, next_change_in};

use crate::state::State;

impl State {
    /// Re-arms the timer for the earliest thing due.
    pub(crate) fn schedule_lock_wake(&mut self) {
        if let Some(token) = self.lock.wake.take() {
            self.common.event_loop_handle.remove(token);
        }
        let Some(lock) = self.shell.lock_screen.active() else {
            return;
        };
        let now = Instant::now();
        let clock = now + next_change_in(&self.lock.scene.current().config);
        let due = lock
            .flow
            .next_deadline()
            .map_or(clock, |deadline| deadline.min(clock));
        let timer = Timer::from_duration(due.saturating_duration_since(now));
        match self
            .common
            .event_loop_handle
            .insert_source(timer, |_, _, state| {
                state.lock.wake = None;
                state.on_lock_wake();
                TimeoutAction::Drop
            }) {
            Ok(token) => self.lock.wake = Some(token),
            Err(err) => tracing::warn!(%err, "lock screen timer not armed"),
        }
    }

    fn on_lock_wake(&mut self) {
        let face = ClockFace::now(&self.lock.scene.current().config);
        if face != self.lock.face {
            self.lock.face = face;
            self.mark_lock_dirty();
        }
        if let Some(lock) = self.shell.lock_screen.active_mut()
            && lock.flow.poll(Instant::now())
        {
            lock.dirty = true;
            self.queue_redraw();
        }
        self.schedule_lock_wake();
    }
}
