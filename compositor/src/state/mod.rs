mod actions;
mod backend;
mod client;
mod common;
mod config;
mod input;
mod wayland;

use calloop::EventLoop;

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
