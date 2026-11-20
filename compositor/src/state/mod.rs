mod actions;
mod activity;
pub mod agent_access;
pub mod agent_seats;
mod app_icons;
mod backend;
mod client;
mod common;
mod config;
mod control;
mod cursor;
mod display;
pub mod driven_seat;
pub mod guest_seats;
mod idle;
mod input;
pub mod launch_target;
pub mod lock_screen;
pub mod outputs;
mod overview;
pub mod session_env;
mod visibility;
mod wayland;
mod windowcontrol;
pub mod windows;

use calloop::EventLoop;
use smithay::{
    desktop::find_popup_root_surface,
    input::pointer::{CursorIcon, CursorImageStatus},
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point},
};

pub use crate::state::{
    backend::BackendState,
    client::ClientState,
    common::{CommonState, Exit, RESTART_SESSION_STATUS},
    config::ConfigState,
    display::DisplayGamma,
    input::InputState,
    wayland::WaylandState,
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
    /// The built-in lock screen's scene, fonts and password checker.
    pub lock: lock_screen::LockScreenState,
    /// What `org.freedesktop.ScreenSaver` clients hold the screen on for.
    pub screensaver_holds: idle::ScreenSaverHolds,
    /// Every mapped window as the agent daemon, the context engine and
    /// crownplugind see it, and which windows are off limits to captures.
    pub windows: windows::WindowDirectory,
    /// Apps waiting for the user's answer about agent access.
    pub agent_prompts: agent_access::PendingPrompts,
    /// The seats behind `crownos_agent_cursor_v1`, one per agent.
    pub agent_seats: agent_seats::AgentSeats,
    /// The seats behind `crownos_guest_seat_v1`, one per remote desktop guest.
    pub guest_seats: guest_seats::GuestSeats,
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
        if self.input.cursor.shows(&root) {
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

    /// A window or workspace capture has to notice a commit even when nothing
    /// on screen shows the window, so commits reach it directly.
    pub(crate) fn damage_window_captures(&mut self, surface: &WlSurface) {
        if !self.capture.follows_windows() {
            return;
        }
        let root = root_surface(surface);
        let root = self
            .shell
            .popups
            .find_popup(&root)
            .and_then(|popup| find_popup_root_surface(&popup).ok())
            .unwrap_or(root);
        if let Some(window) = self.shell.window_id(&root)
            && let Some(location) = self.shell.location(window)
        {
            self.capture.mark_window_damaged(window, location.workspace);
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
        let visual_context_rules = config.current.visual_context.clone();
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
            lock: lock_screen::LockScreenState::default(),
            screensaver_holds: idle::ScreenSaverHolds::default(),
            windows: windows::WindowDirectory::new(&visual_context_rules),
            agent_prompts: agent_access::PendingPrompts::default(),
            agent_seats: agent_seats::AgentSeats::default(),
            guest_seats: guest_seats::GuestSeats::default(),
        };
        state.arm_activity_flush();
        // The global was created advertising everything the renderer can do;
        // the config gets the first word on what it will actually do.
        state.sync_background_effect_capabilities();
        state.animate_cursor()?;

        Ok(state)
    }
}
