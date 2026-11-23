//! The consent dialog the compositor started, and whether it showed its
//! surface. A choice counts only from the process the compositor started,
//! after that same process mapped a verified consent layer. One dialog is open
//! at a time, for typing, runs and proxies, context access and agent access;
//! a few more questions wait, and the rest are refused.

use std::collections::VecDeque;

use crownos_config::consent_dialog::ConsentQuestion;
use smithay::{desktop::LayerSurface, reexports::wayland_server::Resource};
use tokio::sync::oneshot;

use crate::{
    state::{ClientState, State},
    utils::{
        consent_dialog::{ConsentOutcome, DialogExit, dialog_command, wait_for_dialog},
        consent_surface::consent_dialog_executable,
    },
};

const AUDIT: &str = "crownpositor::audit::consent";
/// How many questions may wait while a dialog is open.
const MAX_WAITING: usize = 4;
/// How many of them one asker may have, so one client cannot fill the queue.
const MAX_WAITING_PER_ASKER: usize = 1;

type OnAnswer = Box<dyn FnOnce(&mut State, u64, ConsentOutcome) + Send>;

#[derive(Default)]
pub struct ConsentDialogs {
    next_ticket: u64,
    open: Option<OpenDialog>,
    waiting: VecDeque<WaitingQuestion>,
}

struct OpenDialog {
    ticket: u64,
    pid: i32,
    shown: bool,
    withdraw: Option<oneshot::Sender<()>>,
}

struct WaitingQuestion {
    ticket: u64,
    asker: String,
    question: ConsentQuestion,
    on_answer: OnAnswer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Admission {
    Start,
    Wait,
    Refuse,
}

impl ConsentDialogs {
    fn admission(&self, asker: &str) -> Admission {
        let waiting_of_asker = self
            .waiting
            .iter()
            .filter(|waiting| waiting.asker == asker)
            .count();
        if self.open.is_none() && self.waiting.is_empty() {
            Admission::Start
        } else if self.waiting.len() < MAX_WAITING && waiting_of_asker < MAX_WAITING_PER_ASKER {
            Admission::Wait
        } else {
            Admission::Refuse
        }
    }
}

impl State {
    /// Shows the question when no other dialog is open, or queues it, and
    /// calls `on_answer` with its ticket and the answer on the compositor
    /// thread. `asker` names who asks, such as an app key; each asker may have
    /// one question waiting. Returns the ticket, for [`Self::withdraw_consent`].
    pub fn ask_consent(
        &mut self,
        asker: &str,
        question: ConsentQuestion,
        on_answer: impl FnOnce(&mut State, u64, ConsentOutcome) + Send + 'static,
    ) -> u64 {
        self.consent_dialogs.next_ticket += 1;
        let ticket = self.consent_dialogs.next_ticket;
        let on_answer: OnAnswer = Box::new(on_answer);
        match self.consent_dialogs.admission(asker) {
            Admission::Start => self.start_consent(ticket, question, on_answer),
            Admission::Wait => self.consent_dialogs.waiting.push_back(WaitingQuestion {
                ticket,
                asker: asker.to_owned(),
                question,
                on_answer,
            }),
            Admission::Refuse => {
                tracing::warn!(target: AUDIT, ticket, asker, "too many consent questions; refused");
                self.answer_later(ticket, on_answer, ConsentOutcome::Unavailable);
            }
        }
        ticket
    }

    /// Closes the dialog of `ticket`, or takes it out of the queue; its
    /// answer is [`ConsentOutcome::Withdrawn`].
    pub fn withdraw_consent(&mut self, ticket: u64) {
        if let Some(open) = self
            .consent_dialogs
            .open
            .as_mut()
            .filter(|open| open.ticket == ticket)
        {
            if let Some(withdraw) = open.withdraw.take() {
                let _ = withdraw.send(());
            }
            return;
        }
        if let Some(index) = self
            .consent_dialogs
            .waiting
            .iter()
            .position(|waiting| waiting.ticket == ticket)
            && let Some(waiting) = self.consent_dialogs.waiting.remove(index)
        {
            self.answer_later(ticket, waiting.on_answer, ConsentOutcome::Withdrawn);
        }
    }

    fn answer_later(&self, ticket: u64, on_answer: OnAnswer, outcome: ConsentOutcome) {
        self.common
            .tasks
            .deliver(move |state| on_answer(state, ticket, outcome));
    }

    fn start_consent(&mut self, ticket: u64, question: ConsentQuestion, on_answer: OnAnswer) {
        let Some(executable) = consent_dialog_executable() else {
            tracing::warn!(target: AUDIT, "the consent dialog is not installed");
            self.answer_later(ticket, on_answer, ConsentOutcome::Unavailable);
            return self.start_next_consent();
        };
        let spawned = self
            .common
            .tasks
            .spawn_process(&mut dialog_command(&question, &executable));
        let child = match spawned {
            Ok(child) => child,
            Err(error) => {
                tracing::warn!(target: AUDIT, %error, "cannot start the consent dialog");
                self.answer_later(ticket, on_answer, ConsentOutcome::Unavailable);
                return self.start_next_consent();
            }
        };
        let Some(pid) = child.id().and_then(|pid| i32::try_from(pid).ok()) else {
            self.answer_later(ticket, on_answer, ConsentOutcome::Unavailable);
            return self.start_next_consent();
        };
        let (withdraw, withdrawn) = oneshot::channel();
        self.consent_dialogs.open = Some(OpenDialog {
            ticket,
            pid,
            shown: false,
            withdraw: Some(withdraw),
        });
        self.common
            .tasks
            .run(wait_for_dialog(child, withdrawn), move |state, exit| {
                let answer = state.consent_answer(&question, exit);
                on_answer(state, ticket, answer);
                state.start_next_consent();
            });
    }

    fn start_next_consent(&mut self) {
        if self.consent_dialogs.open.is_some() {
            return;
        }
        if let Some(next) = self.consent_dialogs.waiting.pop_front() {
            self.start_consent(next.ticket, next.question, next.on_answer);
        }
    }

    /// Records that a verified consent layer of a dialog this compositor
    /// started was mapped.
    pub fn consent_layer_mapped(&mut self, layer: &LayerSurface) {
        let Some(process) = layer.wl_surface().client().and_then(|client| {
            client
                .get_data::<ClientState>()
                .and_then(|state| state.peer_process.as_ref())
                .filter(|process| process.is_alive() && !process.is_preloaded())
                .map(|process| process.pid())
        }) else {
            return;
        };
        if let Some(open) = self
            .consent_dialogs
            .open
            .as_mut()
            .filter(|open| open.pid == process)
        {
            open.shown = true;
        }
    }

    fn consent_answer(&mut self, question: &ConsentQuestion, exit: DialogExit) -> ConsentOutcome {
        let shown = self
            .consent_dialogs
            .open
            .take()
            .is_some_and(|open| open.shown);
        decide(question, exit, shown)
    }
}

/// The answer from the dialog's exit status and whether it showed itself. An
/// allow, or any answer with the extra questions' values ("never allow"),
/// counts only from a dialog that showed itself.
fn decide(question: &ConsentQuestion, exit: DialogExit, shown: bool) -> ConsentOutcome {
    match exit {
        DialogExit::Exited(status) => match question.answer(status) {
            Ok(answer) if !shown && (answer.granted || answer.option_values.is_some()) => {
                tracing::warn!(target: AUDIT, "a dialog answered without showing itself");
                ConsentOutcome::Unavailable
            }
            Ok(answer) => ConsentOutcome::Answered(answer),
            Err(not_asked) => {
                tracing::warn!(target: AUDIT, %not_asked, "the consent dialog asked nobody");
                ConsentOutcome::Unavailable
            }
        },
        DialogExit::Failed => ConsentOutcome::Unavailable,
        DialogExit::TimedOut => ConsentOutcome::TimedOut,
        DialogExit::Withdrawn => ConsentOutcome::Withdrawn,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn question() -> ConsentQuestion {
        ConsentQuestion {
            dangerous: true,
            title: String::new(),
            message: String::new(),
            grant: "Allow".into(),
            deny: "Deny".into(),
            options: Vec::new(),
        }
    }

    #[test]
    fn an_allow_counts_only_from_a_dialog_that_showed_itself() {
        let allowed = || DialogExit::Exited(Some(16));
        assert!(decide(&question(), allowed(), true).granted());
        assert_eq!(
            decide(&question(), allowed(), false),
            ConsentOutcome::Unavailable
        );
        let denied = decide(&question(), DialogExit::Exited(Some(17)), true);
        assert!(matches!(denied, ConsentOutcome::Answered(ref answer) if !answer.granted));
        assert_eq!(
            decide(&question(), DialogExit::Exited(Some(17)), false),
            ConsentOutcome::Unavailable,
            "a deny with values from a dialog that never showed"
        );
        assert!(matches!(
            decide(&question(), DialogExit::Exited(Some(1)), true),
            ConsentOutcome::Answered(_)
        ));
        assert_eq!(
            decide(&question(), DialogExit::Exited(Some(2)), true),
            ConsentOutcome::Unavailable
        );
        assert_eq!(
            decide(&question(), DialogExit::TimedOut, true),
            ConsentOutcome::TimedOut
        );
    }

    #[test]
    fn a_second_question_waits_and_the_queue_is_bounded() {
        let mut dialogs = ConsentDialogs::default();
        assert_eq!(dialogs.admission("first"), Admission::Start);
        dialogs.open = Some(OpenDialog {
            ticket: 1,
            pid: 1,
            shown: false,
            withdraw: None,
        });
        for ticket in 2..2 + MAX_WAITING as u64 {
            let asker = format!("asker-{ticket}");
            assert_eq!(dialogs.admission(&asker), Admission::Wait);
            dialogs.waiting.push_back(WaitingQuestion {
                ticket,
                asker,
                question: question(),
                on_answer: Box::new(|_, _, _| {}),
            });
        }
        assert_eq!(dialogs.admission("another"), Admission::Refuse);
        dialogs.waiting.pop_front();
        assert_eq!(dialogs.admission("another"), Admission::Wait);
    }

    #[test]
    fn one_asker_cannot_fill_the_queue() {
        let mut dialogs = ConsentDialogs {
            open: Some(OpenDialog {
                ticket: 1,
                pid: 1,
                shown: false,
                withdraw: None,
            }),
            ..ConsentDialogs::default()
        };
        assert_eq!(dialogs.admission("agent"), Admission::Wait);
        dialogs.waiting.push_back(WaitingQuestion {
            ticket: 2,
            asker: "agent".to_owned(),
            question: question(),
            on_answer: Box::new(|_, _, _| {}),
        });
        assert_eq!(dialogs.admission("agent"), Admission::Refuse);
        assert_eq!(dialogs.admission("other"), Admission::Wait);
    }
}
