//! One consent question of crownos-secretsd for a run or a proxy use.

use crownos_protocols::secret_input::v1::server::crownos_secret_consent_v1::CrownosSecretConsentV1;
use wayland_server::Resource;

/// What the agent wants the secrets for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentAction {
    Run,
    Proxy,
}

/// A question the compositor has not answered yet. Its texts are display
/// text from the daemon.
#[derive(Debug)]
pub struct SecretConsentQuestion {
    resource: CrownosSecretConsentV1,
    pub agent_app_key: String,
    pub secret_names: String,
    pub action: ConsentAction,
    pub detail: String,
}

impl SecretConsentQuestion {
    pub(super) const fn new(
        resource: CrownosSecretConsentV1,
        agent_app_key: String,
        secret_names: String,
        action: ConsentAction,
        detail: String,
    ) -> Self {
        Self {
            resource,
            agent_app_key,
            secret_names,
            action,
            detail,
        }
    }

    pub fn resource(&self) -> &CrownosSecretConsentV1 {
        &self.resource
    }

    /// Sends the answer, when the daemon still holds the question.
    pub fn answer(&self, granted: bool) {
        if self.resource.is_alive() {
            self.resource.consent(u32::from(granted));
        }
    }
}
