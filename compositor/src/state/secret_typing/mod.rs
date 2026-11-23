//! Typing a secret for crownos-secretsd: one request at a time, confirmed in
//! the portal dialog, then typed through text-input into the window that the
//! request names.

mod checks;
mod text_entry;

use std::time::{Duration, Instant};

use calloop::timer::{TimeoutAction, Timer};
use crownos_protocols::secret_input::v1::server::crownos_secret_input_request_v1::CrownosSecretInputRequestV1;
use protocols::crownos_secret_input::{Outcome, RequestDetails, SecretInputRequest, ValueKind};
use smithay::{
    reexports::wayland_server::{Resource, backend::ObjectId},
    wayland::{input_method::InputMethodSeat, text_input::TextInputSeat},
};

use self::checks::{
    FieldCheck, check_field, check_target, display_text, is_terminal_app, outcome_after_wait,
};
use crate::{
    state::State,
    utils::{
        id::WindowId,
        portal::{
            SecretTypingAnswer, SecretTypingPrompt, ask_secret_typing, close_dialog,
            secret_typing_request_path,
        },
        surface,
    },
};

const AUDIT: &str = "crownpositor::audit::secret_input";
/// How long a field has to take the focus and declare its purpose.
const FIELD_WAIT: Duration = Duration::from_millis(2000);
const POLL_INTERVAL: Duration = Duration::from_millis(25);
/// How long the window stays sensitive after the last value.
const SENSITIVE_AFTER: Duration = Duration::from_secs(30);

/// The request being asked about or typed. There is at most one.
#[derive(Default)]
pub struct SecretTyping {
    active: Option<ActiveTyping>,
    next_ticket: u64,
}

struct ActiveTyping {
    request: SecretInputRequest,
    window: WindowId,
    ticket: u64,
    phase: Phase,
    turned_input_method_on: bool,
}

#[derive(Clone)]
enum Phase {
    Asking,
    Answered,
    Typing {
        next_value: usize,
        deadline: Instant,
        tab_from: Option<(ObjectId, u32)>,
    },
}

impl State {
    pub fn start_secret_typing(&mut self, request: SecretInputRequest) {
        if self.secret_typing.active.is_some() || request.has_bad_value() {
            return finish_request(request, Outcome::Refused);
        }
        if self.shell.is_locked() {
            return finish_request(request, Outcome::SessionLocked);
        }
        let (window, window_title) = match self.typing_window(request.details()) {
            Ok(window) => window,
            Err(outcome) => return finish_request(request, outcome),
        };
        if is_terminal_app(&request.details().app_id) {
            return finish_request(request, Outcome::Refused);
        }
        self.secret_typing.next_ticket += 1;
        let ticket = self.secret_typing.next_ticket;
        let details = request.details();
        let prompt = SecretTypingPrompt {
            ticket,
            agent: display_text(&details.agent),
            secret_names: display_text(&details.secret_names),
            app_id: display_text(&details.app_id),
            window_title: display_text(&window_title),
            origin: display_text(&details.origin),
        };
        tracing::info!(
            target: AUDIT,
            app_id = details.app_id,
            secrets = details.secret_names,
            ticket,
            "asking the user to allow typing"
        );
        self.secret_typing.active = Some(ActiveTyping {
            request,
            window,
            ticket,
            phase: Phase::Asking,
            turned_input_method_on: false,
        });
        self.common
            .tasks
            .run(ask_secret_typing(prompt), move |state, answer| {
                state.secret_typing_answered(ticket, answer);
            });
    }

    pub fn withdraw_secret_typing(&mut self, resource: &CrownosSecretInputRequestV1) {
        let withdrawn = self
            .secret_typing
            .active
            .as_ref()
            .is_some_and(|active| active.request.is_for(resource));
        if withdrawn {
            self.finish_secret_typing(Outcome::Cancelled);
        }
    }

    fn typing_window(&self, details: &RequestDetails) -> Result<(WindowId, String), Outcome> {
        let tile = self
            .wayland
            .foreign_toplevels
            .window(&details.window_identifier)
            .and_then(|window| self.shell.tile(window))
            .ok_or(Outcome::UnknownWindow)?;
        check_target(&details.app_id, surface::app_id(tile.surface()).as_deref())?;
        Ok((tile.id(), tile.title().to_owned()))
    }

    fn secret_typing_answered(&mut self, ticket: u64, answer: Result<SecretTypingAnswer, String>) {
        let Some(active) = self.secret_typing.active.as_ref() else {
            return;
        };
        if active.ticket != ticket {
            return;
        }
        let window = active.window;
        let outcome = match answer {
            Ok(SecretTypingAnswer::Allowed) => None,
            Ok(SecretTypingAnswer::Denied) => Some(Outcome::Cancelled),
            Ok(SecretTypingAnswer::TimedOut) => Some(Outcome::TimedOut),
            Err(error) => {
                tracing::warn!(target: AUDIT, %error, "nobody could be asked");
                Some(Outcome::Refused)
            }
        };
        let outcome = outcome
            .or_else(|| self.shell.is_locked().then_some(Outcome::SessionLocked))
            .or_else(|| match self.typing_window(active.request.details()) {
                Ok((still, _)) if still == window => None,
                Ok(_) => Some(Outcome::UnknownWindow),
                Err(outcome) => Some(outcome),
            });
        if let Some(outcome) = outcome {
            if let Some(active) = self.secret_typing.active.as_mut() {
                active.phase = Phase::Answered;
            }
            return self.finish_secret_typing(outcome);
        }
        let handle = self.text_input_handle();
        let turn_on =
            !handle.compositor_input_method() && !self.wayland.seat.input_method().has_instance();
        if turn_on {
            handle.set_compositor_input_method(true);
        }
        if let Some(active) = self.secret_typing.active.as_mut() {
            active.turned_input_method_on = turn_on;
            active.phase = Phase::Typing {
                next_value: 0,
                deadline: Instant::now() + FIELD_WAIT,
                tab_from: None,
            };
        }
        self.arm_secret_typing_steps();
    }

    fn arm_secret_typing_steps(&mut self) {
        let armed = self.common.event_loop_handle.insert_source(
            Timer::from_duration(POLL_INTERVAL),
            |_, _, state| {
                if state.step_secret_typing() {
                    TimeoutAction::ToDuration(POLL_INTERVAL)
                } else {
                    TimeoutAction::Drop
                }
            },
        );
        if let Err(error) = armed {
            tracing::warn!(target: AUDIT, %error, "cannot schedule the typing");
            self.finish_secret_typing(Outcome::Refused);
        }
    }

    /// Types the next value when its field is ready. Returns whether to look again.
    fn step_secret_typing(&mut self) -> bool {
        let Some(active) = self.secret_typing.active.as_ref() else {
            return false;
        };
        let Phase::Typing {
            next_value,
            deadline,
            tab_from,
        } = active.phase.clone()
        else {
            return false;
        };
        let window = active.window;
        let Some(value) = active.request.values().get(next_value) else {
            self.finish_secret_typing(Outcome::Typed);
            return false;
        };
        let needs_password_field =
            active.request.details().require_password_field && value.kind == ValueKind::Password;
        let tab_after = value.tab_after;
        let value_count = active.request.values().len();
        if !active.request.is_pending() {
            self.finish_secret_typing(Outcome::Cancelled);
            return false;
        }
        if self.shell.is_locked() {
            self.finish_secret_typing(Outcome::SessionLocked);
            return false;
        }
        let Some(window_surface) = self.shell.tile(window).map(|tile| tile.surface().clone())
        else {
            self.finish_secret_typing(Outcome::UnknownWindow);
            return false;
        };
        let focus_on_window =
            self.shell.focused_window_id() == Some(window) && self.keyboard_is_in(&window_surface);
        let text_input = focus_on_window
            .then(|| self.active_text_input_in(&window_surface))
            .flatten();
        let applied = text_input
            .as_ref()
            .and_then(|text_input| self.wayland.text_input_purposes.applied(text_input));
        let enables_before = match (&tab_from, &text_input) {
            (Some((object, enables)), Some(text_input)) if text_input.id() == *object => {
                Some(*enables)
            }
            _ => None,
        };
        match check_field(applied, needs_password_field, enables_before) {
            FieldCheck::Refuse(outcome) => {
                self.finish_secret_typing(outcome);
                return false;
            }
            FieldCheck::Wait if Instant::now() >= deadline => {
                self.finish_secret_typing(outcome_after_wait(
                    focus_on_window,
                    needs_password_field,
                ));
                return false;
            }
            FieldCheck::Wait => return true,
            FieldCheck::Ready => {}
        }
        let remaining = u32::try_from(value_count - next_value).unwrap_or(u32::MAX);
        self.windows
            .flag(window, FIELD_WAIT * remaining + SENSITIVE_AFTER);
        if let Some(active) = self.secret_typing.active.as_ref()
            && let Some(value) = active.request.values().get(next_value)
        {
            text_entry::commit_text(self.wayland.seat.text_input(), &value.text);
        }
        let tab_from = if tab_after {
            self.press_tab();
            text_input
                .zip(applied)
                .map(|(text_input, applied)| (text_input.id(), applied.enables))
        } else {
            None
        };
        if next_value + 1 == value_count {
            self.windows.flag(window, SENSITIVE_AFTER);
            self.finish_secret_typing(Outcome::Typed);
            return false;
        }
        if let Some(active) = self.secret_typing.active.as_mut() {
            active.phase = Phase::Typing {
                next_value: next_value + 1,
                deadline: Instant::now() + FIELD_WAIT,
                tab_from,
            };
        }
        true
    }

    fn finish_secret_typing(&mut self, outcome: Outcome) {
        let Some(active) = self.secret_typing.active.take() else {
            return;
        };
        if active.turned_input_method_on {
            self.text_input_handle().set_compositor_input_method(false);
        }
        if matches!(active.phase, Phase::Asking) {
            self.common
                .tasks
                .spawn(close_dialog(secret_typing_request_path(active.ticket)));
        }
        tracing::info!(
            target: AUDIT,
            app_id = active.request.details().app_id,
            ticket = active.ticket,
            ?outcome,
            "typing request ended"
        );
        active.request.finish(outcome);
    }
}

fn finish_request(request: SecretInputRequest, outcome: Outcome) {
    tracing::info!(
        target: AUDIT,
        app_id = request.details().app_id,
        ?outcome,
        "typing request refused before the dialog"
    );
    request.finish(outcome);
}
