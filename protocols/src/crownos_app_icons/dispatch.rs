//! Request routing for `crownos_app_icons_v1`.

use crownos_protocols::app_icons::v1::server::{
    crownos_app_icon_manager_v1::{self, CrownosAppIconManagerV1},
    crownos_app_icon_v1::{self, CrownosAppIconV1},
};
use smithay::utils::Rectangle;
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, backend::ClientId,
};

use super::{AppIconData, AppIconsGlobalData, AppIconsHandler, AppIconsState, IconPlacement};

/// The bounds every impl in this module shares, named once.
pub trait AppIconsDispatch:
    GlobalDispatch<CrownosAppIconManagerV1, AppIconsGlobalData>
    + Dispatch<CrownosAppIconManagerV1, ()>
    + Dispatch<CrownosAppIconV1, AppIconData>
    + AppIconsHandler
    + 'static
{
}

impl<D> AppIconsDispatch for D where
    D: GlobalDispatch<CrownosAppIconManagerV1, AppIconsGlobalData>
        + Dispatch<CrownosAppIconManagerV1, ()>
        + Dispatch<CrownosAppIconV1, AppIconData>
        + AppIconsHandler
        + 'static
{
}

impl<D: AppIconsDispatch> GlobalDispatch<CrownosAppIconManagerV1, AppIconsGlobalData, D>
    for AppIconsState
{
    fn bind(
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<CrownosAppIconManagerV1>,
        _global_data: &AppIconsGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, ());
    }

    fn can_view(client: Client, global_data: &AppIconsGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D: AppIconsDispatch> Dispatch<CrownosAppIconManagerV1, (), D> for AppIconsState {
    fn request(
        state: &mut D,
        _client: &Client,
        _manager: &CrownosAppIconManagerV1,
        request: crownos_app_icon_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        let crownos_app_icon_manager_v1::Request::GetAppIcon {
            id,
            surface,
            app_id,
        } = request
        else {
            return;
        };
        let icon = data_init.init(
            id,
            AppIconData {
                surface: surface.downgrade(),
                app_id,
            },
        );
        state.app_icons_state().insert(icon);
    }
}

impl<D: AppIconsDispatch> Dispatch<CrownosAppIconV1, AppIconData, D> for AppIconsState {
    fn request(
        state: &mut D,
        _client: &Client,
        icon: &CrownosAppIconV1,
        request: crownos_app_icon_v1::Request,
        data: &AppIconData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            crownos_app_icon_v1::Request::SetRectangle {
                x,
                y,
                width,
                height,
                corner_radius,
            } => {
                if width <= 0 || height <= 0 || corner_radius < 0.0 {
                    icon.post_error(
                        crownos_app_icon_v1::Error::InvalidRectangle,
                        "an icon needs a positive size and a non-negative corner radius",
                    );
                    return;
                }
                let placement = IconPlacement {
                    rect: Rectangle::new((x, y).into(), (width, height).into()),
                    corner_radius,
                };
                state.app_icons_state().place(icon, placement);
            }
            crownos_app_icon_v1::Request::Activate => {
                if !state.activate_app(&data.app_id) {
                    state.app_icons_state().launches.arm(data.app_id.clone());
                    icon.launch();
                }
            }
            crownos_app_icon_v1::Request::Destroy => {}
            _ => {}
        }
    }

    fn destroyed(state: &mut D, _client: ClientId, icon: &CrownosAppIconV1, _data: &AppIconData) {
        state.app_icons_state().remove(icon);
    }
}
