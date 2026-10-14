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

pub mod copy;
pub mod copy_cursor;
mod cursor;
pub mod delivery;
pub mod formats;
mod nv12;
mod pacing;
mod render;
pub mod screencopy;
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

use smithay::{
    backend::{allocator::dmabuf::Dmabuf, renderer::Bind},
    wayland::image_copy_capture::{CaptureFailureReason, DmabufConstraints},
};

use crate::backend::capture::{
    copy::{CopyOutcome, CopySession},
    copy_cursor::CopyCursor,
    screencopy::Screencopies,
};

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
    /// `ext-image-copy-capture-v1` sessions, and their cursor sessions.
    pub copies: Vec<CopySession>,
    pub copy_cursors: Vec<CopyCursor>,
    /// `wlr-screencopy` frames waiting to be filled.
    pub screencopies: Screencopies,
    /// What copy-capture clients may hand over as dmabufs, worked out once.
    dmabuf: Option<Option<DmabufConstraints>>,
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
        for copy in self
            .copies
            .iter_mut()
            .filter(|copy| copy.shown_on.as_ref() == Some(output))
        {
            copy.dirty = true;
        }
        self.screencopies.mark_output_damaged(output);
    }

    /// A window committed. Sessions capturing it, or the workspace it is on,
    /// must look again even when nothing on screen shows it.
    pub fn mark_window_damaged(&mut self, window: WindowId, workspace: WorkspaceId) {
        let shows = |source: &CaptureSource| match source {
            CaptureSource::Window(id) => *id == window,
            CaptureSource::Workspace(id) => *id == workspace,
            CaptureSource::Output(_) => false,
        };
        for session in self
            .sessions
            .iter_mut()
            .filter(|session| shows(&session.source))
        {
            session.dirty = true;
        }
        for copy in self.copies.iter_mut().filter(|copy| shows(&copy.source)) {
            copy.dirty = true;
        }
    }

    /// Whether any session captures something other than a whole output.
    pub fn follows_windows(&self) -> bool {
        let follows = |source: &CaptureSource| !matches!(source, CaptureSource::Output(_));
        self.sessions.iter().any(|session| follows(&session.source))
            || self.copies.iter().any(|copy| follows(&copy.source))
    }

    /// The dmabuf constraints of copy-capture sessions: the primary GPU's
    /// render node and the 8-bit RGB formats it draws into. `None` where
    /// clients cannot share buffers with it, as nested.
    pub fn dmabuf_constraints(&mut self, backend: &mut BackendState) -> Option<DmabufConstraints> {
        self.dmabuf
            .get_or_insert_with(|| {
                let BackendState::Kms(kms) = backend else {
                    return None;
                };
                let node = kms.primary_render_node;
                let renderer = kms.primary_renderer().ok()?;
                let formats = formats::rgb_formats(
                    &Bind::<Dmabuf>::supported_formats(&renderer).unwrap_or_default(),
                );
                Some(DmabufConstraints {
                    node,
                    formats: formats
                        .into_iter()
                        .map(|format| (format.format, format.modifiers))
                        .collect(),
                })
            })
            .clone()
    }

    /// Ends every session capturing `output`, which is going away.
    pub fn output_removed(&mut self, output: &Output) {
        let gone = CaptureSource::Output(output.clone());
        self.sessions.retain(|session| {
            let captures = session.source == gone;
            if captures {
                session.protocol.stop(StopReason::SourceDestroyed);
            }
            !captures
        });
        self.copies.retain(|copy| copy.source != gone);
        self.copy_cursors.retain(|cursor| cursor.source != gone);
        self.screencopies.output_removed(output);
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

/// Evaluates `$body` with `$renderer` bound to the renderer captures draw
/// with: the primary GPU's, or the nested window's.
macro_rules! with_capture_renderer {
    ($backend:expr, |$renderer:ident| $body:expr) => {
        match $backend {
            BackendState::Kms(kms) => match kms.primary_renderer() {
                Ok(mut renderer) => {
                    let $renderer = &mut renderer;
                    Some($body)
                }
                Err(err) => {
                    tracing::warn!(%err, "no renderer for a capture frame");
                    None
                }
            },
            BackendState::Winit(winit) => {
                let $renderer = winit.backend.renderer();
                Some($body)
            }
            BackendState::Unset => None,
        }
    };
}

/// Renders every session that is due. Called once per loop iteration.
pub fn render_pending(state: &mut State) {
    render_copies(state);
    render_screencopies(state);
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
        let Some(outcome) =
            with_capture_renderer!(backend, |renderer| capture_frame(renderer, session, scene))
        else {
            return;
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

/// Completes copy-capture frames whose source changed, and moves cursor
/// sessions along with the pointer.
fn render_copies(state: &mut State) {
    if state.capture.copies.is_empty() && state.capture.copy_cursors.is_empty() {
        return;
    }
    let now: Duration = state.wayland.clock.now().into();
    {
        let State {
            capture,
            shell,
            input,
            ..
        } = state;
        capture.copy_cursors.retain_mut(|cursor| {
            let Some(placement) = cursor.source.place(shell) else {
                return false;
            };
            cursor.update(&input.cursor, &placement, input.pointer_location, now);
            true
        });
        capture.copies.retain_mut(|copy| {
            let Some(placement) = copy.source.place(shell) else {
                for frame in copy.pending.drain(..) {
                    frame.fail(CaptureFailureReason::Stopped);
                }
                return false;
            };
            if copy.shown_on.as_ref() != Some(placement.output()) {
                copy.shown_on = Some(placement.output().clone());
                copy.dirty = true;
            }
            true
        });
    }

    let mut failed = Vec::new();
    for index in 0..state.capture.copies.len() {
        let copy = &state.capture.copies[index];
        if !copy.dirty || copy.pending.is_empty() {
            continue;
        }
        let Some(scale) = copy
            .shown_on
            .as_ref()
            .map(|output| output.current_scale().fractional_scale())
        else {
            continue;
        };
        state.layout_menus(scale);
        let dma = state.capture.dmabuf_constraints(&mut state.backend);

        let State {
            backend,
            shell,
            capture,
            input,
            config,
            text,
            ..
        } = state;
        let copy = &mut capture.copies[index];
        let Some(placement) = copy.source.place(shell) else {
            continue;
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
        let Some(outcome) =
            with_capture_renderer!(backend, |renderer| copy.copy(renderer, scene, dma))
        else {
            continue;
        };

        match outcome {
            Ok(CopyOutcome::Copied(damage)) => {
                copy::complete(copy, damage, now);
                copy.dirty = copy.copier.targets.blur.wants_redraw();
                if let Some((output, windows)) = callbacks {
                    for window in &windows {
                        window.send_frame(&output, now, None, |_, _| Some(output.clone()));
                    }
                }
            }
            Ok(CopyOutcome::Unchanged) => copy.dirty = false,
            Ok(CopyOutcome::Reconfigured) => {}
            Err(err) => {
                tracing::warn!(%err, "copy-capture frame failed");
                failed.push(index);
            }
        }
    }
    for index in failed.into_iter().rev() {
        let copy = state.capture.copies.remove(index);
        for frame in copy.pending {
            frame.fail(CaptureFailureReason::Unknown);
        }
    }
}

/// Fills every `wlr-screencopy` frame that is due.
fn render_screencopies(state: &mut State) {
    let pending = &mut state.capture.screencopies.pending;
    pending.retain(|pending| pending.frame.is_alive());
    if pending.iter().all(|pending| !pending.due) {
        return;
    }
    let now: Duration = state.wayland.clock.now().into();
    let (due, waiting): (Vec<_>, Vec<_>) = std::mem::take(pending)
        .into_iter()
        .partition(|pending| pending.due);
    state.capture.screencopies.pending = waiting;

    for pending in due {
        state.layout_menus(pending.output.current_scale().fractional_scale());
        let State {
            backend,
            shell,
            capture,
            input,
            config,
            text,
            ..
        } = state;
        let source = CaptureSource::Output(pending.output.clone());
        let Some(placement) = source.place(shell) else {
            pending.frame.fail();
            continue;
        };
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
        let copier = capture.screencopies.copier(&pending.output);
        let outcome = with_capture_renderer!(backend, |renderer| screencopy::copy(
            renderer,
            copier,
            &pending.frame,
            scene
        ));
        match outcome {
            Some(Ok(Some(damage))) => pending.frame.success(&damage, now),
            Some(Ok(None)) => capture
                .screencopies
                .pending
                .push(screencopy::PendingScreencopy {
                    due: false,
                    ..pending
                }),
            Some(Err(err)) => {
                tracing::warn!(%err, "screencopy frame failed");
                pending.frame.fail();
            }
            None => pending.frame.fail(),
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
