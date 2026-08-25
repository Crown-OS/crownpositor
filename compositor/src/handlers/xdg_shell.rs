use smithay::{
    delegate_xdg_shell,
    desktop::{PopupKind, Window},
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge,
        wayland_server::protocol::{wl_output::WlOutput, wl_seat::WlSeat, wl_surface::WlSurface},
    },
    utils::{Logical, Serial, Size, SERIAL_COUNTER},
    wayland::{
        compositor::with_states,
        shell::xdg::{
            Configure, PopupSurface, PositionerState, SurfaceCachedState, ToplevelSurface,
            XdgShellHandler, XdgShellState, XdgToplevelSurfaceData,
        },
    },
};

use crate::{
    handlers::seat::KeyboardFocusTarget,
    layout::floating,
    shell::tile::{Tile, WindowState},
    state::State,
    utils::id::WindowId,
};

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.shell.xdg_shell_state
    }

    /// Mapping is deferred to the first buffer commit.
    ///
    /// At this point the client has not committed, so `app_id`, `title`,
    /// `parent()` and the size hints are all unset — and every auto-float
    /// decision depends on exactly those.
    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let wl_surface = surface.wl_surface().clone();
        let window = Window::new_wayland_window(surface.clone());

        // Let the client propose its own size; the layout overrides it on the
        // next refresh.
        surface.with_pending_state(|state| {
            state.size = None;
            state.bounds = self
                .shell
                .focused_monitor()
                .map(|monitor| monitor.usable().size);
        });
        surface.send_configure();

        self.shell.push_unmapped(window, wl_surface);
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        let wl_surface = surface.wl_surface();

        if self.shell.take_unmapped(wl_surface).is_some() {
            return;
        }

        let Some(id) = self.shell.window_id(wl_surface) else {
            return;
        };
        self.shell.remove_tile(id);
        self.queue_redraw();
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        // TODO: unconstrain the popup against its output before tracking it.
        let _ = self.shell.popups.track_popup(PopupKind::Xdg(surface));
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: WlSeat, _serial: Serial) {}

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_geometry();
            state.positioner = positioner;
        });
        surface.send_repositioned(token);
    }

    fn popup_destroyed(&mut self, _surface: PopupSurface) {}

    fn move_request(&mut self, surface: ToplevelSurface, _seat: WlSeat, serial: Serial) {
        if let Some(window) = self.window_for(&surface) {
            self.start_move(&window, serial);
        }
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        _seat: WlSeat,
        serial: Serial,
        edges: ResizeEdge,
    ) {
        if let Some(window) = self.window_for(&surface) {
            self.start_resize(&window, serial, edges);
        }
    }

    /// Only honoured for floating windows.
    ///
    /// A tiled window already fills the slot the layout gave it, so "maximize"
    /// has nothing to add — and many toolkits ask for it on startup, which would
    /// otherwise leave every GTK app maximized and defeat tiling entirely. The
    /// client learns it is tiled from the `Tiled*` states in the configure.
    fn maximize_request(&mut self, surface: ToplevelSurface) {
        let floating = self
            .shell
            .window_id(surface.wl_surface())
            .and_then(|id| self.shell.tile(id))
            .is_some_and(|tile| tile.state().is_floating());

        if floating {
            self.request_state(&surface, Some(WindowState::Maximized));
        } else {
            surface.send_configure();
        }
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        self.request_state(&surface, None);
    }

    fn fullscreen_request(&mut self, surface: ToplevelSurface, _output: Option<WlOutput>) {
        // TODO: honour the requested output by moving the window there first.
        self.request_state(&surface, Some(WindowState::Fullscreen));
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        self.request_state(&surface, None);
    }

    /// Nothing is minimized yet, so acknowledge without changing state rather
    /// than leaving the client waiting on a configure that never comes.
    fn minimize_request(&mut self, surface: ToplevelSurface) {
        surface.send_configure();
    }

    /// `app_id` and `title` can change after the window is mapped, and window
    /// rules matched on them — so re-resolve rather than keeping a stale verdict.
    fn app_id_changed(&mut self, surface: ToplevelSurface) {
        self.refresh_rules(&surface);
    }

    fn title_changed(&mut self, surface: ToplevelSurface) {
        self.refresh_rules(&surface);
    }

    /// Without this the compositor never learns that a client adopted a size,
    /// so a grouped reflow could not be tracked at all.
    fn ack_configure(&mut self, surface: WlSurface, configure: Configure) {
        let Configure::Toplevel(configure) = configure else {
            return;
        };
        if let Some(id) = self.shell.window_id(&surface) {
            self.shell.ack_configure(id, configure.serial);
        }
    }
}

impl State {
    fn window_for(&self, surface: &ToplevelSurface) -> Option<Window> {
        self.shell.window_for_surface(surface.wl_surface()).cloned()
    }

    /// A client asking to enter a state, or `None` to leave the one it is in.
    ///
    /// Always sends a configure, even when nothing changed: the protocol says a
    /// request is answered with one, and a client that does not hear back will
    /// sit waiting.
    fn request_state(&mut self, surface: &ToplevelSurface, state: Option<WindowState>) {
        let Some(id) = self.shell.window_id(surface.wl_surface()) else {
            surface.send_configure();
            return;
        };

        match state {
            Some(state) => self.shell.set_window_state(id, state),
            None => self.shell.restore_window(id),
        };

        self.shell.refresh();
        surface.send_configure();
    }

    /// Re-resolves window rules after `app_id` or `title` changed.
    ///
    /// Only the *presentation* knobs are re-applied. Re-running the float
    /// decision would yank a window the user had placed by hand, so tiling state
    /// is decided once, at map time, and left alone.
    fn refresh_rules(&mut self, surface: &ToplevelSurface) {
        let Some(id) = self.shell.window_id(surface.wl_surface()) else {
            return;
        };

        let (app_id, title) = with_states(surface.wl_surface(), |states| {
            let data = states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .expect("a toplevel always has its xdg data")
                .lock()
                .unwrap();
            (data.app_id.clone(), data.title.clone())
        });

        let rules =
            self.shell
                .resolve_rules(app_id.as_deref(), title.as_deref(), &self.config.current);
        let opacity = self.config.current.opacity_for(&rules);

        if let Some(tile) = self.shell.tile_mut(id) {
            tile.set_opacity(opacity);
        }
    }

    /// Promotes a pending toplevel once its first buffer lands.
    pub fn map_pending(&mut self, surface: &WlSurface) -> Option<WindowId> {
        if !self.shell.pending_unmapped(surface) {
            return None;
        }

        let unmapped = self.shell.take_unmapped(surface)?;
        let toplevel = unmapped.window.toplevel()?.clone();

        let (app_id, title, parent) = with_states(surface, |states| {
            let data = states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .expect("a toplevel always has its xdg data")
                .lock()
                .unwrap();
            (data.app_id.clone(), data.title.clone(), data.parent.clone())
        });

        let (min_size, max_size) = size_hints(surface);
        let location = self.shell.default_location()?;
        let area = self
            .shell
            .workspace(location)
            .map(|workspace| workspace.area())
            .unwrap_or_default();

        let mut rules =
            self.shell
                .resolve_rules(app_id.as_deref(), title.as_deref(), &self.config.current);
        // Structural heuristics apply only where the config is silent, so a rule
        // can always force a dialog back into the tiling.
        if rules.floating.is_none() && auto_float(parent.is_some(), min_size, max_size, area.size) {
            rules.floating = Some(true);
        }

        let id = WindowId::next();
        let opacity = self.config.current.opacity_for(&rules);
        let mut tile = Tile::new(
            id,
            unmapped.window.clone(),
            surface.clone(),
            rules.clone(),
            opacity,
        );
        tile.set_size_hints(min_size, max_size);

        if tile.state().is_floating() {
            let parent_rect = parent
                .as_ref()
                .and_then(|parent| self.shell.window_id(parent))
                .and_then(|parent| self.shell.tile(parent))
                .map(Tile::target);
            let size = size_or(unmapped.window.geometry().size, area.size);
            let cascade = self.shell.next_cascade();
            tile.set_floating_rect(floating::place(area, size, parent_rect, cascade));
        }

        if rules.fullscreen.unwrap_or(false) {
            tile.set_state(WindowState::Fullscreen);
        } else if rules.maximized.unwrap_or(false) {
            tile.set_state(WindowState::Maximized);
        }

        self.shell.insert_tile(tile, location);

        if rules.focus.unwrap_or(true) {
            self.shell.focus_window(id);
        }

        tracing::debug!(
            %id,
            app_id = app_id.as_deref().unwrap_or("<none>"),
            state = ?self.shell.tile(id).map(Tile::state),
            "mapped a toplevel"
        );

        let _ = toplevel;
        Some(id)
    }
}

/// Structural reasons a toplevel should float, ahead of any config rule.
fn auto_float(
    has_parent: bool,
    min: Size<i32, Logical>,
    max: Size<i32, Logical>,
    area: Size<i32, Logical>,
) -> bool {
    // A transient toplevel: Blender's Preferences window, GIMP's dialogs, every
    // "Save As" that is a toplevel rather than a popup.
    if has_parent {
        return true;
    }
    // The client says it is not resizable.
    if min != Size::default() && min == max {
        return true;
    }
    // A max size small in both axes is a dialog by any other name.
    max.w > 0 && max.h > 0 && max.w * 2 < area.w && max.h * 2 < area.h
}

fn size_hints(surface: &WlSurface) -> (Size<i32, Logical>, Size<i32, Logical>) {
    with_states(surface, |states| {
        let mut cached = states.cached_state.get::<SurfaceCachedState>();
        let current = cached.current();
        (current.min_size, current.max_size)
    })
}

fn size_or(size: Size<i32, Logical>, fallback: Size<i32, Logical>) -> Size<i32, Logical> {
    if size.w > 0 && size.h > 0 {
        size
    } else {
        Size::from((fallback.w / 2, fallback.h / 2))
    }
}

/// Sends the initial configure once a surface has committed for the first time.
pub fn handle_commit(state: &mut State, surface: &WlSurface) {
    state.map_pending(surface);

    if let Some(toplevel) = state
        .shell
        .window_for_surface(surface)
        .and_then(Window::toplevel)
        .cloned()
    {
        let initial_configure_sent = with_states(surface, |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .expect("a toplevel always has its xdg data")
                .lock()
                .unwrap()
                .initial_configure_sent
        });

        if !initial_configure_sent {
            toplevel.send_configure();
        }
    }

    // Size hints can change at any time and feed the next relayout.
    if let Some(id) = state.shell.window_id(surface) {
        let (min, max) = size_hints(surface);
        if let Some(tile) = state.shell.tile_mut(id) {
            tile.set_size_hints(min, max);
        }
    }

    state.shell.popups.commit(surface);

    if let Some(PopupKind::Xdg(popup)) = state.shell.popups.find_popup(surface)
        && !popup.is_initial_configure_sent()
    {
        popup.send_configure().expect("initial configure failed");
    }
}

delegate_xdg_shell!(State);

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> Size<i32, Logical> {
        (1920, 1080).into()
    }

    fn unset() -> Size<i32, Logical> {
        Size::default()
    }

    #[test]
    fn a_transient_toplevel_floats() {
        // Blender's Preferences window, GIMP's dialogs, most "Save As".
        assert!(auto_float(true, unset(), unset(), area()));
    }

    #[test]
    fn a_fixed_size_toplevel_floats() {
        let fixed = Size::from((400, 300));
        assert!(auto_float(false, fixed, fixed, area()));
    }

    #[test]
    fn a_small_max_size_reads_as_a_dialog() {
        assert!(auto_float(false, unset(), (400, 300).into(), area()));
    }

    #[test]
    fn an_ordinary_window_tiles() {
        assert!(!auto_float(false, unset(), unset(), area()));
        assert!(
            !auto_float(false, (400, 300).into(), unset(), area()),
            "a minimum size alone is not a dialog"
        );
    }

    #[test]
    fn a_large_max_size_still_tiles() {
        // A window that merely caps itself below the screen is not a dialog.
        assert!(!auto_float(false, unset(), (1600, 900).into(), area()));
    }
}
