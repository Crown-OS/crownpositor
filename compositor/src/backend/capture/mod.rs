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
pub mod source;
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
        source::{CaptureSource, View},
    },
    state::{BackendState, State},
    utils::id::{WindowId, WorkspaceId},
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
            .filter(|session| session.shown_on.as_ref() == Some(output))
        {
            session.dirty = true;
        }
    }

    /// A window committed. Sessions capturing it, or the workspace it is on,
    /// must look again even when nothing on screen shows it.
    pub fn mark_window_damaged(&mut self, window: WindowId, workspace: WorkspaceId) {
        for session in self
            .sessions
            .iter_mut()
            .filter(|session| match session.source {
                CaptureSource::Window(id) => id == window,
                CaptureSource::Workspace(id) => id == workspace,
                CaptureSource::Output(_) => false,
            })
        {
            session.dirty = true;
        }
    }

    /// Whether any session captures something other than a whole output.
    pub fn follows_windows(&self) -> bool {
        self.sessions
            .iter()
            .any(|session| !matches!(session.source, CaptureSource::Output(_)))
    }

    /// Ends every session capturing `output`, which is going away.
    pub fn output_removed(&mut self, output: &Output) {
        self.sessions.retain(|session| {
            let captures = session.source == CaptureSource::Output(output.clone());
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

        let mut capabilities = Capability::ToplevelSource | Capability::WorkspaceSource;
        capabilities.set(Capability::ExplicitSync, device.is_some());
        let device_supports_sync = device.is_some();
        self.wayland.screencast_state.set_capabilities(capabilities);
        self.capture.sync_device = device;
        tracing::info!(explicit_sync = device_supports_sync, "screencast ready");
    }
}

/// Renders every session that is due. Called once per loop iteration.
pub fn render_pending(state: &mut State) {
    if state.capture.sessions.is_empty() {
        return;
    }
    stop_lost_sources(state);
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

/// Ends sessions whose window or workspace went away, and notes where the
/// others are now.
fn stop_lost_sources(state: &mut State) {
    let State { capture, shell, .. } = state;
    capture.sessions.retain_mut(|session| {
        let Some(placement) = session.source.place(shell) else {
            session.protocol.stop(StopReason::SourceDestroyed);
            return false;
        };
        if session.shown_on.as_ref() != Some(placement.output()) {
            session.shown_on = Some(placement.output().clone());
            session.dirty = true;
        }
        true
    });
}

fn render_session(state: &mut State, id: &ObjectId, now: Duration) {
    let Some(scale) = state.capture.session_mut(id).and_then(|session| {
        session
            .shown_on
            .as_ref()
            .map(|output| output.current_scale().fractional_scale())
    }) else {
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
        let Some(placement) = session.source.place(shell) else {
            if let Some(session) = capture.remove(id) {
                session.protocol.stop(StopReason::SourceDestroyed);
            }
            return;
        };
        let callbacks = frame_callback_target(&placement);

        let scene = Scene {
            shell,
            placement,
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
        if let Some((output, windows)) = callbacks {
            for window in &windows {
                window.send_frame(&output, now, None, |_, _| Some(output.clone()));
            }
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

/// The windows a window or workspace capture draws, which need frame
/// callbacks from the capture itself: off screen, nothing else paces them.
fn frame_callback_target(
    placement: &source::Placement<'_>,
) -> Option<(smithay::output::Output, Vec<smithay::desktop::Window>)> {
    let windows = match placement.view {
        View::Output => return None,
        View::Window(tile) => vec![tile.window().clone()],
        View::Workspace(workspace) => workspace
            .tiles()
            .iter()
            .map(|tile| tile.window().clone())
            .collect(),
    };
    Some((placement.output().clone(), windows))
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
        let Some(placement) = session.source.place(shell) else {
            continue;
        };
        session.cursor.update(
            &session.protocol,
            &input.cursor,
            placement.on_screen.then_some(placement.area),
            placement.scale(),
            input.pointer_location,
        );
    }
}
