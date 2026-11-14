//! Request routing for `crownos_agent_cursor_v1`.

use crownos_protocols::agent_cursor::v1::server::{
    crownos_agent_cursor_manager_v1::{self, CrownosAgentCursorManagerV1},
    crownos_agent_cursor_v1::{self, CrownosAgentCursorV1},
};
use smithay::utils::Point;
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
    backend::ClientId,
};

use super::{
    AgentCursor, AgentCursorGlobalData, AgentCursorHandler, AgentCursorState, CreateCursorError,
    CursorData, CursorEvent, CursorFrame, RevokeReason, is_valid_name,
};

/// The bounds every impl in this module shares, named once.
pub trait AgentCursorDispatch:
    GlobalDispatch<CrownosAgentCursorManagerV1, AgentCursorGlobalData>
    + Dispatch<CrownosAgentCursorManagerV1, ()>
    + Dispatch<CrownosAgentCursorV1, CursorData>
    + AgentCursorHandler
    + 'static
{
}

impl<D> AgentCursorDispatch for D where
    D: GlobalDispatch<CrownosAgentCursorManagerV1, AgentCursorGlobalData>
        + Dispatch<CrownosAgentCursorManagerV1, ()>
        + Dispatch<CrownosAgentCursorV1, CursorData>
        + AgentCursorHandler
        + 'static
{
}

impl<D: AgentCursorDispatch> GlobalDispatch<CrownosAgentCursorManagerV1, AgentCursorGlobalData, D>
    for AgentCursorState
{
    fn bind(
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<CrownosAgentCursorManagerV1>,
        _global_data: &AgentCursorGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, ());
    }

    fn can_view(client: Client, global_data: &AgentCursorGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D: AgentCursorDispatch> Dispatch<CrownosAgentCursorManagerV1, (), D> for AgentCursorState {
    fn request(
        state: &mut D,
        _client: &Client,
        manager: &CrownosAgentCursorManagerV1,
        request: crownos_agent_cursor_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        use crownos_agent_cursor_manager_v1::{Error, Request};

        let Request::CreateCursor { id, name, color } = request else {
            return;
        };
        if !is_valid_name(&name) {
            return manager.post_error(Error::InvalidName, "invalid cursor name");
        }
        let cursor = AgentCursor(data_init.init(id, CursorData::default()));
        match state.create_agent_cursor(cursor.clone(), &name, color) {
            Ok(seat) => {
                state.agent_cursor_state().record(cursor.clone());
                cursor.created(&seat);
            }
            Err(CreateCursorError::TooMany) => {
                manager.post_error(Error::TooMany, "too many agent cursors");
            }
            Err(CreateCursorError::Refused) => cursor.revoked(RevokeReason::Refused),
        }
    }
}

impl<D: AgentCursorDispatch> Dispatch<CrownosAgentCursorV1, CursorData, D> for AgentCursorState {
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &CrownosAgentCursorV1,
        request: crownos_agent_cursor_v1::Request,
        data: &CursorData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        use crownos_agent_cursor_v1::{Error, Request};

        let cursor = AgentCursor(resource.clone());
        let event = match request {
            Request::PointerMotion { dx, dy } => CursorEvent::Motion {
                delta: Point::from((dx, dy)),
            },
            Request::PointerMotionAbsolute { x, y } => CursorEvent::MotionAbsolute {
                position: Point::from((x, y)),
            },
            Request::Button { button, state } => {
                let WEnum::Value(state) = state else {
                    return resource.post_error(Error::InvalidValue, "unknown button state");
                };
                CursorEvent::Button { button, state }
            }
            Request::Axis { axis, value } => {
                let WEnum::Value(axis) = axis else {
                    return resource.post_error(Error::InvalidValue, "unknown axis");
                };
                CursorEvent::Axis { axis, value }
            }
            Request::Key { key, state } => {
                let WEnum::Value(state) = state else {
                    return resource.post_error(Error::InvalidValue, "unknown key state");
                };
                CursorEvent::Key { key, state }
            }
            Request::Frame { time_ms } => {
                let frame = CursorFrame {
                    events: data.take(),
                    time_ms,
                };
                return state.agent_cursor_frame(&cursor, frame);
            }
            Request::FocusWindow { identifier } => {
                return state.agent_cursor_focus(&cursor, &identifier);
            }
            Request::Destroy => return,
            _ => return,
        };
        data.push(event);
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        resource: &CrownosAgentCursorV1,
        _data: &CursorData,
    ) {
        let cursor = AgentCursor(resource.clone());
        if state.agent_cursor_state().forget(&cursor) {
            state.agent_cursor_destroyed(&cursor);
        }
    }
}
