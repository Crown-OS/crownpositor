//! Notices a display that stops confirming the frames KMS accepted.
//!
//! KMS taking a frame and never flipping it means the driver or the GPU is
//! stuck, not the compositor. Without this, the log of such a session simply
//! stops, exactly like that of a compositor that stopped drawing.
//!
//! The check runs on the event loop, so it cannot speak for a loop that is
//! itself stuck in the first render or the first modeset. Those two steps are
//! logged once per output instead: the last line written says which it was.

use std::time::Duration;

use smithay::{output::Output, reexports::calloop::RegistrationToken};

/// How long a queued frame may go unconfirmed before it is reported.
pub const PATIENCE: Duration = Duration::from_secs(1);

#[derive(Debug)]
pub struct PresentWatch {
    enabled_at: Duration,
    /// When the frame KMS is holding was queued, until its vblank arrives.
    queued_at: Option<Duration>,
    rendered_any: bool,
    queued_any: bool,
    shown_any: bool,
    reported: bool,
    pub timer: Option<RegistrationToken>,
}

impl PresentWatch {
    pub fn new(now: Duration) -> Self {
        Self {
            enabled_at: now,
            queued_at: None,
            rendered_any: false,
            queued_any: false,
            shown_any: false,
            reported: false,
            timer: None,
        }
    }

    pub fn rendering(&mut self, output: &Output) {
        if !std::mem::replace(&mut self.rendered_any, true) {
            tracing::info!(output = output.name(), "rendering the first frame");
        }
    }

    pub fn queued(&mut self, now: Duration, output: &Output) {
        self.queued_at = Some(now);
        if !std::mem::replace(&mut self.queued_any, true) {
            tracing::info!(
                output = output.name(),
                since_enabled_ms = millis(now.saturating_sub(self.enabled_at)),
                "KMS accepted the first frame"
            );
        }
    }

    /// Another VT owns the hardware, so no vblank is owed.
    pub fn forget(&mut self) {
        self.queued_at = None;
    }

    pub fn presented(&mut self, now: Duration, output: &Output) {
        let Some(queued_at) = self.queued_at.take() else {
            return;
        };
        if !self.shown_any {
            self.shown_any = true;
            tracing::info!(
                output = output.name(),
                since_enabled_ms = millis(now.saturating_sub(self.enabled_at)),
                "first frame on screen"
            );
        }
        if std::mem::take(&mut self.reported) {
            tracing::warn!(
                output = output.name(),
                waited_ms = millis(now.saturating_sub(queued_at)),
                "the display confirmed a frame again"
            );
        }
    }

    /// How long until the held frame is overdue, or `None` when there is
    /// nothing left to wait for: no frame is held, or it has been reported.
    #[must_use]
    pub fn check(&mut self, now: Duration, output: &Output) -> Option<Duration> {
        let waited = now.saturating_sub(self.queued_at?);
        if let Some(remaining) = PATIENCE.checked_sub(waited).filter(|left| !left.is_zero()) {
            return Some(remaining);
        }
        if !self.reported {
            self.reported = true;
            tracing::warn!(
                output = output.name(),
                waited_ms = millis(waited),
                first_frame = !self.shown_any,
                "the display has not confirmed a frame KMS accepted"
            );
        }
        None
    }
}

fn millis(duration: Duration) -> u64 {
    duration.as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    fn output() -> Output {
        Output::new(
            "test".into(),
            smithay::output::PhysicalProperties {
                size: (0, 0).into(),
                subpixel: smithay::output::Subpixel::Unknown,
                make: String::new(),
                model: String::new(),
                serial_number: String::new(),
            },
        )
    }

    #[test]
    fn nothing_held_means_nothing_to_wait_for() {
        let mut watch = PresentWatch::new(at(0));
        assert_eq!(watch.check(at(5_000), &output()), None);
    }

    #[test]
    fn a_held_frame_is_rechecked_when_it_would_fall_due() {
        let output = output();
        let mut watch = PresentWatch::new(at(0));
        watch.queued(at(100), &output);
        assert_eq!(watch.check(at(400), &output), Some(at(700)));
    }

    #[test]
    fn an_overdue_frame_is_reported_once_and_cleared_by_its_vblank() {
        let output = output();
        let mut watch = PresentWatch::new(at(0));
        watch.queued(at(0), &output);
        assert_eq!(watch.check(at(1_000), &output), None);
        assert!(watch.reported);
        watch.presented(at(3_000), &output);
        assert!(!watch.reported);
        assert!(watch.shown_any);
        assert_eq!(watch.check(at(9_000), &output), None);
    }

    #[test]
    fn a_vt_switch_owes_no_vblank() {
        let output = output();
        let mut watch = PresentWatch::new(at(0));
        watch.queued(at(0), &output);
        watch.forget();
        assert_eq!(watch.check(at(5_000), &output), None);
        assert!(!watch.reported);
    }

    #[test]
    fn the_first_frame_milestones_are_marked_once() {
        let output = output();
        let mut watch = PresentWatch::new(at(0));
        watch.rendering(&output);
        watch.queued(at(10), &output);
        assert!(watch.rendered_any && watch.queued_any);
        watch.presented(at(20), &output);
        watch.queued(at(30), &output);
        assert_eq!(watch.queued_at, Some(at(30)));
    }
}
