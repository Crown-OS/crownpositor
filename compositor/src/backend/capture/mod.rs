//! Offscreen rendering for `crownos_screencast_v1`: the render half of a
//! capture session.
//!
//! A session renders only when three things line up: its source was damaged
//! (a real output submitted a frame, a virtual output was asked for one, the
//! client forced one), it owns a free slot, and its frame-rate cap allows it.
//! [`render_pending`] runs once per event-loop iteration, after the backends
//! have drawn, and checks exactly that — so an idle desktop costs a capture
//! nothing, and an encoder that holds every slot stops the compositor from
//! rendering for it at all.

mod cursor;
pub mod delivery;
pub mod formats;
mod nv12;
mod pacing;
mod render;
mod session;
pub mod sync;

use std::time::Duration;

use smithay::{
    backend::drm::DrmDeviceFd,
    output::Output,
    reexports::{
        calloop::timer::{TimeoutAction, Timer},
        wayland_server::backend::ObjectId,
    },
    wayland::drm_syncobj::supports_syncobj_eventfd,
};

use protocols::crownos_screencast::{Capability, CursorMode, StopReason};

pub use session::CaptureSession;

use crate::{
    backend::capture::{
        pacing::next_frame_at,
        render::{Outcome, Scene, capture_frame},
    },
    state::{BackendState, State},
};

#[derive(Default)]
pub struct CaptureState {
    sessions: Vec<CaptureSession>,
    /// The primary GPU, when it can import syncobj timelines and wait on
    /// them with an eventfd. `None` means explicit sync is not advertised.
    sync_device: Option<DrmDeviceFd>,
}

impl CaptureState {
    pub fn insert(&mut self, session: CaptureSession) {
        self.sessions.push(session);
    }

    pub fn remove(&mut self, id: &ObjectId) -> Option<CaptureSession> {
        let index = self
            .sessions
            .iter()
            .position(|session| session.protocol.id() == *id)?;
        Some(self.sessions.swap_remove(index))
    }

    pub fn session_mut(&mut self, id: &ObjectId) -> Option<&mut CaptureSession> {
        self.sessions
            .iter_mut()
            .find(|session| session.protocol.id() == *id)
    }

    pub fn sync_device(&self) -> Option<&DrmDeviceFd> {
        self.sync_device.as_ref()
    }

    /// Something on `output` changed; its sessions should look again.
    pub fn mark_output_damaged(&mut self, output: &Output) {
        for session in self
            .sessions
            .iter_mut()
            .filter(|session| session.output == *output)
        {
            session.dirty = true;
        }
    }

    /// Ends every session capturing `output`, which is going away.
    pub fn output_removed(&mut self, output: &Output) {
        self.sessions.retain(|session| {
            let captures = session.output == *output;
            if captures {
                session.protocol.stop(StopReason::SourceDestroyed);
            }
            !captures
        });
    }
}

impl State {
    /// Decides whether screencasts can offer explicit sync, once the backend
    /// is up and the primary GPU is known.
    pub fn configure_capture_sync(&mut self) {
        let device = self
            .backend
            .kms()
            .and_then(|kms| {
                kms.devices
                    .get(&kms.primary_node)
                    .map(|device| device.drm.device_fd().clone())
            })
            .filter(supports_syncobj_eventfd);

        let capabilities = if device.is_some() {
            Capability::ExplicitSync
        } else {
            Capability::empty()
        };
        self.wayland.screencast_state.set_capabilities(capabilities);
        self.capture.sync_device = device;
        tracing::info!(explicit_sync = !capabilities.is_empty(), "screencast ready");
    }
}

/// Renders every session that is due. Called once per loop iteration.
pub fn render_pending(state: &mut State) {
    if state.capture.sessions.is_empty() {
        return;
    }
    report_cursors(state);

    let now: Duration = state.wayland.clock.now().into();
    let due: Vec<ObjectId> = state
        .capture
        .sessions
        .iter()
        .filter(|session| session.dirty && !session.protocol.is_stopped())
        .map(|session| session.protocol.id())
        .collect();

    for id in due {
        if !pace(state, &id, now) {
            render_session(state, &id, now);
        }
    }
}

/// Holds back a session its frame-rate cap does not allow yet, arranging to
/// be woken when it does. Returns whether the session was held back.
fn pace(state: &mut State, id: &ObjectId, now: Duration) -> bool {
    let handle = state.common.event_loop_handle.clone();
    let Some(session) = state.capture.session_mut(id) else {
        return true;
    };
    if !session.protocol.is_running() {
        return false;
    }
    let Some(due) = next_frame_at(
        session.last_frame_at,
        session.protocol.frame_interval(),
        now,
    ) else {
        return false;
    };
    if session.pacing_timer.is_none() {
        let id = id.clone();
        session.pacing_timer = handle
            .insert_source(Timer::from_duration(due - now), move |_, _, state| {
                if let Some(session) = state.capture.session_mut(&id) {
                    session.pacing_timer = None;
                }
                TimeoutAction::Drop
            })
            .inspect_err(|err| tracing::warn!(%err, "failed to arm a capture pacing timer"))
            .ok();
    }
    true
}

fn render_session(state: &mut State, id: &ObjectId, now: Duration) {
    let Some(scale) = state
        .capture
        .session_mut(id)
        .map(|session| session.output.current_scale().fractional_scale())
    else {
        return;
    };
    state.layout_menus(scale);

    let outcome = {
        let State {
            backend,
            shell,
            capture,
            input,
            config,
            text,
            ..
        } = state;
        let Some(session) = capture.session_mut(id) else {
            return;
        };
        let Some(monitor) = shell.monitor(&session.output) else {
            if let Some(session) = capture.remove(id) {
                session.protocol.stop(StopReason::SourceDestroyed);
            }
            return;
        };

        let scene = Scene {
            shell,
            monitor,
            cursor: &mut input.cursor,
            pointer: input.pointer_location,
            appearance: &config.current.appearance,
            glass: &config.current.glass,
            text,
            hovered: input.hovered_control,
        };
        let outcome = match backend {
            BackendState::Kms(kms) => match kms.primary_renderer() {
                Ok(mut renderer) => capture_frame(&mut renderer, session, scene),
                Err(err) => {
                    tracing::warn!(%err, "no renderer for a capture frame");
                    return;
                }
            },
            BackendState::Winit(winit) => capture_frame(winit.backend.renderer(), session, scene),
            BackendState::Unset => return,
        };

        session.dirty = session.targets.blur.wants_redraw();
        if matches!(outcome, Ok(Outcome::Rendered(_))) {
            session.last_frame_at = Some(now);
        }
        outcome
    };

    match outcome {
        Ok(Outcome::Idle | Outcome::Reconfigured) => {}
        Ok(Outcome::Rendered(frame)) => delivery::deliver(state, id.clone(), frame, now),
        Err(err) => {
            tracing::warn!(%err, "capture frame failed; stopping the session");
            if let Some(session) = state.capture.remove(id) {
                session.protocol.stop(StopReason::RenderFailed);
            }
        }
    }
}

fn report_cursors(state: &mut State) {
    let State {
        capture,
        shell,
        input,
        ..
    } = state;
    for session in capture.sessions.iter_mut().filter(|session| {
        session.protocol.cursor_mode() == CursorMode::Metadata && session.protocol.is_running()
    }) {
        let Some(monitor) = shell.monitor(&session.output) else {
            continue;
        };
        session.cursor.update(
            &session.protocol,
            &input.cursor,
            monitor.geometry(),
            session.output.current_scale().fractional_scale(),
            input.pointer_location,
        );
    }
}
