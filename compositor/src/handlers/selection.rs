use smithay::{
    input::{
        Seat,
        dnd::{DnDGrab, DndGrabHandler, GrabType, Source},
        pointer::Focus,
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::Serial,
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

use crate::state::State;

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
        let grab = DnDGrab::new_pointer(&self.common.display_handle, start_data, source, seat);
        pointer.set_grab(self, grab, serial, Focus::Keep);
    }
}

impl DndGrabHandler for State {}

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
