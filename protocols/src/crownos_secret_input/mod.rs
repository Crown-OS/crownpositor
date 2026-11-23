//! Server-side `crownos_secret_input_v1`.
//!
//! The protocol half reads and checks the values and collects one request
//! until its commit. The compositor makes every decision in
//! [`SecretInputHandler`]: the dialog, the focus and field checks and the
//! typing. The values are zeroed when the request is dropped.

mod dispatch;
mod request;
mod sealed_value;

use crownos_protocols::secret_input::v1::server::crownos_secret_input_manager_v1::CrownosSecretInputManagerV1;
use crownos_protocols::secret_input::v1::server::crownos_secret_input_request_v1::CrownosSecretInputRequestV1;
use wayland_server::{Client, DisplayHandle, backend::GlobalId};

pub use crownos_protocols::secret_input::v1::server::crownos_secret_input_request_v1::Outcome;
pub use dispatch::{RequestData, SecretInputDispatch};
pub use request::{RequestBuilder, RequestDetails, SecretInputRequest, TypingValue, ValueKind};
pub use sealed_value::{MAX_VALUE_BYTES, ValueRefusal, read_sealed_value};

/// The protocol version this module speaks.
pub const VERSION: u32 = 1;

/// What the compositor provides for this protocol to be delegated to
/// [`SecretInputState`].
pub trait SecretInputHandler {
    /// A request was committed. Answer it with [`SecretInputRequest::finish`],
    /// now or later.
    fn secret_input_committed(&mut self, request: SecretInputRequest);

    /// The client destroyed a committed request before its answer.
    fn secret_input_withdrawn(&mut self, resource: &CrownosSecretInputRequestV1);
}

/// Global data: which clients may see the manager.
pub struct SecretInputGlobalData {
    filter: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

/// Delegate type for the `crownos_secret_input_manager_v1` global.
pub struct SecretInputState {
    global: GlobalId,
}

impl SecretInputState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: SecretInputDispatch,
        F: Fn(&Client) -> bool + Send + Sync + 'static,
    {
        let global = display.create_global::<D, CrownosSecretInputManagerV1, _>(
            VERSION,
            SecretInputGlobalData {
                filter: Box::new(filter),
            },
        );
        Self { global }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }
}
