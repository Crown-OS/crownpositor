use smithay::{
    desktop::{
        PopupKind, WindowSurfaceType, find_popup_root_surface, get_popup_toplevel_coords,
        layer_map_for_output,
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Rectangle},
    wayland::shell::xdg::PopupSurface,
};

use crate::shell::{Shell, monitor::Monitor};

impl Shell {
    /// Slides, flips or resizes a popup, as its positioner allows, until it
    /// fits on the output its root surface lives on.
    pub fn unconstrain_popup(&self, popup: &PopupSurface) {
        let bounds = self.popup_bounds(popup);
        popup.with_pending_state(|state| {
            state.geometry = match bounds {
                Some(bounds) => state.positioner.get_unconstrained_geometry(bounds),
                None => state.positioner.get_geometry(),
            };
        });
    }

    /// The output, in the coordinate space the positioner works in: relative
    /// to the window geometry of the popup's immediate parent.
    fn popup_bounds(&self, popup: &PopupSurface) -> Option<Rectangle<i32, Logical>> {
        let kind = PopupKind::Xdg(popup.clone());
        let root = find_popup_root_surface(&kind).ok()?;
        let (monitor, root_origin) = self
            .window_geometry_origin(&root)
            .or_else(|| self.layer_geometry_origin(&root))?;

        let mut bounds = Rectangle::from_size(monitor.geometry().size);
        bounds.loc -= root_origin + get_popup_toplevel_coords(&kind);
        Some(bounds)
    }

    /// Where a toplevel's window geometry starts, output-local, matching the
    /// origin the renderer draws its surface tree at.
    fn window_geometry_origin(&self, root: &WlSurface) -> Option<(&Monitor, Point<i32, Logical>)> {
        let id = self.window_id(root)?;
        let monitor = self.monitor_by_id(self.location(id)?.output)?;
        let tile = self.tile(id)?;
        Some((
            monitor,
            tile.content_rect().loc + tile.window().geometry().loc,
        ))
    }

    fn layer_geometry_origin(&self, root: &WlSurface) -> Option<(&Monitor, Point<i32, Logical>)> {
        let output = self.output_for_layer(root)?;
        let origin = {
            let map = layer_map_for_output(output);
            let layer = map.layer_for_surface(root, WindowSurfaceType::TOPLEVEL)?;
            map.layer_geometry(layer)?.loc
        };
        Some((self.monitor(output)?, origin))
    }
}
