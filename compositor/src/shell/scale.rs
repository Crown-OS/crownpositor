//! What scale clients are asked to render for.
//!
//! A client picks its own buffer scale; all a compositor can do is advertise a
//! preference, and it has three ways to say it — `wp_fractional_scale`'s
//! 120ths, `wl_surface`'s integer `preferred_buffer_scale`, and, for toolkits
//! older than both (GTK3, SDL2), `wl_surface.enter` naming an output whose
//! `wl_output.scale` they read. A client told none of them renders at 1x, and
//! the renderer then stretches that buffer to fill a scaled output. That
//! stretch is why a fractional output looks soft.

use smithay::{
    desktop::{Window, find_popup_root_surface, layer_map_for_output, space::SpaceElement},
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
    pub fn output_for_surface(&self, surface: &WlSurface) -> Option<&Output> {
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
    /// first buffer it draws is already the right size. It is not told it
    /// entered that output, though: a window rule may still place it elsewhere,
    /// and a toolkit that saw two outputs would render for the denser one.
    pub fn advertise_output(&self, surface: &WlSurface) {
        let placed = self.output_for_surface(surface);
        if let Some(output) = placed {
            output.enter(surface);
        }
        if let Some(output) = placed.or_else(|| self.focused_output()) {
            Preference::from(output).send_to(surface);
        }
    }

    /// Re-advertises to everything on `output`, for when the output changed
    /// rather than the surfaces on it, or when surfaces arrived from another
    /// output.
    pub fn readvertise_output(&self, output: &Output) {
        let preference = Preference::from(output);
        self.with_surfaces_on(output, |surface, states| {
            output.enter(surface);
            preference.send(surface, states);
        });
    }

    /// Takes a window off one output and puts it on another, surfaces, popups
    /// and all.
    pub fn transfer_window(&self, window: &Window, from: &Output, to: &Output) {
        window.output_leave(from);
        let preference = Preference::from(to);
        window.with_surfaces(|surface, states| {
            to.enter(surface);
            preference.send(surface, states);
        });
    }

    /// Every surface on `output`: its windows on every workspace and its layer
    /// surfaces, each with their subsurfaces and popups.
    pub fn with_surfaces_on(
        &self,
        output: &Output,
        mut visit: impl FnMut(&WlSurface, &SurfaceData),
    ) {
        let Some(monitor) = self.monitor(output) else {
            return;
        };
        for tile in monitor.workspaces().iter().flat_map(Workspace::tiles) {
            tile.window().with_surfaces(&mut visit);
        }

        // A second guard for the same output deadlocks, so keep the scope tight.
        let map = layer_map_for_output(output);
        for layer in map.layers() {
            layer.with_surfaces(&mut visit);
        }
    }
}
