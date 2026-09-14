//! The `Dispatch` impls, one per interface.
//!
//! Request handling is deliberately thin: validation that needs to know about
//! hardware belongs to the compositor, so everything here is either protocol
//! bookkeeping or one of the errors the XML actually names.

use smithay::utils::Transform;
use wayland_protocols_wlr::output_management::v1::server::{
    zwlr_output_configuration_head_v1::{self, ZwlrOutputConfigurationHeadV1},
    zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1::{self, AdaptiveSyncState, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
};

use super::{
    HeadConfig, HeadId, ModeRequest, OutputConfigRequest, OutputManagementGlobalData,
    OutputManagementHandler, OutputManagementState, VERSION,
    config::{self, PendingConfiguration, PendingConfigurationInner, PendingHeadConfiguration,
             PendingHeadConfigurationInner},
    head::{self, HeadData, ManagerInstance, ModeData},
};

/// The bounds every impl in this module shares, named once.
pub trait ManagementDispatch:
    GlobalDispatch<ZwlrOutputManagerV1, OutputManagementGlobalData>
    + Dispatch<ZwlrOutputManagerV1, ()>
    + Dispatch<ZwlrOutputHeadV1, HeadData>
    + Dispatch<ZwlrOutputModeV1, ModeData>
    + Dispatch<ZwlrOutputConfigurationV1, PendingConfiguration>
    + Dispatch<ZwlrOutputConfigurationHeadV1, PendingHeadConfiguration>
    + OutputManagementHandler
    + 'static
{
}

impl<D> ManagementDispatch for D where
    D: GlobalDispatch<ZwlrOutputManagerV1, OutputManagementGlobalData>
        + Dispatch<ZwlrOutputManagerV1, ()>
        + Dispatch<ZwlrOutputHeadV1, HeadData>
        + Dispatch<ZwlrOutputModeV1, ModeData>
        + Dispatch<ZwlrOutputConfigurationV1, PendingConfiguration>
        + Dispatch<ZwlrOutputConfigurationHeadV1, PendingHeadConfiguration>
        + OutputManagementHandler
        + 'static
{
}

// --- MARK: manager ---

impl<D: ManagementDispatch> GlobalDispatch<ZwlrOutputManagerV1, OutputManagementGlobalData, D>
    for OutputManagementState
{
    fn bind(
        state: &mut D,
        dh: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrOutputManagerV1>,
        _global_data: &OutputManagementGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        let obj = data_init.init(resource, ());
        let this = state.output_management_state();

        let mut instance = ManagerInstance {
            obj,
            heads: Vec::new(),
        };
        head::send_all::<D>(dh, &mut instance, &this.heads);
        instance.obj.done(this.serial);

        this.instances.push(instance);
    }

    fn can_view(client: Client, global_data: &OutputManagementGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D: ManagementDispatch> Dispatch<ZwlrOutputManagerV1, (), D> for OutputManagementState {
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &ZwlrOutputManagerV1,
        request: zwlr_output_manager_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            zwlr_output_manager_v1::Request::CreateConfiguration { id, serial } => {
                let current = state.output_management_state().serial;
                let obj = data_init.init(id, PendingConfiguration::new(
                    PendingConfigurationInner::new(serial),
                ));
                // Still initialised, because the client will destroy it and a
                // dead object would make that a protocol error.
                if serial != current {
                    obj.cancelled();
                }
            }
            zwlr_output_manager_v1::Request::Stop => {
                resource.finished();
                drop_instance(state, resource);
            }
            _ => {}
        }
    }

    fn destroyed(
        state: &mut D,
        _client: wayland_server::backend::ClientId,
        resource: &ZwlrOutputManagerV1,
        _data: &(),
    ) {
        drop_instance(state, resource);
    }
}

fn drop_instance<D: ManagementDispatch>(state: &mut D, resource: &ZwlrOutputManagerV1) {
    state
        .output_management_state()
        .instances
        .retain(|instance| instance.obj != *resource);
}

// --- MARK: head and mode ---

impl<D: ManagementDispatch> Dispatch<ZwlrOutputHeadV1, HeadData, D> for OutputManagementState {
    fn request(
        _state: &mut D,
        _client: &Client,
        _resource: &ZwlrOutputHeadV1,
        request: zwlr_output_head_v1::Request,
        _data: &HeadData,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        // `release` is the only request, and it is a destructor.
        let zwlr_output_head_v1::Request::Release = request else {
            return;
        };
    }

    fn destroyed(
        state: &mut D,
        _client: wayland_server::backend::ClientId,
        resource: &ZwlrOutputHeadV1,
        _data: &HeadData,
    ) {
        for instance in &mut state.output_management_state().instances {
            instance.heads.retain(|head| head.obj != *resource);
        }
    }
}

impl<D: ManagementDispatch> Dispatch<ZwlrOutputModeV1, ModeData, D> for OutputManagementState {
    fn request(
        _state: &mut D,
        _client: &Client,
        _resource: &ZwlrOutputModeV1,
        request: zwlr_output_mode_v1::Request,
        _data: &ModeData,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        let zwlr_output_mode_v1::Request::Release = request else {
            return;
        };
    }

    fn destroyed(
        state: &mut D,
        _client: wayland_server::backend::ClientId,
        resource: &ZwlrOutputModeV1,
        _data: &ModeData,
    ) {
        for instance in &mut state.output_management_state().instances {
            for head in &mut instance.heads {
                head.modes.retain(|mode| mode != resource);
            }
        }
    }
}

// --- MARK: configuration ---

impl<D: ManagementDispatch> Dispatch<ZwlrOutputConfigurationV1, PendingConfiguration, D>
    for OutputManagementState
{
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &ZwlrOutputConfigurationV1,
        request: zwlr_output_configuration_v1::Request,
        data: &PendingConfiguration,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            zwlr_output_configuration_v1::Request::EnableHead { id, head } => {
                let Some(head_id) = head_id(&head) else {
                    return;
                };
                let mut pending = config::lock(data);
                if refuse_used(resource, &pending) {
                    return;
                }
                if pending.is_configured(&head_id) {
                    resource.post_error(
                        zwlr_output_configuration_v1::Error::AlreadyConfiguredHead,
                        format!("head {head_id} was already configured"),
                    );
                    return;
                }

                let obj = data_init.init(
                    id,
                    PendingHeadConfiguration::new(PendingHeadConfigurationInner::new(
                        head_id.clone(),
                    )),
                );
                pending.heads.push((head_id, Some(obj)));
            }

            zwlr_output_configuration_v1::Request::DisableHead { head } => {
                let Some(head_id) = head_id(&head) else {
                    return;
                };
                let mut pending = config::lock(data);
                if refuse_used(resource, &pending) {
                    return;
                }
                if pending.is_configured(&head_id) {
                    resource.post_error(
                        zwlr_output_configuration_v1::Error::AlreadyConfiguredHead,
                        format!("head {head_id} was already configured"),
                    );
                    return;
                }
                pending.heads.push((head_id, None));
            }

            zwlr_output_configuration_v1::Request::Apply => {
                resolve(state, resource, data, Intent::Apply);
            }
            zwlr_output_configuration_v1::Request::Test => {
                resolve(state, resource, data, Intent::Test);
            }
            _ => {}
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Intent {
    Apply,
    Test,
}

/// Marks the configuration used and refuses a second attempt.
fn refuse_used(resource: &ZwlrOutputConfigurationV1, pending: &PendingConfigurationInner) -> bool {
    if pending.used {
        resource.post_error(
            zwlr_output_configuration_v1::Error::AlreadyUsed,
            "this configuration has already been applied or tested",
        );
        return true;
    }
    false
}

/// Turns the pending state into an [`OutputConfigRequest`] and hands it over.
///
/// The three cancellation paths below are races rather than client mistakes —
/// a monitor came or went between the client reading the head list and sending
/// `apply` — so they end the configuration politely instead of killing the
/// client, which is what wlroots clients expect and what makes hotplug during
/// a settings-panel drag survivable.
fn resolve<D: ManagementDispatch>(
    state: &mut D,
    resource: &ZwlrOutputConfigurationV1,
    data: &PendingConfiguration,
    intent: Intent,
) {
    let (serial, heads) = {
        let mut pending = config::lock(data);
        if refuse_used(resource, &pending) {
            return;
        }
        pending.used = true;
        (pending.serial, std::mem::take(&mut pending.heads))
    };

    let this = state.output_management_state();
    if serial != this.serial {
        resource.cancelled();
        return;
    }
    if heads.len() != this.heads.len() {
        resource.cancelled();
        return;
    }

    let mut request = OutputConfigRequest {
        heads: Vec::with_capacity(heads.len()),
    };
    for (id, head) in heads {
        let Some(snapshot) = this.snapshot(&id) else {
            resource.cancelled();
            return;
        };

        let config = match head {
            None => HeadConfig::Disabled,
            Some(obj) => {
                let Some(head_data) = obj.data::<PendingHeadConfiguration>() else {
                    resource.cancelled();
                    return;
                };
                let pending = config::lock(head_data);
                // A mode the client picked may have been finished since, in
                // which case its index no longer means what it meant.
                if let Some(ModeRequest::Advertised(index)) = pending.mode
                    && snapshot.modes.get(index).is_none()
                {
                    resource.cancelled();
                    return;
                }
                pending.to_config()
            }
        };
        request.heads.push((id, config));
    }

    let accepted = match intent {
        Intent::Apply => state.apply_output_config(request),
        Intent::Test => state.test_output_config(request),
    };

    if accepted {
        resource.succeeded();
    } else {
        resource.failed();
    }
}

fn head_id(head: &ZwlrOutputHeadV1) -> Option<HeadId> {
    head.data::<HeadData>().map(|data| data.id.clone())
}

// --- MARK: configuration head ---

impl<D: ManagementDispatch> Dispatch<ZwlrOutputConfigurationHeadV1, PendingHeadConfiguration, D>
    for OutputManagementState
{
    fn request(
        _state: &mut D,
        _client: &Client,
        resource: &ZwlrOutputConfigurationHeadV1,
        request: zwlr_output_configuration_head_v1::Request,
        data: &PendingHeadConfiguration,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        let mut pending = config::lock(data);

        match request {
            zwlr_output_configuration_head_v1::Request::SetMode { mode } => {
                if set_once(resource, pending.mode.is_some()) {
                    return;
                }
                let Some(mode_data) = mode.data::<ModeData>() else {
                    return;
                };
                if mode_data.head != pending.head {
                    resource.post_error(
                        zwlr_output_configuration_head_v1::Error::InvalidMode,
                        "mode does not belong to this head",
                    );
                    return;
                }
                pending.mode = Some(ModeRequest::Advertised(mode_data.index));
            }

            zwlr_output_configuration_head_v1::Request::SetCustomMode {
                width,
                height,
                refresh,
            } => {
                if set_once(resource, pending.mode.is_some()) {
                    return;
                }
                if width <= 0 || height <= 0 || refresh < 0 {
                    resource.post_error(
                        zwlr_output_configuration_head_v1::Error::InvalidCustomMode,
                        "a custom mode needs a positive size and a non-negative refresh",
                    );
                    return;
                }
                pending.mode = Some(ModeRequest::Custom {
                    size: (width, height).into(),
                    // Zero is the protocol's "you decide".
                    refresh: (refresh > 0).then_some(refresh),
                });
            }

            zwlr_output_configuration_head_v1::Request::SetPosition { x, y } => {
                if set_once(resource, pending.position.is_some()) {
                    return;
                }
                pending.position = Some((x, y).into());
            }

            zwlr_output_configuration_head_v1::Request::SetTransform { transform } => {
                if set_once(resource, pending.transform.is_some()) {
                    return;
                }
                let Some(transform) = transform_from_wl(transform) else {
                    resource.post_error(
                        zwlr_output_configuration_head_v1::Error::InvalidTransform,
                        "transform is outside the wl_output.transform enum",
                    );
                    return;
                };
                pending.transform = Some(transform);
            }

            zwlr_output_configuration_head_v1::Request::SetScale { scale } => {
                if set_once(resource, pending.scale.is_some()) {
                    return;
                }
                let scale = scale.into();
                if scale <= 0.0 {
                    resource.post_error(
                        zwlr_output_configuration_head_v1::Error::InvalidScale,
                        "scale must be greater than zero",
                    );
                    return;
                }
                pending.scale = Some(scale);
            }

            zwlr_output_configuration_head_v1::Request::SetAdaptiveSync { state } => {
                if set_once(resource, pending.adaptive_sync.is_some()) {
                    return;
                }
                let enabled = match state {
                    WEnum::Value(AdaptiveSyncState::Enabled) => true,
                    WEnum::Value(AdaptiveSyncState::Disabled) => false,
                    _ => {
                        resource.post_error(
                            zwlr_output_configuration_head_v1::Error::InvalidAdaptiveSyncState,
                            "adaptive sync state is outside the enum",
                        );
                        return;
                    }
                };
                pending.adaptive_sync = Some(enabled);
            }

            _ => {}
        }
    }
}

/// Every setter on a configuration head may be sent at most once.
fn set_once(resource: &ZwlrOutputConfigurationHeadV1, already: bool) -> bool {
    if already {
        resource.post_error(
            zwlr_output_configuration_head_v1::Error::AlreadySet,
            "this property has already been set on this head",
        );
    }
    already
}

/// smithay only converts the other way, and a client can send any integer.
fn transform_from_wl(transform: WEnum<wayland_server::protocol::wl_output::Transform>) -> Option<Transform> {
    use wayland_server::protocol::wl_output::Transform as Wl;

    Some(match transform.into_result().ok()? {
        Wl::Normal => Transform::Normal,
        Wl::_90 => Transform::_90,
        Wl::_180 => Transform::_180,
        Wl::_270 => Transform::_270,
        Wl::Flipped => Transform::Flipped,
        Wl::Flipped90 => Transform::Flipped90,
        Wl::Flipped180 => Transform::Flipped180,
        Wl::Flipped270 => Transform::Flipped270,
        _ => return None,
    })
}

/// Wires `$ty` up as the dispatch target for every interface in this protocol.
#[macro_export]
macro_rules! delegate_output_management {
    ($ty:ty) => {
        type __OutputManagementState = $crate::output_management::OutputManagementState;

        ::wayland_server::delegate_global_dispatch!($ty: [
            $crate::output_management::reexports::zwlr_output_manager_v1::ZwlrOutputManagerV1:
                $crate::output_management::OutputManagementGlobalData
        ] => __OutputManagementState);

        ::wayland_server::delegate_dispatch!($ty: [
            $crate::output_management::reexports::zwlr_output_manager_v1::ZwlrOutputManagerV1: ()
        ] => __OutputManagementState);

        ::wayland_server::delegate_dispatch!($ty: [
            $crate::output_management::reexports::zwlr_output_head_v1::ZwlrOutputHeadV1:
                $crate::output_management::HeadData
        ] => __OutputManagementState);

        ::wayland_server::delegate_dispatch!($ty: [
            $crate::output_management::reexports::zwlr_output_mode_v1::ZwlrOutputModeV1:
                $crate::output_management::ModeData
        ] => __OutputManagementState);

        ::wayland_server::delegate_dispatch!($ty: [
            $crate::output_management::reexports::zwlr_output_configuration_v1::ZwlrOutputConfigurationV1:
                $crate::output_management::PendingConfiguration
        ] => __OutputManagementState);

        ::wayland_server::delegate_dispatch!($ty: [
            $crate::output_management::reexports::zwlr_output_configuration_head_v1::ZwlrOutputConfigurationHeadV1:
                $crate::output_management::PendingHeadConfiguration
        ] => __OutputManagementState);
    };
}

const _: () = {
    // The advertised version has to match what `head.rs` gates events on.
    assert!(VERSION == 4);
};
