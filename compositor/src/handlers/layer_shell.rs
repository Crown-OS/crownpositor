use smithay::{
    delegate_layer_shell,
    desktop::{LayerSurface, WindowSurfaceType, layer_map_for_output},
    output::Output,
    reexports::wayland_server::protocol::{wl_output::WlOutput, wl_surface::WlSurface},
    wayland::shell::wlr_layer::{
        Layer, LayerSurface as WlrLayerSurface, WlrLayerShellHandler, WlrLayerShellState,
    },
};

use crate::state::State;

impl WlrLayerShellHandler for State {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.shell.layer_shell
    }

    fn new_layer_surface(
        &mut self,
        surface: WlrLayerSurface,
        wl_output: Option<WlOutput>,
        _layer: Layer,
        namespace: String,
    ) {
        // A null output does not mean "close" — the compositor picks.
        let output = wl_output
            .as_ref()
            .and_then(Output::from_resource)
            .filter(|output| self.shell.contains_output(output))
            .or_else(|| self.shell.focused_output().cloned());

        let Some(output) = output else {
            tracing::warn!(
                namespace,
                "no output available for a layer surface, closing"
            );
            surface.send_close();
            return;
        };

        let layer = LayerSurface::new(surface, namespace);
        let wl_surface = layer.wl_surface().clone();

        // A second guard for the same output deadlocks, so keep the scope tight.
        let mapped = {
            let mut map = layer_map_for_output(&output);
            map.map_layer(&layer)
        };

        if let Err(err) = mapped {
            tracing::warn!(%err, "failed to map a layer surface, closing");
            layer.layer_surface().send_close();
            return;
        }

        self.shell.track_layer(wl_surface, &output);
        self.shell.refresh_usable(&output);
    }

    /// The trait default is a no-op, so without this a bar that exits leaves its
    /// exclusive zone reserved forever.
    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        let Some(output) = self.shell.untrack_layer(surface.wl_surface()) else {
            return;
        };

        {
            let mut map = layer_map_for_output(&output);
            let layer = map
                .layer_for_surface(surface.wl_surface(), WindowSurfaceType::TOPLEVEL)
                .cloned();
            if let Some(layer) = layer {
                map.unmap_layer(&layer);
            }
        }

        self.shell.refresh_usable(&output);
    }
}

impl State {
    /// `LayerMap::arrange` deliberately never sends the *initial* configure: the
    /// protocol requires it in response to the initial commit, so the client can
    /// set a size first. Without this a bar maps and then waits forever.
    pub fn handle_layer_commit(&mut self, surface: &WlSurface) {
        let Some(output) = self.shell.output_for_layer(surface).cloned() else {
            return;
        };

        {
            let mut map = layer_map_for_output(&output);
            // Anchors, margins and the exclusive zone can all change at runtime.
            map.arrange();

            if let Some(layer) = map.layer_for_surface(surface, WindowSurfaceType::TOPLEVEL) {
                layer.layer_surface().send_pending_configure();
            }
        }

        self.shell.refresh_usable(&output);
    }
}

delegate_layer_shell!(State);
