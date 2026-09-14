//! Server-side `zwlr_output_management_unstable_v1`, version 4.
//!
//! The protocol lets a privileged client enumerate the compositor's outputs
//! and reconfigure them as one transaction: build a configuration, describe
//! every head in it, then `test` or `apply`. A serial guards the whole thing,
//! so a configuration built against a stale view of the world is cancelled
//! rather than applied to the wrong monitors.
//!
//! # Heads are data, not outputs
//!
//! Unlike the equivalent module in cosmic-comp, nothing here names
//! `smithay::output::Output`. The compositor hands over a [`HeadSnapshot`] per
//! head and gets an [`OutputConfigRequest`] back, which buys three things:
//!
//! * the `protocols` crate keeps its rule that nothing in it reaches back into
//!   the compositor,
//! * a *disabled* head needs no `Output` to exist, so the compositor's "one
//!   door in, one door out" rule for outputs survives — an output that is off
//!   is simply not there, rather than being a zombie with its global removed,
//! * a head is not tied to a DRM connector, so the winit backend and the
//!   virtual outputs crownconnect will create are ordinary heads.

mod config;
mod dispatch;
mod head;

use std::sync::Arc;

use smithay::utils::{Logical, Physical, Point, Raw, Size, Transform};
use wayland_protocols_wlr::output_management::v1::server::{
    zwlr_output_configuration_head_v1::ZwlrOutputConfigurationHeadV1,
    zwlr_output_configuration_v1::ZwlrOutputConfigurationV1,
    zwlr_output_head_v1::ZwlrOutputHeadV1,
    zwlr_output_manager_v1::ZwlrOutputManagerV1,
    zwlr_output_mode_v1::ZwlrOutputModeV1,
};
use wayland_server::{Client, Dispatch, DisplayHandle, GlobalDispatch, backend::GlobalId};

pub use config::{
    HeadConfig, ModeRequest, OutputConfigRequest, PendingConfiguration, PendingHeadConfiguration,
};
pub use head::{HeadData, ModeData};

/// The generated bindings, so [`delegate_output_management!`] can name every
/// interface through `$crate` instead of requiring the compositor to depend on
/// `wayland-protocols-wlr` itself.
///
/// [`delegate_output_management!`]: crate::delegate_output_management
#[doc(hidden)]
pub mod reexports {
    pub use wayland_protocols_wlr::output_management::v1::server::*;
}

/// The protocol version this module speaks.
///
/// 4 is the newest: it adds `adaptive_sync` to heads and `set_adaptive_sync`
/// to configuration heads.
pub const VERSION: u32 = 4;

/// A head's identity for as long as the session lasts.
///
/// The connector name on KMS (`"DP-1"`), `"winit"` in a nested session, and a
/// namespaced string for a virtual output. It is what
/// `zwlr_output_head_v1.name` promises to be — stable, and usable to find the
/// head again across a `done`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HeadId(Arc<str>);

impl HeadId {
    pub fn new(name: impl Into<Arc<str>>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for HeadId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One mode a head can be set to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeSnapshot {
    pub size: Size<i32, Physical>,
    /// Millihertz, as the protocol and DRM both count it.
    pub refresh: i32,
    pub preferred: bool,
}

/// Whether a head is currently doing adaptive sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AdaptiveSync {
    #[default]
    Disabled,
    Enabled,
}

/// What the hardware is willing to do, which the protocol has no event for.
///
/// Carried anyway: the compositor needs it to decide whether enabling VRR
/// forces a modeset, and the settings GUI wants to grey the toggle out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VrrSupport {
    #[default]
    Unsupported,
    RequiresModeset,
    Supported,
}

/// Everything the protocol can say about one head.
///
/// Built by the compositor on demand and diffed against the last one, so the
/// only thing that has to be right is that equal snapshots mean "nothing
/// changed" — hence `PartialEq` on every field.
#[derive(Debug, Clone, PartialEq)]
pub struct HeadSnapshot {
    pub id: HeadId,
    /// Human-readable, e.g. `"Dell Inc. U2720Q (DP-1)"`.
    pub description: String,
    /// `None` suppresses the event entirely rather than sending a placeholder:
    /// a client must be able to tell "no EDID" from a monitor actually called
    /// "Unknown".
    pub make: Option<String>,
    pub model: Option<String>,
    pub serial: Option<String>,
    /// Millimetres, which is `Raw` rather than `Physical`: these are not
    /// pixels and must never be scaled.
    pub physical_size_mm: Size<i32, Raw>,
    /// Order is stable and matches the compositor's own mode list, because
    /// [`ModeRequest::Advertised`] indexes into it.
    pub modes: Vec<ModeSnapshot>,
    /// Index into [`Self::modes`]. `None` only while the head is disabled.
    pub current_mode: Option<usize>,
    pub enabled: bool,
    pub position: Point<i32, Logical>,
    pub transform: Transform,
    pub scale: f64,
    pub adaptive_sync: AdaptiveSync,
    pub vrr_support: VrrSupport,
}

/// What the compositor must provide for this protocol to be delegated to
/// [`OutputManagementState`].
pub trait OutputManagementHandler {
    fn output_management_state(&mut self) -> &mut OutputManagementState;

    /// Would this configuration apply? Must not change anything.
    fn test_output_config(&mut self, request: OutputConfigRequest) -> bool;

    /// Apply it, and say whether that worked.
    ///
    /// A `false` here becomes `failed` on the wire, so it must mean the
    /// compositor is back in the state it started in.
    fn apply_output_config(&mut self, request: OutputConfigRequest) -> bool;
}

/// Which clients may see the manager global.
pub struct OutputManagementGlobalData {
    filter: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

/// Delegate type for the `zwlr_output_manager_v1` global.
pub struct OutputManagementState {
    global: GlobalId,
    /// Held because head and mode objects are created outside a request — on
    /// a hotplug, not on a bind — and `Client::create_resource` needs it.
    dh: DisplayHandle,
    /// Bumped whenever [`Self::set_heads`] finds a real change. Every
    /// configuration carries the serial it was built against.
    serial: u32,
    /// The advertised state, and the baseline the next diff runs against.
    heads: Vec<HeadSnapshot>,
    instances: Vec<head::ManagerInstance>,
}

impl OutputManagementState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: GlobalDispatch<ZwlrOutputManagerV1, OutputManagementGlobalData>
            + Dispatch<ZwlrOutputManagerV1, ()>
            + Dispatch<ZwlrOutputHeadV1, HeadData>
            + Dispatch<ZwlrOutputModeV1, ModeData>
            + Dispatch<ZwlrOutputConfigurationV1, config::PendingConfiguration>
            + Dispatch<ZwlrOutputConfigurationHeadV1, config::PendingHeadConfiguration>
            + OutputManagementHandler
            + 'static,
        F: Fn(&Client) -> bool + Send + Sync + 'static,
    {
        let global = display.create_global::<D, ZwlrOutputManagerV1, _>(
            VERSION,
            OutputManagementGlobalData {
                filter: Box::new(filter),
            },
        );

        Self {
            global,
            dh: display.clone(),
            serial: 0,
            heads: Vec::new(),
            instances: Vec::new(),
        }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }

    pub fn serial(&self) -> u32 {
        self.serial
    }

    pub fn heads(&self) -> &[HeadSnapshot] {
        &self.heads
    }

    /// Replaces the advertised state, telling every client what changed.
    ///
    /// Returns without touching the serial when the snapshots are identical.
    /// That no-op is load-bearing: applying a configuration makes the
    /// compositor re-snapshot, and a serial bump there would `cancelled` the
    /// very client that just succeeded, which reads as a failure and makes
    /// well-behaved clients retry forever.
    pub fn set_heads<D>(&mut self, heads: Vec<HeadSnapshot>)
    where
        D: Dispatch<ZwlrOutputHeadV1, HeadData> + Dispatch<ZwlrOutputModeV1, ModeData> + 'static,
    {
        if self.heads == heads {
            return;
        }

        self.serial = self.serial.wrapping_add(1);
        head::apply_diff::<D>(&self.dh, &mut self.instances, &self.heads, &heads);
        self.heads = heads;

        for instance in &self.instances {
            instance.obj.done(self.serial);
        }
    }

    fn snapshot(&self, id: &HeadId) -> Option<&HeadSnapshot> {
        self.heads.iter().find(|head| head.id == *id)
    }
}
