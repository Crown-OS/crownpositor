//! What keeps the session from going idle besides input: a Wayland idle
//! inhibitor on a surface that is on screen, or an `org.freedesktop.ScreenSaver`
//! hold — how browsers and players keep the screen on through a video.

mod holds;
mod screensaver;

use smithay::utils::IsAlive;
use zbus::connection;

pub use holds::ScreenSaverHolds;
use screensaver::Call;

use crate::{
    state::{BackendState, State},
    utils::surface::root_surface,
};

impl State {
    /// An inhibitor only holds while its window is on screen: a video player
    /// left on another workspace must not keep the display awake. A
    /// subsurface's inhibitor, a video overlay's, counts by its window.
    pub fn refresh_idle_inhibit(&mut self) {
        let surfaces = &mut self.wayland.idle_inhibiting_surfaces;
        surfaces.retain(IsAlive::alive);
        let inhibited = !self.screensaver_holds.is_empty()
            || surfaces
                .iter()
                .any(|surface| self.shell.is_surface_visible(&root_surface(surface)));
        self.wayland.idle_notifier_state.set_is_inhibited(inhibited);
    }

    /// Owns `org.freedesktop.ScreenSaver` on the session bus. Only a session
    /// on real hardware: a nested compositor shares the host's bus.
    pub(crate) fn start_screensaver_service(&mut self) {
        if !matches!(self.backend, BackendState::Kms(_)) {
            return;
        }
        let tasks = self.common.tasks.clone();
        let deliver = move |call| tasks.deliver(move |state| state.on_screensaver_call(call));
        self.common.tasks.spawn(async move {
            let served = match connection::Builder::session() {
                Ok(bus) => screensaver::serve(bus, deliver).await,
                Err(err) => Err(err),
            };
            if let Err(err) = served {
                tracing::warn!(%err, "no org.freedesktop.ScreenSaver: D-Bus idle holds are ignored");
            }
        });
    }

    fn on_screensaver_call(&mut self, call: Call) {
        let holds = &mut self.screensaver_holds;
        match call {
            Call::Hold { cookie, owner } => holds.hold(cookie, owner),
            Call::Release { cookie, owner } => holds.release(cookie, &owner),
            Call::Left(owner) => holds.release_owner(&owner),
            Call::Activity => self.note_user_activity(),
        }
    }
}
