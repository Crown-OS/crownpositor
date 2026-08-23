use std::collections::HashSet;

use anyhow::Context;
use calloop::LoopHandle;
use smithay::{
    input::{Seat, SeatState},
    reexports::{
        wayland_protocols_misc::server_decoration::server::org_kde_kwin_server_decoration_manager::Mode as KdeDefaultMode,
        wayland_server::{
            protocol::{wl_shm, wl_surface::WlSurface},
            Client, DisplayHandle,
        },
    },
    utils::{Clock, Monotonic},
    wayland::{
        compositor::CompositorState,
        cursor_shape::CursorShapeManagerState,
        dmabuf::DmabufState,
        fractional_scale::FractionalScaleManagerState,
        idle_inhibit::IdleInhibitManagerState,
        idle_notify::IdleNotifierState,
        keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitState,
        output::OutputManagerState,
        presentation::PresentationState,
        selection::{
            data_device::DataDeviceState,
            ext_data_control::DataControlState as ExtDataControlState,
            primary_selection::PrimarySelectionState,
            wlr_data_control::DataControlState as WlrDataControlState,
        },
        session_lock::SessionLockManagerState,
        shell::{kde::decoration::KdeDecorationState, xdg::decoration::XdgDecorationState},
        shm::ShmState,
        viewporter::ViewporterState,
    },
};

use smithay::reexports::wayland_protocols::ext::background_effect::v1::server::ext_background_effect_manager_v1::Capability as BackgroundEffectCapability;

use crate::{protocols::background_effect::BackgroundEffectState, state::State};

pub struct WaylandState {
    pub background_effect_state: BackgroundEffectState,
    pub compositor_state: CompositorState,
    // pub corner_radius_state: CornerRadiusState,
    pub data_device_state: DataDeviceState,
    pub dmabuf_state: DmabufState,
    pub fractional_scale_state: FractionalScaleManagerState,
    pub keyboard_shortcuts_inhibit_state: KeyboardShortcutsInhibitState,
    pub output_state: OutputManagerState,
    // pub output_configuration_state: OutputConfigurationState<State>,
    // pub output_power_state: OutputPowerState,
    pub presentation_state: PresentationState,
    pub primary_selection_state: PrimarySelectionState,
    pub ext_data_control_state: ExtDataControlState,
    pub wlr_data_control_state: WlrDataControlState,
    // pub cosmic_image_capture_source_state: CosmicImageCaptureSourceState,
    // pub output_capture_source_state: OutputCaptureSourceState,
    // pub toplevel_capture_source_state: ToplevelCaptureSourceState,
    // pub image_copy_capture_state: ImageCopyCaptureState,
    pub seat_state: SeatState<State>,
    pub seat: Seat<State>,
    pub session_lock_manager_state: SessionLockManagerState,
    pub idle_notifier_state: IdleNotifierState<State>,
    pub idle_inhibit_manager_state: IdleInhibitManagerState,
    pub idle_inhibiting_surfaces: HashSet<WlSurface>,
    /// Surfaces holding an active `zwp_keyboard_shortcuts_inhibitor`.
    pub shortcuts_inhibiting_surfaces: HashSet<WlSurface>,
    pub shm_state: ShmState,
    pub cursor_shape_manager_state: CursorShapeManagerState,
    // pub wl_drm_state: Option<WlDrmState<Option<DrmNode>>>,
    pub viewporter_state: ViewporterState,
    pub kde_decoration_state: KdeDecorationState,
    pub xdg_decoration_state: XdgDecorationState,
    // pub overlap_notify_state: OverlapNotifyState,
    // pub a11y_state: A11yState,
    // pub dbus_state: DBusState,
    // pub keyboard_layout_state: KeyboardLayoutState,
    // pub background_effect_state: BackgroundEffectState,
    pub clock: Clock<Monotonic>,
}

impl WaylandState {
    pub fn try_new(
        display: &DisplayHandle,
        loop_handle: LoopHandle<'static, State>,
    ) -> anyhow::Result<Self> {
        let clock = Clock::<Monotonic>::new();

        // TODO: take these from the active renderer.
        let shm_formats: Vec<wl_shm::Format> = Vec::new();

        let primary_selection_state = PrimarySelectionState::new::<State>(display);

        // TODO: track seats and their devices as they are hot-plugged.
        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(display, "seat-0");
        seat.add_keyboard(Default::default(), 200, 65)
            .with_context(|| "Failed to add a keyboard to the seat")?;
        seat.add_pointer();

        Ok(Self {
            // Blur is advertised unconditionally: capability describes what
            // the compositor *supports*, and a renderer whose blur shaders
            // fail to compile degrades to drawing windows without the effect,
            // which the protocol explicitly allows ("subject to compositor
            // policies").
            background_effect_state: BackgroundEffectState::new::<State>(
                display,
                BackgroundEffectCapability::Blur,
            ),
            compositor_state: CompositorState::new::<State>(display),
            data_device_state: DataDeviceState::new::<State>(display),
            // TODO: `create_global` once the render node's formats are known.
            dmabuf_state: DmabufState::new(),
            fractional_scale_state: FractionalScaleManagerState::new::<State>(display),
            keyboard_shortcuts_inhibit_state: KeyboardShortcutsInhibitState::new::<State>(display),
            output_state: OutputManagerState::new_with_xdg_output::<State>(display),
            presentation_state: PresentationState::new::<State>(display, clock.id() as u32),
            ext_data_control_state: ExtDataControlState::new::<State, _>(
                display,
                Some(&primary_selection_state),
                privileged_client_filter,
            ),
            wlr_data_control_state: WlrDataControlState::new::<State, _>(
                display,
                Some(&primary_selection_state),
                privileged_client_filter,
            ),
            primary_selection_state,
            seat_state,
            seat,
            session_lock_manager_state: SessionLockManagerState::new::<State, _>(
                display,
                privileged_client_filter,
            ),
            idle_notifier_state: IdleNotifierState::new(display, loop_handle),
            idle_inhibit_manager_state: IdleInhibitManagerState::new::<State>(display),
            idle_inhibiting_surfaces: HashSet::new(),
            shortcuts_inhibiting_surfaces: HashSet::new(),
            shm_state: ShmState::new::<State>(display, shm_formats),
            cursor_shape_manager_state: CursorShapeManagerState::new::<State>(display),
            viewporter_state: ViewporterState::new::<State>(display),
            kde_decoration_state: KdeDecorationState::new::<State>(display, KdeDefaultMode::Server),
            xdg_decoration_state: XdgDecorationState::new::<State>(display),
            clock,
        })
    }
}

/// Gates globals that must not be exposed to untrusted clients.
///
/// TODO: only accept clients launched by the shell itself.
fn privileged_client_filter(_client: &Client) -> bool {
    true
}
