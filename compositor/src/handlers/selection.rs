use std::any::Any;

use smithay::{
    backend::input::InputTime,
    input::{
        Seat,
        dnd::{DnDGrab, DndGrabHandler, DndTarget, GrabType, Source},
        pointer::Focus,
    },
    reexports::wayland_server::protocol::{wl_data_source::WlDataSource, wl_surface::WlSurface},
    utils::{Logical, Point, Serial},
    wayland::selection::{
        SelectionHandler,
        data_device::{DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler},
        ext_data_control::{
            DataControlHandler as ExtDataControlHandler, DataControlState as ExtDataControlState,
        },
        primary_selection::{PrimarySelectionHandler, PrimarySelectionState},
        wlr_data_control::{
            DataControlHandler as WlrDataControlHandler, DataControlState as WlrDataControlState,
        },
    },
};

use crate::{
    shell::toplevel_drag::{CarriedSource, DragEnd},
    state::State,
};

/// The compositor never owns a selection: every clipboard and primary
/// selection is some client's source. Keeping one alive after its client
/// exits is `crownos-clipboard`'s job, over ext-data-control.
impl SelectionHandler for State {
    type SelectionUserData = ();
}

impl DataDeviceHandler for State {
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.wayland.data_device_state
    }
}

/// `start_drag` arrives as a client request, not from inside a pointer
/// callback, so taking the pointer grab here cannot deadlock the seat.
impl WaylandDndGrabHandler for State {
    fn dnd_requested<S: Source>(
        &mut self,
        source: S,
        _icon: Option<WlSurface>,
        seat: Seat<Self>,
        serial: Serial,
        grab_type: GrabType,
    ) {
        let pointer = match grab_type {
            GrabType::Pointer => seat.get_pointer(),
            GrabType::Touch => None,
        };
        let Some((pointer, start_data)) =
            pointer.and_then(|pointer| pointer.grab_start_data().map(|data| (pointer, data)))
        else {
            source.cancel();
            return;
        };
        let toplevel_drag = (&source as &dyn Any)
            .downcast_ref::<WlDataSource>()
            .and_then(|data_source| {
                let drag = self.wayland.xdg_toplevel_drag_state.drag_for(data_source)?;
                Some((data_source.clone(), drag))
            });
        let display = &self.common.display_handle;
        match toplevel_drag {
            Some((data_source, drag)) => {
                let source = CarriedSource::new(data_source, drag.clone());
                let grab = DnDGrab::new_pointer(display, start_data, source, seat);
                pointer.set_grab(self, grab, serial, Focus::Keep);
                self.begin_toplevel_drag(drag);
            }
            None => {
                let grab = DnDGrab::new_pointer(display, start_data, source, seat);
                pointer.set_grab(self, grab, serial, Focus::Keep);
            }
        }
        // smithay only offers the drag on the first motion, and Chromium
        // cancels one released before its first `enter`: a motion in place
        // offers it to the surface under the cursor straight away.
        self.warp_pointer(self.input.pointer_location, InputTime::now());
    }
}

/// Both run from inside the pointer grab, so neither may touch the seat.
impl DndGrabHandler for State {
    fn dropped(
        &mut self,
        _target: Option<DndTarget<'_, Self>>,
        validated: bool,
        _seat: Seat<Self>,
        _location: Point<f64, Logical>,
    ) {
        self.end_toplevel_drag(if validated {
            DragEnd::Accepted
        } else {
            DragEnd::Declined
        });
    }

    fn cancelled(&mut self, _seat: Seat<Self>, _location: Point<f64, Logical>) {
        self.end_toplevel_drag(DragEnd::Aborted);
    }
}

impl PrimarySelectionHandler for State {
    fn primary_selection_state(&mut self) -> &mut PrimarySelectionState {
        &mut self.wayland.primary_selection_state
    }
}

impl ExtDataControlHandler for State {
    fn data_control_state(&mut self) -> &mut ExtDataControlState {
        &mut self.wayland.ext_data_control_state
    }
}

impl WlrDataControlHandler for State {
    fn data_control_state(&mut self) -> &mut WlrDataControlState {
        &mut self.wayland.wlr_data_control_state
    }
}
