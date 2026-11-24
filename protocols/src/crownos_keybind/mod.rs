//! Server-side `crownos_keybind_v1`.
//!
//! The protocol half keeps the registrations in the order they arrived and
//! nothing else. Which chord each action gets is the compositor's decision,
//! made in [`KeybindHandler::keybinds_changed`] and given back through
//! [`KeybindRegistration::assign`].

mod dispatch;
mod registration;

use crownos_protocols::keybind::v1::server::{
    crownos_keybind_manager_v1::CrownosKeybindManagerV1, crownos_keybind_v1::CrownosKeybindV1,
};
use wayland_server::{DisplayHandle, backend::GlobalId};

pub use dispatch::KeybindDispatch;
pub use registration::KeybindRegistration;

/// The protocol version this module speaks.
pub const VERSION: u32 = 1;

/// What the compositor provides for this protocol to be delegated to
/// [`KeybindState`].
pub trait KeybindHandler {
    fn keybind_state(&mut self) -> &mut KeybindState;

    /// An action was registered or removed. The compositor assigns the chords
    /// again, and must give the new registration its first `assigned`.
    fn keybinds_changed(&mut self);
}

/// Delegate type for the `crownos_keybind_manager_v1` global.
pub struct KeybindState {
    global: GlobalId,
    registrations: Vec<KeybindRegistration>,
}

impl KeybindState {
    pub fn new<D: KeybindDispatch>(display: &DisplayHandle) -> Self {
        let global = display.create_global::<D, CrownosKeybindManagerV1, _>(VERSION, ());
        Self {
            global,
            registrations: Vec::new(),
        }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }

    /// Every live registration, oldest first.
    pub fn registrations(&self) -> &[KeybindRegistration] {
        &self.registrations
    }

    pub fn registrations_mut(&mut self) -> &mut [KeybindRegistration] {
        &mut self.registrations
    }

    fn record(&mut self, registration: KeybindRegistration) {
        self.registrations.push(registration);
    }

    fn release(&mut self, resource: &CrownosKeybindV1) -> bool {
        let count = self.registrations.len();
        self.registrations
            .retain(|registration| registration.resource() != resource);
        self.registrations.len() != count
    }
}
