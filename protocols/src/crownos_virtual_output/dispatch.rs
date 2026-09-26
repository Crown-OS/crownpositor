//! Request routing for `crownos_virtual_output_v1`.

use crownos_protocols::virtual_output::v1::server::{
    crownos_virtual_output_manager_v1::{self, CrownosVirtualOutputManagerV1},
    crownos_virtual_output_v1::{self, CrownosVirtualOutputV1},
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, backend::ClientId,
};

use super::{
    Lifecycle, VirtualOutput, VirtualOutputData, VirtualOutputGlobalData, VirtualOutputHandler,
    VirtualOutputState,
    mode::{VirtualMode, is_valid_name},
};

/// The bounds every impl in this module shares, named once.
pub trait VirtualOutputDispatch:
    GlobalDispatch<CrownosVirtualOutputManagerV1, VirtualOutputGlobalData>
    + Dispatch<CrownosVirtualOutputManagerV1, ()>
    + Dispatch<CrownosVirtualOutputV1, VirtualOutputData>
    + VirtualOutputHandler
    + 'static
{
}

impl<D> VirtualOutputDispatch for D where
    D: GlobalDispatch<CrownosVirtualOutputManagerV1, VirtualOutputGlobalData>
        + Dispatch<CrownosVirtualOutputManagerV1, ()>
        + Dispatch<CrownosVirtualOutputV1, VirtualOutputData>
        + VirtualOutputHandler
        + 'static
{
}

impl<D: VirtualOutputDispatch>
    GlobalDispatch<CrownosVirtualOutputManagerV1, VirtualOutputGlobalData, D>
    for VirtualOutputState
{
    fn bind(
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<CrownosVirtualOutputManagerV1>,
        _global_data: &VirtualOutputGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, ());
    }

    fn can_view(client: Client, global_data: &VirtualOutputGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D: VirtualOutputDispatch> Dispatch<CrownosVirtualOutputManagerV1, (), D>
    for VirtualOutputState
{
    fn request(
        state: &mut D,
        _client: &Client,
        manager: &CrownosVirtualOutputManagerV1,
        request: crownos_virtual_output_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        use crownos_virtual_output_manager_v1::Error;

        match request {
            crownos_virtual_output_manager_v1::Request::CreateOutput {
                output,
                name,
                width,
                height,
                refresh_mhz,
                scale_120,
            } => {
                let output = VirtualOutput(data_init.init(output, VirtualOutputData::new()));
                if !is_valid_name(&name) {
                    return manager.post_error(
                        Error::InvalidName,
                        "the name must be 1 to 32 characters of [A-Za-z0-9_-]",
                    );
                }
                match VirtualMode::new(width, height, refresh_mhz, scale_120) {
                    Ok(mode) => state.create_virtual_output(output, &name, mode),
                    Err(error) => manager.post_error(Error::InvalidMode, error.to_string()),
                }
            }
            crownos_virtual_output_manager_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

impl<D: VirtualOutputDispatch> Dispatch<CrownosVirtualOutputV1, VirtualOutputData, D>
    for VirtualOutputState
{
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &CrownosVirtualOutputV1,
        request: crownos_virtual_output_v1::Request,
        data: &VirtualOutputData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        if data.lifecycle() != Lifecycle::Live {
            return;
        }
        let output = VirtualOutput(resource.clone());

        match request {
            crownos_virtual_output_v1::Request::SetMode {
                width,
                height,
                refresh_mhz,
                scale_120,
            } => match VirtualMode::new(width, height, refresh_mhz, scale_120) {
                Ok(mode) => state.set_virtual_output_mode(&output, mode),
                Err(error) => resource.post_error(
                    crownos_virtual_output_v1::Error::InvalidMode,
                    error.to_string(),
                ),
            },
            crownos_virtual_output_v1::Request::RequestFrame => {
                state.request_virtual_output_frame(&output);
            }
            crownos_virtual_output_v1::Request::Destroy => {}
            _ => {}
        }
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        resource: &CrownosVirtualOutputV1,
        data: &VirtualOutputData,
    ) {
        if data.advance(&[Lifecycle::Pending, Lifecycle::Live], Lifecycle::Closed) {
            state.destroy_virtual_output(&VirtualOutput(resource.clone()));
        }
    }
}
