use smithay::{
    desktop::{PopupKind, Window},
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge,
        wayland_server::protocol::{wl_output::WlOutput, wl_seat::WlSeat, wl_surface::WlSurface},
    },
    utils::{Logical, Serial, Size},
    wayland::{
        compositor::with_states,
        shell::xdg::{
            Configure, PopupSurface, PositionerState, SurfaceCachedState, ToplevelSurface,
            XdgShellHandler, XdgShellState, XdgToplevelSurfaceData,
        },
    },
};

use super::xdg_decoration::requested_chrome;
use crate::{
    layout::placement,
    shell::tile::{Chrome, Tile, WindowState},
    state::{State, launch_target::LaunchTarget},
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

        self.shell
            .push_unmapped(window, surface.wl_surface().clone());
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        let wl_surface = surface.wl_surface();

        if self.shell.take_unmapped(wl_surface).is_some() {
            return;
        }

        let Some(id) = self.shell.window_id(wl_surface) else {
            return;
        };
        self.shell.forget_carried(id);
        if !self.close_window(id) {
            self.shell.remove_tile(id);
        }
        self.shell.menus.forget(id);
        self.queue_redraw();
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.shell.unconstrain_popup(&surface);
        if let Err(err) = self.shell.popups.track_popup(PopupKind::Xdg(surface)) {
            tracing::warn!(%err, "failed to track a popup");
        }
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: WlSeat, _serial: Serial) {}

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| state.positioner = positioner);
        self.shell.unconstrain_popup(&surface);
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

    fn minimize_request(&mut self, surface: ToplevelSurface) {
        if let Some(id) = self.shell.window_id(surface.wl_surface()) {
            self.minimize_window(id);
        }
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
            self.shell
                .request_unmapped_state(surface.wl_surface(), state);
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
    fn refresh_rules(&mut self, surface: &ToplevelSurface) {
        let Some(id) = self.shell.window_id(surface.wl_surface()) else {
            return;
        };
        if self.refresh_window_rules(id, surface.wl_surface()) {
            self.queue_redraw();
        }
    }

    /// Re-resolves every mapped window's rules, for an edited rule list.
    pub fn reapply_window_rules(&mut self) {
        let windows: Vec<_> = self
            .shell
            .tiles()
            .map(|tile| (tile.id(), tile.surface().clone()))
            .collect();
        for (id, surface) in windows {
            self.refresh_window_rules(id, &surface);
        }
    }

    /// Only the *presentation* knobs are re-applied. Re-running the float
    /// decision would yank a window the user had placed by hand, so tiling state
    /// is decided once, at map time, and left alone.
    ///
    /// Returns whether the title changed, which the frame has to redraw for.
    fn refresh_window_rules(&mut self, id: WindowId, surface: &WlSurface) -> bool {
        let (app_id, title) = with_states(surface, |states| {
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

        self.shell.tile_mut(id).is_some_and(|tile| {
            tile.apply_presentation_rules(&rules, opacity);
            tile.set_title(title.as_deref().unwrap_or_default())
        })
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

        // Not yet in the shell, so the commit handler skipped it: without this
        // a client that sets no window geometry maps with a zero size.
        unmapped.window.on_commit();
        let (min_size, max_size) = size_hints(surface);
        let carried = self.carried_offset(surface);
        let target = self.launch_target(surface);
        let location = carried
            .and_then(|_| self.carried_location())
            .or_else(|| target.and_then(|target| self.launch_location(target)))
            .or_else(|| self.shell.default_location())?;
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
        let chrome = requested_chrome(&toplevel);
        let dialog = rules.floating.is_none()
            && (auto_float(
                app_id.as_deref(),
                parent.is_some(),
                min_size,
                max_size,
                area.size,
            ) || is_splash(title.as_deref(), chrome));
        if dialog {
            rules.floating = Some(true);
        }
        // Nobody watches a headless window, so it keeps the size it asks for
        // and never takes the keyboard from the user.
        if target == Some(LaunchTarget::Headless) {
            rules.floating = Some(true);
            rules.focus = Some(false);
        }
        // A window born mid-drag rides the cursor until it is dropped.
        if carried.is_some() {
            rules.floating = Some(true);
            rules.fullscreen = Some(false);
            rules.maximized = Some(false);
        }

        let id = WindowId::next();
        let opacity = self.config.current.opacity_for(&rules);
        let mut tile = Tile::new(
            id,
            unmapped.window.clone(),
            surface.clone(),
            rules.clone(),
            opacity,
            self.config.current.appearance.titlebar_height.into(),
        );
        tile.set_size_hints(min_size, max_size);
        tile.set_chrome(chrome);
        tile.set_dialog(dialog);
        let fixed_size = min_size != Size::default() && min_size == max_size;
        tile.set_title(title.as_deref().unwrap_or_default());

        if tile.state().is_floating() {
            // Every pop-up opens in the middle of the screen, whatever opened
            // it; only ordinary floating windows cascade.
            let centered = dialog || fixed_size || parent.is_some();
            tile.set_centered(centered);
            let cascade = self.shell.next_cascade();
            tile.set_floating_rect(placement::initial_rect(
                unmapped.window.geometry().size,
                tile.insets(),
                area,
                centered.then_some(area),
                cascade,
            ));
        }

        let requested = unmapped.requested_state;
        let maximizable = tile.state().is_floating();
        if rules
            .fullscreen
            .unwrap_or(requested == Some(WindowState::Fullscreen))
        {
            tile.set_state(WindowState::Fullscreen);
        } else if rules
            .maximized
            .unwrap_or(maximizable && requested == Some(WindowState::Maximized))
        {
            tile.set_state(WindowState::Maximized);
        }

        self.shell.insert_tile(tile, location);
        self.grow_if_launched(id);

        if rules.focus.unwrap_or(true) {
            self.shell.focus_window(id);
        }
        if let Some(offset) = carried {
            self.carry_mapped(id, offset);
        }

        // A client can name its menu before it maps, in which case the address
        // is already sitting on the surface and nothing has read it yet.
        let address = with_states(surface, protocols::appmenu::menu_address);
        if address.is_some() {
            self.fetch_menu(id, address);
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

const PORTAL_APP_ID_PREFIXES: [&str; 2] =
    ["xdg-desktop-portal", "org.freedesktop.impl.portal.desktop."];

/// What the polkit agents in the wild put in their app id: gnome, kde, mate,
/// xfce, hyprpolkitagent, lxqt-policykit-agent and soteria.
const AUTH_AGENT_APP_ID_MARKERS: [&str; 3] = ["polkit", "policykit", "soteria"];

/// Structural reasons a toplevel should float, ahead of any config rule.
fn auto_float(
    app_id: Option<&str>,
    has_parent: bool,
    min: Size<i32, Logical>,
    max: Size<i32, Logical>,
    area: Size<i32, Logical>,
) -> bool {
    // The file chooser and friends, which run as their own process and so have
    // no parent to be transient for.
    if app_id.is_some_and(|app_id| {
        PORTAL_APP_ID_PREFIXES
            .iter()
            .any(|prefix| app_id.starts_with(prefix))
    }) {
        return true;
    }
    // A password prompt has no parent and often lets itself be resized, yet
    // tiling it would shove the user's windows aside to ask one question.
    if app_id.is_some_and(|app_id| {
        AUTH_AGENT_APP_ID_MARKERS
            .iter()
            .any(|marker| app_id.contains(marker))
    }) {
        return true;
    }
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

/// A frameless window with no title, like DaVinci Resolve's loading screen:
/// there is nothing to call it in a switcher and nothing to drag it by.
fn is_splash(title: Option<&str>, chrome: Chrome) -> bool {
    chrome == Chrome::Client && title.is_none_or(str::is_empty)
}

fn size_hints(surface: &WlSurface) -> (Size<i32, Logical>, Size<i32, Logical>) {
    with_states(surface, |states| {
        let mut cached = states.cached_state.get::<SurfaceCachedState>();
        let current = cached.current();
        (current.min_size, current.max_size)
    })
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

    // Size hints can change at any time and feed the next relayout, and a
    // client that still owns its own size may have just changed it.
    if let Some(id) = state.shell.window_id(surface) {
        let (min, max) = size_hints(surface);
        let resized = state.shell.tile_mut(id).is_some_and(|tile| {
            tile.set_size_hints(min, max);
            tile.adopt_client_size()
        });
        if state.shell.cap_hidden_size(id) || resized {
            state.shell.mark_dirty(id);
        }
    }

    state.shell.popups.commit(surface);

    if let Some(PopupKind::Xdg(popup)) = state.shell.popups.find_popup(surface)
        && !popup.is_initial_configure_sent()
    {
        popup.send_configure().expect("initial configure failed");
    }
}

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
        assert!(auto_float(None, true, unset(), unset(), area()));
    }

    #[test]
    fn a_portal_picker_floats() {
        for app_id in [
            "xdg-desktop-portal-gtk",
            "org.freedesktop.impl.portal.desktop.kde",
        ] {
            assert!(auto_float(Some(app_id), false, unset(), unset(), area()));
        }
        assert!(!auto_float(
            Some("org.gnome.Nautilus"),
            false,
            unset(),
            unset(),
            area()
        ));
    }

    #[test]
    fn a_polkit_prompt_floats() {
        for app_id in [
            "polkit-gnome-authentication-agent-1",
            "org.kde.polkit-kde-authentication-agent-1",
            "hyprpolkitagent",
            "lxqt-policykit-agent",
            "gay.vaskel.soteria",
        ] {
            assert!(auto_float(Some(app_id), false, unset(), unset(), area()));
        }
    }

    #[test]
    fn an_untitled_frameless_window_is_a_splash() {
        assert!(is_splash(None, Chrome::Client));
        assert!(is_splash(Some(""), Chrome::Client));
        assert!(!is_splash(Some("davinci-resolve"), Chrome::Client));
        assert!(!is_splash(None, Chrome::Compositor));
    }

    #[test]
    fn a_fixed_size_toplevel_floats() {
        let fixed = Size::from((400, 300));
        assert!(auto_float(None, false, fixed, fixed, area()));
    }

    #[test]
    fn a_small_max_size_reads_as_a_dialog() {
        assert!(auto_float(None, false, unset(), (400, 300).into(), area()));
    }

    #[test]
    fn an_ordinary_window_tiles() {
        assert!(!auto_float(None, false, unset(), unset(), area()));
        assert!(
            !auto_float(None, false, (400, 300).into(), unset(), area()),
            "a minimum size alone is not a dialog"
        );
    }

    #[test]
    fn a_large_max_size_still_tiles() {
        // A window that merely caps itself below the screen is not a dialog.
        assert!(!auto_float(
            None,
            false,
            unset(),
            (1600, 900).into(),
            area()
        ));
    }
}
