use smithay::{
    input::pointer::PointerHandle,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    wayland::pointer_constraints::{
        ConstraintRemove, PointerConstraint, PointerConstraintsHandler,
    },
};

use crate::state::State;

/// Every callback here is reachable from inside smithay's pointer dispatch,
/// with the pointer's mutex held, so none of them touches the seat. The ones
/// that need to act leave a note for the event loop.
impl PointerConstraintsHandler for State {
    fn new_constraint(&mut self, _surface: &WlSurface, _pointer: &PointerHandle<Self>) {
        self.refresh_pointer_constraint();
    }

    fn remove_constraint(
        &mut self,
        surface: &WlSurface,
        _pointer: &PointerHandle<Self>,
        removal: ConstraintRemove,
    ) {
        let hint = match removal {
            ConstraintRemove::Destroyed(PointerConstraint::Locked(locked)) => locked
                .cursor_position_hint()
                .map(|hint| (surface.clone(), hint)),
            _ => None,
        };
        let was_locked = self.input.pointer_lock.is_locked();
        self.input.pointer_lock.released(hint);
        if was_locked {
            self.input.cursor.suppressed = false;
            self.queue_pointer_redraw();
        }
    }
}
