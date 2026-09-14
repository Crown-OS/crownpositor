//! One connected monitor: its CRTC, swapchain, frame pacing and render pass.
//!
//! The [`DrmCompositor`] does the heavy lifting per frame: damage-tracked
//! rendering into a swapchain buffer, direct scanout onto planes when an
//! element's buffer qualifies, and the atomic commit. This module decides
//! *when* it runs — the [`RedrawState`] machine below — and what it draws,
//! which is the same generic element list the winit backend builds.
//!
//! The state machine (niri's model):
//!
//! ```text
//!            queue_redraw()          render: damage            vblank
//!  Idle ─────────────────────▶ Queued ────────────▶ WaitingForVBlank ──▶ Idle
//!                                │                        ▲    │
//!                                │ render: no damage      │    │ queue_redraw()
//!                                ▼                        │    ▼
//!                WaitingForEstimatedVBlank ───────────────┘  redraw_needed = true
//!                     (calloop timer)
//! ```
//!
//! A frame is queued to KMS only when something actually changed; when nothing
//! did, a timer standing in for the missing vblank keeps animation ticks and
//! frame callbacks flowing without submitting identical buffers. Idle outputs
//! park in `Idle` and cost nothing.

use std::time::Duration;

use anyhow::Context as _;
use smithay::{
    backend::drm::{
        DrmDeviceFd, DrmEventMetadata, DrmEventTime, DrmNode,
        compositor::{DrmCompositor, FrameFlags, PrimaryPlaneElement},
    },
    desktop::{layer_map_for_output, utils::OutputPresentationFeedback},
    output::{Output, PhysicalProperties},
    reexports::{
        calloop::{
            self, RegistrationToken,
            timer::{TimeoutAction, Timer},
        },
        drm::control::{Mode as DrmMode, connector, crtc},
        wayland_protocols::wp::presentation_time::server::wp_presentation_feedback,
    },
    utils::{Physical, Rectangle, Scale, Transform},
    wayland::presentation::Refresh,
};

use crate::{
    backend::{
        frame_clock::FrameClock,
        kms::{
            KmsState,
            head::HeadState,
        },
        render::{CrownAllocator, DmabufExporter},
    },
    rendering::{
        self, FrameStyle,
        blur::{BlurCache, BlurConfig, BlurSession},
        rounded::MultiDecorator,
    },
    shell::{Shell, monitor::OutputDescriptor},
    state::State,
};

const CLEAR_COLOR: [f32; 4] = [0.1, 0.1, 0.1, 1.0];

/// 8-bit first: 10-bit formats are the first thing broken driver/panel
/// combinations trip over, and a working desktop beats deeper blacks.
const COLOR_FORMATS: &[smithay::backend::allocator::Fourcc] = &[
    smithay::backend::allocator::Fourcc::Abgr8888,
    smithay::backend::allocator::Fourcc::Argb8888,
];

/// The per-CRTC compositor: our unified allocator in, dmabuf-backed
/// framebuffers out, presentation feedback carried through as frame user data.
pub type SurfaceCompositor =
    DrmCompositor<CrownAllocator, DmabufExporter, OutputPresentationFeedback, DrmDeviceFd>;

/// Where an output is in its redraw cycle. See the module docs for the map.
#[derive(Debug, Default)]
pub enum RedrawState {
    /// Nothing scheduled; the next `queue_redraw` starts a frame.
    #[default]
    Idle,
    /// A frame will be rendered on the next dispatch cycle.
    Queued,
    /// A frame is in the hands of KMS; its vblank has not fired yet.
    WaitingForVBlank {
        /// Whether another frame should be queued as soon as it does.
        redraw_needed: bool,
    },
    /// Nothing was submitted (no damage); a timer stands in for the vblank.
    WaitingForEstimatedVBlank(RegistrationToken),
    /// Same, but a redraw got queued on top while waiting.
    WaitingForEstimatedVBlankAndQueued(RegistrationToken),
}

impl RedrawState {
    /// The transition every "please redraw" request takes, whatever the
    /// current state.
    #[must_use]
    pub fn queue(self) -> Self {
        match self {
            Self::Idle => Self::Queued,
            Self::WaitingForEstimatedVBlank(token) => {
                Self::WaitingForEstimatedVBlankAndQueued(token)
            }
            // Already queued.
            value @ (Self::Queued | Self::WaitingForEstimatedVBlankAndQueued(_)) => value,
            // A frame is in flight; redraw right after it lands.
            Self::WaitingForVBlank { .. } => Self::WaitingForVBlank {
                redraw_needed: true,
            },
        }
    }

    /// Whether a render pass should run this dispatch cycle.
    pub fn is_queued(&self) -> bool {
        matches!(
            self,
            Self::Queued | Self::WaitingForEstimatedVBlankAndQueued(_)
        )
    }
}

/// One monitor the KMS backend is driving.
pub struct Surface {
    pub output: Output,
    pub node: DrmNode,
    pub render_node: DrmNode,
    pub compositor: SurfaceCompositor,
    pub frame_clock: FrameClock,
    pub redraw_state: RedrawState,
    /// Every backdrop's blur pyramid on this output, kept across frames.
    pub blur: BlurCache,
    /// Whether the panel is powered.
    ///
    /// A blanked output keeps its `Output`, its workspaces and its windows —
    /// only scanout stops — which is what separates this from disabling the
    /// output altogether.
    pub powered: bool,
}

/// Brings a head up on `crtc`, creating its surface and its `Output`.
///
/// The head must already be in the device's registry — discovering a connector
/// and lighting it are separate steps, because a head the config switches off
/// still has to exist to be switched back on.
pub fn enable_head(
    state: &mut State,
    node: DrmNode,
    connector: connector::Handle,
    crtc: crtc::Handle,
    drm_mode: DrmMode,
) -> anyhow::Result<()> {
    let State {
        backend,
        shell,
        common,
        config,
        ..
    } = state;
    let Some(kms) = backend.kms() else {
        return Ok(());
    };

    let Some(head) = kms
        .devices
        .get(&node)
        .and_then(|device| device.heads.get(&connector))
        .cloned()
    else {
        anyhow::bail!("connector {connector:?} is not a known head on {node}");
    };

    // Renderer formats have to come out of the GPU that will render, before
    // the device map is mutably borrowed below.
    let Some(render_node) = kms.devices.get(&node).map(|device| device.render_node) else {
        anyhow::bail!("connector {} appeared on unknown GPU {node}", head.name);
    };
    let renderer_formats = {
        let mut renderer = kms
            .gpu_manager
            .single_renderer(&render_node)
            .with_context(|| format!("no renderer for GPU {render_node}"))?;
        let gles: &mut smithay::backend::renderer::gles::GlesRenderer = renderer.as_mut();
        gles.egl_context().dmabuf_render_formats().clone()
    };

    let api = kms.api;
    let vulkan = kms.vulkan.as_ref();
    let Some(device) = kms.devices.get_mut(&node) else {
        return Ok(());
    };

    let drm_surface = device
        .drm
        .create_surface(crtc, drm_mode, &[connector])
        .with_context(|| format!("failed to create a DRM surface for {}", head.name))?;

    let mode = smithay::output::Mode::from(drm_mode);
    let refresh_interval = (mode.refresh > 0).then(|| {
        // `refresh` is in millihertz.
        Duration::from_nanos(1_000_000_000_000 / mode.refresh as u64)
    });

    let output = shell.add_output(
        &common.display_handle,
        &config.current,
        OutputDescriptor {
            name: head.name.clone(),
            physical: PhysicalProperties {
                size: (head.physical_mm.0 as i32, head.physical_mm.1 as i32).into(),
                subpixel: head.subpixel,
                // smithay has no null here, so an unidentified panel keeps the
                // "Unknown" placeholder; the protocol layer filters it out
                // rather than telling clients that is the make.
                make: head.edid.as_ref().map_or("Unknown".into(), |edid| edid.make.clone()),
                model: head.edid.as_ref().map_or("Unknown".into(), |edid| edid.model.clone()),
            },
            modes: head.modes.iter().copied().map(Into::into).collect(),
            preferred: head
                .preferred
                .and_then(|index| head.modes.get(index))
                .map(|mode| smithay::output::Mode::from(*mode)),
            current: mode,
            native_transform: Transform::Normal,
            refresh_interval,
            serial: head.edid.as_ref().and_then(|edid| edid.serial.clone()),
            edid: head.edid.clone(),
        },
    );

    let allocator = device.create_allocator(api, vulkan);
    let compositor = SurfaceCompositor::new(
        smithay::output::OutputModeSource::Auto(output.clone()),
        drm_surface,
        // Default plane set: the compositor filters what scanout may use per
        // frame through `FrameFlags` instead.
        None,
        allocator,
        DmabufExporter::new(device.gbm.clone(), Some(device.render_node)),
        COLOR_FORMATS.iter().copied(),
        renderer_formats,
        device.drm.cursor_size(),
        Some(device.gbm.clone()),
    )
    .with_context(|| format!("failed to create the DRM compositor for {}", head.name))?;

    tracing::info!(output = head.name, ?mode, %node, "monitor enabled");

    device.surfaces.insert(
        crtc,
        Surface {
            output,
            node,
            render_node: device.render_node,
            compositor,
            frame_clock: FrameClock::new(refresh_interval, false),
            // First frame right away.
            redraw_state: RedrawState::Queued,
            blur: BlurCache::default(),
            powered: true,
        },
    );
    if let Some(head) = device.heads.get_mut(&connector) {
        head.state = HeadState::Enabled { crtc };
    }

    state.refresh_output_heads();

    Ok(())
}

/// Switches a head off without forgetting it exists.
///
/// The `Output` goes — which migrates its workspaces and closes its layer
/// surfaces — and dropping the `Surface` is what actually frees the hardware:
/// `AtomicDrmSurface`'s `Drop` resets the planes, connector and CRTC in one
/// modeset, and releases the primary-plane claim that `create_surface` took.
/// Nothing else can free the CRTC for another connector to use.
pub fn disable_head(state: &mut State, node: DrmNode, connector: connector::Handle) {
    let Some(crtc) = state
        .backend
        .kms()
        .and_then(|kms| kms.devices.get(&node))
        .and_then(|device| device.heads.get(&connector))
        .and_then(|head| head.state.crtc())
    else {
        return;
    };

    remove_surface(state, node, crtc);

    if let Some(head) = state
        .backend
        .kms()
        .and_then(|kms| kms.devices.get_mut(&node))
        .and_then(|device| device.heads.get_mut(&connector))
    {
        head.state = HeadState::Disabled;
        tracing::info!(output = head.name, %node, "monitor disabled");
    }

    state.refresh_output_heads();
}

/// Takes down the surface on `crtc`, detaching its output from the shell.
///
/// Shared by disabling a head and unplugging one; the difference is only
/// whether the head stays in the registry afterwards.
fn remove_surface(state: &mut State, node: DrmNode, crtc: crtc::Handle) {
    let handle = state.common.event_loop_handle.clone();
    let display_handle = state.common.display_handle.clone();
    let State { backend, shell, .. } = state;
    let Some(kms) = backend.kms() else {
        return;
    };
    let Some(device) = kms.devices.get_mut(&node) else {
        return;
    };
    let Some(surface) = device.surfaces.remove(&crtc) else {
        return;
    };

    // A parked estimated-vblank timer would fire into a dead output.
    match surface.redraw_state {
        RedrawState::WaitingForEstimatedVBlank(token)
        | RedrawState::WaitingForEstimatedVBlankAndQueued(token) => handle.remove(token),
        _ => {}
    }

    shell.remove_output(&display_handle, &surface.output);
}

/// Forgets a connector that has been unplugged.
pub fn connector_disconnected(state: &mut State, node: DrmNode, connector: connector::Handle) {
    let head = state
        .backend
        .kms()
        .and_then(|kms| kms.devices.get(&node))
        .and_then(|device| device.heads.get(&connector))
        .cloned();
    let Some(head) = head else {
        return;
    };

    if let Some(crtc) = head.state.crtc() {
        remove_surface(state, node, crtc);
    }

    if let Some(device) = state
        .backend
        .kms()
        .and_then(|kms| kms.devices.get_mut(&node))
    {
        device.heads.remove(&connector);
    }

    tracing::info!(output = head.name, %node, "monitor disconnected");
    state.refresh_output_heads();
}

/// Renders every output whose state machine says "queued". Called once per
/// event-loop dispatch, which is how a `queue_redraw` from *inside* event
/// handling turns into a frame *after* all events have been drained —
/// coalescing a burst of commits into one render pass.
pub fn redraw_queued_outputs(state: &mut State) {
    loop {
        let Some(kms) = state.backend.kms() else {
            return;
        };
        let Some((node, crtc)) = kms.devices.iter().find_map(|(node, device)| {
            device
                .surfaces
                .iter()
                .find(|(_, surface)| surface.redraw_state.is_queued())
                .map(|(crtc, _)| (*node, *crtc))
        }) else {
            return;
        };
        render_surface(state, node, crtc);
    }
}

/// One frame for one output.
fn render_surface(state: &mut State, node: DrmNode, crtc: crtc::Handle) {
    let handle = state.common.event_loop_handle.clone();

    // The menu geometry is what both the renderer and the hit test read, so it
    // is settled before either of them runs — and before the borrows below,
    // because laying it out needs the whole state.
    let scale = state
        .backend
        .kms()
        .and_then(|kms| kms.devices.get(&node))
        .and_then(|device| device.surfaces.get(&crtc))
        .map(|surface| surface.output.current_scale().fractional_scale());
    if let Some(scale) = scale {
        state.layout_menus(scale);
    }

    let State {
        backend,
        shell,
        common,
        config,
        clock,
        input,
        text,
        ..
    } = state;
    let blur_config = BlurConfig::from(&config.current.appearance);
    let Some(kms) = backend.kms() else {
        return;
    };

    // Field-split so the renderer (gpu_manager) and the surface (devices) can
    // be borrowed at the same time.
    let KmsState {
        devices,
        gpu_manager,
        primary_render_node,
        ..
    } = kms;
    let Some(device) = devices.get_mut(&node) else {
        return;
    };
    let Some(surface) = device.surfaces.get_mut(&crtc) else {
        return;
    };

    // A blanked panel has no CRTC to commit to. The request is dropped rather
    // than parked, because waking up queues its own redraw.
    if !surface.powered {
        surface.redraw_state = RedrawState::Idle;
        return;
    }

    // Consume the render request, keeping hold of a still-pending estimated
    // vblank timer: whether it is kept, cancelled or replaced depends on how
    // this frame goes.
    let estimated_vblank = match std::mem::take(&mut surface.redraw_state) {
        RedrawState::Queued => None,
        RedrawState::WaitingForEstimatedVBlankAndQueued(token) => Some(token),
        other => {
            // Not queued; nothing asked for this frame.
            surface.redraw_state = other;
            return;
        }
    };

    if !device.drm.is_active() {
        // Another VT owns the hardware; rendering resumes on ActivateSession.
        if let Some(token) = estimated_vblank {
            handle.remove(token);
        }
        return;
    }

    // Animate to the instant this frame will *reach the screen*, not to
    // "now": springs get sampled at display time, which is what keeps motion
    // velocity constant even when render times wobble.
    let target_presentation_time = surface.frame_clock.next_presentation_time();
    let dt = clock.tick_to(target_presentation_time);
    shell.advance_animations(dt);
    let animating = shell.is_animating();
    if !animating {
        shell.settle_animations();
    }

    let mut renderer = match gpu_manager.renderer(
        primary_render_node,
        &surface.render_node,
        surface.compositor.format(),
    ) {
        Ok(renderer) => renderer,
        Err(err) => {
            tracing::warn!(%err, "failed to create the multi renderer");
            queue_estimated_vblank_timer(
                &handle,
                surface,
                node,
                crtc,
                estimated_vblank,
                target_presentation_time,
                animating,
            );
            return;
        }
    };

    let Some(monitor) = shell.monitor(&surface.output) else {
        if let Some(token) = estimated_vblank {
            handle.remove(token);
        }
        return;
    };
    let scale = Scale::from(surface.output.current_scale().fractional_scale());

    // Backdrops blur the framebuffer as they are drawn, so all the frame owes
    // them is the pyramids they cached last time and the bounds to clip to.
    let transform = surface.output.current_transform();
    let bounds: Rectangle<i32, Physical> =
        Rectangle::from_size(monitor.geometry().size.to_physical_precise_round(scale));
    surface.blur.begin_frame();
    let blur = blur_config.enabled.then_some(BlurSession {
        cache: &mut surface.blur,
        config: blur_config,
        transform,
        output: bounds,
    });

    let elements = rendering::output_elements(
        shell,
        monitor,
        &mut renderer,
        &mut MultiDecorator::new(blur),
        &mut input.cursor,
        input.pointer_location,
        scale,
        &mut FrameStyle::new(
            &config.current.appearance,
            scale.y,
            monitor.geometry().loc,
            text,
            shell.focused_window_id(),
            input.hovered_control,
        ),
    );

    let mut submitted = false;
    match surface.compositor.render_frame(
        &mut renderer,
        &elements,
        CLEAR_COLOR,
        FrameFlags::DEFAULT,
    ) {
        Ok(result) => {
            if result.needs_sync() {
                // The swapchain buffer is still being written by the GPU;
                // queueing it unfinished would tear.
                if let PrimaryPlaneElement::Swapchain(element) = &result.primary_element
                    && let Err(err) = element.sync.wait()
                {
                    tracing::warn!(?err, "failed to wait for frame completion");
                }
            }

            if !result.is_empty {
                let feedback = take_presentation_feedbacks(shell, &surface.output);
                match surface.compositor.queue_frame(feedback) {
                    Ok(()) => submitted = true,
                    Err(err) => {
                        tracing::warn!(%err, "failed to queue frame");
                    }
                }
            }
        }
        Err(err) => {
            // Happens legitimately across VT switches.
            tracing::warn!(%err, "failed to render frame");
        }
    }

    // Frame callbacks: what lets clients draw their *next* frame. Sent whether
    // or not this frame had damage, so a client animation never stalls.
    let now = common.start_time.elapsed();
    let throttle = Some(Duration::ZERO);
    for tile in shell.visible_windows(monitor) {
        tile.window()
            .send_frame(&surface.output, now, throttle, |_, _| {
                Some(surface.output.clone())
            });
    }
    {
        let map = layer_map_for_output(&surface.output);
        for layer in map.layers() {
            layer.send_frame(&surface.output, now, throttle, |_, _| {
                Some(surface.output.clone())
            });
        }
    }
    // A client's cursor surface is in neither the shell model nor the layer
    // map, so it needs its own callback or an animated cursor draws once.
    input.cursor.send_frame(&surface.output, now, throttle);

    if submitted {
        // The real vblank takes over: it anchors the frame clock and decides
        // whether another frame follows.
        if let Some(token) = estimated_vblank {
            handle.remove(token);
        }
        surface.redraw_state = RedrawState::WaitingForVBlank {
            redraw_needed: false,
        };
    } else {
        // Nothing reached KMS, so no vblank will fire. A timer at the
        // estimated presentation instant keeps the cycle honest: animations
        // tick at the refresh rate instead of as fast as the CPU can loop,
        // and an idle output schedules nothing at all after it fires.
        queue_estimated_vblank_timer(
            &handle,
            surface,
            node,
            crtc,
            estimated_vblank,
            target_presentation_time,
            animating,
        );
    }

    // A backdrop that owes a halo has to be asked for the frame that pays it;
    // nothing else on screen changed, so no other source would schedule one.
    if surface.blur.wants_redraw() {
        surface.redraw_state = std::mem::take(&mut surface.redraw_state).queue();
    }
}

/// Parks the redraw cycle on a timer that fires when the vblank *would have*.
fn queue_estimated_vblank_timer(
    handle: &calloop::LoopHandle<'static, State>,
    surface: &mut Surface,
    node: DrmNode,
    crtc: crtc::Handle,
    existing: Option<RegistrationToken>,
    target_presentation_time: Duration,
    animating: bool,
) {
    if let Some(token) = existing {
        // The timer from the previous no-damage frame has not fired yet;
        // rescheduling would only push the tick later.
        surface.redraw_state = RedrawState::WaitingForEstimatedVBlank(token);
        return;
    }

    let duration = target_presentation_time.saturating_sub(surface.frame_clock.now());
    match handle.insert_source(Timer::from_duration(duration), move |_, _, state| {
        on_estimated_vblank(state, node, crtc);
        TimeoutAction::Drop
    }) {
        Ok(token) => {
            surface.redraw_state = RedrawState::WaitingForEstimatedVBlank(token);
        }
        Err(err) => {
            // Pathological; degrade to an immediate requeue so a running
            // animation survives, even if it burns CPU until the next real
            // frame lands.
            tracing::error!(%err, "failed to schedule the estimated vblank timer");
            surface.redraw_state = if animating {
                RedrawState::Queued
            } else {
                RedrawState::Idle
            };
        }
    }
}

/// The stand-in for a vblank that never came: keep animations ticking, or go
/// back to sleep.
fn on_estimated_vblank(state: &mut State, node: DrmNode, crtc: crtc::Handle) {
    let State { backend, shell, .. } = state;
    let Some(kms) = backend.kms() else {
        return;
    };
    let Some(surface) = kms
        .devices
        .get_mut(&node)
        .and_then(|device| device.surfaces.get_mut(&crtc))
    else {
        return;
    };

    match std::mem::take(&mut surface.redraw_state) {
        // The token died with this firing.
        RedrawState::WaitingForEstimatedVBlank(_) => {}
        RedrawState::WaitingForEstimatedVBlankAndQueued(_) => {
            surface.redraw_state = RedrawState::Queued;
            return;
        }
        other => {
            tracing::warn!(state = ?other, "estimated vblank in unexpected redraw state");
            surface.redraw_state = other;
            return;
        }
    }

    if shell.is_animating() {
        surface.redraw_state = RedrawState::Queued;
    }
}

/// The hardware told us a frame lit up: anchor the frame clock, resolve
/// presentation feedback, and keep the redraw cycle going if anything wants
/// another frame.
pub fn on_vblank(
    state: &mut State,
    node: DrmNode,
    crtc: crtc::Handle,
    metadata: Option<DrmEventMetadata>,
) {
    let State { backend, shell, .. } = state;
    let Some(kms) = backend.kms() else {
        return;
    };
    let Some(device) = kms.devices.get_mut(&node) else {
        return;
    };
    let Some(surface) = device.surfaces.get_mut(&crtc) else {
        return;
    };

    // The moment of presentation, straight from the driver when it can say.
    let (presentation_time, sequence) = match &metadata {
        Some(metadata) => {
            let time = match metadata.time {
                DrmEventTime::Monotonic(time) => time,
                // A realtime stamp is useless against a monotonic clock.
                DrmEventTime::Realtime(_) => Duration::ZERO,
            };
            (time, metadata.sequence)
        }
        None => (Duration::ZERO, 0),
    };

    match surface.compositor.frame_submitted() {
        Ok(Some(mut feedback)) => {
            let time = if presentation_time.is_zero() {
                surface.frame_clock.now()
            } else {
                presentation_time
            };
            let refresh = surface
                .frame_clock
                .refresh_interval()
                .map(Refresh::Fixed)
                .unwrap_or(Refresh::Unknown);
            let flags = wp_presentation_feedback::Kind::Vsync
                | wp_presentation_feedback::Kind::HwCompletion
                | wp_presentation_feedback::Kind::HwClock;
            feedback.presented::<_, smithay::utils::Monotonic>(
                time,
                refresh,
                sequence as u64,
                flags,
            );
        }
        Ok(None) => {}
        Err(err) => {
            tracing::warn!(%err, "failed to mark frame as submitted");
        }
    }

    surface.frame_clock.presented(presentation_time);

    let redraw_needed = match std::mem::take(&mut surface.redraw_state) {
        RedrawState::WaitingForVBlank { redraw_needed } => redraw_needed,
        other => {
            // Only reachable through driver bugs (spurious vblank); recover
            // rather than poison the state machine.
            tracing::warn!(state = ?other, "vblank in unexpected redraw state");
            false
        }
    };

    if redraw_needed || shell.is_animating() {
        surface.redraw_state = RedrawState::Queued;
    }
}

/// Collects every visible surface's presentation-feedback callback for this
/// output, to be resolved when the frame's vblank arrives.
fn take_presentation_feedbacks(shell: &Shell, output: &Output) -> OutputPresentationFeedback {
    let mut feedback = OutputPresentationFeedback::new(output);

    let flags = |_: &_, _: &_| {
        wp_presentation_feedback::Kind::Vsync | wp_presentation_feedback::Kind::HwCompletion
    };

    if let Some(monitor) = shell.monitor(output) {
        for tile in shell.visible_windows(monitor) {
            tile.window().take_presentation_feedback(
                &mut feedback,
                |_, _| Some(output.clone()),
                flags,
            );
        }
    }

    let map = layer_map_for_output(output);
    for layer in map.layers() {
        layer.take_presentation_feedback(&mut feedback, |_, _| Some(output.clone()), flags);
    }

    feedback
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_from_idle_renders() {
        assert!(matches!(RedrawState::Idle.queue(), RedrawState::Queued));
    }

    #[test]
    fn queue_is_idempotent() {
        assert!(matches!(RedrawState::Queued.queue(), RedrawState::Queued));
    }

    #[test]
    fn queue_while_a_frame_is_in_flight_defers_to_the_vblank() {
        // No double-render: the request rides the pending frame's vblank.
        assert!(matches!(
            RedrawState::WaitingForVBlank {
                redraw_needed: false
            }
            .queue(),
            RedrawState::WaitingForVBlank {
                redraw_needed: true
            }
        ));
    }

    #[test]
    fn queued_states_report_queued() {
        assert!(RedrawState::Queued.is_queued());
        assert!(!RedrawState::Idle.is_queued());
        assert!(
            !RedrawState::WaitingForVBlank {
                redraw_needed: true
            }
            .is_queued(),
            "a deferred redraw must wait for its vblank, not render now"
        );
    }
}
