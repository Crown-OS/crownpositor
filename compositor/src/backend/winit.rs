use std::time::Duration;

use anyhow::{Context, anyhow};
use smithay::{
    backend::{
        egl::EGLDevice,
        renderer::{ImportDma, damage::OutputDamageTracker, gles::GlesRenderer},
        winit::{self, WinitEvent, WinitGraphicsBackend},
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    utils::{Clock as MonotonicClock, Monotonic, Physical, Rectangle, Scale, Transform},
    wayland::dmabuf::{DmabufFeedback, DmabufFeedbackBuilder, DmabufGlobal},
};

use crate::rendering::corners::CornerMemory;
use crate::{
    backend::{
        present::{self, ReleasedClients},
        render::CrownRenderer as _,
    },
    rendering::{
        self, FrameStyle,
        blur::{BlurCache, BlurConfig, BlurSession},
        rounded::GlesDecorator,
    },
    shell::monitor::OutputDescriptor,
    state::{BackendState, State},
};

const REFRESH_RATE: i32 = 60_000;
const CLEAR_COLOR: [f32; 4] = [0.1, 0.1, 0.1, 1.0];

pub struct WinitState {
    pub backend: WinitGraphicsBackend<GlesRenderer>,
    pub output: Output,
    pub damage_tracker: OutputDamageTracker,
    pub dmabuf_global: DmabufGlobal,
    pub dmabuf_feedback: Option<DmabufFeedback>,
    /// Every backdrop's blur pyramid on this output, kept across frames.
    pub blur: BlurCache,
    pub corners: CornerMemory,
}

pub fn init(state: &mut State) -> anyhow::Result<()> {
    let (mut backend, winit_events) = winit::init::<GlesRenderer>()
        .map_err(|err| anyhow!("Failed to initialize the winit backend: {err}"))?;

    let mode = Mode {
        size: backend.window_size(),
        refresh: REFRESH_RATE,
    };

    // The backend describes the output; the shell builds it. That is what makes
    // registration impossible to forget.
    let output = state.shell.add_output(
        &state.common.display_handle,
        &state.config.current,
        OutputDescriptor {
            name: "winit".to_string(),
            physical: PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "Crownpositor".into(),
                model: "Winit".into(),
                serial_number: "Unknown".into(),
            },
            modes: vec![mode],
            preferred: Some(mode),
            current: mode,
            // Winit renders bottom-up, so the output is flipped to compensate.
            native_transform: Transform::Flipped180,
            refresh_interval: Some(Duration::from_nanos(
                1_000_000_000_000 / REFRESH_RATE as u64,
            )),
            serial: None,
            edid: None,
        },
    );

    state.refresh_output_heads();
    crate::backend::debug_frames::init(&output);

    if let Err(err) = backend.renderer().compile_shaders() {
        // Cosmetic, so a compile failure degrades to square corners / no blur.
        tracing::warn!(%err, "failed to compile the effect shaders");
    }

    let (dmabuf_global, dmabuf_feedback) = init_dmabuf(state, &mut backend);

    let damage_tracker = OutputDamageTracker::from_output(&output);
    state.backend = BackendState::Winit(Box::new(WinitState {
        backend,
        output,
        damage_tracker,
        dmabuf_global,
        dmabuf_feedback,
        blur: BlurCache::default(),
        corners: CornerMemory::default(),
    }));

    state
        .common
        .event_loop_handle
        .insert_source(winit_events, move |event, _, state| match event {
            // Winit re-emits Resized on focus changes with an unchanged size,
            // so `set_mode` short-circuits rather than reconfiguring everything.
            WinitEvent::Resized { size, .. } => {
                let Some(output) = state.backend.winit().map(|winit| winit.output.clone()) else {
                    return;
                };
                let mode = Mode {
                    size,
                    refresh: REFRESH_RATE,
                };
                if state
                    .shell
                    .monitor_mut(&output)
                    .is_some_and(|monitor| monitor.set_mode(mode))
                {
                    state.shell.arrange_outputs();
                    state.shell.refresh_usable(&output);
                }
            }
            WinitEvent::Input(event) => state.process_input_event(event),
            WinitEvent::Redraw => {
                let released = signal_commit_timers(state);
                state.clear_blockers(released);
                match render(state) {
                    Ok(released) => state.clear_blockers(released),
                    Err(err) => tracing::error!(?err, "Failed to render the winit output"),
                }
            }
            WinitEvent::CloseRequested => state.common.event_loop_signal.stop(),
            WinitEvent::Focus(_) => {}
        })
        .map_err(|err| anyhow!("Failed to insert the winit event source: {err}"))?;

    Ok(())
}

/// Advertises the renderer's dmabuf formats, preferring v4 feedback and falling back to v3.
fn init_dmabuf(
    state: &mut State,
    backend: &mut WinitGraphicsBackend<GlesRenderer>,
) -> (DmabufGlobal, Option<DmabufFeedback>) {
    let display = &state.common.display_handle;
    let dmabuf_state = &mut state.wayland.dmabuf_state;
    let formats = backend.renderer().dmabuf_formats();

    let render_node = EGLDevice::device_for_display(backend.renderer().egl_context().display())
        .and_then(|device| device.try_get_render_node());

    let feedback = match render_node {
        Ok(Some(node)) => DmabufFeedbackBuilder::new(node.dev_id(), formats.clone())
            .build()
            .ok(),
        Ok(None) => None,
        Err(err) => {
            tracing::warn!(
                ?err,
                "Failed to query the EGL device, falling back to dmabuf v3"
            );
            None
        }
    };

    match feedback {
        Some(feedback) => {
            let global =
                dmabuf_state.create_global_with_default_feedback::<State>(display, &feedback);
            (global, Some(feedback))
        }
        None => {
            tracing::warn!("No render node available, falling back to dmabuf v3");
            (dmabuf_state.create_global::<State>(display, formats), None)
        }
    }
}

/// Winit has no presentation clock, so "the frame about to be drawn" is now.
/// A later timed commit still waiting keeps frames coming.
fn signal_commit_timers(state: &mut State) -> ReleasedClients {
    let mut released = ReleasedClients::default();
    let State {
        backend,
        shell,
        input,
        wayland,
        ..
    } = state;
    if let Some(winit) = backend.winit() {
        let presentation_time = wayland.clock.now().into();
        let waiting = present::signal_commit_timers(
            shell,
            &winit.output,
            &input.cursor,
            presentation_time,
            &mut released,
        );
        if waiting {
            winit.backend.window().request_redraw();
        }
    }
    released
}

/// Draws the one window. Returns the clients whose FIFO barriers it released.
fn render(state: &mut State) -> anyhow::Result<ReleasedClients> {
    let mut released = ReleasedClients::default();
    let Some(scale) = state
        .backend
        .winit()
        .map(|winit| Scale::from(winit.output.current_scale().fractional_scale()))
    else {
        return Ok(released);
    };

    // The menu geometry is what both the renderer and the hit test read, so it
    // is settled before either of them runs — and before the borrows below,
    // because laying it out needs the whole state.
    state.layout_menus(scale.y);
    crate::backend::debug_frames::script(state);

    let State {
        backend,
        common,
        shell,
        clock,
        config,
        input,
        text,
        capture,
        ..
    } = state;

    let Some(winit) = backend.winit() else {
        return Ok(released);
    };

    // A hardcoded age of 0 makes every frame a full repaint.
    let age = winit.backend.buffer_age().unwrap_or(0);

    let dt = clock.tick();
    shell.advance_animations(dt);
    shell
        .surface_animations
        .advance(MonotonicClock::<Monotonic>::new().now().into());
    let animating = shell.is_animating();
    if !animating {
        // Land the last frame on exact integers rather than resting a fraction
        // of a pixel off, where the spring's epsilon stopped it.
        shell.settle_animations();
    }

    let blur_config = BlurConfig::new(&config.current.appearance, &config.current.glass);
    let transform = winit.output.current_transform();

    let submitted = {
        let (renderer, mut framebuffer) = winit
            .backend
            .bind()
            .map_err(|err| anyhow!("Failed to bind the winit framebuffer: {err}"))?;

        let Some(monitor) = shell.monitor(&winit.output) else {
            return Ok(released);
        };

        // Backdrops blur the framebuffer as they are drawn, so all the frame
        // owes them is the pyramids they cached last time and the bounds to
        // clip to.
        let bounds: Rectangle<i32, Physical> =
            Rectangle::from_size(monitor.geometry().size.to_physical_precise_round(scale));
        winit.blur.begin_frame();
        winit.corners.begin_frame();
        let content = winit.blur.content();
        let blur = blur_config.enabled.then_some(BlurSession {
            cache: &mut winit.blur,
            config: blur_config,
            transform,
            output: bounds,
        });

        let elements = rendering::output_elements(
            shell,
            monitor,
            renderer,
            &mut GlesDecorator::new(blur, &mut winit.corners),
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

        content.observe(
            || OutputDamageTracker::from_output(&winit.output),
            &elements,
        );
        let result = winit
            .damage_tracker
            .render_output(renderer, &mut framebuffer, age, &elements, CLEAR_COLOR)
            .with_context(|| "Failed to render the output")?;

        present::track_primary_scanout(shell, &winit.output, &input.cursor, &result.states);
        if shell.session_lock.is_active() {
            shell.session_lock.output_blanked(&winit.output);
        }
        let damage = result.damage.cloned();
        drop(elements);
        if damage.as_ref().is_some_and(|damage| !damage.is_empty()) {
            crate::backend::debug_frames::grab(
                renderer,
                &framebuffer,
                &winit.output,
                damage.as_deref(),
                age,
            );
        }
        damage
    };
    if submitted.as_ref().is_some_and(|damage| !damage.is_empty()) {
        capture.mark_output_damaged(&winit.output);
    }

    winit
        .backend
        .submit(submitted.as_deref())
        .map_err(|err| anyhow!("Failed to submit the winit frame: {err}"))?;
    if crate::backend::debug_frames::compare(
        (
            shell,
            &mut input.cursor,
            input.pointer_location,
            &config.current.appearance,
            text,
            input.hovered_control,
        ),
        winit.backend.renderer(),
        &winit.output,
    ) && let Ok((renderer, framebuffer)) = winit.backend.bind()
    {
        crate::backend::debug_frames::rebind(renderer, &framebuffer);
    }

    let now = common.start_time.elapsed();
    present::send_frame_callbacks(shell, &winit.output, &input.cursor, now);
    present::signal_fifo_barriers(shell, &winit.output, &input.cursor, &mut released);

    // Winit only redraws on demand. Scheduling only while something is moving is
    // what takes an idle desktop from a permanent 60 Hz loop to ~0% CPU; a client
    // that damages its surface wakes us through its own commit — except for a
    // backdrop's owed halo, which nothing else would schedule a frame for.
    if animating || winit.blur.wants_redraw() || std::env::var_os("CROWN_DEBUG_SPIN").is_some() {
        winit.backend.window().request_redraw();
    }

    Ok(released)
}
