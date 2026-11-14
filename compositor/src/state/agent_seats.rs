//! The seats behind `crownos_agent_cursor_v1`: one pointer and keyboard per
//! agent, confined to the agent workspace's output, drawn as a named cursor.
//!
//! Agent input goes straight to Smithay's pointer and keyboard handles, never
//! through [`State::process_input_event`], so no compositor shortcut fires and
//! the user's own pointer and focus are untouched.

use std::collections::HashSet;

use protocols::crownos_agent_cursor::{
    AgentCursor, Axis, ButtonState, CreateCursorError, CursorEvent, CursorFrame, KeyState,
    RevokeReason,
};
use smithay::{
    backend::input::{ButtonState as SeatButton, InputTime, KeyState as SeatKey},
    input::{
        Seat,
        keyboard::{FilterResult, KeyboardHandle, Keycode, XkbConfig},
        pointer::{AxisFrame, ButtonEvent, MotionEvent, PointerHandle},
    },
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER, Serial},
};

use crate::{
    handlers::seat::KeyboardFocusTarget,
    rendering::{cursor::AgentOverlay, decoration::label::Label},
    shell::monitor::Monitor,
    state::State,
    utils::id::WindowId,
};

const AUDIT: &str = "crownpositor::audit::agent_cursor";
/// More agents than this share a desktop badly whatever the compositor does.
const MAX_AGENTS: usize = 8;
const XKB_KEYCODE_OFFSET: u32 = 8;
const LABEL_COLOR: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const MICROS_PER_MILLI: u64 = 1_000;

pub struct AgentSeat {
    cursor: AgentCursor,
    seat: Seat<State>,
    name: String,
    position: Point<f64, Logical>,
    focus: Option<WindowId>,
    held_keys: HashSet<u32>,
    held_buttons: HashSet<u32>,
}

#[derive(Default)]
pub struct AgentSeats {
    seats: Vec<AgentSeat>,
    created: u32,
}

impl AgentSeats {
    fn index_of(&self, cursor: &AgentCursor) -> Option<usize> {
        self.seats.iter().position(|seat| seat.cursor == *cursor)
    }

    pub fn is_empty(&self) -> bool {
        self.seats.is_empty()
    }
}

impl State {
    pub fn create_agent_seat(
        &mut self,
        cursor: AgentCursor,
        name: &str,
        color: u32,
    ) -> Result<String, CreateCursorError> {
        if self.agent_seats.seats.len() >= MAX_AGENTS {
            return Err(CreateCursorError::TooMany);
        }
        if self.shell.is_locked() {
            return Err(CreateCursorError::Refused);
        }
        let bounds = self
            .headless_location()
            .and_then(|location| self.shell.monitor_by_id(location.output))
            .map(Monitor::geometry)
            .ok_or(CreateCursorError::Refused)?;
        self.agent_seats.created += 1;
        let seat_name = format!("agent-{}", self.agent_seats.created);
        let display = self.common.display_handle.clone();
        let mut seat = self
            .wayland
            .seat_state
            .new_wl_seat(&display, seat_name.clone());
        seat.add_pointer();
        if let Err(error) = seat.add_keyboard(XkbConfig::default(), 200, 65) {
            tracing::warn!(target: AUDIT, %error, "an agent seat gets no keyboard");
            if let Some(global) = seat.global() {
                display.remove_global::<State>(global);
            }
            return Err(CreateCursorError::Refused);
        }
        let position = bounds.loc.to_f64() + bounds.size.to_f64().downscale(2.0).to_point();
        let labels = self.agent_labels(name);
        self.input
            .cursor
            .agents
            .push(AgentOverlay::new(position, color, labels));
        self.agent_seats.seats.push(AgentSeat {
            cursor,
            seat,
            name: name.to_owned(),
            position,
            focus: None,
            held_keys: HashSet::new(),
            held_buttons: HashSet::new(),
        });
        tracing::info!(target: AUDIT, agent = name, seat = %seat_name, "agent cursor created");
        self.queue_redraw();
        Ok(seat_name)
    }

    /// The agent's name rasterised at every output's scale.
    fn agent_labels(&mut self, name: &str) -> Vec<(f64, Label)> {
        let scales: Vec<f64> = self
            .shell
            .monitors()
            .iter()
            .map(|monitor| monitor.output().current_scale().fractional_scale())
            .collect();
        scales
            .into_iter()
            .filter_map(|scale| {
                self.text
                    .label(name, scale, LABEL_COLOR, true)
                    .cloned()
                    .map(|label| (scale, label))
            })
            .collect()
    }

    pub fn apply_agent_frame(&mut self, cursor: &AgentCursor, frame: CursorFrame) {
        let Some(index) = self.agent_seats.index_of(cursor) else {
            return;
        };
        if self.shell.is_locked() {
            return;
        }
        let Some(bounds) = self.agent_bounds() else {
            return;
        };
        let seat = self.agent_seats.seats[index].seat.clone();
        let (Some(pointer), Some(keyboard)) = (seat.get_pointer(), seat.get_keyboard()) else {
            return;
        };
        let time = frame_time(frame.time_ms);
        for event in frame.events {
            match event {
                CursorEvent::Motion { delta } => {
                    let position = self.agent_seats.seats[index].position + delta;
                    self.agent_motion(index, &pointer, clamp(position, bounds), time);
                }
                CursorEvent::MotionAbsolute { position } => {
                    let position = bounds.loc.to_f64() + position;
                    self.agent_motion(index, &pointer, clamp(position, bounds), time);
                }
                CursorEvent::Button { button, state } => {
                    let serial = SERIAL_COUNTER.next_serial();
                    let held = &mut self.agent_seats.seats[index].held_buttons;
                    match state {
                        ButtonState::Pressed => held.insert(button),
                        _ => held.remove(&button),
                    };
                    if state == ButtonState::Pressed {
                        self.agent_focus_under(index, &keyboard, serial);
                    }
                    pointer.button(
                        self,
                        &ButtonEvent {
                            serial,
                            time,
                            button,
                            state: seat_button(state),
                        },
                    );
                }
                CursorEvent::Axis { axis, value } => {
                    let axis = match axis {
                        Axis::VerticalScroll => smithay::backend::input::Axis::Vertical,
                        _ => smithay::backend::input::Axis::Horizontal,
                    };
                    pointer.axis(self, AxisFrame::new(time).value(axis, value));
                }
                CursorEvent::Key { key, state } => {
                    let held = &mut self.agent_seats.seats[index].held_keys;
                    match state {
                        KeyState::Pressed => held.insert(key),
                        _ => held.remove(&key),
                    };
                    self.agent_key(&keyboard, key, seat_key(state), time);
                }
            }
        }
        pointer.frame(self);
        self.queue_redraw();
    }

    fn agent_bounds(&self) -> Option<Rectangle<i32, Logical>> {
        let output = self.agent_output_id()?;
        self.shell.monitor_by_id(output).map(Monitor::geometry)
    }

    fn agent_motion(
        &mut self,
        index: usize,
        pointer: &PointerHandle<State>,
        location: Point<f64, Logical>,
        time: InputTime,
    ) {
        self.agent_seats.seats[index].position = location;
        if let Some(overlay) = self.input.cursor.agents.get_mut(index) {
            overlay.position = location;
        }
        let under = self.shell.pointer_focus_under(location);
        pointer.motion(
            self,
            under,
            &MotionEvent {
                location,
                serial: SERIAL_COUNTER.next_serial(),
                time,
            },
        );
    }

    fn agent_key(
        &mut self,
        keyboard: &KeyboardHandle<State>,
        key: u32,
        state: SeatKey,
        time: InputTime,
    ) {
        keyboard.input::<(), _>(
            self,
            Keycode::new(key + XKB_KEYCODE_OFFSET),
            state,
            SERIAL_COUNTER.next_serial(),
            time,
            |_, _, _| FilterResult::Forward,
        );
    }

    /// A press focuses the agent's keyboard on the window under its pointer,
    /// as a click does for the user.
    fn agent_focus_under(
        &mut self,
        index: usize,
        keyboard: &KeyboardHandle<State>,
        serial: Serial,
    ) {
        let location = self.agent_seats.seats[index].position;
        let Some((id, _)) = self.shell.window_under(location) else {
            return;
        };
        if self.agent_seats.seats[index].focus != Some(id) {
            self.set_agent_focus(index, keyboard, id, serial);
        }
    }

    fn set_agent_focus(
        &mut self,
        index: usize,
        keyboard: &KeyboardHandle<State>,
        id: WindowId,
        serial: Serial,
    ) {
        if !self.is_on_agent_workspace(id) {
            return;
        }
        let Some(window) = self.shell.tile(id).map(|tile| tile.window().clone()) else {
            return;
        };
        keyboard.set_focus(self, Some(KeyboardFocusTarget::Window(window)), serial);
        let seat = &mut self.agent_seats.seats[index];
        seat.focus = Some(id);
        let identifier = self
            .wayland
            .foreign_toplevels
            .identifier(id)
            .unwrap_or_default();
        seat.cursor.focus_changed(&identifier);
    }

    pub fn focus_agent_window(&mut self, cursor: &AgentCursor, identifier: &str) {
        let Some(index) = self.agent_seats.index_of(cursor) else {
            return;
        };
        let Some(id) = self.wayland.foreign_toplevels.window(identifier) else {
            return;
        };
        let Some(keyboard) = self.agent_seats.seats[index].seat.get_keyboard() else {
            return;
        };
        self.set_agent_focus(index, &keyboard, id, SERIAL_COUNTER.next_serial());
    }

    /// Releases whatever the agent held, drops its focus and takes its seat
    /// away.
    pub fn destroy_agent_seat(&mut self, cursor: &AgentCursor) {
        let Some(index) = self.agent_seats.index_of(cursor) else {
            return;
        };
        let seat = self.agent_seats.seats[index].seat.clone();
        let time = frame_time(0);
        if let Some(pointer) = seat.get_pointer() {
            let held: Vec<u32> = self.agent_seats.seats[index].held_buttons.drain().collect();
            for button in held {
                pointer.button(
                    self,
                    &ButtonEvent {
                        serial: SERIAL_COUNTER.next_serial(),
                        time,
                        button,
                        state: SeatButton::Released,
                    },
                );
            }
            pointer.frame(self);
        }
        if let Some(keyboard) = seat.get_keyboard() {
            let held: Vec<u32> = self.agent_seats.seats[index].held_keys.drain().collect();
            for key in held {
                self.agent_key(&keyboard, key, SeatKey::Released, time);
            }
            keyboard.set_focus(self, None, SERIAL_COUNTER.next_serial());
        }
        if let Some(global) = seat.global() {
            self.common.display_handle.remove_global::<State>(global);
        }
        let removed = self.agent_seats.seats.remove(index);
        if index < self.input.cursor.agents.len() {
            self.input.cursor.agents.remove(index);
        }
        tracing::info!(target: AUDIT, agent = removed.name, "agent cursor destroyed");
        self.queue_redraw();
    }

    /// Takes every agent's seat away, as the user's shortcut and a session
    /// lock do.
    pub fn revoke_agents(&mut self, reason: RevokeReason) {
        let cursors = self.wayland.agent_cursor_state.revoke_all(reason);
        for cursor in &cursors {
            self.destroy_agent_seat(cursor);
        }
    }

    /// A locked session has no agents at work in it.
    pub fn revoke_agents_if_locked(&mut self) {
        if self.shell.is_locked() && !self.agent_seats.is_empty() {
            self.revoke_agents(RevokeReason::Locked);
        }
    }
}

fn frame_time(time_ms: u32) -> InputTime {
    let millis = if time_ms == 0 {
        let now: std::time::Duration = smithay::utils::Clock::<smithay::utils::Monotonic>::new()
            .now()
            .into();
        now.as_millis() as u64
    } else {
        u64::from(time_ms)
    };
    InputTime::from_micros(millis * MICROS_PER_MILLI)
}

fn clamp(position: Point<f64, Logical>, bounds: Rectangle<i32, Logical>) -> Point<f64, Logical> {
    let bounds = bounds.to_f64();
    Point::from((
        position
            .x
            .clamp(bounds.loc.x, bounds.loc.x + bounds.size.w - 1.0),
        position
            .y
            .clamp(bounds.loc.y, bounds.loc.y + bounds.size.h - 1.0),
    ))
}

const fn seat_button(state: ButtonState) -> SeatButton {
    match state {
        ButtonState::Pressed => SeatButton::Pressed,
        _ => SeatButton::Released,
    }
}

const fn seat_key(state: KeyState) -> SeatKey {
    match state {
        KeyState::Pressed => SeatKey::Pressed,
        _ => SeatKey::Released,
    }
}
