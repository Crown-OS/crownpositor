//! Output configuration, as the protocol and the config file both see it.
//!
//! Everything that changes an output funnels through here: the
//! `zwlr_output_management_v1` handler, a `compositor.ron` edit, and — once
//! the backend learns to reconfigure itself — hotplug. One funnel is what
//! keeps "the config said X" and "wlr-randr said X" from drifting apart.

use std::time::Duration;

use calloop::timer::{TimeoutAction, Timer};
use config::{Compositor, OutputLayout, OutputSetting};
use protocols::output_management::{
    AdaptiveSync, HeadConfig, HeadId, HeadSnapshot, ModeRequest, ModeSnapshot,
    OutputConfigRequest, VrrSupport,
};
use smithay::{
    backend::drm::VrrSupport as DrmVrrSupport,
    output::{Mode, Output},
    reexports::wayland_server::protocol::wl_output::WlOutput,
    utils::{Logical, Point, Rectangle, Transform},
};

use crate::{
    shell::monitor::{Monitor, OutputConfig, logical_size},
    state::{State, backend::DisabledHead},
};

/// Scales outside this are refused rather than clamped: a client that asked
/// for something impossible should be told, not quietly given something else.
const SCALE_RANGE: std::ops::RangeInclusive<f64> = 0.1..=8.0;

/// How far a custom mode's refresh rate may be from an advertised one before
/// it stops counting as the same mode. One hertz, in millihertz.
const REFRESH_TOLERANCE: i32 = 1_000;

/// How long to wait before writing an applied configuration to disk.
///
/// A settings panel dragging a scale slider tests and applies on every frame;
/// without this the config file would be rewritten at input rate.
const PERSIST_DEBOUNCE: Duration = Duration::from_millis(500);

/// Whether a configuration should be carried out or only checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyMode {
    Test,
    Apply,
}

/// One head with every field decided — the client's request merged onto what
/// the head has now, so validation never has to reason about `None`.
#[derive(Debug, Clone)]
struct ResolvedHead {
    id: HeadId,
    enabled: bool,
    mode: Mode,
    position: Point<i32, Logical>,
    transform: Transform,
    scale: f64,
    adaptive_sync: bool,
}

impl ResolvedHead {
    fn geometry(&self) -> Rectangle<i32, Logical> {
        Rectangle::new(
            self.position,
            logical_size(self.mode.size, self.scale, self.transform),
        )
    }
}

/// What a head looks like right now, whether or not it is an output.
///
/// A configuration may enable a head that has no `Monitor`, so resolution
/// cannot read from one.
struct Current {
    mode: Mode,
    modes: Vec<Mode>,
    position: Point<i32, Logical>,
    transform: Transform,
    scale: f64,
    adaptive_sync: bool,
}

impl Current {
    fn from_monitor(config: &OutputConfig, adaptive_sync: bool) -> Self {
        Self {
            mode: config.mode,
            modes: config.modes.clone(),
            position: config.position,
            transform: config.transform,
            scale: config.scale.fractional_scale(),
            adaptive_sync,
        }
    }

    fn from_disabled(head: &DisabledHead) -> Self {
        let mode = head
            .preferred
            .and_then(|index| head.modes.get(index))
            .or_else(|| head.modes.first())
            .copied()
            .unwrap_or(Mode {
                size: (0, 0).into(),
                refresh: 0,
            });

        Self {
            mode,
            modes: head.modes.clone(),
            position: (0, 0).into(),
            transform: Transform::Normal,
            scale: 1.0,
            adaptive_sync: false,
        }
    }
}

/// Why a configuration was refused. Logged, and turned into `failed` on the
/// wire — the protocol has no way to say which head was the problem.
#[derive(Debug, PartialEq)]
enum Rejection {
    NoHeads,
    AllDisabled,
    UnknownHead(String),
    NoSuchMode(String),
    ScaleOutOfRange(String, f64),
    Overlap(String, String),
    /// Needs hardware work the backend cannot do yet.
    Unsupported(String),
}

impl std::fmt::Display for Rejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoHeads => write!(f, "the configuration is empty"),
            Self::AllDisabled => write!(f, "it would disable every output"),
            Self::UnknownHead(head) => write!(f, "head {head} is not connected"),
            Self::NoSuchMode(head) => write!(f, "head {head} has no such mode"),
            Self::ScaleOutOfRange(head, scale) => {
                write!(f, "scale {scale} on head {head} is outside {SCALE_RANGE:?}")
            }
            Self::Overlap(left, right) => write!(f, "heads {left} and {right} would overlap"),
            Self::Unsupported(what) => write!(f, "{what} is not supported yet"),
        }
    }
}

impl State {
    /// The head list as the protocol should see it right now.
    ///
    /// Sorted by id so the diff in `set_heads` is stable — an unsorted list
    /// would make an unrelated hotplug reorder every head and look like a
    /// change to every client.
    pub fn output_head_snapshots(&self) -> Vec<HeadSnapshot> {
        let mut heads: Vec<HeadSnapshot> = self
            .shell
            .monitors()
            .iter()
            .map(|monitor| {
                let output = monitor.output();
                let adaptive_sync = if self.backend.vrr_enabled(output) {
                    AdaptiveSync::Enabled
                } else {
                    AdaptiveSync::Disabled
                };
                head_snapshot(
                    monitor,
                    adaptive_sync,
                    vrr_support(self.backend.vrr_support(output)),
                )
            })
            .chain(self.backend.disabled_heads().iter().map(disabled_snapshot))
            .collect();
        heads.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
        heads
    }

    /// The smithay `Output` a client's `wl_output` refers to.
    ///
    /// `None` for an output unplugged since the client bound it, which is a
    /// race rather than a misbehaving client.
    pub fn output_for(&self, wl_output: &WlOutput) -> Option<Output> {
        Output::from_resource(wl_output).filter(|output| self.shell.contains_output(output))
    }

    /// Everything that has to follow an output change: the head list clients
    /// see, and the gamma ramps the hardware holds.
    ///
    /// The protocol half sends only what actually changed. The gamma half is
    /// here rather than at each call site because a modeset clears the CRTC's
    /// lookup table, so an output that has just been reconfigured has lost
    /// whatever night light put there.
    pub fn refresh_output_heads(&mut self) {
        let heads = self.output_head_snapshots();
        self.wayland
            .output_management_state
            .set_heads::<State>(heads);
        self.apply_display_gamma();
    }

    /// The one door for changing outputs: the protocol, a config reload and
    /// hotplug all arrive here.
    ///
    /// Returns whether the configuration is (or would be) accepted. `Test`
    /// must leave everything exactly as it found it.
    pub fn reconfigure_outputs(&mut self, request: OutputConfigRequest, mode: ApplyMode) -> bool {
        let heads = match self.resolve(&request) {
            Ok(heads) => heads,
            Err(rejection) => {
                tracing::info!(%rejection, "output configuration refused");
                return false;
            }
        };

        if let Err(rejection) = validate(&heads) {
            tracing::info!(%rejection, "output configuration refused");
            return false;
        }

        if let Err(rejection) = self.validate_against_hardware(&heads) {
            tracing::info!(%rejection, "output configuration refused");
            return false;
        }

        if mode == ApplyMode::Test {
            return true;
        }

        self.apply_resolved(&heads);
        self.schedule_persist();
        true
    }

    /// Queues a write-back of the current output state to `compositor.ron`.
    ///
    /// Only ever called after a *successful apply* — never on a test, and
    /// never on a hotplug that changed no setting — because the user may have
    /// the file open in an editor and the compositor would win.
    fn schedule_persist(&mut self) {
        if let Some(token) = self.config.persist.take() {
            self.common.event_loop_handle.remove(token);
        }

        let token = self
            .common
            .event_loop_handle
            .insert_source(Timer::from_duration(PERSIST_DEBOUNCE), |_, _, state| {
                state.config.persist = None;
                state.persist_output_config();
                TimeoutAction::Drop
            })
            .ok();

        self.config.persist = token;
    }

    /// Writes the outputs section back, preserving everything else in the file.
    ///
    /// Read-modify-write rather than serialising the in-memory `Compositor`:
    /// the live config is only partially refreshed by the watcher — a window
    /// rule edit updates the *compiled* rules and not
    /// `compositor.window_rules`, and `startup` is never updated at all — so
    /// writing from memory would silently discard hand-edited sections.
    fn persist_output_config(&mut self) {
        let mut on_disk: Compositor = crownos_config::load(Compositor::SECTION);
        on_disk.outputs = self.derive_output_settings(&on_disk.outputs);
        on_disk.output_layouts = self.derive_output_layouts(on_disk.output_layouts);

        match crownos_config::save(Compositor::SECTION, &on_disk) {
            Ok(()) => {
                // The watcher suppresses the echo of our own write by content
                // hash, so this will not come back as a config change; keeping
                // the in-memory copy in step is what stops the next write from
                // re-diverging.
                self.config.current.compositor = on_disk;
                tracing::debug!("persisted the output configuration");
            }
            Err(err) => tracing::warn!(%err, "failed to persist the output configuration"),
        }
    }

    /// Per-monitor settings, merged onto whatever the file already had.
    ///
    /// An entry keyed on an identity the user wrote by hand is matched and
    /// updated in place rather than being replaced by a connector-named one,
    /// so hand-written config keeps its shape.
    fn derive_output_settings(&self, existing: &[OutputSetting]) -> Vec<OutputSetting> {
        let mut settings = existing.to_vec();

        // A monitor switched off has to be recorded, or it comes back on at
        // the next start and the user has to switch it off again every boot.
        for head in self.backend.disabled_heads() {
            let entry = upsert(&mut settings, &head.name, &head.identity);
            entry.enabled = Some(false);
        }

        for monitor in self.shell.monitors() {
            let config = monitor.config();
            let identity = config.connector.as_str();

            let entry = upsert(&mut settings, &config.name, identity);

            // Explicitly on, so a monitor switched back on stops being
            // suppressed by a stale `enabled: false`.
            entry.enabled = Some(true);
            entry.scale = Some(config.scale.fractional_scale());
            entry.transform = Some(transform_to_config(config.transform));
            entry.mode = Some(format_mode(config.mode));
            // Position lives in the per-set layout; the flat entry keeps it
            // only as the fallback for a monitor set that has none.
            entry.position.get_or_insert((config.position.x, config.position.y));
        }

        settings
    }

    /// The arrangement for exactly the monitors that are plugged in now.
    fn derive_output_layouts(&self, existing: Vec<OutputLayout>) -> Vec<OutputLayout> {
        let present: Vec<String> = self
            .shell
            .monitors()
            .iter()
            .map(|monitor| monitor.config().connector.as_str().to_owned())
            .collect();
        if present.is_empty() {
            return existing;
        }

        let layout = OutputLayout {
            positions: self
                .shell
                .monitors()
                .iter()
                .map(|monitor| {
                    let config = monitor.config();
                    (
                        config.connector.as_str().to_owned(),
                        config.position.x,
                        config.position.y,
                    )
                })
                .collect(),
            heads: present.clone(),
            disabled: Vec::new(),
        };

        // Replace the arrangement for this exact set and leave every other
        // profile alone — that is the whole point of keying on the set.
        let mut layouts: Vec<OutputLayout> = existing
            .into_iter()
            .filter(|existing| !existing.matches(&present))
            .collect();
        layouts.push(layout);
        layouts
    }

    /// Turns one head on or off, if it is not already.
    fn set_head_enabled(&mut self, head: &ResolvedHead, enabled: bool) {
        let already = self.shell.monitor_by_name(head.id.as_str()).is_some();
        if already == enabled {
            return;
        }

        match crate::state::backend::BackendState::set_head_enabled(
            self,
            head.id.as_str(),
            enabled,
        ) {
            Ok(true) => {}
            // The head vanished between validation and here — a hotplug race,
            // not a failure worth undoing the rest of the configuration for.
            Ok(false) => tracing::debug!(head = %head.id, "head disappeared while applying"),
            Err(err) => tracing::error!(%err, head = %head.id, enabled, "failed to switch a monitor"),
        }
    }

    /// The half of validation only the backend can answer.
    ///
    /// Kept separate from [`validate`] so the geometry rules stay unit-testable
    /// without a GPU, and so a nested session refuses exactly the things it
    /// genuinely cannot do rather than pretending.
    fn validate_against_hardware(&self, heads: &[ResolvedHead]) -> Result<(), Rejection> {
        for head in heads.iter().filter(|head| head.enabled) {
            // A head being switched on has no `Output` to ask yet; its mode
            // was already checked against the list the backend advertised.
            let Some(monitor) = self.shell.monitor_by_name(head.id.as_str()) else {
                continue;
            };
            let output = monitor.output();

            if head.mode != monitor.config().mode
                && !self.backend.supports_mode(output, head.mode)
            {
                return Err(Rejection::NoSuchMode(head.id.to_string()));
            }

            if head.adaptive_sync
                && matches!(self.backend.vrr_support(output), DrmVrrSupport::NotSupported)
            {
                return Err(Rejection::Unsupported(format!(
                    "adaptive sync on head {}",
                    head.id
                )));
            }
        }

        Ok(())
    }

    /// Merges a request onto the current state, rejecting anything that names
    /// something the compositor does not have.
    fn resolve(&self, request: &OutputConfigRequest) -> Result<Vec<ResolvedHead>, Rejection> {
        if request.heads.is_empty() {
            return Err(Rejection::NoHeads);
        }

        let disabled = self.backend.disabled_heads();

        request
            .heads
            .iter()
            .map(|(id, config)| {
                if let Some(monitor) = self.shell.monitor_by_name(id.as_str()) {
                    let vrr = self.backend.vrr_enabled(monitor.output());
                    let current = Current::from_monitor(monitor.config(), vrr);
                    return resolve_head(id, config, &current);
                }
                let head = disabled
                    .iter()
                    .find(|head| head.name == id.as_str())
                    .ok_or_else(|| Rejection::UnknownHead(id.to_string()))?;
                resolve_head(id, config, &Current::from_disabled(head))
            })
            .collect()
    }

    /// Pushes an accepted configuration onto the shell.
    ///
    /// Only the parts that need no hardware reconfiguration; `validate` has
    /// already refused anything else, so there is nothing here that can fail
    /// halfway and leave the desktop inconsistent.
    fn apply_resolved(&mut self, heads: &[ResolvedHead]) {
        // Off before on: a head being switched off frees the CRTC that the one
        // being switched on may need, and doing it the other way round fails
        // on a GPU that is exactly at its limit.
        for head in heads.iter().filter(|head| !head.enabled) {
            self.set_head_enabled(head, false);
        }
        for head in heads.iter().filter(|head| head.enabled) {
            self.set_head_enabled(head, true);
        }

        for head in heads.iter().filter(|head| head.enabled) {
            let Some(monitor) = self.shell.monitor_by_name_mut(head.id.as_str()) else {
                continue;
            };

            // The `Output` first, then the hardware: the KMS backend's
            // `DrmCompositor` takes its render size from the `Output`, so a
            // frame rendered between the two would be committed at the size
            // the swapchain no longer has.
            let output = monitor.output().clone();
            let mode_changed = monitor.set_mode(head.mode);
            monitor.set_scale(head.scale);
            monitor.set_transform(head.transform);
            monitor.pin_position(head.position);

            if mode_changed
                && let Err(err) = self.backend.set_output_mode(&output, head.mode)
            {
                tracing::error!(%err, output = %head.id, "failed to set the mode");
            }
            if let Err(err) = self.backend.set_output_vrr(&output, head.adaptive_sync) {
                tracing::warn!(%err, output = %head.id, "failed to set adaptive sync");
            }
        }

        self.shell.arrange_outputs();

        let outputs: Vec<_> = self
            .shell
            .monitors()
            .iter()
            .map(|monitor| monitor.output().clone())
            .collect();
        for output in outputs {
            self.shell.refresh_usable(&output);
            self.shell.advertise_output_scale(&output);
        }

        // Idle, not inline: the client is still inside its `apply`, and the
        // `succeeded` event has to reach it before the `done` that a new
        // serial would produce — otherwise its next configuration is
        // cancelled the instant it is created.
        self.common
            .event_loop_handle
            .insert_idle(|state| state.refresh_output_heads());

        self.queue_redraw();
    }
}

fn resolve_head(
    id: &HeadId,
    config: &HeadConfig,
    current: &Current,
) -> Result<ResolvedHead, Rejection> {
    let HeadConfig::Enabled {
        mode,
        position,
        transform,
        scale,
        adaptive_sync,
    } = config
    else {
        return Ok(ResolvedHead {
            id: id.clone(),
            enabled: false,
            mode: current.mode,
            position: current.position,
            transform: current.transform,
            scale: current.scale,
            adaptive_sync: false,
        });
    };

    let mode = match mode {
        None => current.mode,
        Some(request) => {
            resolve_mode(request, &current.modes).ok_or_else(|| Rejection::NoSuchMode(id.to_string()))?
        }
    };

    Ok(ResolvedHead {
        id: id.clone(),
        enabled: true,
        mode,
        position: position.unwrap_or(current.position),
        transform: transform.unwrap_or(current.transform),
        scale: scale.unwrap_or(current.scale),
        adaptive_sync: adaptive_sync.unwrap_or(current.adaptive_sync),
    })
}

/// Resolves a requested mode to one the hardware actually advertises.
///
/// A custom mode is only honoured when it names an advertised size: this
/// compositor has no modeline calculator, and accepting a made-up timing would
/// mean promising a mode the panel cannot display. The refresh rate is matched
/// to the nearest advertised one rather than compared exactly, because clients
/// round millihertz differently.
fn resolve_mode(request: &ModeRequest, modes: &[Mode]) -> Option<Mode> {
    match request {
        ModeRequest::Advertised(index) => modes.get(*index).copied(),
        ModeRequest::Custom { size, refresh } => {
            let matching = modes.iter().filter(|mode| mode.size == *size);
            match refresh {
                None => matching
                    .max_by_key(|mode| mode.refresh)
                    .copied(),
                Some(wanted) => matching
                    .min_by_key(|mode| (mode.refresh - wanted).abs())
                    .filter(|mode| (mode.refresh - wanted).abs() <= REFRESH_TOLERANCE)
                    .copied(),
            }
        }
    }
}

/// Everything that can be decided without asking the hardware.
fn validate(heads: &[ResolvedHead]) -> Result<(), Rejection> {
    if heads.iter().all(|head| !head.enabled) {
        return Err(Rejection::AllDisabled);
    }

    for head in heads {
        if !SCALE_RANGE.contains(&head.scale) {
            return Err(Rejection::ScaleOutOfRange(head.id.to_string(), head.scale));
        }
    }

    let enabled: Vec<&ResolvedHead> = heads.iter().filter(|head| head.enabled).collect();
    for (index, head) in enabled.iter().enumerate() {
        for other in &enabled[index + 1..] {
            if head.geometry().intersection(other.geometry()).is_some() {
                return Err(Rejection::Overlap(
                    head.id.to_string(),
                    other.id.to_string(),
                ));
            }
        }
    }

    Ok(())
}

fn head_snapshot(
    monitor: &Monitor,
    adaptive_sync: AdaptiveSync,
    vrr_support: VrrSupport,
) -> HeadSnapshot {
    let config = monitor.config();
    let modes = config
        .modes
        .iter()
        .map(|mode| ModeSnapshot {
            size: mode.size,
            refresh: mode.refresh,
            preferred: config.preferred_mode.is_some_and(|preferred| preferred == *mode),
        })
        .collect::<Vec<_>>();

    HeadSnapshot {
        id: HeadId::new(config.name.as_str()),
        description: description(config),
        // The EDID is the only honest source: smithay's `PhysicalProperties`
        // has no null, so it says "Unknown" where the panel said nothing, and
        // repeating that to clients would be a lie rather than an absence.
        make: config.edid.as_ref().map(|edid| edid.make.clone()),
        model: config.edid.as_ref().map(|edid| edid.model.clone()),
        serial: config.edid.as_ref().and_then(|edid| edid.serial.clone()),
        physical_size_mm: monitor.output().physical_properties().size,
        current_mode: modes
            .iter()
            .position(|mode| mode.size == config.mode.size && mode.refresh == config.mode.refresh),
        modes,
        // A monitor the shell holds is on by definition; a head that is off
        // has no `Monitor` at all. Disabled heads arrive with the backend's
        // head registry.
        enabled: true,
        position: config.position,
        transform: config.transform,
        scale: config.scale.fractional_scale(),
        adaptive_sync,
        vrr_support,
    }
}

/// The protocol's view of a monitor that is plugged in but switched off.
///
/// `position`, `transform` and `scale` are suppressed by the protocol for a
/// disabled head, so their values here are never sent.
fn disabled_snapshot(head: &DisabledHead) -> HeadSnapshot {
    HeadSnapshot {
        id: HeadId::new(head.name.as_str()),
        description: match (&head.make, &head.model) {
            (Some(make), Some(model)) => format!("{make} {model} ({})", head.name),
            _ => head.name.clone(),
        },
        make: head.make.clone(),
        model: head.model.clone(),
        serial: head.serial.clone(),
        physical_size_mm: head.physical_mm.into(),
        modes: head
            .modes
            .iter()
            .enumerate()
            .map(|(index, mode)| ModeSnapshot {
                size: mode.size,
                refresh: mode.refresh,
                preferred: head.preferred == Some(index),
            })
            .collect(),
        current_mode: None,
        enabled: false,
        position: (0, 0).into(),
        transform: Transform::Normal,
        scale: 1.0,
        adaptive_sync: AdaptiveSync::Disabled,
        vrr_support: if head.vrr_capable {
            VrrSupport::Supported
        } else {
            VrrSupport::Unsupported
        },
    }
}

/// smithay's spelling of adaptive-sync capability, in the protocol's terms.
fn vrr_support(support: DrmVrrSupport) -> VrrSupport {
    match support {
        DrmVrrSupport::NotSupported => VrrSupport::Unsupported,
        DrmVrrSupport::RequiresModeset => VrrSupport::RequiresModeset,
        DrmVrrSupport::Supported => VrrSupport::Supported,
    }
}

/// The entry for a head, matched on either spelling of its name, created if
/// it is not there yet.
///
/// New entries are keyed on the identity rather than the connector: identity
/// survives a replug into a different port, which is exactly when a user is
/// most annoyed to find their arrangement forgotten.
fn upsert<'a>(
    settings: &'a mut Vec<OutputSetting>,
    name: &str,
    identity: &str,
) -> &'a mut OutputSetting {
    match settings
        .iter()
        .position(|setting| setting.name == name || setting.name == identity)
    {
        Some(index) => &mut settings[index],
        None => {
            settings.push(OutputSetting {
                name: identity.to_owned(),
                ..OutputSetting::default()
            });
            settings.last_mut().expect("just pushed")
        }
    }
}

/// `"2560x1440@144.000"`, the spelling `OutputSetting::mode` documents.
fn format_mode(mode: Mode) -> String {
    format!(
        "{}x{}@{:.3}",
        mode.size.w,
        mode.size.h,
        mode.refresh as f64 / 1000.0
    )
}

fn transform_to_config(transform: Transform) -> config::OutputTransform {
    use config::OutputTransform as Config;
    match transform {
        Transform::Normal => Config::Normal,
        Transform::_90 => Config::R90,
        Transform::_180 => Config::R180,
        Transform::_270 => Config::R270,
        Transform::Flipped => Config::Flipped,
        Transform::Flipped90 => Config::Flipped90,
        Transform::Flipped180 => Config::Flipped180,
        Transform::Flipped270 => Config::Flipped270,
    }
}

/// `"Dell Inc. U2720Q (DP-1)"`, or just the connector name without an EDID.
///
/// Matches what `Output::new` builds internally, so a client that shows this
/// next to a `wl_output` sees the same string twice rather than two spellings
/// of the same monitor.
fn description(config: &OutputConfig) -> String {
    match &config.edid {
        Some(edid) => format!("{} {} ({})", edid.make, edid.model, config.name),
        None => config.name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(width: i32, height: i32, refresh: i32) -> Mode {
        Mode {
            size: (width, height).into(),
            refresh,
        }
    }

    fn modes() -> Vec<Mode> {
        vec![
            mode(2560, 1440, 144_000),
            mode(2560, 1440, 60_000),
            mode(1920, 1080, 60_000),
        ]
    }

    fn head(name: &str, position: (i32, i32), size: (i32, i32)) -> ResolvedHead {
        ResolvedHead {
            id: HeadId::new(name),
            enabled: true,
            mode: mode(size.0, size.1, 60_000),
            position: position.into(),
            transform: Transform::Normal,
            scale: 1.0,
            adaptive_sync: false,
        }
    }

    #[test]
    fn an_advertised_mode_resolves_by_index() {
        assert_eq!(
            resolve_mode(&ModeRequest::Advertised(1), &modes()),
            Some(mode(2560, 1440, 60_000))
        );
        assert_eq!(resolve_mode(&ModeRequest::Advertised(9), &modes()), None);
    }

    #[test]
    fn a_custom_mode_must_name_a_size_the_hardware_advertises() {
        assert_eq!(
            resolve_mode(
                &ModeRequest::Custom {
                    size: (3840, 2160).into(),
                    refresh: None,
                },
                &modes()
            ),
            None,
            "a size no mode has cannot be synthesised"
        );
    }

    #[test]
    fn a_custom_mode_without_a_refresh_takes_the_fastest() {
        assert_eq!(
            resolve_mode(
                &ModeRequest::Custom {
                    size: (2560, 1440).into(),
                    refresh: None,
                },
                &modes()
            ),
            Some(mode(2560, 1440, 144_000))
        );
    }

    #[test]
    fn a_custom_refresh_snaps_to_the_nearest_within_a_hertz() {
        // Clients round millihertz differently, so 59.9 Hz means 60.
        assert_eq!(
            resolve_mode(
                &ModeRequest::Custom {
                    size: (1920, 1080).into(),
                    refresh: Some(59_900),
                },
                &modes()
            ),
            Some(mode(1920, 1080, 60_000))
        );
    }

    #[test]
    fn a_custom_refresh_further_than_a_hertz_is_refused() {
        assert_eq!(
            resolve_mode(
                &ModeRequest::Custom {
                    size: (1920, 1080).into(),
                    refresh: Some(75_000),
                },
                &modes()
            ),
            None
        );
    }

    #[test]
    fn a_side_by_side_layout_is_accepted() {
        let heads = [
            head("DP-1", (0, 0), (1920, 1080)),
            head("DP-2", (1920, 0), (2560, 1440)),
        ];
        assert_eq!(validate(&heads), Ok(()));
    }

    #[test]
    fn a_stacked_layout_is_accepted() {
        let heads = [
            head("DP-1", (0, 0), (1920, 1080)),
            head("eDP-1", (0, 1080), (1920, 1080)),
        ];
        assert_eq!(validate(&heads), Ok(()));
    }

    #[test]
    fn overlapping_outputs_are_refused() {
        let heads = [
            head("DP-1", (0, 0), (1920, 1080)),
            head("DP-2", (1900, 0), (1920, 1080)),
        ];
        assert_eq!(
            validate(&heads),
            Err(Rejection::Overlap("DP-1".into(), "DP-2".into()))
        );
    }

    #[test]
    fn scale_changes_the_footprint_that_is_checked_for_overlap() {
        // At 1x these would touch; at 0.5x the first is half as wide, so the
        // gap the second sits in is real.
        let mut heads = [
            head("DP-1", (0, 0), (1920, 1080)),
            head("DP-2", (960, 0), (1920, 1080)),
        ];
        assert!(validate(&heads).is_err());

        heads[0].scale = 2.0;
        assert_eq!(validate(&heads), Ok(()));
    }

    #[test]
    fn a_transform_rotates_the_footprint() {
        // Rotated, the 1920x1080 panel is 1080 wide, so a neighbour at 1080
        // fits where one at 1920 would have been needed.
        let mut heads = [
            head("DP-1", (0, 0), (1920, 1080)),
            head("DP-2", (1080, 0), (1920, 1080)),
        ];
        assert!(validate(&heads).is_err());

        heads[0].transform = Transform::_90;
        assert_eq!(validate(&heads), Ok(()));
    }

    #[test]
    fn disabling_one_of_two_outputs_is_allowed() {
        let mut heads = [
            head("DP-1", (0, 0), (1920, 1080)),
            head("DP-2", (1920, 0), (1920, 1080)),
        ];
        heads[1].enabled = false;
        assert_eq!(validate(&heads), Ok(()));
    }

    #[test]
    fn a_disabled_output_is_not_checked_for_overlap() {
        // Both claim the same rectangle, but only one of them is on.
        let mut heads = [
            head("DP-1", (0, 0), (1920, 1080)),
            head("DP-2", (0, 0), (1920, 1080)),
        ];
        assert!(validate(&heads).is_err());

        heads[1].enabled = false;
        assert_eq!(validate(&heads), Ok(()));
    }

    #[test]
    fn a_disabled_head_resolves_against_its_preferred_mode() {
        let disabled = DisabledHead {
            name: "DP-3".into(),
            identity: "Dell U2720Q ABC".into(),
            make: None,
            model: None,
            serial: None,
            physical_mm: (600, 340),
            modes: vec![mode(1920, 1080, 60_000), mode(2560, 1440, 144_000)],
            preferred: Some(1),
            vrr_capable: false,
        };

        let current = Current::from_disabled(&disabled);
        assert_eq!(current.mode, mode(2560, 1440, 144_000));
        assert_eq!(current.scale, 1.0);
    }

    #[test]
    fn a_head_with_no_modes_resolves_to_a_zero_mode_rather_than_panicking() {
        let disabled = DisabledHead {
            name: "DP-4".into(),
            identity: "DP-4".into(),
            make: None,
            model: None,
            serial: None,
            physical_mm: (0, 0),
            modes: Vec::new(),
            preferred: None,
            vrr_capable: false,
        };
        assert_eq!(Current::from_disabled(&disabled).mode.refresh, 0);
    }

    #[test]
    fn an_existing_entry_is_updated_rather_than_duplicated() {
        // The user wrote this by hand, keyed on the connector name.
        let mut settings = vec![OutputSetting {
            name: "eDP-1".into(),
            scale: Some(2.0),
            ..OutputSetting::default()
        }];

        upsert(&mut settings, "eDP-1", "Some Panel XYZ").enabled = Some(false);

        assert_eq!(settings.len(), 1, "the hand-written entry must be reused");
        assert_eq!(settings[0].name, "eDP-1", "its spelling must be kept");
        assert_eq!(settings[0].scale, Some(2.0), "unrelated fields survive");
        assert_eq!(settings[0].enabled, Some(false));
    }

    #[test]
    fn a_new_entry_is_keyed_on_the_identity() {
        let mut settings = Vec::new();
        upsert(&mut settings, "DP-1", "Dell U2720Q ABC").scale = Some(1.5);

        // Identity, not connector: a replug into another port keeps working.
        assert_eq!(settings[0].name, "Dell U2720Q ABC");
    }

    #[test]
    fn an_entry_is_matched_on_either_spelling_of_its_name() {
        let mut settings = vec![OutputSetting {
            name: "Dell U2720Q ABC".into(),
            ..OutputSetting::default()
        }];
        upsert(&mut settings, "DP-1", "Dell U2720Q ABC").enabled = Some(true);
        assert_eq!(settings.len(), 1);
    }

    #[test]
    fn turning_everything_off_is_refused() {
        let mut heads = [head("DP-1", (0, 0), (1920, 1080))];
        heads[0].enabled = false;
        assert_eq!(validate(&heads), Err(Rejection::AllDisabled));
    }

    #[test]
    fn a_scale_outside_the_supported_range_is_refused() {
        let mut heads = [head("DP-1", (0, 0), (1920, 1080))];
        heads[0].scale = 12.0;
        assert_eq!(
            validate(&heads),
            Err(Rejection::ScaleOutOfRange("DP-1".into(), 12.0))
        );
    }
}
