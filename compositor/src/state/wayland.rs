use std::collections::HashSet;

use anyhow::Context;
use calloop::LoopHandle;
use smithay::{
    input::{Seat, SeatState, keyboard::XkbConfig},
    reexports::{
        wayland_protocols_misc::server_decoration::server::org_kde_kwin_server_decoration_manager::Mode as KdeDefaultMode,
        wayland_server::{
            DisplayHandle,
            protocol::{wl_shm, wl_surface::WlSurface},
        },
    },
    utils::{Clock, Monotonic},
    wayland::{
        alpha_modifier::AlphaModifierState,
        commit_timing::CommitTimingManagerState,
        compositor::CompositorState,
        content_type::ContentTypeState,
        cursor_shape::CursorShapeManagerState,
        dmabuf::DmabufState,
        drm_syncobj::DrmSyncobjState,
        fifo::FifoManagerState,
        fractional_scale::FractionalScaleManagerState,
        idle_inhibit::IdleInhibitManagerState,
        idle_notify::IdleNotifierState,
        keyboard_shortcuts_inhibit::{KeyboardShortcutsInhibitState, KeyboardShortcutsInhibitor},
        output::OutputManagerState,
        pointer_constraints::PointerConstraintsState,
        pointer_gestures::PointerGesturesState,
        presentation::PresentationState,
        relative_pointer::RelativePointerManagerState,
        security_context::SecurityContextState,
        selection::{
            data_device::DataDeviceState,
            ext_data_control::DataControlState as ExtDataControlState,
            primary_selection::PrimarySelectionState,
            wlr_data_control::DataControlState as WlrDataControlState,
        },
        session_lock::SessionLockManagerState,
        shell::{kde::decoration::KdeDecorationState, xdg::decoration::XdgDecorationState},
        shm::ShmState,
        single_pixel_buffer::SinglePixelBufferState,
        tablet_manager::TabletManagerState,
        viewporter::ViewporterState,
        xdg_activation::XdgActivationState,
    },
};

use protocols::{
    appmenu::AppmenuState,
    background_effect::{BackgroundEffectState, Capability as BackgroundEffectCapability},
    color_management::ColorManagementState,
    crownos_agent_access::AgentAccessState,
    crownos_background_effects::{
        BackgroundEffectsState as CrownosBackgroundEffectsState,
        Capability as CrownosEffectCapability,
    },
    crownos_input::InputState as CrownosInputState,
    crownos_screencast::ScreencastState,
    crownos_surface_animation::SurfaceAnimationState,
    crownos_surface_visibility::SurfaceVisibilityState,
    crownos_virtual_output::VirtualOutputState,
    gamma_control::GammaControlState,
    output_management::OutputManagementState,
    output_power::OutputPowerState,
    tearing_control::TearingControlState,
};

use crate::{
    state::{State, agent_access::AgentAccessPolicy},
    utils::privilege::is_privileged,
};

pub struct WaylandState {
    /// `crownos_agent_access_v1`. Privileged: a grant lets an app hand its
    /// whole UI to an AI agent.
    pub agent_access_state: AgentAccessState,
    /// Who answers `crownos_agent_access_v1` requests.
    pub agent_access_policy: AgentAccessPolicy,
    pub appmenu_state: AppmenuState,
    pub background_effect_state: BackgroundEffectState,
    /// `wp_color_management_v1`. Unprivileged: any client may describe its own
    /// colours, and reading an output's is not sensitive.
    pub color_management_state: ColorManagementState,
    pub compositor_state: CompositorState,
    /// `crownos_background_effects`. The richer, CrownOS-only counterpart of
    /// `ext-background-effect-v1`: parametric shapes, tint, vibrancy, a
    /// refractive rim and a drop shadow, for the shell and the system apps.
    pub crownos_background_effects_state: CrownosBackgroundEffectsState,
    // pub corner_radius_state: CornerRadiusState,
    pub data_device_state: DataDeviceState,
    pub dmabuf_state: DmabufState,
    /// `wp_linux_drm_syncobj_manager_v1`, once the primary GPU is known to
    /// support timeline eventfds.
    pub drm_syncobj_state: Option<DrmSyncobjState>,
    pub fifo_manager_state: FifoManagerState,
    pub commit_timing_manager_state: CommitTimingManagerState,
    pub fractional_scale_state: FractionalScaleManagerState,
    pub keyboard_shortcuts_inhibit_state: KeyboardShortcutsInhibitState,
    pub output_state: OutputManagerState,
    /// `zwlr_gamma_control_v1`. Privileged: a client holding it can make the
    /// screen unreadable.
    pub gamma_control_state: GammaControlState,
    /// `zwlr_output_management_v1`. Privileged: it can turn every monitor off.
    pub output_management_state: OutputManagementState,
    /// `zwlr_output_power_management_v1`. Privileged: it can blank the screen.
    pub output_power_state: OutputPowerState,
    /// `zwp_pointer_gestures_v1`. Held only to keep the global alive — the
    /// events themselves go out through the seat's pointer.
    pub pointer_gestures_state: PointerGesturesState,
    pub pointer_constraints_state: PointerConstraintsState,
    pub relative_pointer_state: RelativePointerManagerState,
    pub presentation_state: PresentationState,
    pub primary_selection_state: PrimarySelectionState,
    pub ext_data_control_state: ExtDataControlState,
    pub wlr_data_control_state: WlrDataControlState,
    /// `crownos_screencast_v1`. Privileged: it reads every pixel on screen.
    pub screencast_state: ScreencastState,
    /// `crownos_virtual_output_v1`. Privileged: it adds outputs windows can
    /// be moved onto, out of the user's sight.
    pub virtual_output_state: VirtualOutputState,
    /// `crownos_input_v1`. Privileged: an injector types into any window and
    /// a capture sees every keystroke.
    pub crownos_input_state: CrownosInputState,
    /// `crownos_surface_animation_v1`. Unprivileged: a client only animates
    /// its own subsurfaces. The running springs live on the shell.
    pub surface_animation_state: SurfaceAnimationState,
    /// `crownos_surface_visibility_v1`. Unprivileged: a client only learns
    /// about surfaces it owns.
    pub surface_visibility_state: SurfaceVisibilityState,
    // pub cosmic_image_capture_source_state: CosmicImageCaptureSourceState,
    // pub output_capture_source_state: OutputCaptureSourceState,
    // pub toplevel_capture_source_state: ToplevelCaptureSourceState,
    // pub image_copy_capture_state: ImageCopyCaptureState,
    /// `wp_security_context_v1`. Sandbox engines register a listener per app,
    /// and every client that connects through it loses the privileged globals.
    pub security_context_state: SecurityContextState,
    pub seat_state: SeatState<State>,
    pub seat: Seat<State>,
    pub session_lock_manager_state: SessionLockManagerState,
    pub idle_notifier_state: IdleNotifierState<State>,
    pub idle_inhibit_manager_state: IdleInhibitManagerState,
    pub idle_inhibiting_surfaces: HashSet<WlSurface>,
    /// Every live `zwp_keyboard_shortcuts_inhibitor`, active only while its
    /// surface holds the keyboard.
    pub shortcuts_inhibitors: Vec<KeyboardShortcutsInhibitor>,
    pub shm_state: ShmState,
    pub cursor_shape_manager_state: CursorShapeManagerState,
    // pub wl_drm_state: Option<WlDrmState<Option<DrmNode>>>,
    pub viewporter_state: ViewporterState,
    pub content_type_state: ContentTypeState,
    pub single_pixel_buffer_state: SinglePixelBufferState,
    pub alpha_modifier_state: AlphaModifierState,
    pub xdg_activation_state: XdgActivationState,
    pub tearing_control_state: TearingControlState,
    pub tablet_manager_state: TabletManagerState,
    pub kde_decoration_state: KdeDecorationState,
    pub xdg_decoration_state: XdgDecorationState,
    // pub overlap_notify_state: OverlapNotifyState,
    // pub a11y_state: A11yState,
    // pub dbus_state: DBusState,
    // pub keyboard_layout_state: KeyboardLayoutState,
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
        // The swap lives in the keymap rather than in a keycode rewrite on the
        // way in, so the map handed to clients says the same thing the events
        // do: xkb resolves the keysyms, the Caps LED follows the key that now
        // locks it, and nothing in the input path has to lie about which key
        // was pressed.
        // TODO: layout, variant and these options belong in the config, next to
        // the repeat rate that is just as hardcoded.
        let xkb = XkbConfig {
            options: Some("caps:swapescape".to_owned()),
            ..Default::default()
        };
        seat.add_keyboard(xkb, 200, 65)
            .with_context(|| "Failed to add a keyboard to the seat")?;
        seat.add_pointer();

        Ok(Self {
            agent_access_state: AgentAccessState::new::<State, _>(display, is_privileged),
            agent_access_policy: AgentAccessPolicy::from_env(),
            // Everything the renderer can do, which the config then narrows
            // to what it will do — see
            // `State::sync_background_effect_capabilities`. A renderer whose
            // blur shaders fail to compile degrades to drawing windows without
            // the effect rather than withdrawing the capability, which the
            // protocol explicitly allows ("subject to compositor policies").
            appmenu_state: AppmenuState::new::<State>(display),
            background_effect_state: BackgroundEffectState::new::<State>(
                display,
                BackgroundEffectCapability::Blur,
            ),
            color_management_state: ColorManagementState::new::<State, _>(display, |_| true),
            compositor_state: CompositorState::new_v6::<State>(display),
            crownos_background_effects_state: CrownosBackgroundEffectsState::new::<State>(
                display,
                CrownosEffectCapability::all(),
            ),
            data_device_state: DataDeviceState::new::<State>(display),
            // TODO: `create_global` once the render node's formats are known.
            dmabuf_state: DmabufState::new(),
            drm_syncobj_state: None,
            fifo_manager_state: FifoManagerState::new::<State>(display),
            commit_timing_manager_state: CommitTimingManagerState::new::<State>(display),
            fractional_scale_state: FractionalScaleManagerState::new::<State>(display),
            keyboard_shortcuts_inhibit_state: KeyboardShortcutsInhibitState::new::<State>(display),
            output_state: OutputManagerState::new_with_xdg_output::<State>(display),
            gamma_control_state: GammaControlState::new::<State, _>(display, is_privileged),
            output_management_state: OutputManagementState::new::<State, _>(display, is_privileged),
            output_power_state: OutputPowerState::new::<State, _>(display, is_privileged),
            pointer_gestures_state: PointerGesturesState::new::<State>(display),
            pointer_constraints_state: PointerConstraintsState::new::<State>(display),
            relative_pointer_state: RelativePointerManagerState::new::<State>(display),
            presentation_state: PresentationState::new::<State>(display, clock.id() as u32),
            ext_data_control_state: ExtDataControlState::new::<State, _>(
                display,
                Some(&primary_selection_state),
                is_privileged,
            ),
            wlr_data_control_state: WlrDataControlState::new::<State, _>(
                display,
                Some(&primary_selection_state),
                is_privileged,
            ),
            primary_selection_state,
            screencast_state: ScreencastState::new::<State, _>(display, is_privileged),
            virtual_output_state: VirtualOutputState::new::<State, _>(display, is_privileged),
            crownos_input_state: CrownosInputState::new::<State, _>(display, is_privileged),
            surface_animation_state: SurfaceAnimationState::new::<State>(display),
            surface_visibility_state: SurfaceVisibilityState::new::<State>(display),
            security_context_state: SecurityContextState::new::<State, _>(display, is_privileged),
            seat_state,
            seat,
            session_lock_manager_state: SessionLockManagerState::new::<State, _>(
                display,
                is_privileged,
            ),
            idle_notifier_state: IdleNotifierState::new(display, loop_handle),
            idle_inhibit_manager_state: IdleInhibitManagerState::new::<State>(display),
            idle_inhibiting_surfaces: HashSet::new(),
            shortcuts_inhibitors: Vec::new(),
            shm_state: ShmState::new::<State>(display, shm_formats),
            cursor_shape_manager_state: CursorShapeManagerState::new::<State>(display),
            viewporter_state: ViewporterState::new::<State>(display),
            content_type_state: ContentTypeState::new::<State>(display),
            single_pixel_buffer_state: SinglePixelBufferState::new::<State>(display),
            alpha_modifier_state: AlphaModifierState::new::<State>(display),
            xdg_activation_state: XdgActivationState::new::<State>(display),
            tearing_control_state: TearingControlState::new::<State>(display),
            tablet_manager_state: TabletManagerState::new::<State>(display),
            kde_decoration_state: KdeDecorationState::new::<State>(display, KdeDefaultMode::Server),
            xdg_decoration_state: XdgDecorationState::new::<State>(display),
            clock,
        })
    }
}
