use std::time::Duration;

use anyhow::{Context, anyhow};
use smithay::{
    backend::{
        egl::EGLDevice,
        renderer::{ImportDma, damage::OutputDamageTracker, gles::GlesRenderer},
        winit::{self, WinitEvent, WinitGraphicsBackend},
    },
    desktop::layer_map_for_output,
    output::{Mode, Output, PhysicalProperties, Subpixel},
    utils::{Scale, Transform},
    wayland::dmabuf::{DmabufFeedback, DmabufFeedbackBuilder, DmabufGlobal},
};

use crate::{
    backend::render::CrownRenderer as _,
    rendering::{
        self,
        blur::{self, BlurBuffers, BlurConfig},
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
    /// The cached blurred-background pipeline for this output.
    pub blur: BlurBuffers,
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
        },
    );

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
        blur: BlurBuffers::default(),
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
                if let Err(err) = render(state) {
                    tracing::error!(?err, "Failed to render the winit output");
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

fn render(state: &mut State) -> anyhow::Result<()> {
    let State {
        backend,
        common,
        shell,
        clock,
        config,
        input,
        ..
    } = state;
    // Physical pixels, because that is the space the shader works in.
    let radius = config.current.appearance.border_radius as f32;

    let Some(winit) = backend.winit() else {
        return Ok(());
    };

    let scale = Scale::from(winit.output.current_scale().fractional_scale());
    // A hardcoded age of 0 makes every frame a full repaint.
    let age = winit.backend.buffer_age().unwrap_or(0);

    let dt = clock.tick();
    shell.advance_animations(dt);
    let animating = shell.is_animating();
    if !animating {
        // Land the last frame on exact integers rather than resting a fraction
        // of a pixel off, where the spring's epsilon stopped it.
        shell.settle_animations();
    }

    // The blur pre-pass renders offscreen, so it has to run before the main
    // framebuffer is bound. Skipped — a cheap scan — while no visible window
    // has a blur region committed.
    let blur_config = BlurConfig::from(&config.current.appearance);
    let backdrop = {
        let Some(monitor) = shell.monitor(&winit.output) else {
            return Ok(());
        };
        let wants_blur = blur_config.enabled
            && shell
                .visible_windows(monitor)
                .any(|tile| blur::window_blur_bounds(tile.window()).is_some());
        if wants_blur {
            let size: smithay::utils::Size<i32, smithay::utils::Physical> =
                monitor.geometry().size.to_physical_precise_round(scale);
            let renderer = winit.backend.renderer();
            let sources = blur::source_elements(monitor, renderer, scale);
            match winit
                .blur
                .update(renderer, size, scale, &sources, CLEAR_COLOR, &blur_config)
            {
                Ok(()) => winit.blur.source(blur_config.noise),
                Err(err) => {
                    tracing::warn!(%err, "blur pre-pass failed; drawing without blur");
                    None
                }
            }
        } else {
            None
        }
    };

    let submitted = {
        let (renderer, mut framebuffer) = winit
            .backend
            .bind()
            .map_err(|err| anyhow!("Failed to bind the winit framebuffer: {err}"))?;

        let Some(monitor) = shell.monitor(&winit.output) else {
            return Ok(());
        };
        let elements = rendering::output_elements(
            shell,
            monitor,
            renderer,
            &mut GlesDecorator::new(backdrop),
            &mut input.cursor,
            input.pointer_location,
            scale,
            radius,
        );

        let result = winit
            .damage_tracker
            .render_output(renderer, &mut framebuffer, age, &elements, CLEAR_COLOR)
            .with_context(|| "Failed to render the output")?;

        result.damage.cloned()
    };

    winit
        .backend
        .submit(submitted.as_deref())
        .map_err(|err| anyhow!("Failed to submit the winit frame: {err}"))?;

    let now = common.start_time.elapsed();
    let throttle = Some(Duration::ZERO);

    if let Some(monitor) = shell.monitor(&winit.output) {
        for tile in shell.visible_windows(monitor) {
            tile.window()
                .send_frame(&winit.output, now, throttle, |_, _| {
                    Some(winit.output.clone())
                });
        }
    }

    // Without these a bar with a clock renders once and freezes.
    {
        let map = layer_map_for_output(&winit.output);
        for layer in map.layers() {
            layer.send_frame(&winit.output, now, throttle, |_, _| {
                Some(winit.output.clone())
            });
        }
    }
    input.cursor.send_frame(&winit.output, now, throttle);

    // Winit only redraws on demand. Scheduling only while something is moving is
    // what takes an idle desktop from a permanent 60 Hz loop to ~0% CPU; a client
    // that damages its surface wakes us through its own commit.
    if animating {
        winit.backend.window().request_redraw();
    }

    Ok(())
}
