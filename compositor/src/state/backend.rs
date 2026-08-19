use smithay::{
    backend::{allocator::dmabuf::Dmabuf, renderer::ImportDma},
    output::Output,
};

use crate::backend::winit::WinitState;

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
}

impl BackendState {
    pub fn try_new() -> anyhow::Result<Self> {
        Ok(Self::Unset)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Unset => "none",
            Self::Winit(_) => "winit",
        }
    }

    /// Schedules a frame for an output.
    ///
    /// `None` means "whichever output this backend drives", which is all a
    /// single-output backend can answer.
    pub fn queue_redraw(&mut self, output: Option<&Output>) {
        match self {
            Self::Unset => {}
            Self::Winit(winit) => {
                if output.is_none_or(|output| *output == winit.output) {
                    winit.backend.window().request_redraw();
                }
            }
        }
    }

    /// Whether this backend's renderer can import a dmabuf.
    ///
    /// Lives here so `handlers/dmabuf.rs` does not have to know which renderer
    /// is in play.
    pub fn can_import_dmabuf(&mut self, dmabuf: &Dmabuf) -> bool {
        match self {
            Self::Unset => false,
            Self::Winit(winit) => winit
                .backend
                .renderer()
                .import_dmabuf(dmabuf, None)
                .is_ok(),
        }
    }

    /// Only `backend/winit.rs` should call this. Everything else goes through the
    /// methods above, so a new backend does not ripple into unrelated code.
    pub(crate) fn winit(&mut self) -> Option<&mut WinitState> {
        match self {
            Self::Winit(winit) => Some(winit),
            Self::Unset => None,
        }
    }
}
