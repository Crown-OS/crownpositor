//! Request routing for `crownos_secret_input_v1`.

use std::sync::{Mutex, PoisonError};

use crownos_protocols::secret_input::v1::server::{
    crownos_secret_input_manager_v1::{self, CrownosSecretInputManagerV1},
    crownos_secret_input_request_v1::{self, CrownosSecretInputRequestV1, Error, Kind},
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
    backend::ClientId,
};

use super::{
    SecretInputGlobalData, SecretInputHandler, SecretInputState,
    request::{RequestBuilder, RequestDetails, SecretInputRequest, TypingValue, ValueKind},
    sealed_value::read_sealed_value,
};

/// The user data of a `crownos_secret_input_request_v1` object.
pub type RequestData = Mutex<RequestBuilder>;

/// The bounds every impl in this module shares, named once.
pub trait SecretInputDispatch:
    GlobalDispatch<CrownosSecretInputManagerV1, SecretInputGlobalData>
    + Dispatch<CrownosSecretInputManagerV1, ()>
    + Dispatch<CrownosSecretInputRequestV1, RequestData>
    + SecretInputHandler
    + 'static
{
}

impl<D> SecretInputDispatch for D where
    D: GlobalDispatch<CrownosSecretInputManagerV1, SecretInputGlobalData>
        + Dispatch<CrownosSecretInputManagerV1, ()>
        + Dispatch<CrownosSecretInputRequestV1, RequestData>
        + SecretInputHandler
        + 'static
{
}

impl<D: SecretInputDispatch> GlobalDispatch<CrownosSecretInputManagerV1, SecretInputGlobalData, D>
    for SecretInputState
{
    fn bind(
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<CrownosSecretInputManagerV1>,
        _global_data: &SecretInputGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, ());
    }

    fn can_view(client: Client, global_data: &SecretInputGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D: SecretInputDispatch> Dispatch<CrownosSecretInputManagerV1, (), D> for SecretInputState {
    fn request(
        _state: &mut D,
        _client: &Client,
        _manager: &CrownosSecretInputManagerV1,
        request: crownos_secret_input_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        let crownos_secret_input_manager_v1::Request::CreateRequest {
            id,
            window_identifier,
            expected_app_key,
            require_password_field,
            agent,
            secret_name,
            origin,
        } = request
        else {
            return;
        };
        let details = RequestDetails {
            window_identifier,
            expected_app_key,
            require_password_field: require_password_field != 0,
            agent,
            secret_names: secret_name,
            origin,
        };
        data_init.init(id, Mutex::new(RequestBuilder::new(details)));
    }
}

impl<D: SecretInputDispatch> Dispatch<CrownosSecretInputRequestV1, RequestData, D>
    for SecretInputState
{
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &CrownosSecretInputRequestV1,
        request: crownos_secret_input_request_v1::Request,
        data: &RequestData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        let mut builder = data.lock().unwrap_or_else(PoisonError::into_inner);
        match request {
            crownos_secret_input_request_v1::Request::AddValue {
                value,
                kind,
                tab_after,
            } => {
                if builder.committed {
                    return resource.post_error(Error::AlreadyCommitted, "add_value after commit");
                }
                let kind = match kind {
                    WEnum::Value(Kind::Username) => Some(ValueKind::Username),
                    WEnum::Value(Kind::Password) => Some(ValueKind::Password),
                    _ => None,
                };
                match (read_sealed_value(value), kind) {
                    (Ok(text), Some(kind)) => builder.values.push(TypingValue {
                        text,
                        kind,
                        tab_after: tab_after != 0,
                    }),
                    (Err(refusal), _) => {
                        tracing::info!(%refusal, "a secret input value was refused");
                        builder.refused = true;
                    }
                    (Ok(_), None) => builder.refused = true,
                }
            }
            crownos_secret_input_request_v1::Request::Commit => {
                if builder.committed {
                    return resource.post_error(Error::AlreadyCommitted, "commit after commit");
                }
                if builder.values.is_empty() && !builder.refused {
                    return resource.post_error(Error::NoValue, "commit without a value");
                }
                builder.committed = true;
                let request = SecretInputRequest::new(
                    resource.clone(),
                    std::mem::take(&mut builder.details),
                    std::mem::take(&mut builder.values),
                    builder.refused,
                );
                drop(builder);
                state.secret_input_committed(request);
            }
            _ => {}
        }
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        resource: &CrownosSecretInputRequestV1,
        data: &RequestData,
    ) {
        let committed = data
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .committed;
        if committed {
            state.secret_input_withdrawn(resource);
        }
    }
}
