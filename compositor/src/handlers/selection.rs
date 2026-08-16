use std::os::fd::OwnedFd;

use smithay::{
    delegate_data_control, delegate_data_device, delegate_ext_data_control,
    delegate_primary_selection,
    input::Seat,
    wayland::selection::{
        SelectionHandler, SelectionSource, SelectionTarget,
        data_device::{
            ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
        },
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

impl SelectionHandler for State {
    type SelectionUserData = ();

    fn new_selection(
        &mut self,
        _ty: SelectionTarget,
        _source: Option<SelectionSource>,
        _seat: Seat<Self>,
    ) {
    }

    fn send_selection(
        &mut self,
        _ty: SelectionTarget,
        _mime_type: String,
        _fd: OwnedFd,
        _seat: Seat<Self>,
        _user_data: &Self::SelectionUserData,
    ) {
        // TODO: only needed once the compositor owns a selection itself.
    }
}

impl DataDeviceHandler for State {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.wayland.data_device_state
    }
}

impl ClientDndGrabHandler for State {}

impl ServerDndGrabHandler for State {}

impl PrimarySelectionHandler for State {
    fn primary_selection_state(&self) -> &PrimarySelectionState {
        &self.wayland.primary_selection_state
    }
}

impl ExtDataControlHandler for State {
    fn data_control_state(&self) -> &ExtDataControlState {
        &self.wayland.ext_data_control_state
    }
}

impl WlrDataControlHandler for State {
    fn data_control_state(&self) -> &WlrDataControlState {
        &self.wayland.wlr_data_control_state
    }
}

delegate_data_device!(State);
delegate_primary_selection!(State);
delegate_ext_data_control!(State);
delegate_data_control!(State);
