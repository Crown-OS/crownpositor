use std::time::Duration;

use smithay::{
    input::Seat,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    wayland::xdg_activation::{
        XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData,
    },
};

use crate::state::State;

/// Long enough for a launcher's app to start, short enough that a stale token
/// cannot yank focus minutes later.
const TOKEN_LIFETIME: Duration = Duration::from_secs(10);

impl XdgActivationHandler for State {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.wayland.xdg_activation_state
    }

    /// Focus is only handed over when the token was minted by the client the
    /// user was interacting with, so nothing can steal focus from a game on
    /// its own.
    fn request_activation(
        &mut self,
        token: XdgActivationToken,
        token_data: XdgActivationTokenData,
        surface: WlSurface,
    ) {
        let honoured = token_data.timestamp.elapsed() < TOKEN_LIFETIME
            && self.token_serial_is_current(&token_data);
        self.wayland.xdg_activation_state.remove_token(&token);
        if !honoured {
            return;
        }
        let Some(id) = self.shell.window_id(&surface) else {
            return;
        };

        self.shell.focus_window(id);
        self.shell.refresh();
        self.update_keyboard_focus();
        self.queue_redraw();
    }
}

impl State {
    /// Whether the token's input serial is no older than the moment the
    /// keyboard last entered a surface, i.e. its client held focus when the
    /// user acted.
    fn token_serial_is_current(&self, token_data: &XdgActivationTokenData) -> bool {
        let Some((serial, seat)) = &token_data.serial else {
            return false;
        };
        let ours = Seat::<State>::from_resource(seat).is_some_and(|seat| seat == self.wayland.seat);
        let keyboard = self.wayland.seat.get_keyboard();
        ours && keyboard
            .and_then(|keyboard| keyboard.last_enter())
            .is_some_and(|entered| serial.is_no_older_than(&entered))
    }
}
