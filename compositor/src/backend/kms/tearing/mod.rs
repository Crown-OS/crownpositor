//! Async page flips for a fullscreen game that asked to tear.
//!
//! smithay's `DrmCompositor` only commits on vblank, so a frame that may tear
//! is flipped here instead: the compositor still assigns planes and tests the
//! frame, and when the game's buffer alone landed on the primary plane, that
//! buffer's framebuffer is committed with `PAGE_FLIP_ASYNC` and nothing else.
//! `DrmCompositor` never learns of it; its next vsync commit re-sends the
//! primary plane anyway, which puts it back in charge.
//!
//! A flip the driver refuses costs nothing: the frame `DrmCompositor` already
//! prepared is queued with vsync instead.

mod caps;
mod commit;
mod eligibility;
mod framebuffer;

use smithay::{
    backend::{
        allocator::gbm::GbmDevice,
        drm::{DrmDeviceFd, DrmSurface},
        renderer::utils::Buffer,
    },
    desktop::utils::OutputPresentationFeedback,
};

pub use eligibility::{BufferLayout, FrameShape};

use crate::backend::kms::tearing::{
    caps::AsyncCaps,
    commit::{FlipError, flip_async},
    eligibility::may_flip_async,
    framebuffer::{FramebufferCache, SharedFramebuffer},
};

/// Refusals in a row after which the driver is taken at its word until the
/// buffer layout changes.
const MAX_REJECTIONS: u8 = 3;

/// A buffer shown by an async flip, kept alive, and unreleased to the
/// client, for as long as the plane may still be scanning it out.
struct ShownBuffer {
    _buffer: Buffer,
    _framebuffer: SharedFramebuffer,
}

pub enum FlipOutcome {
    Flipped,
    /// Not flipped; the feedback goes with the vsync frame instead.
    Declined(OutputPresentationFeedback),
}

pub struct Tearing {
    caps: AsyncCaps,
    gbm: GbmDevice<DrmDeviceFd>,
    framebuffers: FramebufferCache,
    /// The layout of the bare game buffer on the primary plane, when that is
    /// what is there: the only state an async flip may continue from.
    on_screen: Option<BufferLayout>,
    in_flight: Option<(ShownBuffer, OutputPresentationFeedback)>,
    shown: Option<ShownBuffer>,
    rejections: u8,
}

impl Tearing {
    pub fn new(surface: &DrmSurface, gbm: GbmDevice<DrmDeviceFd>) -> Option<Self> {
        Some(Self {
            caps: AsyncCaps::probe(surface)?,
            gbm,
            framebuffers: FramebufferCache::default(),
            on_screen: None,
            in_flight: None,
            shown: None,
            rejections: 0,
        })
    }

    /// Shows `buffer` at once if this frame allows it.
    pub fn try_flip(
        &mut self,
        surface: &DrmSurface,
        frame: FrameShape,
        buffer: Option<Buffer>,
        feedback: OutputPresentationFeedback,
    ) -> FlipOutcome {
        let next = buffer.as_deref().and_then(BufferLayout::of);
        let allowed = self.rejections < MAX_REJECTIONS
            && may_flip_async(&frame, self.on_screen, next, self.in_flight.is_some());
        let Some(buffer) = buffer.filter(|_| allowed) else {
            return FlipOutcome::Declined(feedback);
        };

        let flipped = self
            .framebuffers
            .get_or_add(surface.device_fd(), &self.gbm, &buffer)
            .and_then(|framebuffer| {
                flip_async(surface, &self.caps, *(*framebuffer).as_ref())?;
                Ok(framebuffer)
            });
        match flipped {
            Ok(framebuffer) => {
                self.rejections = 0;
                let shown = ShownBuffer {
                    _buffer: buffer,
                    _framebuffer: framebuffer,
                };
                self.in_flight = Some((shown, feedback));
                FlipOutcome::Flipped
            }
            Err(err) => {
                if matches!(err, FlipError::Rejected) {
                    self.rejections += 1;
                }
                tracing::debug!(%err, "falling back to a vsync flip");
                FlipOutcome::Declined(feedback)
            }
        }
    }

    /// Records what a vsync frame put on the primary plane.
    pub fn synced(&mut self, frame: FrameShape, layout: Option<BufferLayout>) {
        let on_screen = frame.is_bare_game().then_some(layout).flatten();
        if on_screen != self.on_screen {
            self.rejections = 0;
        }
        self.on_screen = on_screen;
    }

    /// Takes a page-flip event if it was for an async flip, returning that
    /// flip's feedback. The buffer it replaced goes back to the client.
    pub fn complete(&mut self) -> Option<OutputPresentationFeedback> {
        let (shown, feedback) = self.in_flight.take()?;
        self.shown = Some(shown);
        Some(feedback)
    }

    /// A vsync frame reached the screen, over whatever an async flip left.
    pub fn vsync_presented(&mut self) {
        self.shown = None;
    }

    /// Forgets everything: the CRTC was reset or taken away.
    pub fn reset(&mut self) {
        self.on_screen = None;
        self.in_flight = None;
        self.shown = None;
        self.rejections = 0;
        self.framebuffers.clear();
    }
}
