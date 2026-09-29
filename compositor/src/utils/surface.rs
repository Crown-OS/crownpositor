//! Questions asked of a `wl_surface` from several places.

use smithay::{
    reexports::{
        wayland_protocols::wp::content_type::v1::server::wp_content_type_v1::Type as ContentType,
        wayland_server::protocol::wl_surface::WlSurface,
    },
    wayland::{
        compositor::{get_parent, with_states},
        content_type::ContentTypeSurfaceCachedState,
    },
};

/// The top of the subsurface tree `surface` hangs off.
pub fn root_surface(surface: &WlSurface) -> WlSurface {
    let mut root = surface.clone();
    while let Some(parent) = get_parent(&root) {
        root = parent;
    }
    root
}

/// Whether the client labelled this surface as a game through
/// `wp_content_type_v1`.
pub fn is_game_content(surface: &WlSurface) -> bool {
    with_states(surface, |states| {
        *states
            .cached_state
            .get::<ContentTypeSurfaceCachedState>()
            .current()
            .content_type()
            == ContentType::Game
    })
}
