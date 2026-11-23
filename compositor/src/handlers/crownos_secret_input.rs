//! Delegation glue for `crownos_secret_input_v1`. The protocol lives in
//! [`protocols::crownos_secret_input`], the flow in
//! [`crate::state::secret_typing`].

use crownos_protocols::secret_input::v1::server::{
    crownos_secret_consent_v1::CrownosSecretConsentV1,
    crownos_secret_input_manager_v1::CrownosSecretInputManagerV1,
    crownos_secret_input_request_v1::CrownosSecretInputRequestV1,
};
use protocols::crownos_secret_input::{
    RequestData, SecretConsentQuestion, SecretInputGlobalData, SecretInputHandler,
    SecretInputRequest, SecretInputState,
};
use wayland_server::{delegate_dispatch, delegate_global_dispatch};

use crate::state::State;

impl SecretInputHandler for State {
    fn secret_input_committed(&mut self, request: SecretInputRequest) {
        self.start_secret_typing(request);
    }

    fn secret_input_withdrawn(&mut self, resource: &CrownosSecretInputRequestV1) {
        self.withdraw_secret_typing(resource);
    }

    fn secret_consent_asked(&mut self, question: SecretConsentQuestion) {
        self.ask_secret_consent(question);
    }

    fn secret_consent_withdrawn(&mut self, resource: &CrownosSecretConsentV1) {
        self.withdraw_secret_consent(resource);
    }
}

delegate_global_dispatch!(State: [CrownosSecretInputManagerV1: SecretInputGlobalData] => SecretInputState);
delegate_dispatch!(State: [CrownosSecretInputManagerV1: ()] => SecretInputState);
delegate_dispatch!(State: [CrownosSecretInputRequestV1: RequestData] => SecretInputState);
delegate_dispatch!(State: [CrownosSecretConsentV1: ()] => SecretInputState);
