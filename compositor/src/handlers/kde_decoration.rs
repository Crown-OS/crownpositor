use std::cell::Cell;

use smithay::{
    reexports::{
        wayland_protocols_misc::server_decoration::server::org_kde_kwin_server_decoration::{
            Mode, OrgKdeKwinServerDecoration,
        },
        wayland_server::{WEnum, protocol::wl_surface::WlSurface},
    },
    wayland::{
        compositor::with_states,
        shell::kde::decoration::{KdeDecorationHandler, KdeDecorationState},
    },
};

use crate::{shell::tile::Tile, state::State};

/// The mode a surface last settled on through the KDE protocol. Kept on the
/// surface rather than the tile, because GTK 3 asks before its window maps.
#[derive(Default)]
struct KdeDecorationRequest(Cell<Option<Mode>>);

/// What `surface` asked for through the KDE protocol; `None` while it holds no
/// decoration object.
pub fn requested_mode(surface: &WlSurface) -> Option<Mode> {
    with_states(surface, |states| {
        states
            .data_map
            .get::<KdeDecorationRequest>()
            .and_then(|request| request.0.get())
    })
}

impl KdeDecorationHandler for State {
    fn kde_decoration_state(&self) -> &KdeDecorationState {
        &self.wayland.kde_decoration_state
    }

    /// The protocol has the server announce its policy to every new object,
    /// and a client that never asks for anything else has taken it.
    fn new_decoration(&mut self, surface: &WlSurface, decoration: &OrgKdeKwinServerDecoration) {
        decoration.mode(Mode::Server);
        self.follow_kde_request(surface, Some(Mode::Server));
    }

    fn request_mode(
        &mut self,
        surface: &WlSurface,
        decoration: &OrgKdeKwinServerDecoration,
        mode: WEnum<Mode>,
    ) {
        let WEnum::Value(mode) = mode else {
            return;
        };
        decoration.mode(mode);
        self.follow_kde_request(surface, Some(mode));
    }

    fn release(&mut self, _decoration: &OrgKdeKwinServerDecoration, surface: &WlSurface) {
        self.follow_kde_request(surface, None);
    }
}

impl State {
    fn follow_kde_request(&mut self, surface: &WlSurface, mode: Option<Mode>) {
        with_states(surface, |states| {
            states
                .data_map
                .insert_if_missing(KdeDecorationRequest::default);
            if let Some(request) = states.data_map.get::<KdeDecorationRequest>() {
                request.0.set(mode);
            }
        });

        let toplevel = self
            .shell
            .window_id(surface)
            .and_then(|id| self.shell.tile(id))
            .and_then(Tile::toplevel)
            .cloned();
        if let Some(toplevel) = toplevel {
            self.follow_decoration_request(&toplevel);
        }
    }
}
