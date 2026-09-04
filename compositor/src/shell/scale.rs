//! What scale clients are asked to render for.
//!
//! A client picks its own buffer scale; all a compositor can do is advertise a
//! preference, and it has two ways to say it — `wl_surface`'s integer
//! `preferred_buffer_scale` and `wp_fractional_scale`'s 120ths. A client told
//! neither renders at 1x, and the renderer then stretches that buffer to fill a
//! scaled output. That stretch is why a fractional output looks soft.

use smithay::{
    desktop::{find_popup_root_surface, layer_map_for_output},
    output::Output,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::Transform,
    wayland::{
        compositor::{SurfaceData, get_parent, send_surface_state, with_states},
        fractional_scale::with_fractional_scale,
    },
};

use crate::shell::{Shell, monitor::Monitor, workspace::Workspace};

/// One output's rendering preference, in both dialects.
#[derive(Clone, Copy)]
struct Preference {
    /// `wl_output` and `wl_surface` only speak whole numbers, so a fractional
    /// output rounds up and lets the renderer scale back down. Downsampling
    /// costs sharpness; upsampling costs detail that was never drawn.
    integer: i32,
    fractional: f64,
}

impl From<&Output> for Preference {
    fn from(output: &Output) -> Self {
        let scale = output.current_scale();
        Self {
            integer: scale.integer_scale(),
            fractional: scale.fractional_scale(),
        }
    }
}

impl Preference {
    /// Both calls compare against what the surface was last told, so repeating
    /// an unchanged preference sends nothing.
    ///
    /// The transform is always `Normal`: an output's is currently whatever the
    /// backend needs to correct its own framebuffer — winit flips 180 because
    /// it renders bottom-up — and asking clients to pre-transform for that is
    /// not something the compositor can honestly promise yet.
    fn send(&self, surface: &WlSurface, states: &SurfaceData) {
        send_surface_state(surface, states, self.integer, Transform::Normal);
        with_fractional_scale(states, |scale| scale.set_preferred_scale(self.fractional));
    }

    fn send_to(&self, surface: &WlSurface) {
        with_states(surface, |states| self.send(surface, states));
    }
}

impl Shell {
    /// The output a surface anywhere in a window's or layer surface's tree
    /// lives on.
    ///
    /// Subsurfaces are walked first because that answers for every ordinary
    /// commit; only a surface that turns out to be nobody's window or layer is
    /// worth searching the popup tree for, popups being parented outside
    /// `wl_subsurface`.
    fn output_for_surface(&self, surface: &WlSurface) -> Option<&Output> {
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }

        self.output_hosting(&root).or_else(|| {
            let popup = self.popups.find_popup(&root)?;
            self.output_hosting(&find_popup_root_surface(&popup).ok()?)
        })
    }

    fn output_hosting(&self, root: &WlSurface) -> Option<&Output> {
        self.window_id(root)
            .and_then(|id| self.location(id))
            .and_then(|at| self.monitor_by_id(at.output))
            .map(Monitor::output)
            .or_else(|| self.output_for_layer(root))
    }

    /// Advertises to a single surface, on commit.
    ///
    /// Every subsurface and popup commits before it can show anything, so
    /// visiting only the committing surface still reaches all of them.
    ///
    /// A surface on no output yet — a toplevel between its first configure and
    /// its first buffer — is given the focused output's preference, so the very
    /// first buffer it draws is already the right size.
    pub fn advertise_scale(&self, surface: &WlSurface) {
        let Some(output) = self
            .output_for_surface(surface)
            .or_else(|| self.focused_output())
        else {
            return;
        };
        Preference::from(output).send_to(surface);
    }

    /// Re-advertises to everything on `output`, for when the output changed
    /// rather than the surfaces on it.
    pub fn advertise_output_scale(&self, output: &Output) {
        let Some(monitor) = self.monitor(output) else {
            return;
        };
        let preference = Preference::from(output);

        for tile in monitor.workspaces().iter().flat_map(Workspace::tiles) {
            tile.window()
                .with_surfaces(|surface, states| preference.send(surface, states));
        }

        // A second guard for the same output deadlocks, so keep the scope tight.
        let map = layer_map_for_output(output);
        for layer in map.layers() {
            layer.with_surfaces(|surface, states| preference.send(surface, states));
        }
    }
}
