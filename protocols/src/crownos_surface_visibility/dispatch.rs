//! Request routing for `crownos_surface_visibility_v1`.

use crownos_protocols::surface_visibility::v1::server::{
    crownos_surface_visibility_manager_v1::{self, CrownosSurfaceVisibilityManagerV1},
    crownos_surface_visibility_v1::{self, CrownosSurfaceVisibilityV1},
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, backend::ClientId,
};

use super::{SurfaceVisibilityHandler, SurfaceVisibilityState};

/// The bounds every impl in this module shares, named once.
pub trait SurfaceVisibilityDispatch:
    GlobalDispatch<CrownosSurfaceVisibilityManagerV1, ()>
    + Dispatch<CrownosSurfaceVisibilityManagerV1, ()>
    + Dispatch<CrownosSurfaceVisibilityV1, ()>
    + SurfaceVisibilityHandler
    + 'static
{
}

impl<D> SurfaceVisibilityDispatch for D where
    D: GlobalDispatch<CrownosSurfaceVisibilityManagerV1, ()>
        + Dispatch<CrownosSurfaceVisibilityManagerV1, ()>
        + Dispatch<CrownosSurfaceVisibilityV1, ()>
        + SurfaceVisibilityHandler
        + 'static
{
}

impl<D: SurfaceVisibilityDispatch> GlobalDispatch<CrownosSurfaceVisibilityManagerV1, (), D>
    for SurfaceVisibilityState
{
    fn bind(
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<CrownosSurfaceVisibilityManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, ());
    }
}

impl<D: SurfaceVisibilityDispatch> Dispatch<CrownosSurfaceVisibilityManagerV1, (), D>
    for SurfaceVisibilityState
{
    fn request(
        state: &mut D,
        _client: &Client,
        _manager: &CrownosSurfaceVisibilityManagerV1,
        request: crownos_surface_visibility_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        if let crownos_surface_visibility_manager_v1::Request::GetSurfaceVisibility {
            id,
            surface,
        } = request
        {
            let resource = data_init.init(id, ());
            state.surface_visibility_state().observe(resource, surface);
        }
    }
}

impl<D: SurfaceVisibilityDispatch> Dispatch<CrownosSurfaceVisibilityV1, (), D>
    for SurfaceVisibilityState
{
    fn request(
        _state: &mut D,
        _client: &Client,
        _resource: &CrownosSurfaceVisibilityV1,
        _request: crownos_surface_visibility_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        resource: &CrownosSurfaceVisibilityV1,
        _data: &(),
    ) {
        state.surface_visibility_state().forget(resource);
    }
}
