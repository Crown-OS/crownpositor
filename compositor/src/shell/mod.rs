//! The model: monitors own workspaces, workspaces own tiles, and the indices
//! here are the only way to get from a surface to any of it.

pub mod grab;
pub mod monitor;
pub mod tile;
pub mod transaction;
pub mod workspace;
pub mod workspace_switch;

use std::{collections::HashMap, time::Instant};

use smithay::{
    desktop::{layer_map_for_output, space::SpaceElement, PopupManager, Window, WindowSurfaceType},
    output::Output,
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel::State as XdgState,
        wayland_server::{protocol::wl_surface::WlSurface, DisplayHandle},
    },
    utils::{Logical, Point, Rectangle, Size},
    wayland::shell::{
        wlr_layer::WlrLayerShellState,
        xdg::{ToplevelStateSet, XdgShellState},
    },
};

use config::{Config, ResolvedRule};

use crate::{
    animations::spring::SpringProfile,
    layout::{Direction, Gaps, LayoutKind, LayoutOp},
    shell::{
        monitor::{
            output_from_descriptor, output_id, ConnectorId, Monitor, OutputConfig, OutputDescriptor,
        },
        tile::{Tile, WindowState},
        transaction::Transaction,
        workspace::{Workspace, WorkspaceRef},
    },
    state::State,
    utils::id::{OutputId, WindowId, WorkspaceId},
};

/// Writes a tile's geometry and state into its toplevel's pending state.
///
/// `send_pending_configure` already returns `None` when nothing differs, so
/// re-writing an unchanged state costs a comparison and sends nothing.
fn configure(
    tile: &mut Tile,
    bounds: Size<i32, Logical>,
    activated: bool,
) -> Option<smithay::utils::Serial> {
    let toplevel = tile.toplevel().cloned()?;
    let state = tile.state();
    let size = tile.target().size;

    toplevel.with_pending_state(|pending| {
        pending.size = Some(size);
        pending.bounds = Some(bounds);

        toggle(&mut pending.states, XdgState::Activated, activated);
        toggle(
            &mut pending.states,
            XdgState::Maximized,
            state == WindowState::Maximized,
        );
        toggle(
            &mut pending.states,
            XdgState::Fullscreen,
            state == WindowState::Fullscreen,
        );

        // Tells the client its edges are not freely resizable, so it can drop
        // its own shadows and rounded corners.
        let tiled = state.is_tiled();
        for edge in [
            XdgState::TiledLeft,
            XdgState::TiledRight,
            XdgState::TiledTop,
            XdgState::TiledBottom,
        ] {
            toggle(&mut pending.states, edge, tiled);
        }
    });

    let serial = toplevel.send_pending_configure()?;
    tile.record_sent(size, serial);
    Some(serial)
}

fn toggle(states: &mut ToplevelStateSet, state: XdgState, on: bool) {
    if on {
        states.set(state);
    } else {
        states.unset(state);
    }
}

/// Which workspace on which output a window lives on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    pub output: OutputId,
    pub workspace: WorkspaceId,
}

/// A toplevel that exists but has not committed a buffer yet.
///
/// It has no `WindowId` and is on no workspace, because until the first buffer
/// lands we do not know its `app_id`, parent or size hints — and every
/// auto-float decision depends on exactly those.
pub struct UnmappedWindow {
    pub window: Window,
    pub surface: WlSurface,
}

pub struct Shell {
    monitors: Vec<Monitor>,
    focused_output: usize,
    /// Workspaces rescued from the last output to be unplugged. Without this, a
    /// lid close on a single-output session would destroy every window.
    headless: Vec<Workspace>,
    migrated: HashMap<ConnectorId, Vec<WorkspaceId>>,

    surface_to_window: HashMap<WlSurface, WindowId>,
    window_to_location: HashMap<WindowId, Location>,
    layer_to_output: HashMap<WlSurface, OutputId>,

    pub xdg_shell_state: XdgShellState,
    pub layer_shell: WlrLayerShellState,
    pub popups: PopupManager,

    pub activated: Option<Window>,

    global_layout: LayoutKind,
    gaps: Gaps,
    /// The feel every monitor's viewport is given, kept here so a monitor
    /// plugged in later gets the same one. `None` disables motion.
    animation: Option<SpringProfile>,
    unmapped: Vec<UnmappedWindow>,
    /// Grouped configures still waiting on their acks.
    transactions: Vec<Transaction>,
    /// Bumped per floating placement so a burst of dialogs cascades.
    cascade: usize,
}

impl Shell {
    pub fn try_new(display: &DisplayHandle, config: &Config) -> anyhow::Result<Self> {
        Ok(Self {
            monitors: Vec::new(),
            focused_output: 0,
            headless: Vec::new(),
            migrated: HashMap::new(),
            surface_to_window: HashMap::new(),
            window_to_location: HashMap::new(),
            layer_to_output: HashMap::new(),
            xdg_shell_state: XdgShellState::new::<State>(display),
            layer_shell: WlrLayerShellState::new::<State>(display),
            popups: PopupManager::default(),
            activated: None,
            global_layout: config.compositor.layout.into(),
            gaps: Gaps {
                inner: config.appearance.gaps_inner.into(),
                outer: config.appearance.gaps_outer.into(),
            },
            animation: SpringProfile::from_config(config.appearance.animations),
            unmapped: Vec::new(),
            transactions: Vec::new(),
            cascade: 0,
        })
    }

    // ---- outputs ----

    pub fn monitors(&self) -> &[Monitor] {
        &self.monitors
    }

    pub fn is_empty(&self) -> bool {
        self.monitors.is_empty()
    }

    pub fn monitor(&self, output: &Output) -> Option<&Monitor> {
        let id = output_id(output);
        self.monitors.iter().find(|monitor| monitor.id() == id)
    }

    pub fn monitor_mut(&mut self, output: &Output) -> Option<&mut Monitor> {
        let id = output_id(output);
        self.monitors.iter_mut().find(|monitor| monitor.id() == id)
    }

    pub fn monitor_by_id(&self, id: OutputId) -> Option<&Monitor> {
        self.monitors.iter().find(|monitor| monitor.id() == id)
    }

    pub fn monitor_by_id_mut(&mut self, id: OutputId) -> Option<&mut Monitor> {
        self.monitors.iter_mut().find(|monitor| monitor.id() == id)
    }

    pub fn focused_monitor(&self) -> Option<&Monitor> {
        self.monitors.get(self.focused_output)
    }

    pub fn focused_monitor_mut(&mut self) -> Option<&mut Monitor> {
        self.monitors.get_mut(self.focused_output)
    }

    pub fn focused_output(&self) -> Option<&Output> {
        self.focused_monitor().map(Monitor::output)
    }

    pub fn contains_output(&self, output: &Output) -> bool {
        self.monitor(output).is_some()
    }

    /// The monitor containing a global logical point.
    pub fn monitor_at(&self, location: Point<f64, Logical>) -> Option<&Monitor> {
        self.monitors
            .iter()
            .find(|monitor| monitor.geometry().to_f64().contains(location))
    }

    /// The only way an `Output` enters the compositor.
    pub fn add_output(
        &mut self,
        dh: &DisplayHandle,
        config: &Config,
        descriptor: OutputDescriptor,
    ) -> Output {
        let output = output_from_descriptor(&descriptor);
        let global = output.create_global::<State>(dh);
        let id = output_id(&output);

        let connector = ConnectorId::new(
            &descriptor.name,
            &descriptor.physical.make,
            &descriptor.physical.model,
            descriptor.serial.as_deref(),
        );

        let mut output_config = OutputConfig {
            name: descriptor.name.clone(),
            connector: connector.clone(),
            mode: descriptor.current,
            preferred_mode: descriptor.preferred,
            modes: descriptor.modes.clone(),
            scale: output.current_scale(),
            transform: descriptor.native_transform,
            position: (0, 0).into(),
            refresh_interval: descriptor.refresh_interval,
            enabled: true,
            default_layout: None,
        };

        if let Some(setting) = config.output_setting(&descriptor.name, connector.as_str()) {
            output_config.default_layout = setting.layout.map(Into::into);
            output_config.enabled = setting.enabled.unwrap_or(true);
        }

        let mut monitor = Monitor::new(
            id,
            output.clone(),
            Some(global),
            output_config,
            self.global_layout,
            self.gaps,
        );
        monitor.set_animation_profile(self.animation);
        self.monitors.push(monitor);

        self.reclaim_migrated(&connector, id);
        self.adopt_headless(id);
        self.apply_output_settings(config);
        self.arrange_outputs();
        self.refresh_usable(&output);

        output
    }

    /// The only way an `Output` leaves.
    pub fn remove_output(&mut self, dh: &DisplayHandle, output: &Output) {
        let Some(index) = self
            .monitors
            .iter()
            .position(|monitor| monitor.id() == output_id(output))
        else {
            return;
        };

        // Layer surfaces cannot migrate: a `zwlr_layer_surface_v1` is bound to
        // one output for its lifetime.
        {
            let map = layer_map_for_output(output);
            for layer in map.layers() {
                layer.layer_surface().send_close();
            }
        }
        self.layer_to_output
            .retain(|_, owner| *owner != output_id(output));

        let survivor = (0..self.monitors.len())
            .find(|candidate| *candidate != index)
            .map(|candidate| self.monitors[candidate].id());

        let mut monitor = self.monitors.remove(index);
        let connector = monitor.config().connector.clone();

        // Drop the trailing empty; it carries nothing worth moving.
        let mut orphans = monitor.drain_workspaces();
        if orphans.last().is_some_and(Workspace::is_empty) {
            orphans.pop();
        }

        match survivor {
            Some(target) => {
                let moved: Vec<WorkspaceId> = orphans.iter().map(Workspace::id).collect();
                self.migrated.insert(connector, moved);
                for workspace in orphans {
                    self.adopt(workspace, target);
                }
            }
            // No survivor: park them rather than destroying every window.
            None => {
                for workspace in orphans {
                    for tile in workspace.tiles() {
                        tile.window().output_leave(output);
                    }
                    self.headless.push(workspace);
                }
            }
        }

        if let Some(global) = monitor.take_global() {
            dh.remove_global::<State>(global);
        }
        self.focused_output = self
            .focused_output
            .min(self.monitors.len().saturating_sub(1));

        self.arrange_outputs();
        self.normalize_all();
    }

    /// Moves a rescued workspace onto a monitor, before its trailing empty.
    fn adopt(&mut self, workspace: Workspace, target: OutputId) {
        let ids: Vec<WindowId> = workspace.tiles().iter().map(Tile::id).collect();
        let workspace_id = workspace.id();

        let Some(monitor) = self.monitor_by_id_mut(target) else {
            self.headless.push(workspace);
            return;
        };

        let mut workspace = workspace;
        workspace.set_output(target);
        let output = monitor.output().clone();
        for tile in workspace.tiles() {
            tile.window().output_enter(&output, Rectangle::default());
        }
        monitor.push_workspace(workspace);

        for id in ids {
            self.window_to_location.insert(
                id,
                Location {
                    output: target,
                    workspace: workspace_id,
                },
            );
        }
    }

    fn reclaim_migrated(&mut self, connector: &ConnectorId, target: OutputId) {
        let Some(ids) = self.migrated.remove(connector) else {
            return;
        };
        for id in ids {
            let Some(source) = self
                .monitors
                .iter()
                .position(|monitor| monitor.index_of(id).is_some())
            else {
                continue;
            };
            if self.monitors[source].id() == target {
                continue;
            }
            let Some(workspace) = self.monitors[source].take_workspace(id) else {
                continue;
            };
            self.adopt(workspace, target);
        }
    }

    fn adopt_headless(&mut self, target: OutputId) {
        for workspace in std::mem::take(&mut self.headless) {
            self.adopt(workspace, target);
        }
    }

    /// Places every monitor and keeps the list sorted by x, so index order
    /// matches spatial order — which is what `Direction::Left/Right` and
    /// `monitor_at` rely on.
    ///
    /// Config-pinned outputs keep their position; the rest are packed left to
    /// right into the space that is left.
    pub fn arrange_outputs(&mut self) {
        for monitor in &mut self.monitors {
            if let Some(fixed) = monitor.fixed_position() {
                monitor.set_position(fixed);
            }
        }

        // Start packing past the rightmost pinned output, so an auto output
        // cannot land on top of one the user placed deliberately.
        let mut x = self
            .monitors
            .iter()
            .filter(|monitor| monitor.fixed_position().is_some())
            .map(|monitor| monitor.geometry().loc.x + monitor.geometry().size.w)
            .max()
            .unwrap_or(0)
            .max(0);

        for monitor in &mut self.monitors {
            if monitor.fixed_position().is_some() {
                continue;
            }
            monitor.set_position((x, 0).into());
            x += monitor.config().logical_size().w;
        }

        self.monitors
            .sort_by_key(|monitor| monitor.config().position.x);
    }

    /// Re-derives one output's usable area from its layer map.
    pub fn refresh_usable(&mut self, output: &Output) -> bool {
        let zone = {
            let mut map = layer_map_for_output(output);
            map.arrange();
            map.non_exclusive_zone()
        };
        self.monitor_mut(output)
            .is_some_and(|monitor| monitor.set_usable(zone))
    }

    /// Pushes the per-output half of the config onto every monitor.
    pub fn apply_output_settings(&mut self, config: &Config) {
        let fallback = config.display.scale.factor();
        let mut changed = false;

        for index in 0..self.monitors.len() {
            let (connector, name) = {
                let monitor = &self.monitors[index];
                (
                    monitor.config().connector.as_str().to_owned(),
                    monitor.config().name.clone(),
                )
            };
            let setting = config.output_setting(&name, &connector).cloned();
            changed |= self.monitors[index].apply_settings(setting.as_ref(), fallback);
        }

        if changed {
            self.arrange_outputs();
            let outputs: Vec<Output> = self
                .monitors
                .iter()
                .map(|monitor| monitor.output().clone())
                .collect();
            for output in outputs {
                self.refresh_usable(&output);
            }
        }
    }

    pub fn normalize_all(&mut self) {
        let global = self.global_layout;
        for monitor in &mut self.monitors {
            monitor.normalize(global);
        }
    }

    // ---- layer surfaces ----

    pub fn track_layer(&mut self, surface: WlSurface, output: &Output) {
        self.layer_to_output.insert(surface, output_id(output));
    }

    pub fn untrack_layer(&mut self, surface: &WlSurface) -> Option<Output> {
        let id = self.layer_to_output.remove(surface)?;
        self.monitor_by_id(id)
            .map(|monitor| monitor.output().clone())
    }

    pub fn output_for_layer(&self, surface: &WlSurface) -> Option<&Output> {
        let id = self.layer_to_output.get(surface)?;
        self.monitor_by_id(*id).map(Monitor::output)
    }

    // ---- windows ----

    pub fn global_layout(&self) -> LayoutKind {
        self.global_layout
    }

    pub fn set_global_layout(&mut self, kind: LayoutKind) {
        if self.global_layout == kind {
            return;
        }
        self.global_layout = kind;
        for monitor in &mut self.monitors {
            for workspace in monitor.workspaces_mut() {
                workspace.dirty.layout = true;
            }
        }
    }

    pub fn set_gaps(&mut self, gaps: Gaps) {
        if self.gaps == gaps {
            return;
        }
        self.gaps = gaps;
        for monitor in &mut self.monitors {
            monitor.set_gaps(gaps);
        }
    }

    /// The feel every viewport animates with. `None` switches motion off.
    pub fn set_workspace_animation(&mut self, profile: Option<SpringProfile>) {
        self.animation = profile;
        for monitor in &mut self.monitors {
            monitor.set_animation_profile(profile);
        }
    }

    // ---- interactive workspace switching ----

    pub fn is_swiping_workspaces(&self) -> bool {
        self.focused_monitor().is_some_and(Monitor::is_swiping)
    }

    /// Hands the focused output's viewport to the fingers.
    pub fn begin_workspace_swipe(&mut self) {
        if let Some(monitor) = self.focused_monitor_mut() {
            monitor.begin_switch_gesture();
        }
    }

    /// `travelled` is cumulative horizontal travel in pages, positive
    /// rightward.
    pub fn update_workspace_swipe(&mut self, travelled: f64) {
        if let Some(monitor) = self.focused_monitor_mut() {
            monitor.update_switch_gesture(travelled);
        }
    }

    /// Releases the swipe into its spring. `velocity` is in pages per second,
    /// positive rightward; returns whether the active workspace moved.
    pub fn end_workspace_swipe(&mut self, velocity: f64) -> bool {
        let global = self.global_layout;
        self.focused_monitor_mut()
            .is_some_and(|monitor| monitor.end_switch_gesture(velocity, global))
    }

    pub fn cancel_workspace_swipe(&mut self) {
        if let Some(monitor) = self.focused_monitor_mut() {
            monitor.cancel_switch_gesture();
        }
    }

    pub fn window_id(&self, surface: &WlSurface) -> Option<WindowId> {
        self.surface_to_window.get(surface).copied()
    }

    pub fn location(&self, id: WindowId) -> Option<Location> {
        self.window_to_location.get(&id).copied()
    }

    pub fn tile(&self, id: WindowId) -> Option<&Tile> {
        let location = self.location(id)?;
        self.workspace(location)?.tile(id)
    }

    pub fn tile_mut(&mut self, id: WindowId) -> Option<&mut Tile> {
        let location = self.location(id)?;
        self.workspace_mut(location)?.tile_mut(id)
    }

    pub fn workspace(&self, location: Location) -> Option<&Workspace> {
        self.monitor_by_id(location.output)?
            .workspace(location.workspace)
    }

    pub fn workspace_mut(&mut self, location: Location) -> Option<&mut Workspace> {
        self.monitor_by_id_mut(location.output)?
            .workspace_mut(location.workspace)
    }

    /// O(1), replacing the linear scan over every window on every commit.
    pub fn window_for_surface(&self, surface: &WlSurface) -> Option<&Window> {
        self.tile(self.window_id(surface)?).map(Tile::window)
    }

    pub fn pending_unmapped(&self, surface: &WlSurface) -> bool {
        self.unmapped.iter().any(|entry| &entry.surface == surface)
    }

    pub fn push_unmapped(&mut self, window: Window, surface: WlSurface) {
        self.unmapped.push(UnmappedWindow { window, surface });
    }

    pub fn take_unmapped(&mut self, surface: &WlSurface) -> Option<UnmappedWindow> {
        let index = self
            .unmapped
            .iter()
            .position(|entry| &entry.surface == surface)?;
        Some(self.unmapped.remove(index))
    }

    pub fn next_cascade(&mut self) -> usize {
        let cascade = self.cascade;
        self.cascade = self.cascade.wrapping_add(1);
        cascade
    }

    /// One of two functions that may change tile membership.
    pub fn insert_tile(&mut self, tile: Tile, at: Location) -> bool {
        let (id, surface) = (tile.id(), tile.surface().clone());
        let Some(workspace) = self.workspace_mut(at) else {
            return false;
        };
        workspace.push_tile(tile);
        self.surface_to_window.insert(surface, id);
        self.window_to_location.insert(id, at);
        true
    }

    /// The other. Also clears the window from every layout's side tables, so an
    /// algorithm cannot retain a dead id.
    pub fn remove_tile(&mut self, id: WindowId) -> Option<Tile> {
        let at = self.window_to_location.remove(&id)?;
        let workspace = self.workspace_mut(at)?;
        let tile = workspace.take_tile(id)?;
        self.surface_to_window.remove(tile.surface());
        for transaction in &mut self.transactions {
            transaction.forget(id);
        }
        Some(tile)
    }

    /// Take-then-insert, so a failed destination cannot leave a half-updated
    /// index behind.
    pub fn move_tile(&mut self, id: WindowId, to: Location) -> bool {
        let Some(tile) = self.remove_tile(id) else {
            return false;
        };
        self.insert_tile(tile, to)
    }

    pub fn focus_window(&mut self, id: WindowId) -> bool {
        let Some(location) = self.location(id) else {
            return false;
        };
        if let Some(index) = self
            .monitors
            .iter()
            .position(|monitor| monitor.id() == location.output)
        {
            self.focused_output = index;
        }
        let global = self.global_layout;
        let Some(workspace) = self.workspace_mut(location) else {
            return false;
        };
        let changed = workspace.focus_window(id);
        // Harmless for layouts that fit everything on screen.
        workspace.reveal_focus(global);
        changed
    }

    pub fn focused_window(&self) -> Option<&Window> {
        let monitor = self.focused_monitor()?;
        let workspace = monitor.active();
        workspace.tile(workspace.focus()?).map(Tile::window)
    }

    /// Where a new window should go.
    pub fn default_location(&self) -> Option<Location> {
        let monitor = self.focused_monitor()?;
        Some(Location {
            output: monitor.id(),
            workspace: monitor.active().id(),
        })
    }

    pub fn resolve_rules(
        &self,
        app_id: Option<&str>,
        title: Option<&str>,
        config: &Config,
    ) -> ResolvedRule {
        config.window_rules.resolve(app_id, title)
    }

    /// The topmost window at a global logical point, and where it sits.
    ///
    /// Walks the same workspaces, in the same order, that the renderer draws,
    /// so what you click is what you see on top — including mid-switch, where
    /// two workspaces share the output and both are shifted sideways.
    pub fn window_under(
        &self,
        location: Point<f64, Logical>,
    ) -> Option<(WindowId, Point<i32, Logical>)> {
        let monitor = self.monitor_at(location)?;
        let origin = monitor.geometry().loc;
        let output_local = location - origin.to_f64();

        monitor
            .visible_workspaces()
            .find_map(|(workspace, offset)| {
                let local = output_local - offset;
                let at = |tile: &Tile| {
                    (
                        tile.id(),
                        origin + (tile.target().loc.to_f64() + offset).to_i32_round(),
                    )
                };

                // A fullscreen window swallows every click on its workspace.
                if let Some(tile) = workspace.fullscreen().and_then(|id| workspace.tile(id)) {
                    return Some(at(tile));
                }

                workspace
                    .stacking_order()
                    .find(|tile| {
                        tile.target().to_f64().contains(local)
                            && tile
                                .window()
                                .is_in_input_region(&(local - tile.target().loc.to_f64()))
                    })
                    .map(at)
            })
    }

    pub fn surface_under(
        &self,
        location: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        let (id, window_location) = self.window_under(location)?;
        let window = self.tile(id)?.window();
        window
            .surface_under(location - window_location.to_f64(), WindowSurfaceType::ALL)
            .map(|(surface, offset)| (surface, (offset + window_location).to_f64()))
    }

    // ---- actions ----

    pub fn switch_workspace(&mut self, target: WorkspaceRef) -> bool {
        let global = self.global_layout;
        self.focused_monitor_mut()
            .is_some_and(|monitor| monitor.switch_to(target, global))
    }

    /// Moves the focused window to another workspace on the same output.
    pub fn move_focused_to_workspace(&mut self, target: WorkspaceRef, follow: bool) -> bool {
        let Some(id) = self.focused_window_id() else {
            return false;
        };
        let Some(monitor) = self.focused_monitor() else {
            return false;
        };

        let index = monitor.resolve(target);
        let Some(destination) = monitor.workspaces().get(index).map(Workspace::id) else {
            return false;
        };
        let location = Location {
            output: monitor.id(),
            workspace: destination,
        };
        if self.location(id) == Some(location) {
            return false;
        }

        if !self.move_tile(id, location) {
            return false;
        }
        if let Some(workspace) = self.workspace_mut(location) {
            workspace.focus_window(id);
        }
        if follow {
            self.switch_workspace(WorkspaceRef::Index(index));
        } else {
            // The window left, so the source may now be reapable.
            let global = self.global_layout;
            if let Some(monitor) = self.focused_monitor_mut() {
                monitor.normalize(global);
            }
        }
        true
    }

    pub fn focused_window_id(&self) -> Option<WindowId> {
        self.focused_monitor()?.active().focus()
    }

    /// Directional focus: the layout's own answer first, then list order.
    pub fn focus_direction(&mut self, dir: Direction) -> bool {
        let Some(next) = self.neighbour(dir) else {
            return false;
        };
        self.focus_window(next)
    }

    fn neighbour(&self, dir: Direction) -> Option<WindowId> {
        let workspace = self.focused_monitor()?.active();
        let from = workspace.focus()?;
        workspace.neighbour(from, dir, self.global_layout)
    }

    /// Moves the focused window within its workspace's layout order.
    pub fn move_focused(&mut self, dir: Direction) -> bool {
        let Some(from) = self.focused_window_id() else {
            return false;
        };
        let Some(to) = self.neighbour(dir) else {
            return false;
        };
        let Some(location) = self.location(from) else {
            return false;
        };
        self.workspace_mut(location)
            .is_some_and(|workspace| workspace.swap(from, to))
    }

    pub fn focus_output_direction(&mut self, dir: Direction) -> bool {
        // The monitor list is kept sorted by x, so this is index arithmetic.
        let next = match dir {
            Direction::Left | Direction::Up => self.focused_output.checked_sub(1),
            Direction::Right | Direction::Down => Some(self.focused_output + 1),
        };
        let Some(next) = next.filter(|index| *index < self.monitors.len()) else {
            return false;
        };
        self.focused_output = next;
        true
    }

    pub fn move_focused_to_output(&mut self, dir: Direction) -> bool {
        let Some(id) = self.focused_window_id() else {
            return false;
        };
        let from = self.focused_output;
        if !self.focus_output_direction(dir) {
            return false;
        }

        let Some(monitor) = self.focused_monitor() else {
            self.focused_output = from;
            return false;
        };
        let (output, workspace) = (monitor.id(), monitor.active().id());
        let target_output = monitor.output().clone();
        let source_output = self.monitors[from].output().clone();

        if !self.move_tile(id, Location { output, workspace }) {
            return false;
        }

        if let Some(tile) = self.tile(id) {
            tile.window().output_leave(&source_output);
            tile.window()
                .output_enter(&target_output, Rectangle::default());
        }
        self.focus_window(id);
        self.normalize_all();
        true
    }

    pub fn toggle_floating(&mut self) -> bool {
        let Some(id) = self.focused_window_id() else {
            return false;
        };
        let area = self
            .location(id)
            .and_then(|at| self.workspace(at))
            .map(Workspace::area);
        let cascade = self.next_cascade();

        let Some(area) = area else { return false };
        let Some(tile) = self.tile_mut(id) else {
            return false;
        };

        tile.toggle_floating();
        if tile.state().is_floating() && tile.floating_rect().size.w == 0 {
            let size = Size::from((area.size.w / 2, area.size.h / 2));
            tile.set_floating_rect(crate::layout::floating::place(area, size, None, cascade));
        }
        self.mark_focused_dirty();
        true
    }

    /// Fullscreen and maximize are the same shape: enter the state, or leave it
    /// if already in it.
    pub fn toggle_window_state(&mut self, state: WindowState) -> bool {
        let Some(id) = self.focused_window_id() else {
            return false;
        };
        self.toggle_state_of(id, state)
    }

    pub fn toggle_state_of(&mut self, id: WindowId, state: WindowState) -> bool {
        let Some(tile) = self.tile_mut(id) else {
            return false;
        };
        if tile.state() == state {
            tile.restore();
        } else {
            tile.set_state(state);
        }
        self.mark_dirty(id);
        true
    }

    /// A client asking for a state, rather than the user toggling it.
    ///
    /// Only one window per workspace may be fullscreen, so entering it demotes
    /// whoever held it — otherwise two windows would both claim the output.
    pub fn set_window_state(&mut self, id: WindowId, state: WindowState) -> bool {
        if state == WindowState::Fullscreen
            && let Some(location) = self.location(id)
            && let Some(previous) = self.workspace(location).and_then(Workspace::fullscreen)
            && previous != id
            && let Some(tile) = self.tile_mut(previous)
        {
            tile.restore();
        }

        let Some(tile) = self.tile_mut(id) else {
            return false;
        };
        if tile.state() == state {
            return false;
        }
        tile.set_state(state);
        self.mark_dirty(id);
        true
    }

    /// Returns a window to whatever it was before it was maximized or
    /// fullscreened. A no-op if it is already tiled or floating.
    pub fn restore_window(&mut self, id: WindowId) -> bool {
        let Some(tile) = self.tile_mut(id) else {
            return false;
        };
        if matches!(tile.state(), WindowState::Tiled | WindowState::Floating) {
            return false;
        }
        tile.restore();
        self.mark_dirty(id);
        true
    }

    fn mark_dirty(&mut self, id: WindowId) {
        if let Some(location) = self.location(id)
            && let Some(workspace) = self.workspace_mut(location)
        {
            workspace.dirty.layout = true;
        }
    }

    pub fn toggle_global_layout(&mut self) -> bool {
        let next = match self.global_layout {
            LayoutKind::MasterStack => LayoutKind::ScrollingColumns,
            _ => LayoutKind::MasterStack,
        };
        self.set_global_layout(next);
        true
    }

    /// Cycles this workspace's override: none -> master -> scrolling -> none.
    pub fn cycle_workspace_layout(&mut self) -> bool {
        let Some(monitor) = self.focused_monitor_mut() else {
            return false;
        };
        let next = match monitor.active().mode_override() {
            None => Some(LayoutKind::MasterStack),
            Some(LayoutKind::MasterStack) => Some(LayoutKind::ScrollingColumns),
            Some(_) => None,
        };
        monitor.active_mut().set_mode_override(next);
        true
    }

    pub fn set_workspace_layout(&mut self, kind: LayoutKind) -> bool {
        let Some(monitor) = self.focused_monitor_mut() else {
            return false;
        };
        monitor.active_mut().set_mode_override(Some(kind));
        true
    }

    pub fn apply_layout_op(&mut self, op: LayoutOp) -> bool {
        let global = self.global_layout;
        self.focused_monitor_mut()
            .is_some_and(|monitor| monitor.active_mut().apply_layout_op(op, global))
    }

    fn mark_focused_dirty(&mut self) {
        if let Some(monitor) = self.focused_monitor_mut() {
            monitor.active_mut().dirty.layout = true;
        }
    }

    /// Moves a floating window, keeping it on screen.
    pub fn move_floating(&mut self, id: WindowId, to: Point<i32, Logical>) -> bool {
        let Some(area) = self
            .location(id)
            .and_then(|at| self.workspace(at))
            .map(Workspace::area)
        else {
            return false;
        };
        let Some(tile) = self.tile_mut(id) else {
            return false;
        };
        if !tile.state().is_floating() {
            return false;
        }

        let rect = Rectangle::new(to, tile.floating_rect().size);
        tile.set_floating_rect(crate::layout::floating::clamp_into(rect, area));
        self.mark_dirty(id);
        true
    }

    /// Resizes a floating window, respecting its own size hints.
    pub fn resize_floating(&mut self, id: WindowId, rect: Rectangle<i32, Logical>) -> bool {
        let Some(area) = self
            .location(id)
            .and_then(|at| self.workspace(at))
            .map(Workspace::area)
        else {
            return false;
        };
        let Some(tile) = self.tile_mut(id) else {
            return false;
        };
        if !tile.state().is_floating() {
            return false;
        }

        let size = tile.info().constrain(rect.size);
        tile.set_floating_rect(crate::layout::floating::clamp_into(
            Rectangle::new(rect.loc, size),
            area,
        ));
        self.mark_dirty(id);
        true
    }

    /// The one reconciliation pass, run once per event-loop iteration.
    ///
    /// Handlers only mutate the model and set dirty bits; everything geometric
    /// happens here. Costs a few bool checks when nothing is dirty, so running
    /// it on every iteration — including every pointer motion — is fine.
    pub fn refresh(&mut self) {
        self.reap_dead();
        self.resolve_transactions();
        self.normalize_all();

        let global = self.global_layout;
        let mut resized = Vec::new();
        for monitor in &mut self.monitors {
            for workspace in monitor.workspaces_mut() {
                if workspace.dirty.layout || workspace.dirty.area {
                    workspace.arrange(global, &mut resized);
                    tracing::debug!(
                        workspace = %workspace.id(),
                        layout = ?workspace.effective_kind(global),
                        area = ?workspace.area(),
                        tiles = ?workspace
                            .tiles()
                            .iter()
                            .map(|tile| (tile.id(), tile.target()))
                            .collect::<Vec<_>>(),
                        "arranged"
                    );
                }
            }
        }

        self.send_configures();

        for monitor in &mut self.monitors {
            for workspace in monitor.workspaces_mut() {
                workspace.dirty.clear();
            }
        }

        #[cfg(debug_assertions)]
        self.assert_invariants();
    }

    /// Tells every window what the layout decided.
    ///
    /// Everything configured in one pass goes into one transaction, so a reflow
    /// is tracked as a group rather than as N unrelated resizes.
    fn send_configures(&mut self) {
        let focused = self.monitors.get(self.focused_output).map(Monitor::id);
        let mut awaiting = HashMap::new();

        for monitor in &mut self.monitors {
            let active = monitor.active().id();
            let is_focused = Some(monitor.id()) == focused;

            for workspace in monitor.workspaces_mut() {
                let on_screen = workspace.id() == active;
                let bounds = workspace.area().size;
                let focus = workspace.focus();

                for tile in workspace.tiles_mut() {
                    let activated = on_screen && is_focused && Some(tile.id()) == focus;
                    if let Some(serial) = configure(tile, bounds, activated) {
                        awaiting.insert(tile.id(), serial);
                    }
                }
            }
        }

        if !awaiting.is_empty() {
            self.transactions
                .push(Transaction::new(awaiting, Instant::now()));
        }
    }

    /// Records a client acking a configure.
    pub fn ack_configure(&mut self, id: WindowId, serial: smithay::utils::Serial) {
        for transaction in &mut self.transactions {
            transaction.ack(id, serial);
        }
    }

    /// Drops transactions that are complete or have run out of time.
    fn resolve_transactions(&mut self) {
        let now = Instant::now();
        self.transactions
            .retain(|transaction| !transaction.is_empty() && !transaction.expired(now));
    }

    /// Whether every window has adopted the size it was last given.
    pub fn settled(&self) -> bool {
        self.transactions.is_empty()
    }

    /// Advances every spring by `dt`.
    ///
    /// Driven from the render path rather than the event loop: the loop wakes on
    /// input too, and a 1000 Hz mouse would step springs a thousand times per
    /// frame nobody sees.
    pub fn advance_animations(&mut self, dt: f32) {
        for monitor in &mut self.monitors {
            monitor.switch_mut().step(dt);
            for workspace in monitor.workspaces_mut() {
                for tile in workspace.tiles_mut() {
                    tile.anim_mut().step(dt);
                }
            }
        }
    }

    /// Short-circuits on the first moving spring, because this is asked once per
    /// frame to decide whether to schedule another.
    pub fn is_animating(&self) -> bool {
        self.monitors.iter().any(|monitor| {
            monitor.is_switching()
                || monitor
                    .workspaces()
                    .iter()
                    .any(|workspace| workspace.tiles().iter().any(|tile| !tile.anim().at_rest()))
        })
    }

    /// Lands every spring on exact integers once motion stops.
    pub fn settle_animations(&mut self) {
        for monitor in &mut self.monitors {
            monitor.switch_mut().settle();
            for workspace in monitor.workspaces_mut() {
                for tile in workspace.tiles_mut() {
                    tile.anim_mut().settle();
                }
            }
        }
    }

    /// The windows drawn on an output right now, front to back.
    ///
    /// Mid-switch that spans two workspaces, which is what keeps the incoming
    /// one's clients receiving frame callbacks — without them a swipe reveals a
    /// workspace frozen on its last frame.
    pub fn visible_windows<'a>(&self, monitor: &'a Monitor) -> impl Iterator<Item = &'a Tile> {
        monitor
            .visible_workspaces()
            .flat_map(|(workspace, _)| workspace.stacking_order())
    }

    /// Drops tiles whose window is gone. This is the logic the old
    /// `IsAlive for WindowElement` got backwards.
    pub fn reap_dead(&mut self) {
        let dead: Vec<WindowId> = self
            .monitors
            .iter()
            .flat_map(Monitor::workspaces)
            .chain(self.headless.iter())
            .flat_map(Workspace::tiles)
            .filter(|tile| !tile.alive())
            .map(Tile::id)
            .collect();

        for id in dead {
            self.remove_tile(id);
        }
    }

    #[cfg(debug_assertions)]
    pub fn assert_invariants(&self) {
        for (surface, id) in &self.surface_to_window {
            let location = self.window_to_location[id];
            let workspace = self
                .workspace(location)
                .unwrap_or_else(|| panic!("{id} indexed to a workspace that is gone"));
            let tile = workspace
                .tile(*id)
                .unwrap_or_else(|| panic!("{id} indexed but not in its workspace"));
            assert_eq!(
                tile.surface(),
                surface,
                "{id} indexed under the wrong surface"
            );
        }

        let stored: usize = self
            .monitors
            .iter()
            .flat_map(Monitor::workspaces)
            .chain(self.headless.iter())
            .map(Workspace::len)
            .sum();
        assert_eq!(
            self.window_to_location.len(),
            stored,
            "the index and the workspaces disagree on how many windows exist"
        );

        for monitor in &self.monitors {
            assert!(
                monitor.workspaces().last().is_some_and(Workspace::is_empty),
                "monitor {} has no trailing empty workspace",
                monitor.id()
            );
            assert!(monitor.active_index() < monitor.workspaces().len());
            for workspace in monitor.workspaces() {
                assert!(
                    workspace
                        .focus()
                        .is_none_or(|focus| workspace.contains(focus)),
                    "workspace {} focuses a window it does not hold",
                    workspace.id()
                );
            }
        }

        assert!(
            self.monitors.is_empty() || self.focused_output < self.monitors.len(),
            "focused output is out of range"
        );
    }
}
