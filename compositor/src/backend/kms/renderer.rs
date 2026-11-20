//! A GPU's renderer, made only while one of its heads is lit.
//!
//! The DRM device of every GPU opens at once, so its connectors are known.
//! The GBM device and the EGL display of a GPU other than the primary are
//! made when one of its heads lights up, and dropped when it has had no lit
//! head for [`LINGER`], so the GPU can suspend. The primary GPU composites
//! every output and keeps its renderer for the whole session.

use std::time::Duration;

use calloop::{
    RegistrationToken,
    timer::{TimeoutAction, Timer},
};
use smithay::backend::{
    allocator::gbm::GbmDevice,
    drm::{DrmDeviceFd, DrmNode},
};

use crate::{
    backend::{
        kms::{KmsError, KmsState},
        render::KmsGpuManager,
    },
    state::State,
};

/// Long enough that switching a monitor off and on again, or a deep colour
/// rebuild, does not tear the renderer down in between.
const LINGER: Duration = Duration::from_secs(10);

#[derive(Debug, Default, PartialEq, Eq)]
pub enum RendererState<Token> {
    #[default]
    Detached,
    Attached,
    /// Attached, and released when the timer `Token` fires.
    Releasing(Token),
}

/// What attaching a renderer has to do.
#[derive(Debug, PartialEq, Eq)]
pub enum Attach<Token> {
    Create,
    AlreadyAttached,
    /// The renderer is still there; only the pending release goes.
    CancelRelease(Token),
}

impl<Token> RendererState<Token> {
    /// Marks the renderer attached. After [`Attach::Create`], the caller
    /// sets the state back with [`Self::failed_to_attach`] when it could
    /// not make one.
    pub fn attach(&mut self) -> Attach<Token> {
        match std::mem::replace(self, Self::Attached) {
            Self::Detached => Attach::Create,
            Self::Attached => Attach::AlreadyAttached,
            Self::Releasing(token) => Attach::CancelRelease(token),
        }
    }

    pub fn failed_to_attach(&mut self) {
        *self = Self::Detached;
    }

    /// Whether to start the release timer: a renderer is attached, no head
    /// of the GPU is lit, and the GPU is not the primary.
    pub fn should_release(&self, is_primary: bool, lit_heads: usize) -> bool {
        !is_primary && lit_heads == 0 && matches!(self, Self::Attached)
    }

    pub fn release_after(&mut self, token: Token) {
        *self = Self::Releasing(token);
    }

    /// The release timer fired. Returns whether to drop the renderer now.
    pub fn timer_fired(&mut self, lit_heads: usize) -> bool {
        match self {
            Self::Releasing(_) if lit_heads == 0 => {
                *self = Self::Detached;
                true
            }
            Self::Releasing(_) => {
                *self = Self::Attached;
                false
            }
            Self::Detached | Self::Attached => false,
        }
    }

    /// The GPU is going away. Returns the pending release to cancel, and
    /// whether a renderer is attached.
    pub fn detach_for_removal(&mut self) -> (Option<Token>, bool) {
        match std::mem::take(self) {
            Self::Detached => (None, false),
            Self::Attached => (None, true),
            Self::Releasing(token) => (Some(token), true),
        }
    }
}

/// Makes a GBM device on `fd` and the EGL display for `render_node`.
pub fn create_renderer(
    gpu_manager: &mut KmsGpuManager,
    fd: DrmDeviceFd,
    node: DrmNode,
    render_node: DrmNode,
) -> Result<GbmDevice<DrmDeviceFd>, KmsError> {
    let gbm = GbmDevice::new(fd).map_err(|source| KmsError::Gbm {
        path: node.to_string(),
        source,
    })?;
    gpu_manager
        .as_mut()
        .add_node(render_node, gbm.clone())
        .map_err(|err| KmsError::AddNode(err, render_node))?;
    Ok(gbm)
}

impl KmsState {
    /// Makes the renderer of the GPU at `node` if it has none, and cancels
    /// a pending release through `cancel`.
    fn attach_renderer(
        &mut self,
        node: DrmNode,
        cancel: impl FnOnce(RegistrationToken),
    ) -> Result<(), KmsError> {
        let Some(device) = self.devices.get_mut(&node) else {
            return Ok(());
        };
        match device.renderer.attach() {
            Attach::Create => {}
            Attach::AlreadyAttached => return Ok(()),
            Attach::CancelRelease(token) => {
                cancel(token);
                return Ok(());
            }
        }
        let fd = device.drm.device_fd().clone();
        match create_renderer(&mut self.gpu_manager, fd, node, device.render_node) {
            Ok(gbm) => {
                tracing::info!(%node, render_node = %device.render_node, "renderer attached");
                device.gbm = Some(gbm);
                Ok(())
            }
            Err(err) => {
                device.renderer.failed_to_attach();
                Err(err)
            }
        }
    }
}

/// Makes the renderer a head on the GPU at `node` needs before it lights.
pub fn attach_renderer(state: &mut State, node: DrmNode) -> Result<(), KmsError> {
    let handle = state.common.event_loop_handle.clone();
    let Some(kms) = state.backend.kms() else {
        return Ok(());
    };
    kms.attach_renderer(node, |token| handle.remove(token))
}

/// Starts the release timer once the GPU at `node` has no lit head.
pub fn release_when_idle(state: &mut State, node: DrmNode) {
    let handle = state.common.event_loop_handle.clone();
    let Some(kms) = state.backend.kms() else {
        return;
    };
    let is_primary = node == kms.primary_node;
    let Some(device) = kms.devices.get_mut(&node) else {
        return;
    };
    if !device
        .renderer
        .should_release(is_primary, device.surfaces.len())
    {
        return;
    }
    let timer = Timer::from_duration(LINGER);
    match handle.insert_source(timer, move |_, _, state| {
        release_renderer(state, node);
        TimeoutAction::Drop
    }) {
        Ok(token) => device.renderer.release_after(token),
        Err(err) => tracing::warn!(%err, %node, "the renderer of this GPU stays attached"),
    }
}

fn release_renderer(state: &mut State, node: DrmNode) {
    let Some(kms) = state.backend.kms() else {
        return;
    };
    let Some(device) = kms.devices.get_mut(&node) else {
        return;
    };
    if !device.renderer.timer_fired(device.surfaces.len()) {
        return;
    }
    kms.gpu_manager.as_mut().remove_node(&device.render_node);
    device.gbm = None;
    tracing::info!(%node, "renderer released; the GPU has no lit head");
}

#[cfg(test)]
mod tests {
    use super::*;

    type FakeToken = u32;

    fn attached() -> RendererState<FakeToken> {
        let mut state = RendererState::default();
        assert_eq!(state.attach(), Attach::Create);
        state
    }

    #[test]
    fn the_first_head_creates_the_renderer_and_the_next_reuses_it() {
        let mut state = attached();
        assert_eq!(state, RendererState::Attached);
        assert_eq!(state.attach(), Attach::AlreadyAttached);
    }

    #[test]
    fn a_failed_attach_leaves_the_gpu_detached() {
        let mut state = attached();
        state.failed_to_attach();
        assert_eq!(state, RendererState::Detached);
        assert_eq!(state.attach(), Attach::Create);
    }

    #[test]
    fn the_last_head_going_off_starts_the_release_on_a_secondary_gpu_only() {
        let state = attached();
        assert!(state.should_release(false, 0));
        assert!(!state.should_release(false, 1));
        assert!(!state.should_release(true, 0));
        assert!(!RendererState::<FakeToken>::Detached.should_release(false, 0));
    }

    #[test]
    fn the_timer_releases_a_gpu_that_stayed_idle() {
        let mut state = attached();
        state.release_after(7);
        assert!(state.timer_fired(0));
        assert_eq!(state, RendererState::Detached);
        assert!(!state.timer_fired(0));
    }

    #[test]
    fn lighting_a_head_while_the_release_is_pending_cancels_it() {
        let mut state = attached();
        state.release_after(7);
        assert_eq!(state.attach(), Attach::CancelRelease(7));
        assert_eq!(state, RendererState::Attached);
        assert!(!state.timer_fired(0));
    }

    #[test]
    fn a_timer_that_finds_a_lit_head_keeps_the_renderer() {
        let mut state = attached();
        state.release_after(7);
        assert!(!state.timer_fired(1));
        assert_eq!(state, RendererState::Attached);
    }

    #[test]
    fn removing_the_gpu_cancels_a_pending_release() {
        let mut state = attached();
        state.release_after(7);
        assert_eq!(state.detach_for_removal(), (Some(7), true));
        assert_eq!(state.detach_for_removal(), (None, false));
        assert_eq!(attached().detach_for_removal(), (None, true));
    }
}
