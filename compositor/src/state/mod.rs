mod backend;
mod client;
mod common;
mod shell;
mod wayland;

use calloop::EventLoop;

pub use crate::state::{
    backend::BackendState, client::ClientState, common::CommonState, shell::ShellState,
    wayland::WaylandState,
};

pub struct State {
    pub common: CommonState,
    pub backend: BackendState,
    pub wayland: WaylandState,
    pub shell: ShellState,
}

impl State {
    pub fn try_new(event_loop: &mut EventLoop<'static, State>) -> anyhow::Result<Self> {
        let common = CommonState::try_new(event_loop)?;
        let backend = BackendState::try_new()?;
        let wayland =
            WaylandState::try_new(&common.display_handle, common.event_loop_handle.clone())?;
        let shell = ShellState::try_new(&common.display_handle)?;

        Ok(Self {
            common,
            backend,
            wayland,
            shell,
        })
    }
}
