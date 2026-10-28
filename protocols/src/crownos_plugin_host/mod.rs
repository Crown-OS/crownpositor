//! Server-side `crownos_plugin_host_v1`: crownplugind's link to the
//! compositor.
//!
//! The protocol half reads payloads safely and enforces the wire limits;
//! what a scene, a slot or an asset means — and whether it is acceptable — is
//! the compositor's call through [`PluginHostHandler`]. One client at a time
//! may hold the global.

mod dispatch;
mod payload;

use crownos_protocols::plugin_host::v1::server::crownos_plugin_host_v1::CrownosPluginHostV1;
use wayland_server::{Client, DisplayHandle, Resource, backend::GlobalId};

pub use dispatch::PluginHostDispatch;
pub use payload::PayloadError;

pub const VERSION: u32 = 1;

/// Payload caps, as the protocol states them.
pub const SCENE_BYTES: usize = 256 * 1024;
pub const SLOT_BYTES: usize = 256 * 1024;
pub const ASSET_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionOutcome {
    Done,
    Invalid,
    Refused,
}

/// Why a lock request was refused; either way the client is disconnected
/// with the matching protocol error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestError {
    Invalid(String),
    TooLarge,
}

pub trait PluginHostHandler {
    fn plugin_host_state(&mut self) -> &mut PluginHostState;
    fn run_plugin_action(&mut self, action: &str) -> ActionOutcome;
    fn set_lock_scene(&mut self, bytes: Vec<u8>) -> Result<(), RequestError>;
    fn set_lock_slot(&mut self, id: &str, bytes: Vec<u8>) -> Result<(), RequestError>;
    fn put_lock_asset(&mut self, id: &str, bytes: Vec<u8>) -> Result<(), RequestError>;
    /// The client went away: whatever it sent should be forgotten.
    fn plugin_host_gone(&mut self);
}

pub struct PluginHostGlobalData {
    filter: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

pub struct PluginHostState {
    global: GlobalId,
    bound: Option<CrownosPluginHostV1>,
    locked: bool,
    watching: u32,
}

impl PluginHostState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: PluginHostDispatch,
        F: Fn(&Client) -> bool + Send + Sync + 'static,
    {
        let global = display.create_global::<D, CrownosPluginHostV1, _>(
            VERSION,
            PluginHostGlobalData {
                filter: Box::new(filter),
            },
        );
        Self {
            global,
            bound: None,
            locked: false,
            watching: 0,
        }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }

    /// Tells the bound client, if the lock state changed.
    pub fn set_locked(&mut self, locked: bool) {
        if self.locked == locked {
            return;
        }
        self.locked = locked;
        if let Some(host) = &self.bound {
            host.lock_state(u32::from(locked));
        }
    }

    /// The event classes the client asked for, as the `watch` bitfield.
    pub fn watching(&self) -> u32 {
        self.watching
    }

    fn is_bound(&self) -> bool {
        self.bound.as_ref().is_some_and(Resource::is_alive)
    }
}
