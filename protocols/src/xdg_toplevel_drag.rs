//! Server-side `xdg-toplevel-drag-v1` (staging).
//!
//! A drag-and-drop that carries a window: a tab torn out of a browser becomes
//! a toplevel that rides the cursor until it is dropped, and can be dragged
//! back over a tab strip to dock again. Only the bookkeeping lives here —
//! which toplevel rides which `wl_data_source`, at what offset, and whether the
//! drag is still running. Moving the window, and keeping it out of drop-target
//! picking, is the compositor's.

use std::sync::{Mutex, MutexGuard, PoisonError};

use smithay::utils::{Logical, Point};
use wayland_protocols::xdg::{
    shell::server::xdg_toplevel::XdgToplevel,
    toplevel_drag::v1::server::{
        xdg_toplevel_drag_manager_v1::{self, XdgToplevelDragManagerV1},
        xdg_toplevel_drag_v1::{self, XdgToplevelDragV1},
    },
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
    backend::{ClientId, GlobalId},
    protocol::wl_data_source::WlDataSource,
};

/// What the compositor has to provide for this protocol to be delegated to
/// [`XdgToplevelDragState`].
pub trait XdgToplevelDragHandler {
    fn xdg_toplevel_drag_state(&mut self) -> &mut XdgToplevelDragState;

    /// A toplevel was attached to `drag`, before the drag started or while it
    /// runs. It may not be mapped yet: Chromium attaches a torn-out tab's new
    /// window before its first commit, and the offset is then all there is to
    /// place it by.
    fn toplevel_attached(&mut self, drag: &ToplevelDrag, attachment: &Attachment);
}

/// A toplevel riding a drag, and where the cursor holds it.
#[derive(Debug, Clone, PartialEq)]
pub struct Attachment {
    pub toplevel: XdgToplevel,
    /// The cursor hotspot relative to the toplevel's window geometry.
    pub offset: Point<i32, Logical>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Phase {
    /// `start_drag` has not been honoured with this drag's source yet.
    #[default]
    Prepared,
    Dragging,
    /// Dropped or cancelled: nothing rides it any more, and the object may go.
    Ended,
}

#[derive(Debug, Default)]
struct Progress {
    phase: Phase,
    attachment: Option<Attachment>,
}

/// User data of an [`XdgToplevelDragV1`].
#[derive(Debug)]
pub struct ToplevelDragData {
    source: WlDataSource,
    progress: Mutex<Progress>,
}

impl ToplevelDragData {
    fn progress(&self) -> MutexGuard<'_, Progress> {
        self.progress.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A drag that may carry a toplevel. A handle to the protocol object, so
/// cloning it is cheap and comparing two compares the objects.
#[derive(Debug, Clone, PartialEq)]
pub struct ToplevelDrag(XdgToplevelDragV1);

impl ToplevelDrag {
    fn data(&self) -> Option<&ToplevelDragData> {
        self.0.data()
    }

    pub fn source(&self) -> Option<&WlDataSource> {
        self.data().map(|data| &data.source)
    }

    /// The toplevel riding the drag. `None` once it lost its role: the
    /// protocol detaches an unmapped toplevel by itself.
    pub fn attachment(&self) -> Option<Attachment> {
        let data = self.data()?;
        let progress = data.progress();
        progress
            .attachment
            .clone()
            .filter(|attachment| attachment.toplevel.is_alive())
    }

    /// Between `start_drag` and the drop or cancel.
    pub fn is_dragging(&self) -> bool {
        self.data()
            .is_some_and(|data| data.progress().phase == Phase::Dragging)
    }

    /// `start_drag` was honoured with this drag's source.
    pub fn start(&self) {
        self.set_phase(Phase::Dragging);
    }

    /// The drag was dropped or cancelled.
    pub fn end(&self) {
        if let Some(data) = self.data() {
            let mut progress = data.progress();
            progress.phase = Phase::Ended;
            progress.attachment = None;
        }
    }

    fn set_phase(&self, phase: Phase) {
        if let Some(data) = self.data() {
            data.progress().phase = phase;
        }
    }
}

/// Delegate type for the [`XdgToplevelDragManagerV1`] global.
#[derive(Debug)]
pub struct XdgToplevelDragState {
    global: GlobalId,
    /// Every live drag object. A handful at most — one per drag a client is
    /// preparing or running — so a scan beats a map keyed on the source.
    drags: Vec<ToplevelDrag>,
}

impl XdgToplevelDragState {
    pub fn new<D>(display: &DisplayHandle) -> Self
    where
        D: GlobalDispatch<XdgToplevelDragManagerV1, ()>
            + Dispatch<XdgToplevelDragManagerV1, ()>
            + Dispatch<XdgToplevelDragV1, ToplevelDragData>
            + XdgToplevelDragHandler
            + 'static,
    {
        Self {
            global: display.create_global::<D, XdgToplevelDragManagerV1, _>(1, ()),
            drags: Vec::new(),
        }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }

    /// The drag a client created for `source`, if it made one.
    pub fn drag_for(&self, source: &WlDataSource) -> Option<ToplevelDrag> {
        self.drags
            .iter()
            .find(|drag| drag.source() == Some(source))
            .cloned()
    }
}

impl<D> GlobalDispatch<XdgToplevelDragManagerV1, (), D> for XdgToplevelDragState
where
    D: GlobalDispatch<XdgToplevelDragManagerV1, ()>
        + Dispatch<XdgToplevelDragManagerV1, ()>
        + Dispatch<XdgToplevelDragV1, ToplevelDragData>
        + XdgToplevelDragHandler
        + 'static,
{
    fn bind(
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<XdgToplevelDragManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, ());
    }
}

impl<D> Dispatch<XdgToplevelDragManagerV1, (), D> for XdgToplevelDragState
where
    D: Dispatch<XdgToplevelDragManagerV1, ()>
        + Dispatch<XdgToplevelDragV1, ToplevelDragData>
        + XdgToplevelDragHandler
        + 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        manager: &XdgToplevelDragManagerV1,
        request: xdg_toplevel_drag_manager_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        let xdg_toplevel_drag_manager_v1::Request::GetXdgToplevelDrag { id, data_source } = request
        else {
            return;
        };
        let taken = state
            .xdg_toplevel_drag_state()
            .drag_for(&data_source)
            .is_some();
        // Initialised before the error: wayland-rs panics on a new_id that
        // leaves the request handler uninitialised.
        let drag = data_init.init(
            id,
            ToplevelDragData {
                source: data_source,
                progress: Mutex::default(),
            },
        );
        if taken {
            manager.post_error(
                xdg_toplevel_drag_manager_v1::Error::InvalidSource,
                "data_source already used for a toplevel drag",
            );
            return;
        }
        state
            .xdg_toplevel_drag_state()
            .drags
            .push(ToplevelDrag(drag));
    }
}

impl<D> Dispatch<XdgToplevelDragV1, ToplevelDragData, D> for XdgToplevelDragState
where
    D: Dispatch<XdgToplevelDragV1, ToplevelDragData> + XdgToplevelDragHandler + 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &XdgToplevelDragV1,
        request: xdg_toplevel_drag_v1::Request,
        data: &ToplevelDragData,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            xdg_toplevel_drag_v1::Request::Attach {
                toplevel,
                x_offset,
                y_offset,
            } => {
                let attachment = Attachment {
                    toplevel,
                    offset: (x_offset, y_offset).into(),
                };
                {
                    let mut progress = data.progress();
                    if progress.phase == Phase::Ended {
                        return;
                    }
                    if progress
                        .attachment
                        .as_ref()
                        .is_some_and(|attached| attached.toplevel.is_alive())
                    {
                        resource.post_error(
                            xdg_toplevel_drag_v1::Error::ToplevelAttached,
                            "a toplevel with an active role is already attached",
                        );
                        return;
                    }
                    progress.attachment = Some(attachment.clone());
                }
                state.toplevel_attached(&ToplevelDrag(resource.clone()), &attachment);
            }
            // A client that destroyed its source has ended the drag as far as
            // it is concerned: Chromium cancels a tab drag exactly that way.
            xdg_toplevel_drag_v1::Request::Destroy
                if data.progress().phase == Phase::Dragging && data.source.is_alive() =>
            {
                resource.post_error(
                    xdg_toplevel_drag_v1::Error::OngoingDrag,
                    "destroyed before the drag ended",
                );
            }
            _ => {}
        }
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        resource: &XdgToplevelDragV1,
        _data: &ToplevelDragData,
    ) {
        state
            .xdg_toplevel_drag_state()
            .drags
            .retain(|drag| drag.0 != *resource);
    }
}
