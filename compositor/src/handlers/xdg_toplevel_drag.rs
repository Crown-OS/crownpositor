//! Delegation glue for `xdg-toplevel-drag-v1`. The protocol lives in
//! [`protocols::xdg_toplevel_drag`]; carrying the window, in
//! [`crate::shell::toplevel_drag`].

use smithay::reexports::wayland_protocols::xdg::toplevel_drag::v1::server::{
    xdg_toplevel_drag_manager_v1::XdgToplevelDragManagerV1, xdg_toplevel_drag_v1::XdgToplevelDragV1,
};
use wayland_server::{delegate_dispatch, delegate_global_dispatch};

use protocols::xdg_toplevel_drag::{
    Attachment, ToplevelDrag, ToplevelDragData, XdgToplevelDragHandler, XdgToplevelDragState,
};

use crate::state::State;

impl XdgToplevelDragHandler for State {
    fn xdg_toplevel_drag_state(&mut self) -> &mut XdgToplevelDragState {
        &mut self.wayland.xdg_toplevel_drag_state
    }

    fn toplevel_attached(&mut self, drag: &ToplevelDrag, _attachment: &Attachment) {
        self.carry_attached_toplevel(drag);
    }
}

delegate_global_dispatch!(State: [XdgToplevelDragManagerV1: ()] => XdgToplevelDragState);
delegate_dispatch!(State: [XdgToplevelDragManagerV1: ()] => XdgToplevelDragState);
delegate_dispatch!(State: [XdgToplevelDragV1: ToplevelDragData] => XdgToplevelDragState);
