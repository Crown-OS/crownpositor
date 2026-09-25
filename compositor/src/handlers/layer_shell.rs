use smithay::{
    desktop::{LayerSurface, WindowSurfaceType, layer_map_for_output},
    output::Output,
    reexports::wayland_server::protocol::{wl_output::WlOutput, wl_surface::WlSurface},
    wayland::{
        compositor::{add_post_commit_hook, add_pre_commit_hook, with_states},
        shell::wlr_layer::{
            Anchor, Layer, LayerSurface as WlrLayerSurface, LayerSurfaceCachedState,
            LayerSurfaceData, WlrLayerShellHandler, WlrLayerShellState,
        },
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

/// Keeps a client alive when it dismisses a panel by destroying its layer
/// surface and then unmapping the `wl_surface` it hung off — the sequence a
/// launcher such as vicinae sends when its window is closed with Esc.
///
/// smithay resets the role's double-buffered state as the role object goes
/// away, but leaves its validation hook on the surface, so the commit that
/// follows is measured against that reset — 0x0 with no anchors — and answered
/// with an `invalid_size` protocol error on an object the client has already
/// destroyed. The client is killed for a request it never made.
///
/// Pre-commit hooks run in the order they were added and smithay adds its own
/// along with the role, so this one has to be installed while the surface is
/// still roleless to land ahead of it. The post-commit hook puts the reset
/// back, so a surface that is later given a fresh layer surface does not
/// inherit an anchor the client never asked for.
pub fn shield_orphaned_layer_state(surface: &WlSurface) {
    add_pre_commit_hook::<State, _>(surface, |state, _, surface| {
        if state.shell.tracks_layer(surface) {
            return;
        }
        with_orphaned_layer_state(surface, |orphan| orphan.anchor = Anchor::all());
    });

    add_post_commit_hook::<State, _>(surface, |state, _, surface| {
        if state.shell.tracks_layer(surface) {
            return;
        }
        with_orphaned_layer_state(surface, |orphan| {
            *orphan = LayerSurfaceCachedState::default()
        });
    });
}

/// Applies `edit` to both halves of the double-buffered state a destroyed
/// layer surface left behind on its `wl_surface`.
fn with_orphaned_layer_state(surface: &WlSurface, edit: impl Fn(&mut LayerSurfaceCachedState)) {
    with_states(surface, |states| {
        if states.data_map.get::<LayerSurfaceData>().is_none() {
            return;
        }

        let mut cached = states.cached_state.get::<LayerSurfaceCachedState>();
        edit(cached.pending());
        edit(cached.current());
    });
}
