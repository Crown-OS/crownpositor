//! The `Dispatch` impls that `smithay::delegate_dispatch2!` would write, with
//! one addition: requests on `zwp_text_input_v3` go through
//! [`TextInputPurposes`](super::text_input_purposes::TextInputPurposes)
//! before smithay handles them.

use std::any::Any;

use smithay::{
    reexports::{
        wayland_protocols::wp::text_input::zv3::server::zwp_text_input_v3::{self, ZwpTextInputV3},
        wayland_server::{
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
            backend::ClientId,
        },
    },
    wayland::{Dispatch2, GlobalDispatch2},
};

use super::State;

impl<I, UserData> Dispatch<I, UserData> for State
where
    I: Resource + 'static,
    I::Request: 'static,
    UserData: Dispatch2<I, State>,
{
    fn request(
        state: &mut Self,
        client: &Client,
        resource: &I,
        request: I::Request,
        data: &UserData,
        dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let (Some(text_input), Some(text_input_request)) = (
            (resource as &dyn Any).downcast_ref::<ZwpTextInputV3>(),
            (&request as &dyn Any).downcast_ref::<zwp_text_input_v3::Request>(),
        ) {
            state
                .wayland
                .text_input_purposes
                .record(text_input, text_input_request);
        }
        data.request(state, client, resource, request, dhandle, data_init);
    }

    fn destroyed(state: &mut Self, client: ClientId, resource: &I, data: &UserData) {
        if let Some(text_input) = (resource as &dyn Any).downcast_ref::<ZwpTextInputV3>() {
            state.wayland.text_input_purposes.forget(text_input);
        }
        data.destroyed(state, client, resource);
    }
}

impl<I, UserData> GlobalDispatch<I, UserData> for State
where
    I: Resource,
    UserData: GlobalDispatch2<I, State>,
{
    fn bind(
        state: &mut Self,
        dhandle: &DisplayHandle,
        client: &Client,
        resource: New<I>,
        data: &UserData,
        data_init: &mut DataInit<'_, Self>,
    ) {
        data.bind(state, dhandle, client, resource, data_init);
    }

    fn can_view(client: Client, data: &UserData) -> bool {
        data.can_view(&client)
    }
}
