//! Server-side `crownos_virtual_output_v1`.
//!
//! Outputs with no display behind them. The protocol half is small: validate
//! names and modes, track whether each object has been answered with
//! `created` or `closed`, and hand everything else to the compositor, which
//! owns the `Output` and its frame clock.

mod dispatch;
mod mode;

use std::sync::Mutex;

use crownos_protocols::virtual_output::v1::server::{
    crownos_virtual_output_manager_v1::CrownosVirtualOutputManagerV1,
    crownos_virtual_output_v1::CrownosVirtualOutputV1,
};
use wayland_server::{
    Client, DisplayHandle, Resource,
    backend::{GlobalId, ObjectId},
};

pub use crownos_protocols::virtual_output::v1::server::crownos_virtual_output_v1::CloseReason;
pub use dispatch::VirtualOutputDispatch;
pub use mode::{ModeError, NAME_PREFIX, VirtualMode, full_name, is_valid_name};

/// The protocol version this module speaks.
pub const VERSION: u32 = 1;

/// What the compositor provides for this protocol to be delegated to
/// [`VirtualOutputState`].
pub trait VirtualOutputHandler {
    fn virtual_output_state(&mut self) -> &mut VirtualOutputState;

    /// Creates the output and answers with [`VirtualOutput::created`] or
    /// [`VirtualOutput::closed`]. `name` is already validated.
    fn create_virtual_output(&mut self, output: VirtualOutput, name: &str, mode: VirtualMode);

    fn set_virtual_output_mode(&mut self, output: &VirtualOutput, mode: VirtualMode);

    fn request_virtual_output_frame(&mut self, output: &VirtualOutput);

    /// The client destroyed the object, or disconnected.
    fn destroy_virtual_output(&mut self, output: &VirtualOutput);
}

/// Global data: which clients may see the manager.
pub struct VirtualOutputGlobalData {
    filter: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

/// Delegate type for the `crownos_virtual_output_manager_v1` global.
pub struct VirtualOutputState {
    global: GlobalId,
}

impl VirtualOutputState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: VirtualOutputDispatch,
        F: Fn(&Client) -> bool + Send + Sync + 'static,
    {
        let global = display.create_global::<D, CrownosVirtualOutputManagerV1, _>(
            VERSION,
            VirtualOutputGlobalData {
                filter: Box::new(filter),
            },
        );
        Self { global }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lifecycle {
    Pending,
    Live,
    Closed,
}

/// User data of a `crownos_virtual_output_v1`.
#[derive(Debug)]
pub struct VirtualOutputData {
    lifecycle: Mutex<Lifecycle>,
}

impl VirtualOutputData {
    fn new() -> Self {
        Self {
            lifecycle: Mutex::new(Lifecycle::Pending),
        }
    }

    fn lifecycle(&self) -> Lifecycle {
        *self
            .lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Moves `from` → `to`, reporting whether the object was in `from`.
    fn advance(&self, from: &[Lifecycle], to: Lifecycle) -> bool {
        let mut lifecycle = self
            .lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let allowed = from.contains(&lifecycle);
        if allowed {
            *lifecycle = to;
        }
        allowed
    }
}

/// The compositor's handle on one virtual output object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualOutput(CrownosVirtualOutputV1);

impl VirtualOutput {
    pub fn id(&self) -> ObjectId {
        self.0.id()
    }

    fn data(&self) -> Option<&VirtualOutputData> {
        self.0.data::<VirtualOutputData>()
    }

    pub fn is_live(&self) -> bool {
        self.data()
            .is_some_and(|data| data.lifecycle() == Lifecycle::Live)
    }

    /// The output exists under `full_name`. Sent at most once.
    pub fn created(&self, full_name: &str) {
        if self
            .data()
            .is_some_and(|data| data.advance(&[Lifecycle::Pending], Lifecycle::Live))
        {
            self.0.created(full_name.to_owned());
        }
    }

    /// The output is gone, or never came to be. The last event.
    pub fn closed(&self, reason: CloseReason) {
        let closing = self.data().is_some_and(|data| {
            data.advance(&[Lifecycle::Pending, Lifecycle::Live], Lifecycle::Closed)
        });
        if closing && self.0.is_alive() {
            self.0.closed(reason);
        }
    }
}
