use std::time::Duration;

use anyhow::{Context, anyhow};
use smithay::{
    backend::{
        egl::EGLDevice,
        renderer::{
            ImportDma, damage::OutputDamageTracker, element::surface::WaylandSurfaceRenderElement,
            gles::GlesRenderer,
        },
        winit::{self, WinitEvent, WinitGraphicsBackend},
    },
    desktop::space::render_output,
    output::{Mode, Output, PhysicalProperties, Subpixel},
    utils::{Rectangle, Transform},
    wayland::dmabuf::{DmabufFeedback, DmabufFeedbackBuilder, DmabufGlobal},
};

use crate::state::{BackendState, State};

const REFRESH_RATE: i32 = 60_000;
const CLEAR_COLOR: [f32; 4] = [0.1, 0.1, 0.1, 1.0];

pub struct WinitState {
    pub backend: WinitGraphicsBackend<GlesRenderer>,
    pub output: Output,
    pub damage_tracker: OutputDamageTracker,
    pub dmabuf_global: DmabufGlobal,
    pub dmabuf_feedback: Option<DmabufFeedback>,
}

pub fn init(state: &mut State) -> anyhow::Result<()> {
    let (mut backend, winit_events) = winit::init::<GlesRenderer>()
        .map_err(|err| anyhow!("Failed to initialize the winit backend: {err}"))?;

    let mode = Mode {
        size: backend.window_size(),
        refresh: REFRESH_RATE,
    };

    let output = Output::new(
        "winit".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Crownpositor".into(),
            model: "Winit".into(),
        },
    );
    output.create_global::<State>(&state.common.display_handle);
    // Winit renders bottom-up, so the output is flipped to compensate.
    output.change_current_state(
        Some(mode),
        Some(Transform::Flipped180),
        None,
        Some((0, 0).into()),
    );
    output.set_preferred(mode);

    state.shell.space.map_output(&output, (0, 0));

    let (dmabuf_global, dmabuf_feedback) = init_dmabuf(state, &mut backend);

    let damage_tracker = OutputDamageTracker::from_output(&output);
    state.backend = BackendState::Winit(Box::new(WinitState {
        backend,
        output,
        damage_tracker,
        dmabuf_global,
        dmabuf_feedback,
    }));

    state
        .common
        .event_loop_handle
        .insert_source(winit_events, move |event, _, state| match event {
            WinitEvent::Resized { size, .. } => {
                if let Some(winit) = state.backend.winit() {
                    winit.output.change_current_state(
                        Some(Mode {
                            size,
                            refresh: REFRESH_RATE,
                        }),
                        None,
                        None,
                        None,
                    );
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
        ..
    } = state;

    let Some(winit) = backend.winit() else {
        return Ok(());
    };

    let damage = Rectangle::from_size(winit.backend.window_size());

    {
        let (renderer, mut framebuffer) = winit
            .backend
            .bind()
            .map_err(|err| anyhow!("Failed to bind the winit framebuffer: {err}"))?;

        render_output::<_, WaylandSurfaceRenderElement<GlesRenderer>, _, _>(
            &winit.output,
            renderer,
            &mut framebuffer,
            1.0,
            0,
            [&shell.space],
            &[],
            &mut winit.damage_tracker,
            CLEAR_COLOR,
        )
        .with_context(|| "Failed to render the output")?;
    }

    winit
        .backend
        .submit(Some(&[damage]))
        .map_err(|err| anyhow!("Failed to submit the winit frame: {err}"))?;

    for window in shell.space.elements() {
        window.send_frame(
            &winit.output,
            common.start_time.elapsed(),
            Some(Duration::ZERO),
            |_, _| Some(winit.output.clone()),
        );
    }

    // Winit only redraws on demand, so keep the frame loop going.
    winit.backend.window().request_redraw();

    Ok(())
}
