//! Locking after `lock.idle_lock_after_secs` without input, unless something
//! on screen inhibits idling — a video playing, a presentation.

use std::time::{Duration, Instant};

use calloop::{
    RegistrationToken,
    timer::{TimeoutAction, Timer},
};

use crate::state::{State, lock_screen::LockReason};

pub struct IdleLock {
    last_activity: Instant,
    timer: Option<RegistrationToken>,
}

impl Default for IdleLock {
    fn default() -> Self {
        Self {
            last_activity: Instant::now(),
            timer: None,
        }
    }
}

impl State {
    /// Any input from the user: the idle clock starts over. Cheap enough for
    /// every event — the timer is not touched.
    pub(crate) fn note_user_activity(&mut self) {
        self.lock.idle.last_activity = Instant::now();
        let seat = self.wayland.seat.clone();
        self.wayland.idle_notifier_state.notify_activity(&seat);
    }

    pub(crate) fn schedule_idle_lock(&mut self) {
        if let Some(token) = self.lock.idle.timer.take() {
            self.common.event_loop_handle.remove(token);
        }
        let Some(after) = self.idle_lock_after() else {
            return;
        };
        if self.shell.lock_screen.is_active() {
            return;
        }
        let due = self.lock.idle.last_activity + after;
        let timer = Timer::from_duration(due.saturating_duration_since(Instant::now()));
        match self
            .common
            .event_loop_handle
            .insert_source(timer, |_, _, state| {
                state.lock.idle.timer = None;
                state.on_idle_timer();
                TimeoutAction::Drop
            }) {
            Ok(token) => self.lock.idle.timer = Some(token),
            Err(err) => tracing::warn!(%err, "idle lock timer not armed"),
        }
    }

    fn on_idle_timer(&mut self) {
        let Some(after) = self.idle_lock_after() else {
            return;
        };
        let idle = self.lock.idle.last_activity.elapsed() >= after;
        let inhibited = self.wayland.idle_notifier_state.is_inhibited();
        if idle && !inhibited {
            self.lock_screen(LockReason::Idle);
        } else if inhibited {
            self.lock.idle.last_activity = Instant::now();
        }
        self.schedule_idle_lock();
    }

    fn idle_lock_after(&self) -> Option<Duration> {
        let seconds = self.lock.scene.current().config.idle_lock_after_secs;
        (seconds > 0).then(|| Duration::from_secs(seconds.into()))
    }
}
