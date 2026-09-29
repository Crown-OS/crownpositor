mod actions;
mod backend;
mod client;
mod common;
mod config;
mod display;
mod idle;
mod input;
pub mod outputs;
mod overview;
pub mod session_env;
mod wayland;

use calloop::EventLoop;
use smithay::{
    desktop::find_popup_root_surface,
    input::pointer::{CursorIcon, CursorImageStatus},
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point},
};

pub use crate::state::{
    backend::BackendState, client::ClientState, common::CommonState, config::ConfigState,
    display::DisplayGamma, input::InputState, wayland::WaylandState,
};
use spacecontrol::animations::spring::Clock;

use crate::{
    backend::{capture::CaptureState, virtual_output::VirtualOutputs},
    rendering::decoration::TextRenderer,
    shell::Shell,
    utils::surface::root_surface,
    xwayland::Xwayland,
};

smithay::delegate_dispatch2!(State);

pub struct State {
    pub common: CommonState,
    pub backend: BackendState,
    pub wayland: WaylandState,
    pub shell: Shell,
    pub input: InputState,
    pub config: ConfigState,
    /// Which outputs the compositor's own gamma ramps are programmed on.
    /// See [`crate::state::display`].
    pub display_gamma: DisplayGamma,
    pub xwayland: Xwayland,
    /// Drives the springs. Owned here because it is per-compositor, not
    /// per-output — every output steps by the same wall-clock delta.
    pub clock: Clock,
    /// Shapes and caches titlebar text. Per-compositor rather than per-output:
    /// the same title on two monitors differs only by scale, which the cache
    /// key already covers.
    pub text: TextRenderer,
    /// The render side of every `crownos_screencast_v1` session.
    pub capture: CaptureState,
    /// Outputs `crownos_virtual_output_v1` clients created.
    pub virtual_outputs: VirtualOutputs,
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

    /// Schedules a frame on the output that shows `surface`, and nowhere else,
    /// so a game on one monitor does not repaint the other at its frame rate.
    /// A surface the shell cannot place repaints everything.
    pub fn queue_redraw_for_surface(&mut self, surface: &WlSurface) {
        let root = root_surface(surface);
        if matches!(&self.input.cursor.status, CursorImageStatus::Surface(cursor) if *cursor == root)
        {
            self.queue_pointer_redraw();
            return;
        }
        let root = self
            .shell
            .popups
            .find_popup(&root)
            .and_then(|popup| find_popup_root_surface(&popup).ok())
            .unwrap_or(root);
        match self.shell.output_showing(&root) {
            Some(output) => self.backend.queue_redraw(Some(&output)),
            None => self.queue_redraw(),
        }
    }

    /// Shows one of the compositor's own cursor shapes. Only for where no
    /// client is in charge of the image — a frame, or mid-grab.
    pub fn show_cursor(&mut self, icon: CursorIcon) {
        let status = CursorImageStatus::Named(icon);
        if self.input.cursor.status == status {
            return;
        }
        self.input.cursor.status = status;
        self.queue_pointer_redraw();
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
        let xwayland = Xwayland::start(&common.event_loop_handle);

        let mut state = Self {
            common,
            backend,
            wayland,
            shell,
            input,
            config,
            display_gamma: DisplayGamma::default(),
            xwayland,
            clock: Clock::new(),
            text: TextRenderer::new(),
            capture: CaptureState::default(),
            virtual_outputs: VirtualOutputs::default(),
        };
        // The global was created advertising everything the renderer can do;
        // the config gets the first word on what it will actually do.
        state.sync_background_effect_capabilities();

        Ok(state)
    }
}
