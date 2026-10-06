//! Everything the compositor keeps for one screencast session between frames.

use std::time::Duration;

use smithay::{
    backend::renderer::damage::OutputDamageTracker,
    output::Output,
    reexports::calloop::RegistrationToken,
    utils::{Physical, Scale, Size, Transform},
};

use protocols::crownos_screencast::ScreencastSession;

use crate::{
    backend::capture::{
        cursor::CursorReport, nv12::Nv12Target, pacing::SlotAges, sync::ExplicitSync,
    },
    rendering::blur::BlurCache,
};

pub struct CaptureSession {
    pub protocol: ScreencastSession,
    pub output: Output,
    /// Something on the source may have changed since the last frame.
    pub dirty: bool,
    pub last_frame_at: Option<Duration>,
    /// Wakes the loop once the frame-rate cap allows the next frame.
    pub pacing_timer: Option<RegistrationToken>,
    pub sync: Option<ExplicitSync>,
    pub cursor: CursorReport,
    pub targets: RenderTargets,
}

/// The renderer-side state, split out so a frame can borrow it while the
/// rest of the session is being read.
#[derive(Default)]
pub struct RenderTargets {
    pub damage: Option<OutputDamageTracker>,
    pub ages: SlotAges,
    pub blur: BlurCache,
    pub nv12: Nv12Target,
}

impl RenderTargets {
    /// Starts over for a new buffer size: nothing rendered so far is valid.
    pub fn reset(&mut self, size: Size<i32, Physical>, scale: f64) {
        self.damage = Some(OutputDamageTracker::new(
            size,
            Scale::from(scale),
            Transform::Normal,
        ));
        self.ages.forget();
        self.nv12.forget();
        self.blur = BlurCache::default();
    }
}

impl CaptureSession {
    pub fn new(protocol: ScreencastSession, output: Output) -> Self {
        Self {
            protocol,
            output,
            dirty: true,
            last_frame_at: None,
            pacing_timer: None,
            sync: None,
            cursor: CursorReport::default(),
            targets: RenderTargets::default(),
        }
    }
}
