//! Server-side `crownos_output_brightness_v1`.
//!
//! Each object watches one output and keeps what it was last told. A client
//! that sets a level always gets the clamped level back, at once and from the
//! request handler, so it can drop its own guess when its round trip ends;
//! every other object of that output hears only of a change. Which device
//! holds the brightness, and what a level does to it, is the compositor's
//! business, reached through [`OutputBrightnessHandler`].

mod dispatch;
mod events;

#[cfg(test)]
mod tests;

use crownos_protocols::output_brightness::v1::server::{
    crownos_output_brightness_manager_v1::CrownosOutputBrightnessManagerV1,
    crownos_output_brightness_v1::CrownosOutputBrightnessV1,
};
use smithay::output::Output;
use wayland_server::{Client, DisplayHandle, Resource, backend::GlobalId};

pub use crownos_protocols::output_brightness::v1::server::crownos_output_brightness_v1::Control;
pub use dispatch::OutputBrightnessDispatch;
pub use events::OutputBrightness;

/// The protocol version this module speaks.
pub const VERSION: u32 = 1;

/// The brightest level, and the largest a client may ask for.
pub const MAX_LEVEL: u32 = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("level {0} is above {MAX_LEVEL}")]
pub struct InvalidLevel(pub u32);

pub fn checked_level(level: u32) -> Result<u32, InvalidLevel> {
    if level > MAX_LEVEL {
        Err(InvalidLevel(level))
    } else {
        Ok(level)
    }
}

/// What the compositor provides for this protocol to be delegated to
/// [`OutputBrightnessState`].
pub trait OutputBrightnessHandler {
    fn output_brightness_state(&mut self) -> &mut OutputBrightnessState;

    /// The control and level of `output` now. `None` for an output the
    /// compositor no longer has: the new object is then inert.
    fn output_brightness(&mut self, output: &Output) -> Option<OutputBrightness>;

    /// Takes a client's level, already checked against [`MAX_LEVEL`], and
    /// returns the control and the clamped level to answer with. `None` when
    /// the output's level cannot change. Must not announce the change: the
    /// protocol state answers the client and tells the other objects.
    fn set_output_brightness(&mut self, output: &Output, level: u32) -> Option<OutputBrightness>;

    /// Reads the level from the hardware again. A change comes back later
    /// through [`OutputBrightnessState::update`].
    fn refresh_output_brightness(&mut self, output: &Output);
}

/// Global data: which clients may see the manager.
pub struct OutputBrightnessGlobalData {
    filter: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

/// One `crownos_output_brightness_v1` and what it was last told.
struct BrightnessWatch {
    object: CrownosOutputBrightnessV1,
    /// `None` once the output is gone: the object is then inert.
    output: Option<Output>,
    sent: Option<OutputBrightness>,
}

impl BrightnessWatch {
    fn tell(&mut self, current: OutputBrightness, answering: bool) {
        let (control, level) = current.events_since(self.sent, answering);
        if let Some(control) = control {
            self.object.control(control);
        }
        if let Some(level) = level {
            self.object.level(level);
        }
        self.sent = Some(current);
    }
}

/// Delegate type for the `crownos_output_brightness_manager_v1` global.
pub struct OutputBrightnessState {
    global: GlobalId,
    watches: Vec<BrightnessWatch>,
}

impl OutputBrightnessState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: OutputBrightnessDispatch,
        F: Fn(&Client) -> bool + Send + Sync + 'static,
    {
        let global = display.create_global::<D, CrownosOutputBrightnessManagerV1, _>(
            VERSION,
            OutputBrightnessGlobalData {
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

    /// Tells every object of `output` what changed since it was last told.
    pub fn update(&mut self, output: &Output, current: OutputBrightness) {
        for watch in self.watches_of(output) {
            watch.tell(current, false);
        }
    }

    /// Makes the objects of an output that went away inert.
    pub fn output_removed(&mut self, output: &Output) {
        for watch in self.watches_of(output) {
            watch.output = None;
        }
    }

    /// Answers `requester`'s accepted `set_level`, and tells the other
    /// objects of `output` if that changed anything.
    fn answer(
        &mut self,
        requester: &CrownosOutputBrightnessV1,
        output: &Output,
        current: OutputBrightness,
    ) {
        for watch in self.watches_of(output) {
            let answering = watch.object.id() == requester.id();
            watch.tell(current, answering);
        }
    }

    fn watched_output(&self, object: &CrownosOutputBrightnessV1) -> Option<Output> {
        self.watches
            .iter()
            .find(|watch| watch.object.id() == object.id())?
            .output
            .clone()
    }

    fn watches_of(&mut self, output: &Output) -> impl Iterator<Item = &mut BrightnessWatch> {
        self.watches
            .iter_mut()
            .filter(move |watch| watch.output.as_ref() == Some(output))
    }
}
