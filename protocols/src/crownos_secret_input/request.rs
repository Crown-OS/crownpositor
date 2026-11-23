//! One typing request: collected until its commit, then handed to the
//! compositor until it is answered.

use crownos_protocols::secret_input::v1::server::crownos_secret_input_request_v1::{
    CrownosSecretInputRequestV1, Outcome,
};
use wayland_server::{Client, Resource};
use zeroize::Zeroizing;

/// What the value is, which decides the field it may go into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    Username,
    Password,
}

/// One value to type. Its text is zeroed on drop and is never formatted.
pub struct TypingValue {
    pub text: Zeroizing<String>,
    pub kind: ValueKind,
    pub tab_after: bool,
}

/// The display text and the target of a request, as the daemon sent them.
#[derive(Debug, Clone, Default)]
pub struct RequestDetails {
    pub window_identifier: String,
    pub app_id: String,
    pub require_password_field: bool,
    pub agent: String,
    pub secret_names: String,
    pub origin: String,
}

/// The user data of a request object before its commit.
#[derive(Default)]
pub struct RequestBuilder {
    pub(super) details: RequestDetails,
    pub(super) values: Vec<TypingValue>,
    pub(super) refused: bool,
    pub(super) committed: bool,
}

impl RequestBuilder {
    pub(super) fn new(details: RequestDetails) -> Self {
        Self {
            details,
            ..Self::default()
        }
    }
}

/// A committed request. Dropping it without [`finish`](Self::finish) leaves
/// the daemon waiting, which is what a dialog still on screen does.
pub struct SecretInputRequest {
    resource: CrownosSecretInputRequestV1,
    details: RequestDetails,
    values: Vec<TypingValue>,
    refused: bool,
}

impl SecretInputRequest {
    pub(super) fn new(
        resource: CrownosSecretInputRequestV1,
        details: RequestDetails,
        values: Vec<TypingValue>,
        refused: bool,
    ) -> Self {
        Self {
            resource,
            details,
            values,
            refused,
        }
    }

    pub fn details(&self) -> &RequestDetails {
        &self.details
    }

    pub fn values(&self) -> &[TypingValue] {
        &self.values
    }

    /// Whether a value broke the rules of `add_value`.
    pub fn has_bad_value(&self) -> bool {
        self.refused
    }

    pub fn client(&self) -> Option<Client> {
        self.resource.client()
    }

    pub fn is_for(&self, resource: &CrownosSecretInputRequestV1) -> bool {
        self.resource == *resource
    }

    /// Whether the client still holds the object, so an answer would reach it.
    pub fn is_pending(&self) -> bool {
        self.resource.is_alive()
    }

    /// Sends the outcome and drops the values.
    pub fn finish(self, outcome: Outcome) {
        if self.resource.is_alive() {
            self.resource.done(outcome);
        }
    }
}
