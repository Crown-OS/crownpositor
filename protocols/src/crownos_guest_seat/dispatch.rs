//! Request routing for `crownos_guest_seat_v1`.

use crownos_protocols::guest_seat::v1::server::{
    crownos_guest_seat_manager_v1::{self, CrownosGuestSeatManagerV1},
    crownos_guest_seat_v1::{self, CrownosGuestSeatV1},
};
use smithay::utils::Point;
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
    backend::ClientId,
};

use super::{
    GuestSeat, GuestSeatData, GuestSeatGlobalData, GuestSeatHandler, GuestSeatState, RevokeReason,
    is_valid_name,
};
use crate::crownos_agent_cursor::{Axis, ButtonState, CursorEvent, CursorFrame, KeyState};

/// The bounds every impl in this module shares, named once.
pub trait GuestSeatDispatch:
    GlobalDispatch<CrownosGuestSeatManagerV1, GuestSeatGlobalData>
    + Dispatch<CrownosGuestSeatManagerV1, ()>
    + Dispatch<CrownosGuestSeatV1, GuestSeatData>
    + GuestSeatHandler
    + 'static
{
}

impl<D> GuestSeatDispatch for D where
    D: GlobalDispatch<CrownosGuestSeatManagerV1, GuestSeatGlobalData>
        + Dispatch<CrownosGuestSeatManagerV1, ()>
        + Dispatch<CrownosGuestSeatV1, GuestSeatData>
        + GuestSeatHandler
        + 'static
{
}

impl<D: GuestSeatDispatch> GlobalDispatch<CrownosGuestSeatManagerV1, GuestSeatGlobalData, D>
    for GuestSeatState
{
    fn bind(
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<CrownosGuestSeatManagerV1>,
        _global_data: &GuestSeatGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, ());
    }

    fn can_view(client: Client, global_data: &GuestSeatGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D: GuestSeatDispatch> Dispatch<CrownosGuestSeatManagerV1, (), D> for GuestSeatState {
    fn request(
        state: &mut D,
        _client: &Client,
        manager: &CrownosGuestSeatManagerV1,
        request: crownos_guest_seat_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        use crownos_guest_seat_manager_v1::{Error, Request};

        let Request::CreateSeat { id, name, output } = request else {
            return;
        };
        if !is_valid_name(&name) {
            return manager.post_error(Error::InvalidName, "invalid guest name");
        }
        let guest = GuestSeat(data_init.init(id, GuestSeatData::default()));
        match state.create_guest_seat(guest.clone(), &name, &output) {
            Some(seat) => {
                state.guest_seat_state().record(guest.clone());
                guest.created(&seat);
            }
            None => guest.revoked(RevokeReason::Refused),
        }
    }
}

impl<D: GuestSeatDispatch> Dispatch<CrownosGuestSeatV1, GuestSeatData, D> for GuestSeatState {
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &CrownosGuestSeatV1,
        request: crownos_guest_seat_v1::Request,
        data: &GuestSeatData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        use crownos_guest_seat_v1::{self as guest, Error, Request};

        let invalid = |what| resource.post_error(Error::InvalidValue, what);
        let event = match request {
            Request::PointerMotion { dx, dy } => CursorEvent::Motion {
                delta: Point::from((dx, dy)),
            },
            Request::PointerMotionAbsolute { x, y } => CursorEvent::MotionAbsolute {
                position: Point::from((x, y)),
            },
            Request::Button { button, state } => {
                let state = match state {
                    WEnum::Value(guest::ButtonState::Pressed) => ButtonState::Pressed,
                    WEnum::Value(guest::ButtonState::Released) => ButtonState::Released,
                    _ => return invalid("unknown button state"),
                };
                CursorEvent::Button { button, state }
            }
            Request::Axis { axis, value } => {
                let axis = match axis {
                    WEnum::Value(guest::Axis::VerticalScroll) => Axis::VerticalScroll,
                    WEnum::Value(guest::Axis::HorizontalScroll) => Axis::HorizontalScroll,
                    _ => return invalid("unknown axis"),
                };
                CursorEvent::Axis { axis, value }
            }
            Request::Key { key, state } => {
                let state = match state {
                    WEnum::Value(guest::KeyState::Pressed) => KeyState::Pressed,
                    WEnum::Value(guest::KeyState::Released) => KeyState::Released,
                    _ => return invalid("unknown key state"),
                };
                CursorEvent::Key { key, state }
            }
            Request::Frame { time_ms } => {
                let frame = CursorFrame {
                    events: data.take(),
                    time_ms,
                };
                return state.guest_seat_frame(&GuestSeat(resource.clone()), frame);
            }
            _ => return,
        };
        data.push(event);
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        resource: &CrownosGuestSeatV1,
        _data: &GuestSeatData,
    ) {
        let guest = GuestSeat(resource.clone());
        if state.guest_seat_state().forget(&guest) {
            state.guest_seat_destroyed(&guest);
        }
    }
}
