mod actions;
mod backend;
mod client;
mod common;
mod config;
mod input;
mod wayland;

use calloop::EventLoop;
use smithay::utils::{Logical, Point};

use crate::{animations::spring::Clock, shell::Shell};
pub use crate::state::{
    backend::BackendState, client::ClientState, common::CommonState, config::ConfigState,
    input::InputState, wayland::WaylandState,
};

pub struct State {
    pub common: CommonState,
    pub backend: BackendState,
    pub wayland: WaylandState,
    pub shell: Shell,
    pub input: InputState,
    pub config: ConfigState,
    /// Drives the springs. Owned here because it is per-compositor, not
    /// per-output — every output steps by the same wall-clock delta.
    pub clock: Clock,
}

impl State {
    /// Schedules a frame.
    ///
    /// Needed because a backend's render pass only re-arms itself while something
    /// is animating; client damage and model changes have to ask explicitly.
    pub fn queue_redraw(&mut self) {
        self.backend.queue_redraw(None);
    }

    /// Schedules a frame on the output containing `location`, and nowhere else.
    ///
    /// The pointer cannot be on two monitors at once, so cursor work — a move,
    /// a shape change — has no business repainting the others.
    pub fn queue_redraw_at(&mut self, location: Point<f64, Logical>) {
        let Some(output) = self
            .shell
            .monitor_at(location)
            .map(|monitor| monitor.output().clone())
        else {
            return;
        };
        self.backend.queue_redraw(Some(&output));
    }

    /// Schedules a frame wherever the cursor currently is.
    pub fn queue_pointer_redraw(&mut self) {
        self.queue_redraw_at(self.input.pointer_location);
    }

    pub fn try_new(event_loop: &mut EventLoop<'static, State>) -> anyhow::Result<Self> {
        let common = CommonState::try_new(event_loop)?;
        let config = ConfigState::init(&common.event_loop_handle)?;
        let input = InputState::new(&config.current);
        let backend = BackendState::try_new()?;
        let wayland =
            WaylandState::try_new(&common.display_handle, common.event_loop_handle.clone())?;
        let shell = Shell::try_new(&common.display_handle, &config.current)?;

        Ok(Self {
            common,
            backend,
            wayland,
            shell,
            input,
            config,
            clock: Clock::new(),
        })
    }
}
