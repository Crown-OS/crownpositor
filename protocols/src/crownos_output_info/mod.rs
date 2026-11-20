//! Server-side `crownos_output_info_v1`.
//!
//! Each object watches one output and keeps the snapshot it was last sent,
//! so [`OutputInfoState::update`] sends only the fields that changed, each
//! group ended by `done`. What the compositor decided about an output is its
//! own business, reached through [`OutputInfoHandler`].

mod dispatch;
mod snapshot;

#[cfg(test)]
mod tests;

use crownos_protocols::output_info::v1::server::{
    crownos_output_info_manager_v1::CrownosOutputInfoManagerV1,
    crownos_output_info_v1::CrownosOutputInfoV1,
};
use smithay::output::Output;
use wayland_server::{Client, DisplayHandle, backend::GlobalId};

pub use crownos_protocols::output_info::v1::server::crownos_output_info_v1::{
    Capability, Shell, State,
};
pub use dispatch::OutputInfoDispatch;
pub use snapshot::{Field, OutputInfo};

/// The protocol version this module speaks.
pub const VERSION: u32 = 1;

/// What the compositor provides for this protocol to be delegated to
/// [`OutputInfoState`].
pub trait OutputInfoHandler {
    fn output_info_state(&mut self) -> &mut OutputInfoState;

    /// What the compositor decided about `output` now. `None` for an output
    /// it no longer has: the new object is then inert.
    fn output_info(&mut self, output: &Output) -> Option<OutputInfo>;
}

/// Global data: which clients may see the manager.
pub struct OutputInfoGlobalData {
    filter: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

/// One `crownos_output_info_v1` and what it was last told.
struct OutputWatch {
    object: CrownosOutputInfoV1,
    /// `None` once the output is gone: the object is then inert.
    output: Option<Output>,
    sent: Option<OutputInfo>,
}

/// Delegate type for the `crownos_output_info_manager_v1` global.
pub struct OutputInfoState {
    global: GlobalId,
    watches: Vec<OutputWatch>,
}

impl OutputInfoState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: OutputInfoDispatch,
        F: Fn(&Client) -> bool + Send + Sync + 'static,
    {
        let global = display.create_global::<D, CrownosOutputInfoManagerV1, _>(
            VERSION,
            OutputInfoGlobalData {
                filter: Box::new(filter),
            },
        );
        Self {
            global,
            watches: Vec::new(),
        }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }

    /// Whether any object watches an output, so a caller can skip building
    /// snapshots nobody reads.
    pub fn is_watched(&self) -> bool {
        self.watches.iter().any(|watch| watch.output.is_some())
    }

    /// Tells every object watching `output` the fields of `info` that changed
    /// since it was last told, followed by `done`. Cheap when nothing changed.
    pub fn update(&mut self, output: &Output, info: &OutputInfo) {
        for watch in &mut self.watches {
            if watch.output.as_ref() != Some(output) {
                continue;
            }
            if info.send(&watch.object, watch.sent.as_ref()) {
                watch.sent = Some(info.clone());
            }
        }
    }

    /// Makes the objects watching an output that went away inert.
    pub fn output_removed(&mut self, output: &Output) {
        for watch in &mut self.watches {
            if watch.output.as_ref() == Some(output) {
                watch.output = None;
            }
        }
    }
}
