//! The nested backend: one host window, split into one or more viewports.

mod edid;
mod hotplug;
mod render;
mod viewport;

pub use hotplug::{set_head_enabled, toggle_connection};

use edid::nested_edid;
use hotplug::bring_up_all;

use anyhow::anyhow;
use smithay::{
    backend::{
        egl::EGLDevice,
        input::{AbsolutePositionEvent, Event, InputEvent},
        renderer::{ImportDma, damage::OutputDamageTracker, gles::GlesRenderer},
        winit::{self, WinitEvent, WinitGraphicsBackend, WinitMouseMovedEvent},
    },
    output::Output,
    utils::{Physical, Point, Size, Transform},
    wayland::dmabuf::{DmabufFeedback, DmabufFeedbackBuilder, DmabufGlobal},
};

use crate::{
    backend::{
        present::{self, ReleasedClients},
        render::CrownRenderer as _,
    },
    shell::monitor::output_from_descriptor,
    state::{BackendState, DisabledHead, State},
};

use viewport::{Connection, Viewport, descriptor, mode_of, split_window, viewport_names};

pub struct WinitState {
    pub backend: WinitGraphicsBackend<GlesRenderer>,
    pub viewports: Vec<Viewport>,
    /// Composites a split window's viewports. Unused with one viewport.
    window_damage_tracker: OutputDamageTracker,
    window_size: Size<i32, Physical>,
    /// The age of the window's next buffer, read after the last swap.
    window_buffer_age: usize,
    pub dmabuf_global: DmabufGlobal,
    pub dmabuf_feedback: Option<DmabufFeedback>,
}

impl WinitState {
    /// Whether the window holds more than one viewport, each drawn into its
    /// own texture first.
    pub fn is_split(&self) -> bool {
        self.viewports.len() > 1
    }

    /// One viewport draws straight into the window, which renders bottom-up.
    fn output_transform(&self) -> Transform {
        output_transform(self.viewports.len())
    }

    pub fn queue_redraw(&mut self, output: Option<&Output>) {
        let mut queued = false;
        for viewport in &mut self.viewports {
            if output.is_none_or(|output| *output == viewport.output) {
                viewport.dirty = true;
                queued = true;
            }
        }
        if queued {
            self.backend.window().request_redraw();
        }
    }

    fn mark_all_dirty(&mut self) {
        for viewport in &mut self.viewports {
            viewport.dirty = true;
        }
    }

    pub fn disabled_heads(&self) -> Vec<DisabledHead> {
        self.viewports
            .iter()
            .filter(|viewport| viewport.connection == Connection::Disabled)
            .map(Viewport::disabled_head)
            .collect()
    }

    fn connected_outputs(&self) -> Vec<Output> {
        self.viewports
            .iter()
            .filter(|viewport| viewport.is_connected())
            .map(|viewport| viewport.output.clone())
            .collect()
    }
}

fn output_transform(viewport_count: usize) -> Transform {
    match viewport_count {
        1 => Transform::Flipped180,
        _ => Transform::Normal,
    }
}

fn window_damage_tracker(size: Size<i32, Physical>) -> OutputDamageTracker {
    OutputDamageTracker::new(size, 1.0, Transform::Flipped180)
}

pub fn init(state: &mut State) -> anyhow::Result<()> {
    let (mut backend, winit_events) = winit::init::<GlesRenderer>()
        .map_err(|err| anyhow!("Failed to initialize the winit backend: {err}"))?;

    let window_size = backend.window_size();
    let names = viewport_names();
    let transform = output_transform(names.len());
    let rects = split_window(window_size, names.len());
    // Every viewport starts switched off; `bring_up_all` then lights the
    // ones the config wants, as a cold plug on hardware does.
    let viewports: Vec<Viewport> = names
        .into_iter()
        .zip(rects)
        .map(|(name, rect)| {
            let edid = nested_edid(&name);
            let output =
                output_from_descriptor(&descriptor(&name, &edid, mode_of(rect.size), transform));
            Viewport::new(name, edid, output, rect, Connection::Disabled)
        })
        .collect();

    if let Err(err) = backend.renderer().compile_shaders() {
        // Cosmetic, so a compile failure degrades to square corners / no blur.
        tracing::warn!(%err, "failed to compile the effect shaders");
    }

    let (dmabuf_global, dmabuf_feedback) = init_dmabuf(state, &mut backend);

    state.backend = BackendState::Winit(Box::new(WinitState {
        backend,
        viewports,
        window_damage_tracker: window_damage_tracker(window_size),
        window_size,
        window_buffer_age: 0,
        dmabuf_global,
        dmabuf_feedback,
    }));
    bring_up_all(state);
    if let Some(first) = state
        .backend
        .winit()
        .and_then(|winit| winit.viewports.first())
    {
        crate::backend::debug_frames::init(&first.output);
    }

    state
        .common
        .event_loop_handle
        .insert_source(winit_events, move |event, _, state| match event {
            WinitEvent::Resized { size, .. } => resize(state, size),
            WinitEvent::Input(InputEvent::PointerMotionAbsolute { event })
                if state.backend.winit().is_some_and(|winit| winit.is_split()) =>
            {
                move_pointer(state, &event)
            }
            WinitEvent::Input(event) => state.process_input_event(event),
            WinitEvent::Redraw => {
                let released = signal_commit_timers(state);
                state.clear_blockers(released);
                match render::render(state) {
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

/// Splits the new window size over the viewports again.
///
/// Winit re-emits Resized on focus changes with an unchanged size, so an
/// unchanged size does nothing rather than reconfiguring everything.
fn resize(state: &mut State, size: Size<i32, Physical>) {
    let Some(winit) = state.backend.winit() else {
        return;
    };
    if winit.window_size == size {
        return;
    }
    winit.window_size = size;
    winit.window_damage_tracker = window_damage_tracker(size);
    let rects = split_window(size, winit.viewports.len());
    let mut resized = Vec::new();
    for (viewport, rect) in winit.viewports.iter_mut().zip(rects) {
        viewport.rect = rect;
        viewport.target = None;
        viewport.dirty = true;
        if viewport.is_connected() {
            resized.push((viewport.output.clone(), viewport.mode()));
        }
    }

    let changed: Vec<Output> = resized
        .into_iter()
        .filter(|(output, mode)| {
            state
                .shell
                .monitor_mut(output)
                .is_some_and(|monitor| monitor.set_only_mode(*mode))
        })
        .map(|(output, _)| output)
        .collect();
    if changed.is_empty() {
        return;
    }
    state.shell.arrange_outputs();
    for output in &changed {
        state.shell.refresh_usable(output);
    }
    state.refresh_output_heads();
}

/// Places the pointer through the viewport under it. Motion over no
/// connected viewport goes nowhere.
fn move_pointer(state: &mut State, event: &WinitMouseMovedEvent) {
    let window_point = Point::<f64, Physical>::from((event.x(), event.y()));
    let State { backend, shell, .. } = &mut *state;
    let Some(location) = backend.winit().and_then(|winit| {
        winit
            .viewports
            .iter()
            .filter(|viewport| viewport.is_connected())
            .find_map(|viewport| {
                let origin = shell.monitor(&viewport.output)?.geometry().loc;
                viewport.layout_point(window_point, origin)
            })
    }) else {
        return;
    };
    state.on_placed_pointer_motion(location, event.time());
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
    let Some(winit) = backend.winit() else {
        return released;
    };
    let presentation_time = wayland.clock.now().into();
    for output in winit.connected_outputs() {
        let waiting = present::signal_commit_timers(
            shell,
            &output,
            &input.cursor,
            presentation_time,
            &mut released,
        );
        if waiting {
            winit.queue_redraw(Some(&output));
        }
    }
    released
}
