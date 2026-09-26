//! Getting a rendered frame to the client once the GPU is done with it, and
//! getting a slot back once the client's release point is signalled.
//!
//! Both are "run this when a file descriptor becomes readable": a render's
//! native fence and a syncobj eventfd are pollable, so neither ever blocks
//! the event loop.

use std::{cell::RefCell, os::fd::OwnedFd, rc::Rc, time::Duration};

use smithay::reexports::{
    calloop::{Interest, Mode, PostAction, generic::Generic},
    wayland_server::backend::ObjectId,
};

use protocols::crownos_screencast::StopReason;

use crate::{
    backend::capture::{render::RenderedFrame, sync::ExplicitSync},
    state::State,
};

/// Sends `frame` to its session: at once with an acquire point the client
/// waits on, or after the GPU finishes when there is no timeline.
pub fn deliver(state: &mut State, id: ObjectId, frame: RenderedFrame, sampled_at: Duration) {
    let Some(session) = state.capture.session_mut(&id) else {
        return;
    };
    let acquire_point = session.sync.as_mut().map(ExplicitSync::issue_acquire_point);
    let RenderedFrame {
        index,
        damage,
        sync,
    } = frame;

    let Some(fence) = sync.export() else {
        if sync.wait().is_err() {
            tracing::warn!("interrupted waiting for a capture render");
        }
        signal_acquire(state, &id, acquire_point);
        present(state, &id, index, &damage, sampled_at, acquire_point);
        return;
    };

    match acquire_point {
        Some(point) => {
            present(state, &id, index, &damage, sampled_at, Some(point));
            when_readable(state, fence, move |state| {
                signal_acquire(state, &id, Some(point));
            });
        }
        None => when_readable(state, fence, move |state| {
            present(state, &id, index, &damage, sampled_at, None);
        }),
    }
}

/// Hands slot `index` back once `point` on the session's release timeline
/// signals.
pub fn await_release(state: &mut State, id: ObjectId, index: usize, point: u64) {
    let eventfd = state
        .capture
        .session_mut(&id)
        .and_then(|session| session.sync.as_ref())
        .map(|sync| sync.release_eventfd(point));

    match eventfd {
        Some(Ok(eventfd)) => when_readable(state, eventfd, move |state| {
            if let Some(session) = state.capture.session_mut(&id)
                && session.protocol.release_signalled(index)
            {
                session.dirty = true;
            }
        }),
        Some(Err(err)) => {
            tracing::warn!(%err, "cannot wait for a capture release point");
            stop(state, &id);
        }
        None => {}
    }
}

fn present(
    state: &mut State,
    id: &ObjectId,
    index: usize,
    damage: &[smithay::utils::Rectangle<i32, smithay::utils::Buffer>],
    sampled_at: Duration,
    acquire_point: Option<u64>,
) {
    if let Some(session) = state.capture.session_mut(id) {
        session
            .protocol
            .present(index, damage, sampled_at, acquire_point.unwrap_or(0));
    }
}

fn signal_acquire(state: &mut State, id: &ObjectId, point: Option<u64>) {
    let Some(point) = point else {
        return;
    };
    let failed = state
        .capture
        .session_mut(id)
        .and_then(|session| session.sync.as_mut())
        .map(|sync| sync.signal_acquired(point))
        .is_some_and(|result| {
            result
                .inspect_err(|err| tracing::warn!(%err, "failed to signal a capture acquire point"))
                .is_err()
        });
    if failed {
        stop(state, id);
    }
}

fn stop(state: &mut State, id: &ObjectId) {
    if let Some(session) = state.capture.remove(id) {
        session.protocol.stop(StopReason::RenderFailed);
    }
}

/// Runs `action` once `fd` is readable. If the loop refuses the source the
/// action runs right away rather than never, so a slot cannot get stuck.
fn when_readable(state: &mut State, fd: OwnedFd, action: impl FnOnce(&mut State) + 'static) {
    let action = Rc::new(RefCell::new(Some(action)));
    let pending = Rc::clone(&action);
    let inserted = state.common.event_loop_handle.insert_source(
        Generic::new(fd, Interest::READ, Mode::OneShot),
        move |_, _, state| {
            if let Some(action) = pending.borrow_mut().take() {
                action(state);
            }
            Ok(PostAction::Remove)
        },
    );
    if let Err(err) = inserted {
        tracing::error!(%err, "failed to watch a capture fence; completing synchronously");
        if let Some(action) = action.borrow_mut().take() {
            action(state);
        }
    }
}
