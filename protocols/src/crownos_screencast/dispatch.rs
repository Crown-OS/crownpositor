//! Request routing for `crownos_screencast_v1`.

use crownos_protocols::screencast::v1::server::{
    crownos_screencast_manager_v1::{self, CrownosScreencastManagerV1, CursorMode},
    crownos_screencast_session_v1::{self, CrownosScreencastSessionV1, StopReason},
};
use smithay::wayland::dmabuf::get_dmabuf;
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
    backend::ClientId,
};

use super::{
    ScreencastGlobalData, ScreencastHandler, ScreencastState,
    constraints::join_halves,
    ring::{Released, Ring},
    session::{AttachedBuffer, Phase, ScreencastSession, SessionData},
};

/// The bounds every impl in this module shares, named once.
pub trait ScreencastDispatch:
    GlobalDispatch<CrownosScreencastManagerV1, ScreencastGlobalData>
    + Dispatch<CrownosScreencastManagerV1, ()>
    + Dispatch<CrownosScreencastSessionV1, SessionData>
    + ScreencastHandler
    + 'static
{
}

impl<D> ScreencastDispatch for D where
    D: GlobalDispatch<CrownosScreencastManagerV1, ScreencastGlobalData>
        + Dispatch<CrownosScreencastManagerV1, ()>
        + Dispatch<CrownosScreencastSessionV1, SessionData>
        + ScreencastHandler
        + 'static
{
}

impl<D: ScreencastDispatch> GlobalDispatch<CrownosScreencastManagerV1, ScreencastGlobalData, D>
    for ScreencastState
{
    fn bind(
        state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<CrownosScreencastManagerV1>,
        _global_data: &ScreencastGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        let manager = data_init.init(resource, ());
        manager.capabilities(state.screencast_state().capabilities);
    }

    fn can_view(client: Client, global_data: &ScreencastGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D: ScreencastDispatch> Dispatch<CrownosScreencastManagerV1, (), D> for ScreencastState {
    fn request(
        state: &mut D,
        _client: &Client,
        manager: &CrownosScreencastManagerV1,
        request: crownos_screencast_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            crownos_screencast_manager_v1::Request::CreateOutputSession {
                session,
                output,
                cursor_mode,
            } => {
                let WEnum::Value(cursor_mode) = cursor_mode else {
                    // Initialised first: a `new_id` left uninitialised takes
                    // the compositor down instead of the client.
                    data_init.init(session, SessionData::new(CursorMode::Hidden));
                    manager.post_error(
                        crownos_screencast_manager_v1::Error::InvalidCursorMode,
                        "cursor_mode is not a value of the cursor_mode enum",
                    );
                    return;
                };
                let session =
                    ScreencastSession(data_init.init(session, SessionData::new(cursor_mode)));
                state.new_screencast_session(session, &output);
            }
            crownos_screencast_manager_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

impl<D: ScreencastDispatch> Dispatch<CrownosScreencastSessionV1, SessionData, D>
    for ScreencastState
{
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &CrownosScreencastSessionV1,
        request: crownos_screencast_session_v1::Request,
        data: &SessionData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        use crownos_screencast_session_v1::{Error, Request};

        let session = ScreencastSession(resource.clone());
        if data.with(|inner| inner.phase == Phase::Stopped) {
            return;
        }

        match request {
            Request::AttachBuffer { index, buffer } => {
                let index = match Ring::<AttachedBuffer>::index(index) {
                    Ok(index) => index,
                    Err(error) => {
                        return resource.post_error(Error::from(error), error.to_string());
                    }
                };
                let Ok(dmabuf) = get_dmabuf(&buffer).cloned() else {
                    return resource.post_error(Error::InvalidBuffer, "the buffer is not a dmabuf");
                };
                let attached = data.with(|inner| {
                    if !inner
                        .constraints
                        .as_ref()
                        .is_some_and(|constraints| constraints.admits(&dmabuf))
                    {
                        return Err((
                            Error::InvalidBuffer,
                            "the buffer does not satisfy the constraints".to_owned(),
                        ));
                    }
                    let buffer = AttachedBuffer {
                        wl_buffer: buffer.downgrade(),
                        dmabuf,
                    };
                    inner
                        .ring
                        .attach(index, buffer)
                        .map_err(|error| (Error::from(error), error.to_string()))
                });
                match attached {
                    Ok(()) => state.screencast_session_ready(&session),
                    Err((error, message)) => resource.post_error(error, message),
                }
            }
            Request::SetTimelines {
                acquire_timeline,
                release_timeline,
            } => {
                let refusal = data.with(|inner| {
                    if inner.has_timelines {
                        Some((Error::TimelinesExist, "timelines are already set"))
                    } else if inner.phase != Phase::Configuring {
                        Some((Error::LateTimelines, "set_timelines must precede start"))
                    } else {
                        None
                    }
                });
                if let Some((error, message)) = refusal {
                    return resource.post_error(error, message);
                }
                let supported = state
                    .screencast_state()
                    .capabilities
                    .contains(super::Capability::ExplicitSync);
                if !supported {
                    return resource.post_error(
                        Error::SyncUnsupported,
                        "the compositor did not advertise explicit_sync",
                    );
                }
                if state.import_screencast_timelines(&session, acquire_timeline, release_timeline) {
                    data.with(|inner| inner.has_timelines = true);
                } else {
                    resource.post_error(
                        Error::InvalidTimeline,
                        "the timelines could not be imported",
                    );
                }
            }
            Request::Start { max_fps_mhz } => {
                let started = data.with(|inner| {
                    if inner.phase != Phase::Configuring {
                        return false;
                    }
                    inner.phase = Phase::Running;
                    inner.max_fps_mhz = max_fps_mhz;
                    inner.needs_full_frame = true;
                    true
                });
                if started {
                    state.screencast_session_ready(&session);
                } else {
                    resource.post_error(Error::AlreadyStarted, "start was sent more than once");
                }
            }
            Request::Release {
                index,
                release_point_hi,
                release_point_lo,
            } => {
                let index = match Ring::<AttachedBuffer>::index(index) {
                    Ok(index) => index,
                    Err(error) => {
                        return resource.post_error(Error::from(error), error.to_string());
                    }
                };
                let point = join_halves(release_point_hi, release_point_lo);
                let released = data.with(|inner| {
                    if inner.has_timelines && !inner.advance_release_point(point) {
                        return Err((
                            Error::InvalidReleasePoint,
                            "the release point does not exceed the previous one".to_owned(),
                        ));
                    }
                    inner
                        .ring
                        .release(index, inner.has_timelines)
                        .map_err(|error| (Error::from(error), error.to_string()))
                });
                match released {
                    Ok(Released::Immediately) => state.screencast_session_ready(&session),
                    Ok(Released::AfterFence) => {
                        state.await_screencast_release(&session, index, point)
                    }
                    Ok(Released::Ignored) => {}
                    Err((error, message)) => resource.post_error(error, message),
                }
            }
            Request::ForceFrame => {
                data.with(|inner| inner.needs_full_frame = true);
                state.screencast_session_ready(&session);
            }
            Request::Stop => {
                session.stop(StopReason::Requested);
                state.screencast_session_ended(&session);
            }
            Request::Destroy => {}
            _ => {}
        }
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        resource: &CrownosScreencastSessionV1,
        data: &SessionData,
    ) {
        data.with(|inner| {
            inner.phase = Phase::Stopped;
            inner.ring.clear();
        });
        state.screencast_session_ended(&ScreencastSession(resource.clone()));
    }
}
