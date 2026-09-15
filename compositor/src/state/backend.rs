use smithay::{
    backend::{allocator::dmabuf::Dmabuf, renderer::ImportDma, session::Session as _},
    output::Output,
};

use crate::backend::{kms::KmsState, winit::WinitState};

/// Which backend is driving the session.
///
/// A closed set rather than `Box<dyn Backend>`: the render path needs each
/// backend's concrete renderer and damage tracker, and boxing would only push
/// that behind a downcast. Adding one means adding a variant and an arm to the
/// two methods below — nothing outside this file and the backend's own module
/// should ever name a variant.
pub enum BackendState {
    Unset,
    Winit(Box<WinitState>),
    Kms(Box<KmsState>),
}

impl BackendState {
    pub fn try_new() -> anyhow::Result<Self> {
        Ok(Self::Unset)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Unset => "none",
            Self::Winit(_) => "winit",
            Self::Kms(_) => "kms",
        }
    }

    /// Schedules a frame for an output.
    ///
    /// `None` means "every output this backend drives", which for a
    /// single-output backend collapses to its one output.
    pub fn queue_redraw(&mut self, output: Option<&Output>) {
        match self {
            Self::Unset => {}
            Self::Winit(winit) => {
                if output.is_none_or(|output| *output == winit.output) {
                    winit.backend.window().request_redraw();
                }
            }
            Self::Kms(kms) => kms.queue_redraw(output),
        }
    }

    /// Whether this backend's renderer can import a dmabuf.
    ///
    /// Lives here so `handlers/dmabuf.rs` does not have to know which renderer
    /// is in play.
    pub fn can_import_dmabuf(&mut self, dmabuf: &Dmabuf) -> bool {
        match self {
            Self::Unset => false,
            Self::Winit(winit) => winit.backend.renderer().import_dmabuf(dmabuf, None).is_ok(),
            Self::Kms(kms) => kms.can_import_dmabuf(dmabuf),
        }
    }

    /// Hands the seat to another virtual terminal.
    ///
    /// Only the KMS backend owns a session to switch; nested backends have a
    /// host compositor sitting between them and the VT, so this is a no-op
    /// there rather than an error.
    pub fn switch_vt(&mut self, vt: i32) {
        match self {
            Self::Unset | Self::Winit(_) => {
                tracing::debug!(vt, backend = self.name(), "no session to switch VTs on");
            }
            Self::Kms(kms) => {
                if let Err(err) = kms.session.change_vt(vt) {
                    tracing::error!(%err, vt, "failed to switch VT");
                }
            }
        }
    }

    /// Only `backend/winit.rs` should call this. Everything else goes through the
    /// methods above, so a new backend does not ripple into unrelated code.
    pub(crate) fn winit(&mut self) -> Option<&mut WinitState> {
        match self {
            Self::Winit(winit) => Some(winit),
            _ => None,
        }
    }

    /// Only `backend/kms/` should call this, for the same reason as `winit`.
    pub(crate) fn kms(&mut self) -> Option<&mut KmsState> {
        match self {
            Self::Kms(kms) => Some(kms),
            _ => None,
        }
    }
}
