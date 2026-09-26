//! Request routing for `crownos_input_v1`.

use crownos_protocols::input::v1::server::{
    crownos_input_capture_v1::{self, CrownosInputCaptureV1},
    crownos_input_injector_v1::{self, CrownosInputInjectorV1},
    crownos_input_manager_v1::{self, CrownosInputManagerV1},
};
use smithay::utils::{Clock, Monotonic, Point};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
    backend::ClientId,
};

use super::{
    CaptureData, InjectorData, InputCapture, InputGlobalData, InputHandler, InputState,
    event::{InjectedEvent, InjectedFrame},
};

/// The bounds every impl in this module shares, named once.
pub trait InputDispatch:
    GlobalDispatch<CrownosInputManagerV1, InputGlobalData>
    + Dispatch<CrownosInputManagerV1, ()>
    + Dispatch<CrownosInputInjectorV1, InjectorData>
    + Dispatch<CrownosInputCaptureV1, CaptureData>
    + InputHandler
    + 'static
{
}

impl<D> InputDispatch for D where
    D: GlobalDispatch<CrownosInputManagerV1, InputGlobalData>
        + Dispatch<CrownosInputManagerV1, ()>
        + Dispatch<CrownosInputInjectorV1, InjectorData>
        + Dispatch<CrownosInputCaptureV1, CaptureData>
        + InputHandler
        + 'static
{
}

impl<D: InputDispatch> GlobalDispatch<CrownosInputManagerV1, InputGlobalData, D> for InputState {
    fn bind(
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<CrownosInputManagerV1>,
        _global_data: &InputGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, ());
    }

    fn can_view(client: Client, global_data: &InputGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D: InputDispatch> Dispatch<CrownosInputManagerV1, (), D> for InputState {
    fn request(
        _state: &mut D,
        _client: &Client,
        _manager: &CrownosInputManagerV1,
        request: crownos_input_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            crownos_input_manager_v1::Request::CreateInjector { injector, .. } => {
                data_init.init(injector, InjectorData::default());
            }
            crownos_input_manager_v1::Request::CreateCapture { capture, .. } => {
                data_init.init(capture, CaptureData);
            }
            crownos_input_manager_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

impl<D: InputDispatch> Dispatch<CrownosInputInjectorV1, InjectorData, D> for InputState {
    fn request(
        state: &mut D,
        _client: &Client,
        injector: &CrownosInputInjectorV1,
        request: crownos_input_injector_v1::Request,
        data: &InjectorData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        use crownos_input_injector_v1::{Error, Request};

        let event = match request {
            Request::PointerMotion { dx, dy } => InjectedEvent::PointerMotion {
                delta: Point::from((dx, dy)),
            },
            Request::PointerMotionAbsolute { output, x, y } => {
                InjectedEvent::PointerMotionAbsolute {
                    output,
                    position: Point::from((x, y)),
                }
            }
            Request::Button { button, state } => {
                let WEnum::Value(state) = state else {
                    return injector.post_error(Error::InvalidValue, "unknown button state");
                };
                data.with(|inner| inner.held.button(button, state));
                InjectedEvent::Button { button, state }
            }
            Request::Axis {
                axis,
                value,
                value120,
            } => {
                let WEnum::Value(axis) = axis else {
                    return injector.post_error(Error::InvalidValue, "unknown axis");
                };
                InjectedEvent::Axis {
                    axis,
                    value,
                    value120,
                }
            }
            Request::Key { key, state } => {
                let WEnum::Value(state) = state else {
                    return injector.post_error(Error::InvalidValue, "unknown key state");
                };
                data.with(|inner| inner.held.key(key, state));
                InjectedEvent::Key { key, state }
            }
            Request::TouchDown { id, output, x, y } => {
                if let Err(error) = data.with(|inner| inner.held.touch_down(id, &output)) {
                    return injector.post_error(Error::InvalidTouchId, error.to_string());
                }
                InjectedEvent::TouchDown {
                    id,
                    output,
                    position: Point::from((x, y)),
                }
            }
            Request::TouchMotion { id, x, y } => {
                match data.with(|inner| inner.held.touch_output(id)) {
                    Ok(output) => InjectedEvent::TouchMotion {
                        id,
                        output,
                        position: Point::from((x, y)),
                    },
                    Err(error) => {
                        return injector.post_error(Error::InvalidTouchId, error.to_string());
                    }
                }
            }
            Request::TouchUp { id } => {
                if let Err(error) = data.with(|inner| inner.held.touch_up(id)) {
                    return injector.post_error(Error::InvalidTouchId, error.to_string());
                }
                InjectedEvent::TouchUp { id }
            }
            Request::TouchCancel => {
                data.with(|inner| inner.held.touch_cancel());
                InjectedEvent::TouchCancel
            }
            Request::Frame {
                time_usec_hi,
                time_usec_lo,
            } => {
                let events = data.with(|inner| std::mem::take(&mut inner.pending));
                if !events.is_empty() {
                    state.inject_input(InjectedFrame {
                        events,
                        time_usec: (u64::from(time_usec_hi) << 32) | u64::from(time_usec_lo),
                    });
                }
                return;
            }
            Request::Destroy => return,
            _ => return,
        };

        data.with(|inner| inner.pending.push(event));
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        _injector: &CrownosInputInjectorV1,
        data: &InjectorData,
    ) {
        let events = data.with(|inner| inner.held.let_go());
        if events.is_empty() {
            return;
        }
        let now: std::time::Duration = Clock::<Monotonic>::new().now().into();
        state.inject_input(InjectedFrame {
            events,
            time_usec: u64::try_from(now.as_micros()).unwrap_or(u64::MAX),
        });
    }
}

impl<D: InputDispatch> Dispatch<CrownosInputCaptureV1, CaptureData, D> for InputState {
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &CrownosInputCaptureV1,
        request: crownos_input_capture_v1::Request,
        _data: &CaptureData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        let capture = InputCapture(resource.clone());
        match request {
            crownos_input_capture_v1::Request::Arm { edges } => match edges {
                WEnum::Value(edges) => state.arm_input_capture(&capture, edges),
                WEnum::Unknown(raw) => resource.post_error(
                    crownos_input_capture_v1::Error::InvalidEdges,
                    format!("{raw:#x} has bits outside the edge enum"),
                ),
            },
            crownos_input_capture_v1::Request::Release { output, x, y } => {
                state.release_input_capture(&capture, &output, Point::from((x, y)));
            }
            crownos_input_capture_v1::Request::Destroy => {}
            _ => {}
        }
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        resource: &CrownosInputCaptureV1,
        _data: &CaptureData,
    ) {
        state.input_capture_destroyed(&InputCapture(resource.clone()));
    }
}
