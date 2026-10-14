//! Questions asked of a `wl_surface` from several places.

use smithay::{
    reexports::{
        wayland_protocols::wp::content_type::v1::server::wp_content_type_v1::Type as ContentType,
        wayland_server::protocol::wl_surface::WlSurface,
    },
    wayland::{
        compositor::{get_parent, with_states},
        content_type::ContentTypeSurfaceCachedState,
        shell::xdg::XdgToplevelSurfaceData,
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

/// The `app_id` a toplevel gave itself, if it gave one.
pub fn app_id(surface: &WlSurface) -> Option<String> {
    with_states(surface, |states| {
        states
            .data_map
            .get::<XdgToplevelSurfaceData>()?
            .lock()
            .ok()?
            .app_id
            .clone()
    })
}

/// Runs `f` on a toplevel's `app_id`, empty when it gave none, without
/// copying it.
pub fn with_app_id<T>(surface: &WlSurface, f: impl FnOnce(&str) -> T) -> T {
    with_states(surface, |states| {
        let data = states
            .data_map
            .get::<XdgToplevelSurfaceData>()
            .and_then(|data| data.lock().ok());
        f(data
            .as_ref()
            .and_then(|data| data.app_id.as_deref())
            .unwrap_or_default())
    })
}

/// Whether a toplevel is transient for another: a dialog, not an application
/// window of its own.
pub fn has_parent(surface: &WlSurface) -> bool {
    with_states(surface, |states| {
        states
            .data_map
            .get::<XdgToplevelSurfaceData>()
            .and_then(|data| data.lock().ok().map(|data| data.parent.is_some()))
            .unwrap_or(false)
    })
}
