//! What the lock screen is doing, one input at a time.
//!
//! The flow never authenticates anything: it hands the typed password out
//! once, in [`Response::Authenticate`], and only [`LockFlow::finish`] with
//! [`AuthOutcome::Accepted`] moves it toward unlocking.

mod notice;

use std::time::{Duration, Instant};

use crownos_config::schema::Lock;

use crate::{
    secret::Password,
    transition::{Moments, SpringValue},
};

pub use crate::transition::Frame;
pub use notice::Notice;

const BACKOFF_FREE_FAILURES: u32 = 2;
const BACKOFF_CAP: Duration = Duration::from_secs(30);
const SHAKE: Duration = Duration::from_millis(420);
const SHAKE_DISTANCE: f32 = 10.0;
const SHAKE_SWINGS: f32 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyInput {
    Char(char),
    Backspace,
    Clear,
    Submit,
    Cancel,
}

#[must_use]
#[derive(Debug)]
pub enum Response {
    Ignored,
    Redraw,
    Authenticate(Password),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthOutcome {
    Accepted,
    Rejected,
}

pub struct LockFlow {
    prompt_timeout: Option<Duration>,
    appear: SpringValue,
    /// Toward 1 while the prompt is up, toward 0 while the clock is.
    reveal: SpringValue,
    dismiss: Option<SpringValue>,
    checking: bool,
    /// Dots shown for the password being checked or accepted, which has
    /// already left this flow.
    submitted_chars: usize,
    password: Password,
    notice: Notice,
    failures: u32,
    retry_at: Option<Instant>,
    shaken: Option<Instant>,
    last_input: Instant,
}

impl LockFlow {
    pub fn new(config: &Lock, now: Instant) -> Self {
        let mut flow = Self {
            prompt_timeout: None,
            appear: SpringValue::moving(0.0, 1.0, now),
            reveal: SpringValue::resting(0.0, now),
            dismiss: None,
            checking: false,
            submitted_chars: 0,
            password: Password::new(),
            notice: Notice::None,
            failures: 0,
            retry_at: None,
            shaken: None,
            last_input: now,
        };
        flow.configure(config);
        flow
    }

    /// Locked already, with no entrance: for locking on the way to suspend,
    /// where nothing may still be animating in when the screen goes dark.
    pub fn settled(config: &Lock, now: Instant) -> Self {
        Self {
            appear: SpringValue::resting(1.0, now),
            ..Self::new(config, now)
        }
    }

    pub fn configure(&mut self, config: &Lock) {
        self.prompt_timeout = (config.prompt_timeout_secs > 0)
            .then(|| Duration::from_secs(config.prompt_timeout_secs.into()));
    }

    pub fn key(&mut self, input: KeyInput, now: Instant) -> Response {
        if self.dismiss.is_some() || self.checking {
            return Response::Ignored;
        }
        self.last_input = now;
        let was_revealed = self.is_prompt_up();
        self.show_prompt(true, now);
        if self.is_waiting(now) {
            self.notice = Notice::Wait {
                until: self.retry_at.unwrap_or(now),
            };
            return Response::Redraw;
        }
        match input {
            KeyInput::Char(c) => {
                self.password.push(c);
                self.clear_stale_notice();
            }
            KeyInput::Backspace => {
                self.password.pop();
            }
            KeyInput::Clear => self.password.clear(),
            KeyInput::Cancel if self.password.is_empty() && was_revealed => {
                self.show_prompt(false, now);
            }
            KeyInput::Cancel => self.password.clear(),
            KeyInput::Submit if self.password.is_empty() => {}
            KeyInput::Submit => {
                self.checking = true;
                self.submitted_chars = self.password.chars();
                self.notice = Notice::Checking;
                return Response::Authenticate(self.password.take());
            }
        }
        Response::Redraw
    }

    /// Any pointer activity: brings the prompt up.
    pub fn pointer(&mut self, now: Instant) -> Response {
        if self.dismiss.is_some() {
            return Response::Ignored;
        }
        self.last_input = now;
        if self.is_prompt_up() {
            return Response::Ignored;
        }
        self.show_prompt(true, now);
        Response::Redraw
    }

    pub fn service_message(&mut self, text: &str) {
        if self.dismiss.is_none() {
            self.notice = Notice::service(text);
        }
    }

    pub fn finish(&mut self, outcome: AuthOutcome, now: Instant) {
        self.checking = false;
        self.last_input = now;
        match outcome {
            AuthOutcome::Accepted => {
                self.notice = Notice::None;
                self.password.clear();
                self.dismiss = Some(SpringValue::moving(0.0, 1.0, now));
            }
            AuthOutcome::Rejected => {
                self.failures = self.failures.saturating_add(1);
                self.shaken = Some(now);
                self.retry_at = backoff(self.failures).map(|wait| now + wait);
                self.notice = self
                    .retry_at
                    .map_or(Notice::Rejected, |until| Notice::Wait { until });
            }
        }
    }

    /// Applies what time alone changes: the prompt timing out, a wait ending.
    /// Returns whether anything visible changed.
    pub fn poll(&mut self, now: Instant) -> bool {
        let mut changed = false;
        if let Some(retry_at) = self.retry_at
            && now >= retry_at
        {
            self.retry_at = None;
            if matches!(self.notice, Notice::Wait { .. }) {
                self.notice = Notice::Rejected;
            }
            changed = true;
        }
        let idle = self
            .prompt_timeout
            .is_some_and(|timeout| now.saturating_duration_since(self.last_input) >= timeout);
        if idle && self.is_prompt_up() && !self.checking && self.dismiss.is_none() {
            self.password.clear();
            self.clear_stale_notice();
            self.show_prompt(false, now);
            changed = true;
        }
        changed
    }

    pub fn frame(&self, now: Instant) -> Frame {
        Moments {
            appear: self.appear.value(now),
            reveal: self.reveal.value(now),
            dismiss: self.dismiss.map_or(0.0, |dismiss| dismiss.value(now)),
        }
        .frame(self.shake(now))
    }

    /// Whether the next frame differs from this one without any input.
    pub fn is_animating(&self, now: Instant) -> bool {
        !self.appear.is_settled(now)
            || !self.reveal.is_settled(now)
            || self.dismiss.is_some_and(|dismiss| !dismiss.is_settled(now))
            || self
                .shaken
                .is_some_and(|since| now.saturating_duration_since(since) < SHAKE)
    }

    /// When [`poll`](Self::poll) next has something to do, if ever.
    pub fn next_deadline(&self) -> Option<Instant> {
        let timeout = self
            .prompt_timeout
            .filter(|_| self.is_prompt_up() && self.dismiss.is_none())
            .map(|timeout| self.last_input + timeout);
        [timeout, self.retry_at].into_iter().flatten().min()
    }

    /// Unlocked, and done animating out: the host can drop the lock.
    pub fn is_finished(&self, now: Instant) -> bool {
        self.dismiss.is_some_and(|dismiss| dismiss.is_settled(now))
    }

    pub fn is_unlocking(&self) -> bool {
        self.dismiss.is_some()
    }

    pub fn is_checking(&self) -> bool {
        self.checking
    }

    pub fn password_chars(&self) -> usize {
        if self.checking || self.dismiss.is_some() {
            self.submitted_chars
        } else {
            self.password.chars()
        }
    }

    pub fn notice(&self) -> &Notice {
        &self.notice
    }

    fn is_prompt_up(&self) -> bool {
        self.reveal.target() > 0.0
    }

    fn show_prompt(&mut self, shown: bool, now: Instant) {
        if shown != self.is_prompt_up() {
            self.reveal.retarget(if shown { 1.0 } else { 0.0 }, now);
        }
    }

    fn is_waiting(&self, now: Instant) -> bool {
        self.retry_at.is_some_and(|until| now < until)
    }

    fn clear_stale_notice(&mut self) {
        if !matches!(self.notice, Notice::Wait { .. } | Notice::Checking) {
            self.notice = Notice::None;
        }
    }

    fn shake(&self, now: Instant) -> f32 {
        let Some(since) = self.shaken else {
            return 0.0;
        };
        let t = now.saturating_duration_since(since).as_secs_f32() / SHAKE.as_secs_f32();
        if t >= 1.0 {
            return 0.0;
        }
        SHAKE_DISTANCE * (1.0 - t) * (t * SHAKE_SWINGS * std::f32::consts::TAU).sin()
    }
}

fn backoff(failures: u32) -> Option<Duration> {
    let extra = failures
        .checked_sub(BACKOFF_FREE_FAILURES)
        .filter(|&n| n > 0)?;
    let seconds = 1u64.checked_shl(extra).unwrap_or(u64::MAX);
    Some(Duration::from_secs(seconds).min(BACKOFF_CAP))
}

#[cfg(test)]
mod tests;
