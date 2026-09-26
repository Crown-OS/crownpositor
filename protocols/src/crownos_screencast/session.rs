//! One capture session: the protocol state behind the object, and the handle
//! the compositor drives it through.

use std::{os::fd::BorrowedFd, sync::Mutex, time::Duration};

use crownos_protocols::screencast::v1::server::{
    crownos_screencast_manager_v1::CursorMode,
    crownos_screencast_session_v1::{CrownosScreencastSessionV1, StopReason},
};
use smithay::{
    backend::allocator::dmabuf::Dmabuf,
    utils::{Buffer as BufferCoords, Point, Rectangle, Size},
};
use wayland_server::{Resource, Weak, backend::ObjectId, protocol::wl_buffer::WlBuffer};

use super::{
    constraints::{BufferConstraints, modifier_halves},
    ring::{Ring, RingError},
};

/// A client buffer sitting in a ring slot.
#[derive(Debug, Clone)]
pub struct AttachedBuffer {
    pub wl_buffer: Weak<WlBuffer>,
    pub dmabuf: Dmabuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Phase {
    Configuring,
    Running,
    Stopped,
}

#[derive(Debug)]
pub(super) struct SessionInner {
    pub(super) phase: Phase,
    pub(super) ring: Ring<AttachedBuffer>,
    pub(super) constraints: Option<BufferConstraints>,
    pub(super) has_timelines: bool,
    /// The highest release point accepted so far; the next must exceed it.
    pub(super) last_release_point: u64,
    pub(super) max_fps_mhz: u32,
    /// The next frame has to be complete: set by `start`, by a constraints
    /// change and by `force_frame`, cleared once such a frame is sent.
    pub(super) needs_full_frame: bool,
    pub(super) cursor_serial: u32,
}

/// User data of a `crownos_screencast_session_v1`.
#[derive(Debug)]
pub struct SessionData {
    pub(super) inner: Mutex<SessionInner>,
    pub(super) cursor_mode: CursorMode,
}

impl SessionData {
    pub(super) fn new(cursor_mode: CursorMode) -> Self {
        Self {
            inner: Mutex::new(SessionInner {
                phase: Phase::Configuring,
                ring: Ring::default(),
                constraints: None,
                has_timelines: false,
                last_release_point: 0,
                max_fps_mhz: 0,
                needs_full_frame: true,
                cursor_serial: 0,
            }),
            cursor_mode,
        }
    }

    pub(super) fn with<T>(&self, change: impl FnOnce(&mut SessionInner) -> T) -> T {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        change(&mut inner)
    }
}

impl SessionInner {
    /// Accepts `point` as the next release point if it moves the release
    /// timeline forward, as `set_timelines` requires.
    pub(super) fn advance_release_point(&mut self, point: u64) -> bool {
        let advances = point > self.last_release_point;
        if advances {
            self.last_release_point = point;
        }
        advances
    }
}

/// A slot claimed for rendering.
#[derive(Debug, Clone)]
pub struct ClaimedSlot {
    pub index: usize,
    pub dmabuf: Dmabuf,
    /// Render everything and report everything as damaged.
    pub full_frame: bool,
}

/// A cursor image in the form `cursor_buffer` sends it.
#[derive(Debug, Clone, Copy)]
pub struct CursorImage<'fd> {
    pub pixels: BorrowedFd<'fd>,
    pub size: Size<u32, BufferCoords>,
    pub stride: u32,
    pub hotspot: Point<i32, BufferCoords>,
}

/// The compositor's handle on a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreencastSession(pub(super) CrownosScreencastSessionV1);

impl ScreencastSession {
    pub fn id(&self) -> ObjectId {
        self.0.id()
    }

    pub fn is_alive(&self) -> bool {
        self.0.is_alive()
    }

    fn data(&self) -> Option<&SessionData> {
        self.0.data::<SessionData>()
    }

    fn with<T: Default>(&self, change: impl FnOnce(&mut SessionInner) -> T) -> T {
        self.data()
            .map(|data| data.with(change))
            .unwrap_or_default()
    }

    pub fn cursor_mode(&self) -> CursorMode {
        self.data()
            .map_or(CursorMode::Hidden, |data| data.cursor_mode)
    }

    pub fn is_running(&self) -> bool {
        self.with(|inner| inner.phase == Phase::Running)
    }

    pub fn is_stopped(&self) -> bool {
        self.data()
            .is_none_or(|data| data.with(|inner| inner.phase == Phase::Stopped))
    }

    pub fn has_timelines(&self) -> bool {
        self.with(|inner| inner.has_timelines)
    }

    /// The shortest time between two frames the client asked for, if any.
    pub fn frame_interval(&self) -> Option<Duration> {
        let max_fps_mhz = self.with(|inner| inner.max_fps_mhz);
        frame_interval(max_fps_mhz)
    }

    pub fn constraint_size(&self) -> Option<Size<i32, BufferCoords>> {
        self.with(|inner| {
            inner
                .constraints
                .as_ref()
                .map(|constraints| constraints.size)
        })
    }

    /// Advertises new constraints. Does nothing when they did not change;
    /// otherwise empties the ring, as the protocol promises, so no frame of
    /// the old size is ever rendered.
    pub fn set_constraints(&self, constraints: BufferConstraints) {
        let changed = self.with(|inner| {
            if inner.phase == Phase::Stopped || inner.constraints.as_ref() == Some(&constraints) {
                return false;
            }
            inner.ring.clear();
            inner.needs_full_frame = true;
            inner.constraints = Some(constraints.clone());
            true
        });
        if !changed {
            return;
        }

        self.0
            .constraints(constraints.size.w as u32, constraints.size.h as u32);
        for format in &constraints.formats {
            let code = format.format as u32;
            self.0.format(code);
            for modifier in &format.modifiers {
                let (hi, lo) = modifier_halves(*modifier);
                self.0.modifier(code, hi, lo);
            }
        }
        self.0.constraints_done();
    }

    /// Claims a free slot, or `None` when the client holds them all, the
    /// session is not running, or a buffer died under it (which stops the
    /// session).
    pub fn claim_slot(&self) -> Option<ClaimedSlot> {
        let claimed = self.with(|inner| {
            if inner.phase != Phase::Running {
                return None;
            }
            let (index, buffer) = inner.ring.acquire()?;
            if buffer.wl_buffer.upgrade().is_err() {
                inner.ring.abandon(index);
                return Some(Err(()));
            }
            Some(Ok(ClaimedSlot {
                index,
                dmabuf: buffer.dmabuf,
                full_frame: inner.needs_full_frame,
            }))
        })?;

        match claimed {
            Ok(slot) => Some(slot),
            Err(()) => {
                self.stop(StopReason::RenderFailed);
                None
            }
        }
    }

    /// Gives a claimed slot back unused.
    pub fn abandon_slot(&self, index: usize) {
        self.with(|inner| inner.ring.abandon(index));
    }

    /// Hands a rendered slot to the client: its damage, then its frame.
    pub fn present(
        &self,
        index: usize,
        damage: &[Rectangle<i32, BufferCoords>],
        sampled_at: Duration,
        acquire_point: u64,
    ) {
        let handed_out = self.with(|inner| {
            let handed_out = inner.phase == Phase::Running && inner.ring.hand_out(index);
            if handed_out {
                inner.needs_full_frame = false;
            }
            handed_out
        });
        if !handed_out {
            return;
        }

        for rect in damage {
            self.0
                .damage(rect.loc.x, rect.loc.y, rect.size.w, rect.size.h);
        }
        let seconds = sampled_at.as_secs();
        self.0.frame(
            index as u32,
            (seconds >> 32) as u32,
            seconds as u32,
            sampled_at.subsec_nanos(),
            (acquire_point >> 32) as u32,
            acquire_point as u32,
        );
    }

    /// The release point of a fenced slot has signalled.
    pub fn release_signalled(&self, index: usize) -> bool {
        self.with(|inner| inner.ring.fence_signalled(index))
    }

    /// Ends the session from the compositor's side.
    pub fn stop(&self, reason: StopReason) {
        let was_live = self.with(|inner| {
            let was_live = inner.phase != Phase::Stopped;
            inner.phase = Phase::Stopped;
            was_live
        });
        if was_live && self.0.is_alive() {
            self.0.stopped(reason);
        }
    }

    pub fn cursor_position(
        &self,
        hotspot_at: Point<i32, BufferCoords>,
        hotspot: Point<i32, BufferCoords>,
    ) {
        self.0
            .cursor_position(hotspot_at.x, hotspot_at.y, hotspot.x, hotspot.y);
    }

    pub fn cursor_leave(&self) {
        self.0.cursor_leave();
    }

    /// Announces a new cursor shape; `None` is a hidden cursor.
    pub fn cursor_shape(&self, image: Option<CursorImage<'_>>) {
        let serial = self.with(|inner| {
            inner.cursor_serial = inner.cursor_serial.wrapping_add(1);
            inner.cursor_serial
        });
        let hotspot = image.map(|image| image.hotspot).unwrap_or_default();
        if let Some(image) = image {
            self.0
                .cursor_buffer(image.pixels, image.size.w, image.size.h, image.stride);
        }
        self.0.cursor_shape(serial, hotspot.x, hotspot.y);
    }
}

impl std::hash::Hash for ScreencastSession {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.id().hash(state);
    }
}

/// Millihertz to the minimum interval between frames; zero is uncapped.
pub fn frame_interval(max_fps_mhz: u32) -> Option<Duration> {
    (max_fps_mhz > 0).then(|| Duration::from_nanos(1_000_000_000_000 / u64::from(max_fps_mhz)))
}

impl From<RingError>
    for crownos_protocols::screencast::v1::server::crownos_screencast_session_v1::Error
{
    fn from(error: RingError) -> Self {
        match error {
            RingError::InvalidIndex(_) => Self::InvalidIndex,
            RingError::AlreadyAttached(_) => Self::AlreadyAttached,
            RingError::NotClientOwned(_) => Self::NotClientOwned,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_points_must_strictly_increase_from_above_zero() {
        let data = SessionData::new(CursorMode::Hidden);
        data.with(|inner| {
            assert!(!inner.advance_release_point(0));
            assert!(inner.advance_release_point(1));
            assert!(!inner.advance_release_point(1));
            assert!(inner.advance_release_point(7));
            assert!(!inner.advance_release_point(3));
            assert_eq!(inner.last_release_point, 7);
        });
    }

    #[test]
    fn zero_millihertz_is_uncapped() {
        assert_eq!(frame_interval(0), None);
    }

    #[test]
    fn sixty_hertz_is_sixteen_and_two_thirds_milliseconds() {
        assert_eq!(
            frame_interval(60_000),
            Some(Duration::from_nanos(16_666_666))
        );
        assert_eq!(
            frame_interval(30_000),
            Some(Duration::from_nanos(33_333_333))
        );
    }
}
