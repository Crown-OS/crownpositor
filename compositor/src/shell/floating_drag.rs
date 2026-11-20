//! A floating window following the pointer, across monitor edges.

use smithay::utils::{Logical, Point};

use crate::{
    layout::placement,
    shell::{Shell, monitor::Monitor},
    utils::id::WindowId,
};

impl Shell {
    /// The global position of the monitor a window is on.
    pub fn output_origin_of(&self, id: WindowId) -> Option<Point<i32, Logical>> {
        let location = self.location(id)?;
        Some(self.monitor_by_id(location.output)?.geometry().loc)
    }

    /// Puts a dragged floating window where `grip` is under `pointer`,
    /// first carrying it to the shown monitor the pointer is on. Returns the
    /// grip, held inside the window when the crossing shrank it.
    pub fn drag_floating(
        &mut self,
        id: WindowId,
        pointer: Point<f64, Logical>,
        grip: Point<f64, Logical>,
    ) -> Point<f64, Logical> {
        let current = self.location(id).map(|at| at.output);
        if let Some(target) = self
            .monitor_at(pointer)
            .map(Monitor::id)
            .filter(|target| current.is_some_and(|current| current != *target))
        {
            self.move_window_to_output(id, target);
        }
        let (Some(origin), Some(size)) = (
            self.output_origin_of(id),
            self.tile(id).map(|tile| tile.floating_rect().size),
        ) else {
            return grip;
        };
        let grip = placement::hold_grip(grip, size);
        self.move_floating(id, (pointer - origin.to_f64() - grip).to_i32_round());
        grip
    }
}
