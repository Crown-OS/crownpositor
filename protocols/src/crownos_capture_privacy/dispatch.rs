//! Request routing for `crownos_capture_privacy_v1`.

use crownos_protocols::capture_privacy::v1::server::{
    crownos_capture_privacy_manager_v1::{self, CrownosCapturePrivacyManagerV1, Error},
    crownos_surface_privacy_v1::{self, CrownosSurfacePrivacyV1},
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, backend::ClientId,
};

use super::{CapturePrivacyHandler, CapturePrivacyState, PrivacyData};

/// The bounds every impl in this module shares, named once.
pub trait CapturePrivacyDispatch:
    GlobalDispatch<CrownosCapturePrivacyManagerV1, ()>
    + Dispatch<CrownosCapturePrivacyManagerV1, ()>
    + Dispatch<CrownosSurfacePrivacyV1, PrivacyData>
    + CapturePrivacyHandler
    + 'static
{
}

impl<D> CapturePrivacyDispatch for D where
    D: GlobalDispatch<CrownosCapturePrivacyManagerV1, ()>
        + Dispatch<CrownosCapturePrivacyManagerV1, ()>
        + Dispatch<CrownosSurfacePrivacyV1, PrivacyData>
        + CapturePrivacyHandler
        + 'static
{
}

impl<D: CapturePrivacyDispatch> GlobalDispatch<CrownosCapturePrivacyManagerV1, (), D>
    for CapturePrivacyState
{
    fn bind(
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<CrownosCapturePrivacyManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, ());
    }
}

impl<D: CapturePrivacyDispatch> Dispatch<CrownosCapturePrivacyManagerV1, (), D>
    for CapturePrivacyState
{
    fn request(
        state: &mut D,
        _client: &Client,
        manager: &CrownosCapturePrivacyManagerV1,
        request: crownos_capture_privacy_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        let crownos_capture_privacy_manager_v1::Request::GetSurfacePrivacy { id, surface } =
            request
        else {
            return;
        };
        if !state.capture_privacy_state().claim(&surface) {
            manager.post_error(
                Error::AlreadyExists,
                "the surface already has a privacy object",
            );
            return;
        }
        data_init.init(
            id,
            PrivacyData {
                surface: surface.downgrade(),
            },
        );
    }
}

impl<D: CapturePrivacyDispatch> Dispatch<CrownosSurfacePrivacyV1, PrivacyData, D>
    for CapturePrivacyState
{
    fn request(
        state: &mut D,
        _client: &Client,
        _resource: &CrownosSurfacePrivacyV1,
        request: crownos_surface_privacy_v1::Request,
        data: &PrivacyData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        let crownos_surface_privacy_v1::Request::SetExcluded { excluded } = request else {
            return;
        };
        let Ok(surface) = data.surface.upgrade() else {
            return;
        };
        if state
            .capture_privacy_state()
            .set_excluded(&surface, excluded != 0)
        {
            state.surface_privacy_changed(&surface);
        }
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        _resource: &CrownosSurfacePrivacyV1,
        data: &PrivacyData,
    ) {
        let Ok(surface) = data.surface.upgrade() else {
            return;
        };
        if state.capture_privacy_state().release(&surface) {
            state.surface_privacy_changed(&surface);
        }
    }
}
