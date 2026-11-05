//! Request routing for `crownos_android_shell_v1`.

use std::time::Instant;

use crownos_protocols::android_shell::v1::server::{
    crownos_android_shell_manager_v1::{self, CrownosAndroidShellManagerV1},
    crownos_android_toplevel_v1::{self, CrownosAndroidToplevelV1},
};
use smithay::wayland::{
    compositor::{add_pre_commit_hook, remove_pre_commit_hook},
    shell::xdg::XdgShellHandler,
};
use wayland_protocols::xdg::shell::server::xdg_toplevel::XdgToplevel;
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
    backend::ClientId, protocol::wl_surface::WlSurface,
};

use super::{
    AndroidPeer, AndroidShellGlobalData, AndroidShellHandler, AndroidShellState, AndroidTask,
    AspectRatio, Extension, TitleColor, details::BufferedDetails,
};

/// The bounds every impl in this module shares, named once.
pub trait AndroidShellDispatch:
    GlobalDispatch<CrownosAndroidShellManagerV1, AndroidShellGlobalData>
    + Dispatch<CrownosAndroidShellManagerV1, Option<AndroidPeer>>
    + Dispatch<CrownosAndroidToplevelV1, ()>
    + AndroidShellHandler
    + XdgShellHandler
    + 'static
{
}

impl<D> AndroidShellDispatch for D where
    D: GlobalDispatch<CrownosAndroidShellManagerV1, AndroidShellGlobalData>
        + Dispatch<CrownosAndroidShellManagerV1, Option<AndroidPeer>>
        + Dispatch<CrownosAndroidToplevelV1, ()>
        + AndroidShellHandler
        + XdgShellHandler
        + 'static
{
}

impl<D: AndroidShellDispatch>
    GlobalDispatch<CrownosAndroidShellManagerV1, AndroidShellGlobalData, D> for AndroidShellState
{
    /// The manager remembers which part of the runtime bound it, which is
    /// what `mark_placeholder` is checked against.
    fn bind(
        _state: &mut D,
        _handle: &DisplayHandle,
        client: &Client,
        resource: New<CrownosAndroidShellManagerV1>,
        global_data: &AndroidShellGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, (global_data.peer_of)(client));
    }

    fn can_view(client: Client, global_data: &AndroidShellGlobalData) -> bool {
        (global_data.peer_of)(&client).is_some()
    }
}

impl<D: AndroidShellDispatch> Dispatch<CrownosAndroidShellManagerV1, Option<AndroidPeer>, D>
    for AndroidShellState
{
    fn request(
        state: &mut D,
        _client: &Client,
        manager: &CrownosAndroidShellManagerV1,
        request: crownos_android_shell_manager_v1::Request,
        peer: &Option<AndroidPeer>,
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            crownos_android_shell_manager_v1::Request::GetAndroidToplevel { id, toplevel } => {
                // Initialised before any error: wayland-rs panics on a new_id
                // that leaves the request handler uninitialised.
                let resource = data_init.init(id, ());
                extend(state, manager, resource, toplevel);
            }
            crownos_android_shell_manager_v1::Request::MarkPlaceholder { toplevel, app_id } => {
                if *peer != Some(AndroidPeer::Daemon) {
                    manager.post_error(
                        crownos_android_shell_manager_v1::Error::NotAllowed,
                        "only the Android runtime daemon may mark placeholders",
                    );
                    return;
                }
                state
                    .android_shell_state()
                    .placeholders
                    .mark(app_id, toplevel, Instant::now());
            }
            crownos_android_shell_manager_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

/// Gives `toplevel` its extension, unless it has one already.
fn extend<D: AndroidShellDispatch>(
    state: &mut D,
    manager: &CrownosAndroidShellManagerV1,
    resource: CrownosAndroidToplevelV1,
    toplevel: XdgToplevel,
) {
    if state.android_shell_state().details(&toplevel).is_some() {
        manager.post_error(
            crownos_android_shell_manager_v1::Error::AlreadyExtended,
            "the xdg_toplevel already has a crownos_android_toplevel_v1",
        );
        return;
    }
    // A toplevel whose role is already gone leaves the extension inert.
    let Some(surface) = state
        .xdg_shell_state()
        .get_toplevel(&toplevel)
        .map(|toplevel| toplevel.wl_surface().clone())
    else {
        return;
    };
    let hook = add_pre_commit_hook::<D, _>(&surface, apply_committed::<D>);
    state.android_shell_state().extensions.push(Extension {
        resource,
        toplevel,
        surface,
        hook,
        details: BufferedDetails::default(),
    });
}

fn apply_committed<D: AndroidShellDispatch>(
    state: &mut D,
    _dhandle: &DisplayHandle,
    surface: &WlSurface,
) {
    if let Some((toplevel, details)) = state.android_shell_state().commit(surface) {
        state.android_details_changed(&toplevel, Some(&details));
    }
}

impl<D: AndroidShellDispatch> Dispatch<CrownosAndroidToplevelV1, (), D> for AndroidShellState {
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &CrownosAndroidToplevelV1,
        request: crownos_android_toplevel_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        let Some(extension) = state.android_shell_state().extension_mut(resource) else {
            return;
        };
        let details = &mut extension.details;
        match request {
            crownos_android_toplevel_v1::Request::SetTask { task_id, package } => {
                details.pending().task = Some(AndroidTask {
                    id: task_id,
                    package,
                });
            }
            crownos_android_toplevel_v1::Request::SetResizeLock {
                lock: WEnum::Value(lock),
            } => details.pending().resize_lock = lock,
            crownos_android_toplevel_v1::Request::SetAspect { width, height } => {
                details.pending().aspect = AspectRatio::new(width, height);
            }
            crownos_android_toplevel_v1::Request::SetTitleColor { argb } => {
                details.pending().title_color = TitleColor::new(argb);
            }
            _ => {}
        }
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        resource: &CrownosAndroidToplevelV1,
        _data: &(),
    ) {
        let Some(extension) = state.android_shell_state().remove(resource) else {
            return;
        };
        if extension.surface.is_alive() {
            remove_pre_commit_hook(&extension.surface, &extension.hook);
        }
        if extension.toplevel.is_alive() {
            state.android_details_changed(&extension.toplevel, None);
        }
    }
}
