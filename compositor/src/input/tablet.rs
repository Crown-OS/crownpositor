//! Graphics tablets through `zwp_tablet_v2`: pressure, tilt and the pen's own
//! buttons, for drawing apps and for games like osu! that read the pen
//! directly. The cursor follows the pen so the user sees where it points.

use smithay::{
    backend::input::{
        AbsolutePositionEvent, Device, DeviceCapability, Event, InputBackend, ProximityState,
        TabletToolButtonEvent, TabletToolEvent, TabletToolProximityEvent, TabletToolTipEvent,
        TabletToolTipState,
    },
    input::tablet::{
        TabletDescriptor, TabletSeatTrait,
        tool::{
            AxisFrame, ButtonEvent, DownEvent, MotionEvent, ProximityInEvent, ProximityOutEvent,
            UpEvent,
        },
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, SERIAL_COUNTER},
};

use crate::{shell::monitor::Monitor, state::State};

impl State {
    pub(super) fn on_device_added<I: InputBackend>(&mut self, device: &I::Device) {
        if device.has_capability(DeviceCapability::TabletTool) {
            self.wayland
                .seat
                .tablet_seat()
                .add_wp_tablet(&self.common.display_handle, &TabletDescriptor::from(device));
        }
    }

    pub(super) fn on_device_removed<I: InputBackend>(&mut self, device: &I::Device) {
        if !device.has_capability(DeviceCapability::TabletTool) {
            return;
        }
        let tablets = self.wayland.seat.tablet_seat();
        tablets.remove_tablet(&TabletDescriptor::from(device));
        if tablets.count_tablets() == 0 {
            tablets.clear_tools();
        }
    }

    pub(super) fn on_tablet_tool_axis<I: InputBackend>(&mut self, event: I::TabletToolAxisEvent) {
        let Some(location) = self.pen_location::<I>(&event) else {
            return;
        };
        let Some(tool) = self.wayland.seat.tablet_seat().get_tool(&event.tool()) else {
            return;
        };
        self.follow_pen(location);
        tool.axis(self, axis_frame::<I>(&event));
        tool.motion(
            self,
            self.pen_focus(location),
            &MotionEvent {
                location,
                serial: SERIAL_COUNTER.next_serial(),
                time: event.time(),
            },
        );
        tool.frame(self, event.time());
    }

    pub(super) fn on_tablet_tool_proximity<I: InputBackend>(
        &mut self,
        event: I::TabletToolProximityEvent,
    ) {
        let Some(location) = self.pen_location::<I>(&event) else {
            return;
        };
        let tablets = self.wayland.seat.tablet_seat();
        let Some(tablet) = tablets.get_tablet(&TabletDescriptor::from(&event.device())) else {
            return;
        };
        let descriptor = event.tool();
        let tool = match tablets.get_tool(&descriptor) {
            Some(tool) => tool,
            None => {
                let display_handle = self.common.display_handle.clone();
                tablets.add_wp_tool(self, &display_handle, &descriptor)
            }
        };

        let serial = SERIAL_COUNTER.next_serial();
        match event.state() {
            ProximityState::In => {
                self.follow_pen(location);
                tool.proximity_in(
                    self,
                    self.pen_focus(location),
                    tablet,
                    &ProximityInEvent {
                        location,
                        axis: Some(axis_frame::<I>(&event)),
                        serial,
                        time: event.time(),
                    },
                );
            }
            ProximityState::Out => tool.proximity_out(
                self,
                &ProximityOutEvent {
                    serial,
                    time: event.time(),
                },
            ),
        }
        tool.frame(self, event.time());
    }

    pub(super) fn on_tablet_tool_tip<I: InputBackend>(&mut self, event: I::TabletToolTipEvent) {
        let Some(tool) = self.wayland.seat.tablet_seat().get_tool(&event.tool()) else {
            return;
        };
        let serial = SERIAL_COUNTER.next_serial();
        match event.tip_state() {
            TabletToolTipState::Down => {
                tool.down(
                    self,
                    &DownEvent {
                        serial,
                        time: event.time(),
                    },
                );
                // A pen touching a window focuses it, as a click does.
                self.focus_under_pointer();
            }
            TabletToolTipState::Up => tool.up(
                self,
                &UpEvent {
                    serial,
                    time: event.time(),
                },
            ),
        }
        tool.frame(self, event.time());
    }

    pub(super) fn on_tablet_tool_button<I: InputBackend>(
        &mut self,
        event: I::TabletToolButtonEvent,
    ) {
        let Some(tool) = self.wayland.seat.tablet_seat().get_tool(&event.tool()) else {
            return;
        };
        tool.button(
            self,
            &ButtonEvent {
                serial: SERIAL_COUNTER.next_serial(),
                button: event.button(),
                state: event.button_state(),
                time: event.time(),
            },
        );
        tool.frame(self, event.time());
    }

    /// The tablet's area mapped onto the output the pointer is on.
    fn pen_location<I: InputBackend>(
        &self,
        event: &impl AbsolutePositionEvent<I>,
    ) -> Option<Point<f64, Logical>> {
        let geometry = self
            .shell
            .monitor_at(self.input.pointer_location)
            .or_else(|| self.shell.focused_monitor())
            .map(Monitor::geometry)?;
        Some(event.position_transformed(geometry.size) + geometry.loc.to_f64())
    }

    /// The client surface under the pen, and its origin.
    fn pen_focus(&self, location: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
        let (target, origin) = self.shell.pointer_focus_under(location)?;
        Some((target.surface()?.clone(), origin))
    }

    /// Moves the cursor with the pen. The pointer's own focus catches up in
    /// the event loop's reconcile.
    fn follow_pen(&mut self, location: Point<f64, Logical>) {
        let previous = self.input.pointer_location;
        self.input.pointer_location = location;
        self.queue_redraw_at(previous);
        self.queue_redraw_at(location);
    }
}

fn axis_frame<I: InputBackend>(event: &impl TabletToolEvent<I>) -> AxisFrame {
    AxisFrame {
        pressure: event.pressure_has_changed().then(|| event.pressure()),
        distance: event.distance_has_changed().then(|| event.distance()),
        tilt: event.tilt_has_changed().then(|| event.tilt()),
        rotation: event.rotation_has_changed().then(|| event.rotation()),
        slider: event.slider_has_changed().then(|| event.slider_position()),
        wheel: event
            .wheel_has_changed()
            .then(|| (event.wheel_delta(), event.wheel_delta_discrete())),
    }
}
