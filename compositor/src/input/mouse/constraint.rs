//! When a `zwp_pointer_constraints_v1` lock or confinement may hold.
//!
//! The compositor always wins: a constraint is only honoured on the surface
//! under the pointer, in the window that holds the keyboard, while no
//! compositor mode owns the pointer. Losing any of those releases it, and
//! regaining them re-activates a persistent one.

use smithay::{
    backend::input::InputTime,
    backend::renderer::utils::with_renderer_surface_state,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Size},
    wayland::pointer_constraints::{PointerConstraint, with_pointer_constraint},
};

use crate::{
    handlers::seat::PointerFocusTarget,
    state::State,
    utils::{constraint_region, surface::root_surface},
};

/// What the active constraint makes of one motion event.
pub enum ConstrainedMotion {
    /// The pointer stays where it is; only relative motion goes out.
    Locked,
    To(Point<f64, Logical>),
}

/// The constraint currently in force, as far as the rest of the compositor
/// needs to know.
#[derive(Debug, Default)]
pub struct PointerLock {
    active: Option<WlSurface>,
    locked: bool,
    /// Where to put the pointer once a lock lets go, surface-local.
    pending_warp: Option<(WlSurface, Point<f64, Logical>)>,
}

impl PointerLock {
    /// Whether the pointer is locked in place, which is also when the cursor
    /// is hidden: it no longer points at anything.
    pub fn is_locked(&self) -> bool {
        self.locked
    }

    fn activated(&mut self, surface: WlSurface, locked: bool) {
        self.active = Some(surface);
        self.locked = locked;
    }

    /// Record-only: smithay reports a release from inside the pointer's own
    /// lock, so nothing here may touch the seat.
    pub fn released(&mut self, hint: Option<(WlSurface, Point<f64, Logical>)>) {
        self.active = None;
        self.locked = false;
        if hint.is_some() {
            self.pending_warp = hint;
        }
    }
}

/// The surface a constraint may apply to, and where its origin is.
struct ConstraintTarget {
    surface: WlSurface,
    origin: Point<f64, Logical>,
}

impl State {
    fn constraint_target(&self) -> Option<ConstraintTarget> {
        if self.mode_owns_input()
            || self.input.capture.is_active()
            || self.shell.menus.open().is_some()
        {
            return None;
        }
        let (target, origin) = self
            .shell
            .pointer_focus_under(self.input.pointer_location)?;
        let PointerFocusTarget::Window { window, surface } = target else {
            return None;
        };
        (self.shell.activated.as_ref() == Some(&window))
            .then_some(ConstraintTarget { surface, origin })
    }

    /// Applies the active constraint to a move from the current location.
    pub(super) fn constrain_motion(&self, to: Point<f64, Logical>) -> ConstrainedMotion {
        let unconstrained = ConstrainedMotion::To(to);
        if self.input.pointer_lock.active.is_none() {
            return unconstrained;
        }
        let (Some(pointer), Some(target)) =
            (self.wayland.seat.get_pointer(), self.constraint_target())
        else {
            return unconstrained;
        };

        let from = self.input.pointer_location;
        let size = surface_size(&target.surface);
        with_pointer_constraint(&target.surface, &pointer, |constraint| match constraint {
            Some(constraint) if constraint.is_active() => match &*constraint {
                PointerConstraint::Locked(_) => ConstrainedMotion::Locked,
                PointerConstraint::Confined(confined) => {
                    let allowed = |point: Point<f64, Logical>| {
                        constraint_region::allows(confined.region(), size, point - target.origin)
                    };
                    ConstrainedMotion::To(constraint_region::confine(from, to, allowed))
                }
            },
            _ => unconstrained,
        })
    }

    /// Activates the constraint under the pointer if the policy allows it.
    ///
    /// Must be called from top-level input or request dispatch, never from a
    /// `PointerTarget` callback.
    pub fn refresh_pointer_constraint(&mut self) {
        let Some(pointer) = self.wayland.seat.get_pointer() else {
            return;
        };
        if pointer.is_grabbed() {
            return;
        }
        let Some(target) = self.constraint_target() else {
            return;
        };
        let focused = pointer
            .current_focus()
            .is_some_and(|focus| focus.surface() == Some(&target.surface));
        if !focused {
            return;
        }

        let local = self.input.pointer_location - target.origin;
        let size = surface_size(&target.surface);
        let activated = with_pointer_constraint(&target.surface, &pointer, |constraint| {
            let constraint = constraint.filter(|constraint| !constraint.is_active())?;
            if !constraint_region::allows(constraint.region(), size, local) {
                return None;
            }
            constraint.activate();
            Some(matches!(&*constraint, PointerConstraint::Locked(_)))
        });

        if let Some(locked) = activated {
            tracing::debug!(locked, "pointer constraint activated");
            self.input.pointer_lock.activated(target.surface, locked);
            if locked {
                self.input.cursor.suppressed = true;
                self.queue_pointer_redraw();
            }
        }
    }

    /// Lets go of a constraint whose window lost the keyboard.
    pub fn release_unfocused_pointer_constraint(&mut self) {
        let Some(surface) = self.input.pointer_lock.active.clone() else {
            return;
        };
        let still_focused = self
            .shell
            .window_for_surface(&root_surface(&surface))
            .is_some_and(|window| self.shell.activated.as_ref() == Some(window));
        if still_focused {
            return;
        }
        let Some(pointer) = self.wayland.seat.get_pointer() else {
            return;
        };

        let hint = with_pointer_constraint(&surface, &pointer, |constraint| {
            let constraint = constraint?;
            let hint = match &*constraint {
                PointerConstraint::Locked(locked) => locked.cursor_position_hint(),
                PointerConstraint::Confined(_) => None,
            };
            constraint.deactivate();
            hint
        });
        tracing::debug!("pointer constraint released by a focus change");
        self.input
            .pointer_lock
            .released(hint.map(|hint| (surface, hint)));
        self.input.cursor.suppressed = false;
        self.queue_pointer_redraw();
    }

    /// Moves the pointer to where the client said its own cursor was, once a
    /// lock has let go. Only while the pointer is still over that surface.
    pub(crate) fn apply_unlock_warp(&mut self) {
        let Some((surface, hint)) = self.input.pointer_lock.pending_warp.take() else {
            return;
        };
        if self.mode_owns_input() {
            return;
        }
        let Some((target, origin)) = self.shell.pointer_focus_under(self.input.pointer_location)
        else {
            return;
        };
        if target.surface() != Some(&surface)
            || !constraint_region::allows(None, surface_size(&surface), hint)
        {
            return;
        }
        self.warp_pointer(origin + hint, InputTime::now());
    }
}

fn surface_size(surface: &WlSurface) -> Size<i32, Logical> {
    with_renderer_surface_state(surface, |state| state.surface_size())
        .flatten()
        .unwrap_or_default()
}
