//! One action an application registered, and the chord it was told it has.

use crownos_protocols::keybind::v1::server::crownos_keybind_v1::CrownosKeybindV1;
use wayland_server::Resource;

#[derive(Debug)]
pub struct KeybindRegistration {
    resource: CrownosKeybindV1,
    app_id: String,
    name: String,
    description: String,
    preferred_keys: String,
    assigned_keys: Option<String>,
}

impl KeybindRegistration {
    pub(super) fn new(
        resource: CrownosKeybindV1,
        app_id: String,
        name: String,
        description: String,
        preferred_keys: String,
    ) -> Self {
        Self {
            resource,
            app_id,
            name,
            description,
            preferred_keys,
            assigned_keys: None,
        }
    }

    pub fn resource(&self) -> &CrownosKeybindV1 {
        &self.resource
    }

    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn preferred_keys(&self) -> &str {
        &self.preferred_keys
    }

    /// The chord the client was last told it has. `None` before the first
    /// `assigned`.
    pub fn assigned_keys(&self) -> Option<&str> {
        self.assigned_keys.as_deref()
    }

    /// Whether `other` registered the same action of the same application.
    pub fn is_same_action(&self, other: &Self) -> bool {
        self.app_id == other.app_id && self.name == other.name
    }

    /// Sends `assigned` when `keys` is new to the client. Returns whether it
    /// was sent.
    pub fn assign(&mut self, keys: &str) -> bool {
        if self.assigned_keys.as_deref() == Some(keys) || !self.resource.is_alive() {
            return false;
        }
        self.resource.assigned(keys.to_owned());
        self.assigned_keys = Some(keys.to_owned());
        true
    }
}
