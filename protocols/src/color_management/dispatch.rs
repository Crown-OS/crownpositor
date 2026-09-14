//! The `Dispatch` impls, one per interface.

use std::sync::{Arc, Mutex};

use smithay::wayland::compositor::with_states;
use wayland_protocols::wp::color_management::v1::server::{
    wp_color_management_output_v1::{self, WpColorManagementOutputV1},
    wp_color_management_surface_feedback_v1::{self, WpColorManagementSurfaceFeedbackV1},
    wp_color_management_surface_v1::{self, WpColorManagementSurfaceV1},
    wp_color_manager_v1::{self, WpColorManagerV1},
    wp_image_description_creator_icc_v1::{self, WpImageDescriptionCreatorIccV1},
    wp_image_description_creator_params_v1::{self, WpImageDescriptionCreatorParamsV1},
    wp_image_description_info_v1::WpImageDescriptionInfoV1,
    wp_image_description_v1::{self, WpImageDescriptionV1},
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
};

use super::{
    capabilities::{Capabilities, Feature, RenderIntent},
    creator::{self, IccCreator, IccFile, Params, ParamsData, IccCreatorData},
    info,
    math::Primaries,
    record::{
        DescriptionKind, IccData, ImageDescription, Luminances, ParametricDescription,
        PrimariesSpec, TransferFunction, chromaticity_from_wire,
    },
    state::{
        ColorDispatch, ColorManagementGlobalData, ColorManagementState,
        DescriptionData, DescriptionState, FeedbackData, InfoData, OutputData, SurfaceColor,
        SurfaceColorCachedState, SurfaceData_, VERSION,
    },
};

/// Locks a mutex, treating poisoning as usable: everything behind these locks
/// is plain values with no invariant a partial write could break.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}

// --- MARK: manager ---

impl<D: ColorDispatch> GlobalDispatch<WpColorManagerV1, ColorManagementGlobalData, D>
    for ColorManagementState
{
    fn bind(
        _state: &mut D,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<WpColorManagerV1>,
        _global_data: &ColorManagementGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        let manager = data_init.init(resource, ());
        let capabilities = Capabilities::for_version(manager.version());

        for intent in &capabilities.intents {
            manager.supported_intent(intent_wire(*intent));
        }
        for feature in &capabilities.features {
            manager.supported_feature(feature_wire(*feature));
        }
        for transfer in &capabilities.transfer_functions {
            manager.supported_tf_named(transfer_wire(*transfer));
        }
        for primaries in &capabilities.primaries {
            manager.supported_primaries_named(primaries_wire(*primaries));
        }
        manager.done();
    }

    fn can_view(client: Client, global_data: &ColorManagementGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D: ColorDispatch> Dispatch<WpColorManagerV1, (), D> for ColorManagementState {
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &WpColorManagerV1,
        request: wp_color_manager_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        let capabilities = Capabilities::for_version(resource.version());

        match request {
            wp_color_manager_v1::Request::GetOutput { id, output } => {
                let object = data_init.init(
                    id,
                    OutputData {
                        output: output.clone(),
                    },
                );
                state.color_management_state().watch_output(&output, object);
            }

            wp_color_manager_v1::Request::GetSurface { id, surface } => {
                // At most one colour-management object per surface: two would
                // fight over the same committed state.
                if has_surface_object(&surface) {
                    resource.post_error(
                        wp_color_manager_v1::Error::SurfaceExists,
                        "this surface already has a wp_color_management_surface_v1",
                    );
                    return;
                }
                mark_surface_object(&surface, true);
                data_init.init(id, SurfaceData_ { surface });
            }

            wp_color_manager_v1::Request::GetSurfaceFeedback { id, surface } => {
                let object = data_init.init(
                    id,
                    FeedbackData {
                        surface: surface.clone(),
                    },
                );
                state
                    .color_management_state()
                    .watch_feedback(&surface, object);
                ColorManagementState::refresh_preferred(state);
            }

            wp_color_manager_v1::Request::CreateIccCreator { obj } => {
                if !capabilities.supports_feature(Feature::IccV2V4) {
                    resource.post_error(
                        wp_color_manager_v1::Error::UnsupportedFeature,
                        "ICC profiles are not supported",
                    );
                    return;
                }
                data_init.init(obj, IccCreatorData::new(IccCreator::default()));
            }

            wp_color_manager_v1::Request::CreateParametricCreator { obj } => {
                if !capabilities.supports_feature(Feature::Parametric) {
                    resource.post_error(
                        wp_color_manager_v1::Error::UnsupportedFeature,
                        "parametric descriptions are not supported",
                    );
                    return;
                }
                data_init.init(obj, ParamsData::new(Params::default()));
            }

            wp_color_manager_v1::Request::CreateWindowsScrgb { image_description } => {
                if !capabilities.supports_feature(Feature::WindowsScrgb) {
                    resource.post_error(
                        wp_color_manager_v1::Error::UnsupportedFeature,
                        "windows-scrgb is not supported",
                    );
                    return;
                }
                finish(state, data_init, image_description, DescriptionKind::WindowsScrgb);
            }

            wp_color_manager_v1::Request::CreateWindowsBt2100 { image_description } => {
                if !capabilities.supports_feature(Feature::WindowsBt2100) {
                    resource.post_error(
                        wp_color_manager_v1::Error::UnsupportedFeature,
                        "windows-bt2100 is not supported",
                    );
                    return;
                }
                finish(state, data_init, image_description, DescriptionKind::WindowsBt2100);
            }

            _ => {}
        }
    }
}

/// Creates a `wp_image_description_v1` that is already `ready`.
fn finish<D: ColorDispatch>(
    state: &mut D,
    data_init: &mut DataInit<'_, D>,
    id: New<WpImageDescriptionV1>,
    kind: DescriptionKind,
) {
    let describable = kind.is_describable();
    let description = state.color_management_state().intern(kind);

    let object = data_init.init(
        id,
        DescriptionData {
            state: Mutex::new(DescriptionState::Pending),
            allows_info: describable,
        },
    );
    ready(&object, description);
}

/// Sends `ready` (or its version 2 successor) and records the description.
fn ready(object: &WpImageDescriptionV1, description: Arc<ImageDescription>) {
    let Some(data) = object.data::<DescriptionData>() else {
        return;
    };

    if description.min_version() > object.version() {
        *lock(&data.state) = DescriptionState::Failed;
        object.failed(
            wp_image_description_v1::Cause::LowVersion,
            "this image description needs a newer version of the interface".into(),
        );
        return;
    }

    if object.version() >= 2 {
        let (high, low) = description.id().halves();
        object.ready2(high, low);
    } else {
        object.ready(description.id().truncated());
    }
    *lock(&data.state) = DescriptionState::Ready(description);
}

fn fail(object: &WpImageDescriptionV1, cause: wp_image_description_v1::Cause, message: String) {
    if let Some(data) = object.data::<DescriptionData>() {
        *lock(&data.state) = DescriptionState::Failed;
    }
    object.failed(cause, message);
}

// --- MARK: output ---

impl<D: ColorDispatch> Dispatch<WpColorManagementOutputV1, OutputData, D>
    for ColorManagementState
{
    fn request(
        state: &mut D,
        _client: &Client,
        _resource: &WpColorManagementOutputV1,
        request: wp_color_management_output_v1::Request,
        data: &OutputData,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        let wp_color_management_output_v1::Request::GetImageDescription { image_description } =
            request
        else {
            return;
        };

        let description = state.output_image_description(&data.output);
        let object = data_init.init(
            image_description,
            DescriptionData {
                state: Mutex::new(DescriptionState::Pending),
                // Reached through an output, so the client is entitled to read
                // the colours back.
                allows_info: true,
            },
        );

        match description {
            Some(description) => ready(&object, description),
            // The output went away between the client binding it and asking.
            None => fail(
                &object,
                wp_image_description_v1::Cause::NoOutput,
                "the output is gone".into(),
            ),
        }
    }

    fn destroyed(
        state: &mut D,
        _client: wayland_server::backend::ClientId,
        resource: &WpColorManagementOutputV1,
        _data: &OutputData,
    ) {
        state.color_management_state().forget_output(resource);
    }
}

// --- MARK: surface ---

/// Whether a surface already has a colour-management object.
///
/// Kept in the surface's own data map rather than a list on the state, so it
/// cannot outlive the surface.
#[derive(Default)]
struct SurfaceObjectMarker(std::sync::atomic::AtomicBool);

fn has_surface_object(surface: &wayland_server::protocol::wl_surface::WlSurface) -> bool {
    with_states(surface, |states| {
        states
            .data_map
            .get::<SurfaceObjectMarker>()
            .is_some_and(|marker| marker.0.load(std::sync::atomic::Ordering::Relaxed))
    })
}

fn mark_surface_object(
    surface: &wayland_server::protocol::wl_surface::WlSurface,
    present: bool,
) {
    with_states(surface, |states| {
        states.data_map.insert_if_missing_threadsafe(SurfaceObjectMarker::default);
        if let Some(marker) = states.data_map.get::<SurfaceObjectMarker>() {
            marker.0.store(present, std::sync::atomic::Ordering::Relaxed);
        }
    });
}

impl<D: ColorDispatch> Dispatch<WpColorManagementSurfaceV1, SurfaceData_, D>
    for ColorManagementState
{
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &WpColorManagementSurfaceV1,
        request: wp_color_management_surface_v1::Request,
        data: &SurfaceData_,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            wp_color_management_surface_v1::Request::SetImageDescription {
                image_description,
                render_intent,
            } => {
                let Some(description) = ready_description(&image_description) else {
                    resource.post_error(
                        wp_color_management_surface_v1::Error::ImageDescription,
                        "the image description is not ready",
                    );
                    return;
                };

                let Some(intent) = intent_from_wire(render_intent) else {
                    resource.post_error(
                        wp_color_management_surface_v1::Error::RenderIntent,
                        "the render intent is outside the enum",
                    );
                    return;
                };
                if !Capabilities::for_version(resource.version()).supports_intent(intent) {
                    resource.post_error(
                        wp_color_management_surface_v1::Error::RenderIntent,
                        "that render intent was not advertised",
                    );
                    return;
                }

                set_surface_color(
                    &data.surface,
                    Some(SurfaceColor {
                        description,
                        intent,
                    }),
                );
                state.surface_color_changed(&data.surface);
            }

            wp_color_management_surface_v1::Request::UnsetImageDescription => {
                set_surface_color(&data.surface, None);
                state.surface_color_changed(&data.surface);
            }

            _ => {}
        }
    }

    fn destroyed(
        state: &mut D,
        _client: wayland_server::backend::ClientId,
        _resource: &WpColorManagementSurfaceV1,
        data: &SurfaceData_,
    ) {
        // Destroying the object removes the description, per the protocol.
        mark_surface_object(&data.surface, false);
        set_surface_color(&data.surface, None);
        state.surface_color_changed(&data.surface);
    }
}

/// Stages a surface's colour, to be applied on its next commit.
fn set_surface_color(
    surface: &wayland_server::protocol::wl_surface::WlSurface,
    color: Option<SurfaceColor>,
) {
    with_states(surface, |states| {
        states
            .cached_state
            .get::<SurfaceColorCachedState>()
            .pending()
            .color = color;
    });
}

fn ready_description(object: &WpImageDescriptionV1) -> Option<Arc<ImageDescription>> {
    let data = object.data::<DescriptionData>()?;
    match &*lock(&data.state) {
        DescriptionState::Ready(description) => Some(Arc::clone(description)),
        _ => None,
    }
}

// --- MARK: feedback ---

impl<D: ColorDispatch> Dispatch<WpColorManagementSurfaceFeedbackV1, FeedbackData, D>
    for ColorManagementState
{
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &WpColorManagementSurfaceFeedbackV1,
        request: wp_color_management_surface_feedback_v1::Request,
        data: &FeedbackData,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        let (id, parametric_only) = match request {
            wp_color_management_surface_feedback_v1::Request::GetPreferred {
                image_description,
            } => (image_description, false),
            wp_color_management_surface_feedback_v1::Request::GetPreferredParametric {
                image_description,
            } => {
                if !Capabilities::for_version(resource.version())
                    .supports_feature(Feature::Parametric)
                {
                    resource.post_error(
                        wp_color_management_surface_feedback_v1::Error::UnsupportedFeature,
                        "parametric descriptions are not supported",
                    );
                    return;
                }
                (image_description, true)
            }
            _ => return,
        };

        let preferred = state.preferred_image_description(&data.surface);
        let object = data_init.init(
            id,
            DescriptionData {
                state: Mutex::new(DescriptionState::Pending),
                allows_info: true,
            },
        );

        match preferred {
            // A profile-backed preference cannot answer the parametric
            // request; the protocol expects an approximation, and refusing is
            // better than inventing one.
            Some(description)
                if parametric_only && description.parametric().is_none() =>
            {
                fail(
                    &object,
                    wp_image_description_v1::Cause::Unsupported,
                    "the preferred description has no parametric form".into(),
                );
            }
            Some(description) => ready(&object, description),
            None => fail(
                &object,
                wp_image_description_v1::Cause::NoOutput,
                "the surface is not on an output".into(),
            ),
        }
    }

    fn destroyed(
        state: &mut D,
        _client: wayland_server::backend::ClientId,
        resource: &WpColorManagementSurfaceFeedbackV1,
        _data: &FeedbackData,
    ) {
        state.color_management_state().forget_feedback(resource);
    }
}

// --- MARK: parametric creator ---

impl<D: ColorDispatch> Dispatch<WpImageDescriptionCreatorParamsV1, ParamsData, D>
    for ColorManagementState
{
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &WpImageDescriptionCreatorParamsV1,
        request: wp_image_description_creator_params_v1::Request,
        data: &ParamsData,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        use wp_image_description_creator_params_v1::{Error, Request};

        let mut params = lock(data);
        if params.used {
            // `create` is a destructor, so anything after it is a bug.
            return;
        }

        macro_rules! set_once {
            ($field:expr, $value:expr) => {{
                if $field.is_some() {
                    resource.post_error(Error::AlreadySet, "this property has already been set");
                    return;
                }
                $field = Some($value);
            }};
        }

        match request {
            Request::SetTfNamed { tf } => {
                let Some(named) = tf.into_result().ok().and_then(|tf| creator::named_transfer(tf as u32))
                else {
                    resource.post_error(Error::InvalidTf, "unknown transfer function");
                    return;
                };
                if !Capabilities::for_version(resource.version()).supports_transfer(named) {
                    resource.post_error(Error::InvalidTf, "that transfer function was not advertised");
                    return;
                }
                set_once!(params.transfer, TransferFunction::Named(named));
            }

            Request::SetTfPower { eexp } => {
                if !Capabilities::for_version(resource.version())
                    .supports_feature(Feature::SetTfPower)
                {
                    resource.post_error(Error::UnsupportedFeature, "power curves are not supported");
                    return;
                }
                // The protocol's range: 1.0 to 10.0, in ten-thousandths.
                if !(10_000..=100_000).contains(&eexp) {
                    resource.post_error(Error::InvalidTf, "the exponent is outside 1.0..=10.0");
                    return;
                }
                set_once!(params.transfer, TransferFunction::Power(eexp));
            }

            Request::SetPrimariesNamed { primaries } => {
                let Some(named) = primaries
                    .into_result()
                    .ok()
                    .and_then(|primaries| creator::named_primaries(primaries as u32))
                else {
                    resource.post_error(Error::InvalidPrimariesNamed, "unknown primaries");
                    return;
                };
                set_once!(params.primaries, PrimariesSpec::Named(named));
            }

            Request::SetPrimaries {
                r_x, r_y, g_x, g_y, b_x, b_y, w_x, w_y,
            } => {
                if !Capabilities::for_version(resource.version())
                    .supports_feature(Feature::SetPrimaries)
                {
                    resource.post_error(Error::UnsupportedFeature, "custom primaries are not supported");
                    return;
                }
                set_once!(
                    params.primaries,
                    PrimariesSpec::Raw(primaries_from_wire(r_x, r_y, g_x, g_y, b_x, b_y, w_x, w_y))
                );
            }

            Request::SetLuminances { min_lum, max_lum, reference_lum } => {
                if !Capabilities::for_version(resource.version())
                    .supports_feature(Feature::SetLuminances)
                {
                    resource.post_error(Error::UnsupportedFeature, "luminances are not supported");
                    return;
                }
                set_once!(
                    params.luminances,
                    Luminances { min_scaled: min_lum, max: max_lum, reference: reference_lum }
                );
            }

            Request::SetMasteringDisplayPrimaries {
                r_x, r_y, g_x, g_y, b_x, b_y, w_x, w_y,
            } => {
                if !Capabilities::for_version(resource.version())
                    .supports_feature(Feature::SetMasteringDisplayPrimaries)
                {
                    resource.post_error(
                        Error::UnsupportedFeature,
                        "mastering primaries are not supported",
                    );
                    return;
                }
                set_once!(
                    params.mastering_primaries,
                    primaries_from_wire(r_x, r_y, g_x, g_y, b_x, b_y, w_x, w_y)
                );
            }

            Request::SetMasteringLuminance { min_lum, max_lum } => {
                set_once!(params.mastering_luminance, (min_lum, max_lum));
            }

            Request::SetMaxCll { max_cll } => set_once!(params.max_cll, max_cll),
            Request::SetMaxFall { max_fall } => set_once!(params.max_fall, max_fall),

            Request::Create { image_description } => {
                params.used = true;

                if let Err(error) = creator::validate(&params) {
                    resource.post_error(
                        creator::params_error(error),
                        "the parameters do not describe an image description",
                    );
                    return;
                }

                let Some(primaries) = creator::resolved_primaries(&params) else {
                    resource.post_error(Error::IncompleteSet, "no primaries were set");
                    return;
                };
                let Some(transfer) = params.transfer else {
                    resource.post_error(Error::IncompleteSet, "no transfer function was set");
                    return;
                };
                let Some(target_primaries) = creator::resolved_target_primaries(&params) else {
                    resource.post_error(Error::IncompleteSet, "no target volume could be resolved");
                    return;
                };

                let described = ParametricDescription {
                    primaries,
                    transfer,
                    luminances: params.resolved_luminances(),
                    target_primaries,
                    target_luminance: creator::resolved_target_luminance(&params),
                    max_cll: params.max_cll,
                    max_fall: params.max_fall,
                };
                drop(params);

                let object = data_init.init(
                    image_description,
                    DescriptionData {
                        state: Mutex::new(DescriptionState::Pending),
                        // Built by the client, so there is nothing to read
                        // back that it does not already know.
                        allows_info: false,
                    },
                );

                if let Err(reason) = state.accept_parametric(&described) {
                    fail(
                        &object,
                        wp_image_description_v1::Cause::Unsupported,
                        reason.into_owned(),
                    );
                    return;
                }

                let description = state
                    .color_management_state()
                    .intern(DescriptionKind::Parametric(described));
                ready(&object, description);
            }

            _ => {}
        }
    }
}

fn primaries_from_wire(
    r_x: i32, r_y: i32, g_x: i32, g_y: i32, b_x: i32, b_y: i32, w_x: i32, w_y: i32,
) -> Primaries {
    Primaries {
        red: chromaticity_from_wire(r_x, r_y),
        green: chromaticity_from_wire(g_x, g_y),
        blue: chromaticity_from_wire(b_x, b_y),
        white: chromaticity_from_wire(w_x, w_y),
    }
}

// --- MARK: ICC creator ---

impl<D: ColorDispatch> Dispatch<WpImageDescriptionCreatorIccV1, IccCreatorData, D>
    for ColorManagementState
{
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &WpImageDescriptionCreatorIccV1,
        request: wp_image_description_creator_icc_v1::Request,
        data: &IccCreatorData,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        use wp_image_description_creator_icc_v1::{Error, Request};

        let mut creator_state = lock(data);
        if creator_state.used {
            return;
        }

        match request {
            Request::SetIccFile { icc_profile, offset, length } => {
                if creator_state.file.is_some() {
                    resource.post_error(Error::AlreadySet, "the profile has already been set");
                    return;
                }
                if length == 0 || length > creator::MAX_ICC_SIZE {
                    resource.post_error(Error::BadSize, "the profile size is out of range");
                    return;
                }
                creator_state.file = Some(IccFile { fd: icc_profile, offset, length });
            }

            Request::Create { image_description } => {
                creator_state.used = true;

                let Some(file) = creator_state.file.take() else {
                    resource.post_error(Error::IncompleteSet, "no profile was set");
                    return;
                };
                drop(creator_state);

                let bytes = match creator::read_icc(&file) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        resource.post_error(
                            creator::icc_error(error),
                            "the profile could not be read",
                        );
                        return;
                    }
                };

                let object = data_init.init(
                    image_description,
                    DescriptionData {
                        state: Mutex::new(DescriptionState::Pending),
                        allows_info: false,
                    },
                );

                if let Err(reason) = creator::validate_icc_header(&bytes) {
                    fail(
                        &object,
                        wp_image_description_v1::Cause::Unsupported,
                        reason.to_owned(),
                    );
                    return;
                }

                let icc = IccData::new(bytes);
                if let Err(reason) = state.accept_icc(&icc) {
                    fail(
                        &object,
                        wp_image_description_v1::Cause::Unsupported,
                        reason.into_owned(),
                    );
                    return;
                }

                let description = state
                    .color_management_state()
                    .intern(DescriptionKind::Icc(icc));
                ready(&object, description);
            }

            _ => {}
        }
    }
}

// --- MARK: image description ---

impl<D: ColorDispatch> Dispatch<WpImageDescriptionV1, DescriptionData, D>
    for ColorManagementState
{
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &WpImageDescriptionV1,
        request: wp_image_description_v1::Request,
        data: &DescriptionData,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        let wp_image_description_v1::Request::GetInformation { information } = request else {
            return;
        };

        let description = match &*lock(&data.state) {
            DescriptionState::Ready(description) => Arc::clone(description),
            _ => {
                resource.post_error(
                    wp_image_description_v1::Error::NotReady,
                    "the image description is not ready",
                );
                return;
            }
        };

        if !data.allows_info || !description.kind().is_describable() {
            resource.post_error(
                wp_image_description_v1::Error::NoInformation,
                "this image description does not carry readable information",
            );
            return;
        }

        let object = data_init.init(information, InfoData);
        // Deferred, not sent here: see `defer_image_description_info`.
        state.defer_image_description_info(object, description);
    }
}

impl<D: ColorDispatch> Dispatch<WpImageDescriptionInfoV1, InfoData, D> for ColorManagementState {
    fn request(
        _state: &mut D,
        _client: &Client,
        _resource: &WpImageDescriptionInfoV1,
        _request: <WpImageDescriptionInfoV1 as Resource>::Request,
        _data: &InfoData,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        // The interface has no requests; it is events and a destructor.
    }
}

// --- MARK: enum conversions ---

fn intent_wire(intent: RenderIntent) -> wp_color_manager_v1::RenderIntent {
    use wp_color_manager_v1::RenderIntent as Wire;
    match intent {
        RenderIntent::Perceptual => Wire::Perceptual,
        RenderIntent::Relative => Wire::Relative,
        RenderIntent::Saturation => Wire::Saturation,
        RenderIntent::Absolute => Wire::Absolute,
        RenderIntent::RelativeBpc => Wire::RelativeBpc,
        RenderIntent::AbsoluteNoAdaptation => Wire::AbsoluteNoAdaptation,
    }
}

fn intent_from_wire(intent: WEnum<wp_color_manager_v1::RenderIntent>) -> Option<RenderIntent> {
    use wp_color_manager_v1::RenderIntent as Wire;
    Some(match intent.into_result().ok()? {
        Wire::Perceptual => RenderIntent::Perceptual,
        Wire::Relative => RenderIntent::Relative,
        Wire::Saturation => RenderIntent::Saturation,
        Wire::Absolute => RenderIntent::Absolute,
        Wire::RelativeBpc => RenderIntent::RelativeBpc,
        Wire::AbsoluteNoAdaptation => RenderIntent::AbsoluteNoAdaptation,
        _ => return None,
    })
}

fn feature_wire(feature: Feature) -> wp_color_manager_v1::Feature {
    use wp_color_manager_v1::Feature as Wire;
    match feature {
        Feature::IccV2V4 => Wire::IccV2V4,
        Feature::Parametric => Wire::Parametric,
        Feature::SetPrimaries => Wire::SetPrimaries,
        Feature::SetTfPower => Wire::SetTfPower,
        Feature::SetLuminances => Wire::SetLuminances,
        Feature::SetMasteringDisplayPrimaries => Wire::SetMasteringDisplayPrimaries,
        Feature::ExtendedTargetVolume => Wire::ExtendedTargetVolume,
        Feature::WindowsScrgb => Wire::WindowsScrgb,
        Feature::WindowsBt2100 => Wire::WindowsBt2100,
    }
}

fn transfer_wire(
    transfer: super::record::NamedTransferFunction,
) -> wp_color_manager_v1::TransferFunction {
    // The generated enum and the wire values are the same numbers, and the
    // conversion cannot fail because the table only holds values the XML
    // defines.
    wp_color_manager_v1::TransferFunction::try_from(creator::transfer_wire_value(transfer))
        .unwrap_or(wp_color_manager_v1::TransferFunction::Gamma22)
}

fn primaries_wire(primaries: super::record::NamedPrimaries) -> wp_color_manager_v1::Primaries {
    wp_color_manager_v1::Primaries::try_from(creator::primaries_wire_value(primaries))
        .unwrap_or(wp_color_manager_v1::Primaries::Srgb)
}

/// Wires `$ty` up as the dispatch target for every interface in this protocol.
#[macro_export]
macro_rules! delegate_color_management {
    ($ty:ty) => {
        type __ColorManagementState = $crate::color_management::state::ColorManagementState;

        ::wayland_server::delegate_global_dispatch!($ty: [
            $crate::color_management::reexports::wp_color_manager_v1::WpColorManagerV1:
                $crate::color_management::state::ColorManagementGlobalData
        ] => __ColorManagementState);

        ::wayland_server::delegate_dispatch!($ty: [
            $crate::color_management::reexports::wp_color_manager_v1::WpColorManagerV1: ()
        ] => __ColorManagementState);

        ::wayland_server::delegate_dispatch!($ty: [
            $crate::color_management::reexports::wp_color_management_output_v1::WpColorManagementOutputV1:
                $crate::color_management::state::OutputData
        ] => __ColorManagementState);

        ::wayland_server::delegate_dispatch!($ty: [
            $crate::color_management::reexports::wp_color_management_surface_v1::WpColorManagementSurfaceV1:
                $crate::color_management::state::SurfaceData_
        ] => __ColorManagementState);

        ::wayland_server::delegate_dispatch!($ty: [
            $crate::color_management::reexports::wp_color_management_surface_feedback_v1::WpColorManagementSurfaceFeedbackV1:
                $crate::color_management::state::FeedbackData
        ] => __ColorManagementState);

        ::wayland_server::delegate_dispatch!($ty: [
            $crate::color_management::reexports::wp_image_description_creator_params_v1::WpImageDescriptionCreatorParamsV1:
                $crate::color_management::creator::ParamsData
        ] => __ColorManagementState);

        ::wayland_server::delegate_dispatch!($ty: [
            $crate::color_management::reexports::wp_image_description_creator_icc_v1::WpImageDescriptionCreatorIccV1:
                $crate::color_management::creator::IccCreatorData
        ] => __ColorManagementState);

        ::wayland_server::delegate_dispatch!($ty: [
            $crate::color_management::reexports::wp_image_description_v1::WpImageDescriptionV1:
                $crate::color_management::state::DescriptionData
        ] => __ColorManagementState);

        ::wayland_server::delegate_dispatch!($ty: [
            $crate::color_management::reexports::wp_image_description_info_v1::WpImageDescriptionInfoV1:
                $crate::color_management::state::InfoData
        ] => __ColorManagementState);
    };
}

const _: () = assert!(VERSION == 3);
