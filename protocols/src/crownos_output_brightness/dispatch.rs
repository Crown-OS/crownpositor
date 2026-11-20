//! Request routing for `crownos_output_brightness_v1`.

use crownos_protocols::output_brightness::v1::server::{
    crownos_output_brightness_manager_v1::{self, CrownosOutputBrightnessManagerV1},
    crownos_output_brightness_v1::{self, CrownosOutputBrightnessV1},
};
use smithay::output::Output;
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, backend::ClientId,
};

use super::{
    BrightnessWatch, OutputBrightnessGlobalData, OutputBrightnessHandler, OutputBrightnessState,
    checked_level,
};

/// The bounds every impl in this module shares, named once.
pub trait OutputBrightnessDispatch:
    GlobalDispatch<CrownosOutputBrightnessManagerV1, OutputBrightnessGlobalData>
    + Dispatch<CrownosOutputBrightnessManagerV1, ()>
    + Dispatch<CrownosOutputBrightnessV1, ()>
    + OutputBrightnessHandler
    + 'static
{
}

impl<D> OutputBrightnessDispatch for D where
    D: GlobalDispatch<CrownosOutputBrightnessManagerV1, OutputBrightnessGlobalData>
        + Dispatch<CrownosOutputBrightnessManagerV1, ()>
        + Dispatch<CrownosOutputBrightnessV1, ()>
        + OutputBrightnessHandler
        + 'static
{
}

impl<D: OutputBrightnessDispatch>
    GlobalDispatch<CrownosOutputBrightnessManagerV1, OutputBrightnessGlobalData, D>
    for OutputBrightnessState
{
    fn bind(
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<CrownosOutputBrightnessManagerV1>,
        _global_data: &OutputBrightnessGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, ());
    }

    fn can_view(client: Client, global_data: &OutputBrightnessGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D: OutputBrightnessDispatch> Dispatch<CrownosOutputBrightnessManagerV1, (), D>
    for OutputBrightnessState
{
    fn request(
        state: &mut D,
        _client: &Client,
        _manager: &CrownosOutputBrightnessManagerV1,
        request: crownos_output_brightness_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        let crownos_output_brightness_manager_v1::Request::GetOutput { id, output } = request
        else {
            return;
        };
        let object = data_init.init(id, ());
        let output = Output::from_resource(&output);
        let current = output
            .as_ref()
            .and_then(|output| state.output_brightness(output));
        let mut watch = BrightnessWatch {
            object,
            output: current.and(output),
            sent: None,
        };
        if let Some(current) = current {
            watch.tell(current, false);
        }
        state.output_brightness_state().watches.push(watch);
    }
}

impl<D: OutputBrightnessDispatch> Dispatch<CrownosOutputBrightnessV1, (), D>
    for OutputBrightnessState
{
    fn request(
        state: &mut D,
        _client: &Client,
        object: &CrownosOutputBrightnessV1,
        request: crownos_output_brightness_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            crownos_output_brightness_v1::Request::SetLevel { level } => {
                let level = match checked_level(level) {
                    Ok(level) => level,
                    Err(error) => {
                        object.post_error(
                            crownos_output_brightness_v1::Error::InvalidLevel,
                            error.to_string(),
                        );
                        return;
                    }
                };
                let Some(output) = state.output_brightness_state().watched_output(object) else {
                    return;
                };
                if let Some(current) = state.set_output_brightness(&output, level) {
                    state
                        .output_brightness_state()
                        .answer(object, &output, current);
                }
            }
            crownos_output_brightness_v1::Request::Refresh => {
                if let Some(output) = state.output_brightness_state().watched_output(object) {
                    state.refresh_output_brightness(&output);
                }
            }
            _ => {}
        }
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        resource: &CrownosOutputBrightnessV1,
        _data: &(),
    ) {
        state
            .output_brightness_state()
            .watches
            .retain(|watch| watch.object.id() != resource.id());
    }
}
