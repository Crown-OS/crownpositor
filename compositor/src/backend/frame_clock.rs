//! Per-output frame pacing, fed by real presentation timestamps.
//!
//! The model (borrowed from niri): every vblank the DRM driver reports a
//! presentation time. Given that anchor and the mode's refresh interval, the
//! next presentation instant is the anchor plus however many whole intervals
//! have elapsed, plus one. Rendering then targets *that* instant — animations
//! advance to where they will be **when the frame lights up**, not where they
//! were when the CPU happened to run. That difference is what separates
//! "smooth" from "almost smooth".
//!
//! Backends without hardware feedback (winit, an output whose last frame had
//! no damage) still work: they simply never call [`FrameClock::presented`],
//! and [`FrameClock::next_presentation_time`] degrades to "now".

use std::{num::NonZeroU64, time::Duration};

use smithay::utils::{Clock, Monotonic};

#[derive(Debug)]
pub struct FrameClock {
    clock: Clock<Monotonic>,
    /// Timestamp of the most recent hardware presentation, on the monotonic
    /// clock. `None` until the first vblank arrives.
    last_presentation_time: Option<Duration>,
    /// The output's refresh interval. `None` means the interval is unknown
    /// (virtual output) and pacing degrades to immediate.
    refresh_interval_ns: Option<NonZeroU64>,
    /// Variable refresh rate: when a frame is late, present immediately
    /// instead of waiting out the rest of the fixed interval.
    vrr: bool,
}

impl FrameClock {
    pub fn new(refresh_interval: Option<Duration>, vrr: bool) -> Self {
        // Sub-second cast is safe for any real display (a 1 Hz mode would be
        // the first to break it, and even that fits in u64 nanoseconds).
        let refresh_interval_ns = refresh_interval
            .map(|interval| interval.as_nanos().min(u128::from(u64::MAX)) as u64)
            .and_then(NonZeroU64::new);

        Self {
            clock: Clock::new(),
            last_presentation_time: None,
            refresh_interval_ns,
            vrr,
        }
    }

    pub fn refresh_interval(&self) -> Option<Duration> {
        self.refresh_interval_ns
            .map(|nanos| Duration::from_nanos(nanos.get()))
    }

    pub fn vrr(&self) -> bool {
        self.vrr
    }

    pub fn set_vrr(&mut self, vrr: bool) {
        if self.vrr != vrr {
            self.vrr = vrr;
            // The old anchor was measured under the other pacing regime.
            self.last_presentation_time = None;
        }
    }

    /// Records a hardware presentation timestamp (monotonic clock).
    ///
    /// Zero means the driver could not say when the frame was shown; keeping
    /// the previous anchor beats poisoning the clock with a fake one.
    pub fn presented(&mut self, presentation_time: Duration) {
        if !presentation_time.is_zero() {
            self.last_presentation_time = Some(presentation_time);
        }
    }

    /// The instant the *next* frame will most likely reach the screen.
    ///
    /// This is what animations should be advanced to and what the estimated
    /// vblank timer should fire at.
    pub fn next_presentation_time(&self) -> Duration {
        let mut now: Duration = self.clock.now().into();

        let (Some(refresh_interval_ns), Some(last_presentation_time)) =
            (self.refresh_interval_ns, self.last_presentation_time)
        else {
            // No anchor or no known interval: "as soon as possible".
            return now;
        };
        let refresh_interval_ns = refresh_interval_ns.get();

        if now <= last_presentation_time {
            // An early vblank: the driver reported a presentation slightly in
            // the future. Step forward one interval so the subtraction below
            // stays meaningful.
            now += Duration::from_nanos(refresh_interval_ns);

            if now < last_presentation_time {
                // More than one interval early should not happen; re-anchor
                // rather than underflow.
                tracing::warn!(
                    ?now,
                    ?last_presentation_time,
                    "got a vblank more than one refresh interval early"
                );
                return last_presentation_time + Duration::from_nanos(refresh_interval_ns);
            }
        }

        let since_last = now - last_presentation_time;
        let since_last_ns =
            since_last.as_secs() * 1_000_000_000 + u64::from(since_last.subsec_nanos());
        // Round *up* to the next multiple of the refresh interval.
        let to_next_ns = (since_last_ns / refresh_interval_ns + 1) * refresh_interval_ns;

        if self.vrr && to_next_ns > refresh_interval_ns {
            // Missed at least one fixed slot, but an adaptive-sync panel can
            // light up as soon as we hand it a frame.
            now
        } else {
            last_presentation_time + Duration::from_nanos(to_next_ns)
        }
    }

    /// Current time on the same clock the presentation timestamps use.
    pub fn now(&self) -> Duration {
        self.clock.now().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIXTY_HZ: Duration = Duration::from_nanos(16_666_667);

    #[test]
    fn without_feedback_the_clock_degrades_to_now() {
        let clock = FrameClock::new(Some(SIXTY_HZ), false);
        let before = clock.now();
        let next = clock.next_presentation_time();
        // "Now", not some interval-aligned point in the future.
        assert!(next >= before);
        assert!(next - before < SIXTY_HZ);
    }

    #[test]
    fn presentations_anchor_the_next_slot() {
        let mut clock = FrameClock::new(Some(SIXTY_HZ), false);
        let now = clock.now();
        clock.presented(now);

        let next = clock.next_presentation_time();
        // Exactly one interval after the anchor (we query immediately after).
        assert_eq!(next, now + SIXTY_HZ);
    }

    #[test]
    fn zero_presentation_time_is_ignored() {
        let mut clock = FrameClock::new(Some(SIXTY_HZ), false);
        let now = clock.now();
        clock.presented(now);
        clock.presented(Duration::ZERO);

        // Still anchored on the real timestamp.
        assert_eq!(clock.next_presentation_time(), now + SIXTY_HZ);
    }

    #[test]
    fn vrr_presents_late_frames_immediately() {
        let mut clock = FrameClock::new(Some(SIXTY_HZ), true);
        let now = clock.now();
        // Anchor two-and-a-bit intervals in the past: a missed frame.
        clock.presented(now - SIXTY_HZ * 2 - Duration::from_millis(1));

        let next = clock.next_presentation_time();
        // An adaptive-sync panel does not wait for the next fixed slot.
        assert!(next < now + SIXTY_HZ);
    }
}
