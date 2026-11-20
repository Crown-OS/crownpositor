//! Request routing for `crownos_output_info_v1`.

use crownos_protocols::output_info::v1::server::{
    crownos_output_info_manager_v1::{self, CrownosOutputInfoManagerV1},
    crownos_output_info_v1::CrownosOutputInfoV1,
};
use smithay::output::Output;
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, backend::ClientId,
};

use super::{OutputInfoGlobalData, OutputInfoHandler, OutputInfoState, OutputWatch};

/// The bounds every impl in this module shares, named once.
pub trait OutputInfoDispatch:
    GlobalDispatch<CrownosOutputInfoManagerV1, OutputInfoGlobalData>
    + Dispatch<CrownosOutputInfoManagerV1, ()>
    + Dispatch<CrownosOutputInfoV1, ()>
    + OutputInfoHandler
    + 'static
{
}

impl<D> OutputInfoDispatch for D where
    D: GlobalDispatch<CrownosOutputInfoManagerV1, OutputInfoGlobalData>
        + Dispatch<CrownosOutputInfoManagerV1, ()>
        + Dispatch<CrownosOutputInfoV1, ()>
        + OutputInfoHandler
        + 'static
{
}

impl<D: OutputInfoDispatch> GlobalDispatch<CrownosOutputInfoManagerV1, OutputInfoGlobalData, D>
    for OutputInfoState
{
    fn bind(
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<CrownosOutputInfoManagerV1>,
        _global_data: &OutputInfoGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, ());
    }

    fn can_view(client: Client, global_data: &OutputInfoGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D: OutputInfoDispatch> Dispatch<CrownosOutputInfoManagerV1, (), D> for OutputInfoState {
    fn request(
        state: &mut D,
        _client: &Client,
        _manager: &CrownosOutputInfoManagerV1,
        request: crownos_output_info_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        let crownos_output_info_manager_v1::Request::GetOutput { id, output } = request else {
            return;
        };
        let object = data_init.init(id, ());
        let info = Output::from_resource(&output)
            .and_then(|output| Some((state.output_info(&output)?, output)));
        let watch = match info {
            Some((info, output)) => {
                info.send(&object, None);
                OutputWatch {
                    object,
                    output: Some(output),
                    sent: Some(info),
                }
            }
            None => OutputWatch {
                object,
                output: None,
                sent: None,
            },
        };
        state.output_info_state().watches.push(watch);
    }
}

impl<D: OutputInfoDispatch> Dispatch<CrownosOutputInfoV1, (), D> for OutputInfoState {
    fn request(
        _state: &mut D,
        _client: &Client,
        _object: &CrownosOutputInfoV1,
        _request: <CrownosOutputInfoV1 as Resource>::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
    }

    fn destroyed(state: &mut D, _client: ClientId, resource: &CrownosOutputInfoV1, _data: &()) {
        state
            .output_info_state()
            .watches
            .retain(|watch| watch.object.id() != resource.id());
    }
}
