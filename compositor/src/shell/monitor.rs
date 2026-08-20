use std::time::Duration;

use smithay::{
    output::{Mode, Output, PhysicalProperties, Scale},
    reexports::wayland_server::backend::GlobalId,
    utils::{Logical, Point, Rectangle, Size, Transform},
};

use config::{OutputSetting, OutputTransform};

use crate::{
    animations::spring::SpringProfile,
    layout::{Gaps, LayoutKind},
    shell::{
        workspace::{Workspace, WorkspaceRef},
        workspace_switch::{PAGE_GAP, WorkspaceSwitch},
    },
    utils::id::{OutputId, WorkspaceId},
};

/// Identity that survives unplug and replug.
///
/// Serial-first, because the kernel reassigns connector names across a docking
/// station replug and workspace restoration keys on this.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConnectorId(String);

impl ConnectorId {
    pub fn new(name: &str, make: &str, model: &str, serial: Option<&str>) -> Self {
        match serial {
            Some(serial) if !serial.is_empty() => Self(format!("{make} {model} {serial}")),
            _ => Self(name.to_owned()),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Everything a backend knows about a newly connected output.
///
/// Deliberately contains no `Output`: the backend cannot build one, so it cannot
/// produce an output the shell does not know about. Forgetting to register was
/// exactly the bug that made every layer surface disappear.
pub struct OutputDescriptor {
    pub name: String,
    pub physical: PhysicalProperties,
    pub modes: Vec<Mode>,
    pub preferred: Option<Mode>,
    pub current: Mode,
    /// Winit needs `Flipped180`; DRM passes the panel's native orientation.
    pub native_transform: Transform,
    pub refresh_interval: Option<Duration>,
    pub serial: Option<String>,
}

/// The live state of one output.
#[derive(Debug, Clone)]
pub struct OutputConfig {
    pub name: String,
    pub connector: ConnectorId,
    pub mode: Mode,
    pub preferred_mode: Option<Mode>,
    pub modes: Vec<Mode>,
    pub scale: Scale,
    pub transform: Transform,
    pub position: smithay::utils::Point<i32, Logical>,
    pub refresh_interval: Option<Duration>,
    pub enabled: bool,
    /// Overrides the compositor-wide default for workspaces created here.
    pub default_layout: Option<LayoutKind>,
}

impl OutputConfig {
    /// Logical size after scale and transform. The one place this arithmetic
    /// lives.
    pub fn logical_size(&self) -> Size<i32, Logical> {
        let logical = self
            .mode
            .size
            .to_f64()
            .to_logical(self.scale.fractional_scale())
            .to_i32_round();
        self.transform.transform_size(logical)
    }

    pub fn logical_geometry(&self) -> Rectangle<i32, Logical> {
        Rectangle::new(self.position, self.logical_size())
    }
}

pub struct Monitor {
    id: OutputId,
    output: Output,
    /// Needed to `remove_global` on unplug; dropping it leaks the `wl_output`.
    /// `None` only in tests, which build a monitor with no display behind it.
    global: Option<GlobalId>,
    config: OutputConfig,

    /// Ordered and never empty; the last entry is always empty, so scrolling
    /// past the end always lands somewhere real.
    workspaces: Vec<Workspace>,
    active: usize,
    previous: usize,
    /// Where the viewport actually is, which trails `active` by an animation
    /// and leads it during a swipe.
    switch: WorkspaceSwitch,

    /// `layer_map_for_output(..).non_exclusive_zone()`, output-local.
    usable: Rectangle<i32, Logical>,
    gaps: Gaps,
    /// Pinned by config; `None` lets `arrange_outputs` place it.
    fixed_position: Option<smithay::utils::Point<i32, Logical>>,
}

impl Monitor {
    pub fn new(
        id: OutputId,
        output: Output,
        global: Option<GlobalId>,
        config: OutputConfig,
        global_layout: LayoutKind,
        gaps: Gaps,
    ) -> Self {
        let kind = config.default_layout.unwrap_or(global_layout);
        let usable = Rectangle::from_size(config.logical_size());

        let mut monitor = Self {
            id,
            output,
            global,
            config,
            workspaces: Vec::new(),
            active: 0,
            previous: 0,
            switch: WorkspaceSwitch::new(0),
            usable,
            gaps,
            fixed_position: None,
        };
        monitor.workspaces.push(Workspace::new(id, kind, gaps));
        monitor.push_areas();
        monitor
    }

    pub fn id(&self) -> OutputId {
        self.id
    }

    pub fn output(&self) -> &Output {
        &self.output
    }

    pub fn take_global(&mut self) -> Option<GlobalId> {
        self.global.take()
    }

    pub fn config(&self) -> &OutputConfig {
        &self.config
    }

    pub fn config_mut(&mut self) -> &mut OutputConfig {
        &mut self.config
    }

    pub fn geometry(&self) -> Rectangle<i32, Logical> {
        self.config.logical_geometry()
    }

    pub fn usable(&self) -> Rectangle<i32, Logical> {
        self.usable
    }

    pub fn workspaces(&self) -> &[Workspace] {
        &self.workspaces
    }

    pub fn workspaces_mut(&mut self) -> &mut [Workspace] {
        &mut self.workspaces
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn active(&self) -> &Workspace {
        &self.workspaces[self.active]
    }

    pub fn active_mut(&mut self) -> &mut Workspace {
        &mut self.workspaces[self.active]
    }

    pub fn workspace(&self, id: WorkspaceId) -> Option<&Workspace> {
        self.workspaces.iter().find(|ws| ws.id() == id)
    }

    pub fn workspace_mut(&mut self, id: WorkspaceId) -> Option<&mut Workspace> {
        self.workspaces.iter_mut().find(|ws| ws.id() == id)
    }

    pub fn index_of(&self, id: WorkspaceId) -> Option<usize> {
        self.workspaces.iter().position(|ws| ws.id() == id)
    }

    pub(super) fn push_workspace(&mut self, workspace: Workspace) {
        self.workspaces.push(workspace);
    }

    /// Empties the list. Only used while dismantling an unplugged monitor.
    pub(super) fn drain_workspaces(&mut self) -> Vec<Workspace> {
        self.active = 0;
        self.previous = 0;
        self.switch.snap_to(0);
        std::mem::take(&mut self.workspaces)
    }

    pub(super) fn take_workspace(&mut self, id: WorkspaceId) -> Option<Workspace> {
        let index = self.index_of(id)?;
        let workspace = self.workspaces.remove(index);
        // Removing renumbers everything after `index`, and a fractional
        // viewport position means nothing across a renumbering.
        let last = self.workspaces.len().saturating_sub(1);
        self.active = self.active.min(last);
        self.previous = self.previous.min(last);
        self.switch.snap_to(self.active);
        Some(workspace)
    }

    // ---- the viewport ----

    pub fn switch(&self) -> &WorkspaceSwitch {
        &self.switch
    }

    pub fn switch_mut(&mut self) -> &mut WorkspaceSwitch {
        &mut self.switch
    }

    pub fn set_animation_profile(&mut self, profile: Option<SpringProfile>) {
        self.switch.set_profile(profile);
    }

    /// Logical pixels from one workspace to the next while they slide past
    /// each other.
    pub fn page_stride(&self) -> f64 {
        (self.config.logical_size().w + PAGE_GAP) as f64
    }

    /// Every workspace with part of itself on screen and where to draw it,
    /// output-local, nearest the centre first.
    ///
    /// Only ever one entry once the viewport has settled, so the ordinary case
    /// costs the same as reading `active`.
    pub fn visible_workspaces(&self) -> impl Iterator<Item = (&Workspace, Point<f64, Logical>)> {
        let stride = self.page_stride();
        let workspaces = &self.workspaces;
        self.switch
            .visible(workspaces.len())
            .filter_map(move |(index, offset)| {
                Some((workspaces.get(index)?, Point::from((offset * stride, 0.0))))
            })
    }

    pub fn is_switching(&self) -> bool {
        self.switch.is_active()
    }

    pub fn is_swiping(&self) -> bool {
        self.switch.is_dragging()
    }

    pub fn begin_switch_gesture(&mut self) {
        self.switch.begin();
    }

    /// `travelled` is the fingers' cumulative horizontal travel in pages,
    /// positive rightward.
    pub fn update_switch_gesture(&mut self, travelled: f64) {
        let last = self.workspaces.len().saturating_sub(1);
        self.switch.drag_to(travelled, last);
    }

    /// Ends the swipe and makes whichever workspace it landed on active, while
    /// the spring — carrying the speed already on screen — covers the rest of
    /// the distance. `velocity` is in pages per second, positive rightward.
    pub fn end_switch_gesture(&mut self, velocity: f64, global: LayoutKind) -> bool {
        let last = self.workspaces.len().saturating_sub(1);
        let target = self.switch.release(velocity, last);
        self.commit(target, global)
    }

    pub fn cancel_switch_gesture(&mut self) {
        self.switch.cancel(self.active);
    }

    /// Pushes the current usable and output areas down to every workspace.
    fn push_areas(&mut self) {
        let output_area = Rectangle::from_size(self.config.logical_size());
        let tiled = shrink(self.usable, self.gaps.outer);
        for workspace in &mut self.workspaces {
            workspace.set_area(tiled, output_area);
            workspace.set_gaps(self.gaps);
        }
    }

    /// Returns whether the area moved, so the caller can skip a relayout.
    pub fn set_usable(&mut self, usable: Rectangle<i32, Logical>) -> bool {
        if self.usable == usable {
            return false;
        }
        self.usable = usable;
        self.push_areas();
        true
    }

    pub fn set_gaps(&mut self, gaps: Gaps) {
        if self.gaps != gaps {
            self.gaps = gaps;
            self.push_areas();
        }
    }

    pub fn set_mode(&mut self, mode: Mode) -> bool {
        if self.config.mode == mode {
            return false;
        }
        self.config.mode = mode;
        self.output
            .change_current_state(Some(mode), None, None, None);
        // A mode change resizes the output, so the layer map has to be
        // re-arranged before the usable area means anything again.
        self.usable = Rectangle::from_size(self.config.logical_size());
        self.push_areas();
        true
    }

    pub fn set_scale(&mut self, scale: f64) -> bool {
        let scale = Scale::Fractional(scale.clamp(0.1, 8.0));
        if self.config.scale.fractional_scale() == scale.fractional_scale() {
            return false;
        }
        self.config.scale = scale;
        self.output
            .change_current_state(None, None, Some(scale), None);
        // Scale changes the logical size, so the usable area is stale until the
        // layer map is re-arranged.
        self.usable = Rectangle::from_size(self.config.logical_size());
        self.push_areas();
        true
    }

    pub fn set_transform(&mut self, transform: Transform) -> bool {
        if self.config.transform == transform {
            return false;
        }
        self.config.transform = transform;
        self.output
            .change_current_state(None, Some(transform), None, None);
        self.usable = Rectangle::from_size(self.config.logical_size());
        self.push_areas();
        true
    }

    /// Whether the config pins this output somewhere, rather than letting
    /// `arrange_outputs` pack it.
    pub fn fixed_position(&self) -> Option<smithay::utils::Point<i32, Logical>> {
        self.fixed_position
    }

    /// Applies the per-output half of the config.
    ///
    /// `fallback_scale` is the system `display.scale`, used when the output has
    /// no override of its own. Returns whether anything moved, so the caller can
    /// skip an arrange.
    pub fn apply_settings(&mut self, setting: Option<&OutputSetting>, fallback_scale: f64) -> bool {
        let mut changed = false;

        let scale = setting.and_then(|s| s.scale).unwrap_or(fallback_scale);
        changed |= self.set_scale(scale);

        if let Some(transform) = setting.and_then(|s| s.transform) {
            changed |= self.set_transform(transform_from_config(transform));
        }

        if let Some(text) = setting.and_then(|s| s.mode.as_deref()) {
            match parse_mode(text) {
                // Only a mode the hardware advertises; a plausible-looking typo
                // must not leave the output on something it cannot display.
                Some(mode) if self.config.modes.iter().any(|m| m.size == mode.size) => {
                    changed |= self.set_mode(mode);
                }
                Some(mode) => {
                    tracing::warn!(output = %self.config.name, ?mode, "output has no such mode");
                }
                None => {
                    tracing::warn!(output = %self.config.name, mode = text, "unparsable mode");
                }
            }
        }

        let fixed = setting.and_then(|s| s.position).map(Into::into);
        if self.fixed_position != fixed {
            self.fixed_position = fixed;
            changed = true;
        }

        self.config.default_layout = setting.and_then(|s| s.layout).map(Into::into);
        self.config.enabled = setting.and_then(|s| s.enabled).unwrap_or(true);

        changed
    }

    pub fn set_position(&mut self, position: smithay::utils::Point<i32, Logical>) -> bool {
        if self.config.position == position {
            return false;
        }
        self.config.position = position;
        self.output
            .change_current_state(None, None, None, Some(position));
        true
    }

    pub fn default_layout(&self, global: LayoutKind) -> LayoutKind {
        self.config.default_layout.unwrap_or(global)
    }

    /// Restores the workspace-list invariants. Every mutation ends here.
    pub fn normalize(&mut self, global: LayoutKind) {
        // Reaping renumbers the list, and a viewport in flight sits *between*
        // two numbers — rebasing it would jump the animation. It happens on the
        // first normalize after the switch lands, which the render loop
        // guarantees will come.
        if !self.switch.is_active() {
            self.reap_empty();
        }
        self.ensure_trailing_empty(global);
        debug_assert!(self.workspaces.last().is_some_and(Workspace::is_empty));
        debug_assert!(self.active < self.workspaces.len());
    }

    /// Drops empty workspaces that are neither active nor trailing.
    ///
    /// The active one is spared even when empty: reaping the workspace the user
    /// is looking at would teleport them mid-gesture. It goes as soon as they
    /// leave, via the `normalize` at the end of the switch.
    fn reap_empty(&mut self) {
        if self.workspaces.is_empty() {
            return;
        }

        // Rebase by id, not index — index arithmetic after a `retain` is where
        // this always breaks.
        let active_id = self.workspaces[self.active].id();
        let previous_id = self.workspaces[self.previous].id();
        let last = self.workspaces.len() - 1;

        let mut index = 0;
        self.workspaces.retain(|ws| {
            let keep = !ws.is_empty() || ws.id() == active_id || index == last;
            index += 1;
            keep
        });

        self.active = self.index_of(active_id).unwrap_or(0);
        self.previous = self.index_of(previous_id).unwrap_or(self.active);
        // The viewport is at rest on the old index; re-anchor it on the new one
        // so the same workspace stays on screen.
        self.switch.snap_to(self.active);
    }

    fn ensure_trailing_empty(&mut self, global: LayoutKind) {
        if self.workspaces.last().is_some_and(Workspace::is_empty) {
            return;
        }
        let kind = self.default_layout(global);
        let mut workspace = Workspace::new(self.id, kind, self.gaps);
        workspace.set_area(
            shrink(self.usable, self.gaps.outer),
            Rectangle::from_size(self.config.logical_size()),
        );
        self.workspaces.push(workspace);
    }

    /// Clamps rather than creating, and never wraps.
    ///
    /// `Super+8` on a three-workspace monitor lands on the trailing empty rather
    /// than conjuring five empties that reaping would immediately delete. And
    /// "next" from the last workspace already means "a new one" — wrapping would
    /// make it mean two different things.
    pub fn resolve(&self, target: WorkspaceRef) -> usize {
        let last = self.workspaces.len().saturating_sub(1);
        match target {
            WorkspaceRef::Index(index) => index.min(last),
            WorkspaceRef::Relative(delta) => (self.active as i64 + delta as i64)
                .clamp(0, last as i64) as usize,
            WorkspaceRef::Previous => self.previous.min(last),
        }
    }

    pub fn switch_to(&mut self, target: WorkspaceRef, global: LayoutKind) -> bool {
        self.activate(self.resolve(target), global)
    }

    pub fn activate(&mut self, index: usize, global: LayoutKind) -> bool {
        let index = index.min(self.workspaces.len().saturating_sub(1));
        self.switch.animate_to(index);
        self.commit(index, global)
    }

    /// Moves the model onto `index`, leaving the viewport alone — the caller
    /// has already aimed it, and a swipe's whole point is that its release
    /// velocity survives this step.
    fn commit(&mut self, index: usize, global: LayoutKind) -> bool {
        let index = index.min(self.workspaces.len().saturating_sub(1));
        let changed = index != self.active;
        if changed {
            self.previous = self.active;
            self.active = index;
        }
        self.normalize(global);
        changed
    }

    pub fn previous_index(&self) -> usize {
        self.previous
    }
}

pub fn shrink(rect: Rectangle<i32, Logical>, by: i32) -> Rectangle<i32, Logical> {
    if by <= 0 {
        return rect;
    }
    // Never collapse to nothing, or a large outer gap on a small output would
    // leave no area to lay anything out in.
    let width = (rect.size.w - by * 2).max(1);
    let height = (rect.size.h - by * 2).max(1);
    Rectangle::new((rect.loc.x + by, rect.loc.y + by).into(), (width, height).into())
}

/// Builds the smithay `Output` a descriptor describes.
pub fn output_from_descriptor(descriptor: &OutputDescriptor) -> Output {
    let output = Output::new(
        descriptor.name.clone(),
        PhysicalProperties {
            size: descriptor.physical.size,
            subpixel: descriptor.physical.subpixel,
            make: descriptor.physical.make.clone(),
            model: descriptor.physical.model.clone(),
        },
    );

    output.change_current_state(
        Some(descriptor.current),
        Some(descriptor.native_transform),
        None,
        Some((0, 0).into()),
    );
    if let Some(preferred) = descriptor.preferred {
        output.set_preferred(preferred);
    }
    for mode in &descriptor.modes {
        output.add_mode(*mode);
    }
    output
}

/// A `Copy` handle for an `Output`, cached in its user data.
///
/// `Output` is `Hash + Eq` but not `Copy`, and threading `&Output` through the
/// model would poison every signature.
pub fn output_id(output: &Output) -> OutputId {
    output
        .user_data()
        .insert_if_missing_threadsafe(OutputId::next);
    *output
        .user_data()
        .get::<OutputId>()
        .expect("just inserted")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A monitor with `count` populated workspaces plus the trailing empty.
    ///
    /// `Workspace::is_empty` is what reaping keys on, so the fake tiles are
    /// simulated by leaving the list alone and driving `resolve` directly —
    /// building real `Tile`s would need live Wayland objects.
    fn monitor(workspaces: usize) -> Monitor {
        let mut monitor = Monitor {
            id: OutputId::next(),
            output: Output::new(
                "test".into(),
                PhysicalProperties {
                    size: (0, 0).into(),
                    subpixel: smithay::output::Subpixel::Unknown,
                    make: "test".into(),
                    model: "test".into(),
                },
            ),
            global: None,
            config: OutputConfig {
                name: "test".into(),
                connector: ConnectorId::new("test", "m", "m", None),
                mode: Mode {
                    size: (1920, 1080).into(),
                    refresh: 60_000,
                },
                preferred_mode: None,
                modes: Vec::new(),
                scale: Scale::Integer(1),
                transform: Transform::Normal,
                position: (0, 0).into(),
                refresh_interval: None,
                enabled: true,
                default_layout: None,
            },
            workspaces: Vec::new(),
            active: 0,
            previous: 0,
            switch: WorkspaceSwitch::new(0),
            usable: Rectangle::from_size((1920, 1080).into()),
            gaps: Gaps::default(),
            fixed_position: None,
        };
        for _ in 0..workspaces {
            monitor
                .workspaces
                .push(Workspace::new(monitor.id, LayoutKind::MasterStack, Gaps::default()));
        }
        monitor
    }

    #[test]
    fn an_index_past_the_end_clamps_instead_of_creating() {
        let monitor = monitor(3);
        // Super+8 on a three-workspace monitor lands on the last one, rather
        // than conjuring five empties that reaping would delete anyway.
        assert_eq!(monitor.resolve(WorkspaceRef::Index(7)), 2);
        assert_eq!(monitor.resolve(WorkspaceRef::Index(1)), 1);
    }

    #[test]
    fn relative_movement_does_not_wrap() {
        let mut monitor = monitor(3);
        monitor.active = 2;
        assert_eq!(
            monitor.resolve(WorkspaceRef::Relative(1)),
            2,
            "next from the last workspace stays put; the trailing empty is what grows the list"
        );

        monitor.active = 0;
        assert_eq!(monitor.resolve(WorkspaceRef::Relative(-1)), 0);
        assert_eq!(monitor.resolve(WorkspaceRef::Relative(1)), 1);
    }

    #[test]
    fn previous_returns_where_you_came_from() {
        let mut monitor = monitor(4);
        monitor.previous = 2;
        assert_eq!(monitor.resolve(WorkspaceRef::Previous), 2);

        // Clamped like everything else, in case the list shrank underneath it.
        monitor.previous = 9;
        assert_eq!(monitor.resolve(WorkspaceRef::Previous), 3);
    }

    #[test]
    fn normalize_reaps_every_empty_workspace_but_the_active_and_trailing_ones() {
        let mut monitor = monitor(4);
        monitor.normalize(LayoutKind::MasterStack);

        // All four were empty. The active one is spared — reaping the workspace
        // the user is looking at would teleport them — and so is the trailing
        // one, so "scroll past the end" always lands somewhere real.
        assert_eq!(monitor.workspaces().len(), 2);
        assert_invariants(&monitor);
    }

    /// The rules `Shell::assert_invariants` enforces, for one monitor.
    fn assert_invariants(monitor: &Monitor) {
        assert!(
            monitor.workspaces().last().is_some_and(Workspace::is_empty),
            "the last workspace must always be empty"
        );
        assert!(monitor.active_index() < monitor.workspaces().len());
        assert!(monitor.previous_index() < monitor.workspaces().len());

        let last = monitor.workspaces().len() - 1;
        for (index, workspace) in monitor.workspaces().iter().enumerate() {
            assert!(
                !workspace.is_empty() || index == monitor.active_index() || index == last,
                "workspace {index} is empty but neither active nor trailing"
            );
        }
    }

    #[test]
    fn normalize_is_idempotent() {
        let mut monitor = monitor(4);
        monitor.normalize(LayoutKind::MasterStack);
        let after_once = monitor.workspaces().len();
        monitor.normalize(LayoutKind::MasterStack);
        assert_eq!(monitor.workspaces().len(), after_once);
        assert_invariants(&monitor);
    }

    /// Runs the viewport spring at 60 Hz until it lands, then snaps it onto the
    /// target — the same two-step the render loop performs every frame.
    fn settle(monitor: &mut Monitor) {
        for _ in 0..600 {
            if !monitor.is_switching() {
                monitor.switch_mut().settle();
                return;
            }
            monitor.switch_mut().step(1.0 / 60.0);
        }
        panic!("the viewport never came to rest");
    }

    #[test]
    fn switching_away_reaps_the_workspace_you_left_once_the_viewport_lands() {
        let mut monitor = monitor(3);
        monitor.activate(1, LayoutKind::MasterStack);
        assert_eq!(
            monitor.workspaces().len(),
            3,
            "renumbering the list under a sliding viewport would jump the animation"
        );

        settle(&mut monitor);
        monitor.normalize(LayoutKind::MasterStack);

        // Index 0 was empty and is no longer active, so it is gone — and the
        // viewport followed the workspace, not the index.
        assert_eq!(monitor.workspaces().len(), 2);
        assert_eq!(monitor.active_index(), 0);
        assert_eq!(monitor.switch().position(), 0.0);
        assert_invariants(&monitor);
    }

    #[test]
    fn activating_slides_the_viewport_rather_than_teleporting_it() {
        let mut monitor = monitor(4);
        monitor.activate(2, LayoutKind::MasterStack);

        assert_eq!(monitor.active_index(), 2, "the model commits immediately");
        assert!(monitor.switch().position() < 2.0, "the pixels catch up");
        assert!(monitor.is_switching());

        settle(&mut monitor);
        assert_eq!(monitor.switch().position(), monitor.active_index() as f64);
    }

    #[test]
    fn a_swipe_commits_the_workspace_it_lands_on() {
        let mut monitor = monitor(4);

        monitor.begin_switch_gesture();
        assert!(monitor.is_swiping());
        // Two thirds of the way to the next workspace, then let go still moving
        // at about a page a second.
        monitor.update_switch_gesture(-0.66);
        let changed = monitor.end_switch_gesture(-1.0, LayoutKind::MasterStack);

        assert!(changed);
        assert_eq!(monitor.active_index(), 1);
        assert!(!monitor.is_swiping());
        settle(&mut monitor);
        assert_eq!(monitor.switch().position(), 1.0);
    }

    #[test]
    fn an_abandoned_swipe_leaves_the_active_workspace_alone() {
        let mut monitor = monitor(4);
        monitor.activate(2, LayoutKind::MasterStack);
        settle(&mut monitor);

        monitor.begin_switch_gesture();
        monitor.update_switch_gesture(-0.2);
        // A frame or two of following, as the render loop would deliver.
        for _ in 0..3 {
            monitor.switch_mut().step(1.0 / 60.0);
        }
        assert!(!monitor.end_switch_gesture(0.0, LayoutKind::MasterStack));
        assert_eq!(monitor.active_index(), 2);
    }

    #[test]
    fn a_cancelled_swipe_returns_to_where_it_started() {
        let mut monitor = monitor(4);

        monitor.begin_switch_gesture();
        monitor.update_switch_gesture(-0.9);
        monitor.cancel_switch_gesture();

        settle(&mut monitor);
        assert_eq!(monitor.active_index(), 0);
        assert_eq!(monitor.switch().position(), 0.0);
    }

    #[test]
    fn two_workspaces_are_on_screen_mid_swipe_and_one_at_rest() {
        let mut monitor = monitor(4);
        assert_eq!(monitor.visible_workspaces().count(), 1);

        monitor.begin_switch_gesture();
        monitor.update_switch_gesture(-0.5);
        // The tracking spring eases after the fingers; give it time to get
        // between the two pages.
        for _ in 0..30 {
            monitor.switch_mut().step(1.0 / 60.0);
        }

        let visible: Vec<_> = monitor
            .visible_workspaces()
            .map(|(_, offset)| offset.x)
            .collect();
        assert_eq!(visible.len(), 2);
        // One page leaving to the left, its neighbour arriving from the right.
        assert!(visible.iter().any(|x| *x < 0.0));
        assert!(visible.iter().any(|x| *x > 0.0));
    }

    #[test]
    fn disabled_animations_switch_without_a_single_extra_frame() {
        let mut monitor = monitor(4);
        monitor.set_animation_profile(None);
        monitor.activate(2, LayoutKind::MasterStack);

        assert!(!monitor.is_switching(), "nothing left to animate");
        // Nothing was in flight, so reaping was not deferred — which renumbers
        // the list. The viewport must have followed the workspace through it.
        assert_eq!(monitor.switch().position(), monitor.active_index() as f64);
        assert_invariants(&monitor);
    }

    #[test]
    fn a_fresh_monitor_already_holds_the_invariant() {
        let monitor = monitor(1);
        assert!(monitor.workspaces().last().unwrap().is_empty());
        assert_eq!(monitor.active_index(), 0);
    }

    #[test]
    fn resolve_survives_a_single_workspace() {
        let monitor = monitor(1);
        for target in [
            WorkspaceRef::Index(9),
            WorkspaceRef::Relative(5),
            WorkspaceRef::Relative(-5),
            WorkspaceRef::Previous,
        ] {
            assert_eq!(monitor.resolve(target), 0, "{target:?} must stay in range");
        }
    }
}

/// `"2560x1440@144.000"`, or `"2560x1440"` for whatever refresh the output has.
fn parse_mode(text: &str) -> Option<Mode> {
    let (size, refresh) = match text.split_once('@') {
        Some((size, refresh)) => (size, Some(refresh)),
        None => (text, None),
    };
    let (w, h) = size.trim().split_once('x')?;

    // An absent refresh defaults; a *malformed* one rejects the whole string.
    // Quietly running a 144 Hz panel at 60 because of a typo is a bad surprise.
    let refresh = match refresh {
        None => 60_000,
        Some(text) => (text.trim().parse::<f64>().ok()? * 1000.0).round() as i32,
    };

    Some(Mode {
        size: (w.trim().parse().ok()?, h.trim().parse().ok()?).into(),
        refresh,
    })
}

fn transform_from_config(transform: OutputTransform) -> Transform {
    match transform {
        OutputTransform::Normal => Transform::Normal,
        OutputTransform::R90 => Transform::_90,
        OutputTransform::R180 => Transform::_180,
        OutputTransform::R270 => Transform::_270,
        OutputTransform::Flipped => Transform::Flipped,
        OutputTransform::Flipped90 => Transform::Flipped90,
        OutputTransform::Flipped180 => Transform::Flipped180,
        OutputTransform::Flipped270 => Transform::Flipped270,
    }
}

#[cfg(test)]
mod mode_tests {
    use super::*;

    #[test]
    fn a_full_mode_string_parses() {
        let mode = parse_mode("2560x1440@144.000").expect("should parse");
        assert_eq!(mode.size, (2560, 1440).into());
        // smithay counts refresh in millihertz.
        assert_eq!(mode.refresh, 144_000);
    }

    #[test]
    fn refresh_is_optional() {
        let mode = parse_mode("1920x1080").expect("should parse");
        assert_eq!(mode.size, (1920, 1080).into());
        assert_eq!(mode.refresh, 60_000);
    }

    #[test]
    fn whitespace_is_tolerated() {
        assert_eq!(
            parse_mode(" 1280 x 720 @ 60 ").map(|m| m.size),
            Some((1280, 720).into())
        );
    }

    #[test]
    fn nonsense_is_rejected_rather_than_guessed() {
        for bad in ["", "1920", "axb", "1920x", "x1080", "1920x1080@nope"] {
            assert!(parse_mode(bad).is_none(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn every_config_transform_maps() {
        // A missing arm would silently rotate someone's display wrong.
        for (from, to) in [
            (OutputTransform::Normal, Transform::Normal),
            (OutputTransform::R90, Transform::_90),
            (OutputTransform::R180, Transform::_180),
            (OutputTransform::R270, Transform::_270),
            (OutputTransform::Flipped, Transform::Flipped),
            (OutputTransform::Flipped90, Transform::Flipped90),
            (OutputTransform::Flipped180, Transform::Flipped180),
            (OutputTransform::Flipped270, Transform::Flipped270),
        ] {
            assert_eq!(transform_from_config(from), to);
        }
    }
}
