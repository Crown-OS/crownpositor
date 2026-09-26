//! The two timelines of an explicitly synchronised session: the compositor
//! signals frames on the acquire timeline, the client signals releases on the
//! release timeline, and neither ever signals the other's.

use std::{
    io,
    os::fd::{BorrowedFd, OwnedFd},
};

use smithay::backend::drm::DrmDeviceFd;

use crate::utils::syncobj::Timeline;

pub struct ExplicitSync {
    acquire: Timeline,
    release: Timeline,
    acquire_points: AcquirePoints,
}

impl ExplicitSync {
    pub fn import(
        device: &DrmDeviceFd,
        acquire: BorrowedFd<'_>,
        release: BorrowedFd<'_>,
    ) -> io::Result<Self> {
        Ok(Self {
            acquire: Timeline::import(device, acquire)?,
            release: Timeline::import(device, release)?,
            acquire_points: AcquirePoints::default(),
        })
    }

    /// The acquire point the next frame is signalled on.
    pub fn issue_acquire_point(&mut self) -> u64 {
        self.acquire_points.issue()
    }

    /// Signals that the frame of `point` is rendered.
    pub fn signal_acquired(&mut self, point: u64) -> io::Result<()> {
        if self.acquire_points.advance_signalled(point) {
            self.acquire.signal(point)?;
        }
        Ok(())
    }

    /// Readable once the client signalled `point` on the release timeline.
    pub fn release_eventfd(&self, point: u64) -> io::Result<OwnedFd> {
        self.release.signalled_eventfd(point)
    }
}

/// Keeps the acquire timeline moving forward only. A session's renders
/// complete in submission order, so a fence that is reported after a later
/// one is already covered by the later point, and signalling it again would
/// move the timeline backwards.
#[derive(Debug, Default)]
struct AcquirePoints {
    issued: u64,
    signalled: u64,
}

impl AcquirePoints {
    fn issue(&mut self) -> u64 {
        self.issued += 1;
        self.issued
    }

    fn advance_signalled(&mut self, point: u64) -> bool {
        let advances = point > self.signalled;
        if advances {
            self.signalled = point;
        }
        advances
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_points_start_above_zero_and_strictly_increase() {
        let mut points = AcquirePoints::default();
        assert_eq!(points.issue(), 1);
        assert_eq!(points.issue(), 2);
        assert_eq!(points.issue(), 3);
    }

    #[test]
    fn a_point_reported_after_a_later_one_is_not_signalled_again() {
        let mut points = AcquirePoints::default();
        assert!(points.advance_signalled(2));
        assert!(!points.advance_signalled(1));
        assert!(!points.advance_signalled(2));
        assert!(points.advance_signalled(3));
    }
}
