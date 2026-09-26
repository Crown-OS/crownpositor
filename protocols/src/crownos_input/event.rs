//! Injected input, as the compositor receives it: one frame of events, in
//! order, with the bookkeeping that lets a dying injector let go of what it
//! was holding.

use smithay::utils::{Logical, Point};
use wayland_server::protocol::wl_output::WlOutput;

pub use crownos_protocols::input::v1::server::crownos_input_injector_v1::{
    Axis, ButtonState, KeyState,
};

#[derive(Debug, Clone, PartialEq)]
pub enum InjectedEvent {
    PointerMotion {
        delta: Point<f64, Logical>,
    },
    PointerMotionAbsolute {
        output: WlOutput,
        position: Point<f64, Logical>,
    },
    Button {
        button: u32,
        state: ButtonState,
    },
    Axis {
        axis: Axis,
        value: f64,
        value120: i32,
    },
    Key {
        key: u32,
        state: KeyState,
    },
    TouchDown {
        id: i32,
        output: WlOutput,
        position: Point<f64, Logical>,
    },
    TouchMotion {
        id: i32,
        output: WlOutput,
        position: Point<f64, Logical>,
    },
    TouchUp {
        id: i32,
    },
    TouchCancel,
}

/// One `frame` of an injector.
#[derive(Debug, Clone, PartialEq)]
pub struct InjectedFrame {
    pub events: Vec<InjectedEvent>,
    pub time_usec: u64,
}

/// What an injector currently holds down, so destroying it can let go.
#[derive(Debug, Default)]
pub struct HeldInput {
    keys: Vec<u32>,
    buttons: Vec<u32>,
    touches: Vec<(i32, WlOutput)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TouchError {
    #[error("touch point {0} is already down")]
    AlreadyDown(i32),
    #[error("touch point {0} is not down")]
    NotDown(i32),
}

impl HeldInput {
    pub fn key(&mut self, key: u32, state: KeyState) {
        track(&mut self.keys, key, state == KeyState::Pressed);
    }

    pub fn button(&mut self, button: u32, state: ButtonState) {
        track(&mut self.buttons, button, state == ButtonState::Pressed);
    }

    pub fn touch_down(&mut self, id: i32, output: &WlOutput) -> Result<(), TouchError> {
        if self.touches.iter().any(|(held, _)| *held == id) {
            return Err(TouchError::AlreadyDown(id));
        }
        self.touches.push((id, output.clone()));
        Ok(())
    }

    /// The output the point went down on.
    pub fn touch_output(&self, id: i32) -> Result<WlOutput, TouchError> {
        self.touches
            .iter()
            .find(|(held, _)| *held == id)
            .map(|(_, output)| output.clone())
            .ok_or(TouchError::NotDown(id))
    }

    pub fn touch_up(&mut self, id: i32) -> Result<(), TouchError> {
        let index = self
            .touches
            .iter()
            .position(|(held, _)| *held == id)
            .ok_or(TouchError::NotDown(id))?;
        self.touches.swap_remove(index);
        Ok(())
    }

    pub fn touch_cancel(&mut self) {
        self.touches.clear();
    }

    /// The events that undo everything still held, and forget it.
    pub fn let_go(&mut self) -> Vec<InjectedEvent> {
        let touches = (!self.touches.is_empty()).then_some(InjectedEvent::TouchCancel);
        self.touches.clear();
        self.keys
            .drain(..)
            .map(|key| InjectedEvent::Key {
                key,
                state: KeyState::Released,
            })
            .chain(self.buttons.drain(..).map(|button| InjectedEvent::Button {
                button,
                state: ButtonState::Released,
            }))
            .chain(touches)
            .collect()
    }
}

fn track(held: &mut Vec<u32>, code: u32, pressed: bool) {
    let position = held.iter().position(|existing| *existing == code);
    match (position, pressed) {
        (None, true) => held.push(code),
        (Some(index), false) => {
            held.swap_remove(index);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letting_go_releases_exactly_what_is_held() {
        let mut held = HeldInput::default();
        held.key(30, KeyState::Pressed);
        held.key(31, KeyState::Pressed);
        held.key(30, KeyState::Released);
        held.button(0x110, ButtonState::Pressed);
        held.button(0x110, ButtonState::Pressed);

        assert_eq!(
            held.let_go(),
            vec![
                InjectedEvent::Key {
                    key: 31,
                    state: KeyState::Released
                },
                InjectedEvent::Button {
                    button: 0x110,
                    state: ButtonState::Released
                },
            ]
        );
        assert!(held.let_go().is_empty(), "nothing is released twice");
    }

    #[test]
    fn an_unknown_touch_id_is_refused() {
        let mut held = HeldInput::default();
        assert_eq!(held.touch_up(3), Err(TouchError::NotDown(3)));
        assert_eq!(held.touch_output(3), Err(TouchError::NotDown(3)));
        held.touch_cancel();
        assert!(held.let_go().is_empty());
    }
}
