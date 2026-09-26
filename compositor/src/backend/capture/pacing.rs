//! The two bits of arithmetic a capture session is paced by: how old a slot's
//! contents are, and when the frame-rate cap next allows a frame.

use std::time::Duration;

use protocols::crownos_screencast::MAX_SLOTS;

/// Which frame each slot last received, so a render into a slot can be damage
/// tracked against what that slot actually holds rather than against the
/// previous frame.
#[derive(Debug, Default, Clone)]
pub struct SlotAges {
    frames: u64,
    last_frame: [Option<u64>; MAX_SLOTS],
}

impl SlotAges {
    /// Buffer age in the sense of `OutputDamageTracker::render_output`: 1 is
    /// "holds the previous frame", 0 is "holds nothing usable".
    pub fn age(&self, index: usize) -> usize {
        self.last_frame
            .get(index)
            .copied()
            .flatten()
            .and_then(|frame| usize::try_from(self.frames + 1 - frame).ok())
            .unwrap_or(0)
    }

    pub fn rendered(&mut self, index: usize) {
        self.frames += 1;
        if let Some(slot) = self.last_frame.get_mut(index) {
            *slot = Some(self.frames);
        }
    }

    pub fn forget(&mut self) {
        *self = Self::default();
    }
}

/// When the next frame may be produced: `None` is now.
pub fn next_frame_at(
    last_frame_at: Option<Duration>,
    interval: Option<Duration>,
    now: Duration,
) -> Option<Duration> {
    let due = last_frame_at? + interval?;
    (due > now).then_some(due)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_slot_has_no_age() {
        assert_eq!(SlotAges::default().age(0), 0);
    }

    #[test]
    fn ages_follow_the_ring() {
        let mut ages = SlotAges::default();
        ages.rendered(0);
        assert_eq!(ages.age(0), 1, "slot 0 holds the previous frame");
        assert_eq!(ages.age(1), 0);

        ages.rendered(1);
        ages.rendered(2);
        assert_eq!(ages.age(0), 3);
        assert_eq!(ages.age(1), 2);
        assert_eq!(ages.age(2), 1);

        ages.forget();
        assert_eq!(ages.age(2), 0);
    }

    #[test]
    fn out_of_range_slots_are_ageless() {
        let mut ages = SlotAges::default();
        ages.rendered(MAX_SLOTS);
        assert_eq!(ages.age(MAX_SLOTS), 0);
    }

    #[test]
    fn the_cap_only_delays_frames_that_come_too_soon() {
        let interval = Some(Duration::from_millis(33));
        let now = Duration::from_millis(1000);

        assert_eq!(next_frame_at(None, interval, now), None);
        assert_eq!(next_frame_at(Some(now), None, now), None);
        assert_eq!(
            next_frame_at(Some(Duration::from_millis(990)), interval, now),
            Some(Duration::from_millis(1023))
        );
        assert_eq!(
            next_frame_at(Some(Duration::from_millis(900)), interval, now),
            None
        );
    }
}
