//! Input from `crownos_input_injector_v1`, fed through the same path as a
//! physical device.
//!
//! Relative motion, buttons, scrolling and keys become events of a tiny
//! [`InputBackend`] and go through [`State::process_input_event`], so an
//! injected keystroke can trigger a compositor shortcut and an injected
//! motion can hit an armed input-capture edge, exactly like local input.
//! Absolute motion names an output, which no `InputBackend` event can carry,
//! so it is mapped through the layout here and warps the pointer directly.
//! Touch is emulated as the primary button of the pointer until the seat
//! grows a touch capability.

use smithay::{
    backend::input::{
        Axis as SmithayAxis, AxisRelativeDirection, AxisSource, ButtonState as SmithayButton,
        Device, DeviceCapability, Event, InputBackend, InputEvent, InputTime,
        KeyState as SmithayKey, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent,
        PointerMotionEvent, UnusedEvent,
    },
    input::keyboard::Keycode,
    output::Output,
    utils::{Logical, Point},
};
use wayland_server::protocol::wl_output::WlOutput;

use protocols::crownos_input::{
    Axis, ButtonState, InjectedEvent, InjectedFrame, KeyState, output_point_to_layout,
};

use crate::state::State;

/// evdev `BTN_LEFT`: what an emulated touch presses.
const TOUCH_BUTTON: u32 = 0x110;
/// XKB keycodes are evdev codes offset by 8, for historical X11 reasons.
const XKB_KEYCODE_OFFSET: u32 = 8;

#[derive(Debug)]
pub struct InjectedInput;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InjectedDevice;

impl Device for InjectedDevice {
    fn id(&self) -> String {
        "crownos-input-injector".to_owned()
    }

    fn name(&self) -> String {
        "CrownOS injected input".to_owned()
    }

    fn has_capability(&self, capability: DeviceCapability) -> bool {
        matches!(
            capability,
            DeviceCapability::Keyboard | DeviceCapability::Pointer
        )
    }

    fn usb_id(&self) -> Option<(u32, u32)> {
        None
    }

    fn syspath(&self) -> Option<std::path::PathBuf> {
        None
    }
}

pub struct Motion {
    delta: Point<f64, Logical>,
    time: InputTime,
}

pub struct Button {
    button: u32,
    state: SmithayButton,
    time: InputTime,
}

pub struct Scroll {
    axis: SmithayAxis,
    value: f64,
    value120: i32,
    time: InputTime,
}

pub struct Key {
    key: u32,
    state: SmithayKey,
    time: InputTime,
}

macro_rules! injected_event {
    ($($event:ty),*) => {$(
        impl Event<InjectedInput> for $event {
            fn time(&self) -> InputTime {
                self.time
            }

            fn device(&self) -> InjectedDevice {
                InjectedDevice
            }
        }
    )*};
}

injected_event!(Motion, Button, Scroll, Key);

impl PointerMotionEvent<InjectedInput> for Motion {
    fn delta_x(&self) -> f64 {
        self.delta.x
    }

    fn delta_y(&self) -> f64 {
        self.delta.y
    }

    fn delta_x_unaccel(&self) -> f64 {
        self.delta.x
    }

    fn delta_y_unaccel(&self) -> f64 {
        self.delta.y
    }
}

impl PointerButtonEvent<InjectedInput> for Button {
    fn button_code(&self) -> u32 {
        self.button
    }

    fn state(&self) -> SmithayButton {
        self.state
    }
}

impl PointerAxisEvent<InjectedInput> for Scroll {
    fn amount(&self, axis: SmithayAxis) -> Option<f64> {
        (axis == self.axis).then_some(self.value)
    }

    fn amount_v120(&self, axis: SmithayAxis) -> Option<f64> {
        (axis == self.axis && self.value120 != 0).then_some(f64::from(self.value120))
    }

    fn source(&self) -> AxisSource {
        if self.value120 != 0 {
            AxisSource::Wheel
        } else {
            AxisSource::Continuous
        }
    }

    fn relative_direction(&self, _axis: SmithayAxis) -> AxisRelativeDirection {
        AxisRelativeDirection::Identical
    }
}

impl KeyboardKeyEvent<InjectedInput> for Key {
    fn key_code(&self) -> Keycode {
        Keycode::new(self.key + XKB_KEYCODE_OFFSET)
    }

    fn state(&self) -> SmithayKey {
        self.state
    }

    fn count(&self) -> u32 {
        u32::from(self.state == SmithayKey::Pressed)
    }
}

impl InputBackend for InjectedInput {
    type Device = InjectedDevice;
    type KeyboardKeyEvent = Key;
    type PointerAxisEvent = Scroll;
    type PointerButtonEvent = Button;
    type PointerMotionEvent = Motion;
    type PointerMotionAbsoluteEvent = UnusedEvent;
    type GestureSwipeBeginEvent = UnusedEvent;
    type GestureSwipeUpdateEvent = UnusedEvent;
    type GestureSwipeEndEvent = UnusedEvent;
    type GesturePinchBeginEvent = UnusedEvent;
    type GesturePinchUpdateEvent = UnusedEvent;
    type GesturePinchEndEvent = UnusedEvent;
    type GestureHoldBeginEvent = UnusedEvent;
    type GestureHoldEndEvent = UnusedEvent;
    type TouchDownEvent = UnusedEvent;
    type TouchUpEvent = UnusedEvent;
    type TouchMotionEvent = UnusedEvent;
    type TouchCancelEvent = UnusedEvent;
    type TouchFrameEvent = UnusedEvent;
    type TabletToolAxisEvent = UnusedEvent;
    type TabletToolProximityEvent = UnusedEvent;
    type TabletToolTipEvent = UnusedEvent;
    type TabletToolButtonEvent = UnusedEvent;
    type SwitchToggleEvent = UnusedEvent;
    type SpecialEvent = ();
}

fn button_state(state: ButtonState) -> SmithayButton {
    match state {
        ButtonState::Pressed => SmithayButton::Pressed,
        _ => SmithayButton::Released,
    }
}

fn key_state(state: KeyState) -> SmithayKey {
    match state {
        KeyState::Pressed => SmithayKey::Pressed,
        _ => SmithayKey::Released,
    }
}

fn scroll_axis(axis: Axis) -> SmithayAxis {
    match axis {
        Axis::HorizontalScroll => SmithayAxis::Horizontal,
        _ => SmithayAxis::Vertical,
    }
}

impl State {
    /// Applies one injected frame, in order.
    pub fn apply_injected_frame(&mut self, frame: InjectedFrame) {
        let time = InputTime::from_micros(frame.time_usec);
        for event in frame.events {
            self.apply_injected_event(event, time);
        }
    }

    fn apply_injected_event(&mut self, event: InjectedEvent, time: InputTime) {
        let event = match event {
            InjectedEvent::PointerMotion { delta } => InputEvent::PointerMotion {
                event: Motion { delta, time },
            },
            InjectedEvent::Button { button, state } => InputEvent::PointerButton {
                event: Button {
                    button,
                    state: button_state(state),
                    time,
                },
            },
            InjectedEvent::Axis {
                axis,
                value,
                value120,
            } => InputEvent::PointerAxis {
                event: Scroll {
                    axis: scroll_axis(axis),
                    value,
                    value120,
                    time,
                },
            },
            InjectedEvent::Key { key, state } => InputEvent::Keyboard {
                event: Key {
                    key,
                    state: key_state(state),
                    time,
                },
            },
            InjectedEvent::PointerMotionAbsolute { output, position } => {
                self.warp_to_output(&output, position, time);
                return;
            }
            InjectedEvent::TouchDown {
                id,
                output,
                position,
            } => {
                if self.input.emulated_touch.is_some() {
                    return;
                }
                self.input.emulated_touch = Some(id);
                self.warp_to_output(&output, position, time);
                self.press_touch_button(SmithayButton::Pressed, time);
                return;
            }
            InjectedEvent::TouchMotion {
                id,
                output,
                position,
            } => {
                if self.input.emulated_touch == Some(id) {
                    self.warp_to_output(&output, position, time);
                }
                return;
            }
            InjectedEvent::TouchUp { id } => {
                if self.input.emulated_touch == Some(id) {
                    self.input.emulated_touch = None;
                    self.press_touch_button(SmithayButton::Released, time);
                }
                return;
            }
            InjectedEvent::TouchCancel => {
                if self.input.emulated_touch.take().is_some() {
                    self.press_touch_button(SmithayButton::Released, time);
                }
                return;
            }
        };
        self.process_input_event::<InjectedInput>(event);
    }

    fn press_touch_button(&mut self, state: SmithayButton, time: InputTime) {
        self.process_input_event::<InjectedInput>(InputEvent::PointerButton {
            event: Button {
                button: TOUCH_BUTTON,
                state,
                time,
            },
        });
    }

    /// Moves the pointer to a point in `output`'s logical space.
    pub fn warp_to_output(
        &mut self,
        output: &WlOutput,
        position: Point<f64, Logical>,
        time: InputTime,
    ) {
        let Some(geometry) = Output::from_resource(output).and_then(|output| {
            self.shell
                .monitor(&output)
                .map(|monitor| monitor.geometry())
        }) else {
            return;
        };
        self.warp_pointer(output_point_to_layout(geometry, position), time);
    }
}
