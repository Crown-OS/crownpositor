//! The consent dialogs the compositor started, and whether each one showed
//! its surface. A choice counts only from the process the compositor started,
//! after that same process mapped a verified consent layer.

use std::collections::HashMap;

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

#[derive(Default)]
pub struct ConsentDialogs {
    next_ticket: u64,
    open: HashMap<u64, OpenDialog>,
}

struct OpenDialog {
    pid: i32,
    shown: bool,
    withdraw: Option<oneshot::Sender<()>>,
}

impl State {
    /// Starts the dialog and calls `on_answer` with its ticket and the answer
    /// on the compositor thread. Returns the ticket, for
    /// [`Self::withdraw_consent`].
    pub fn ask_consent(
        &mut self,
        question: ConsentQuestion,
        on_answer: impl FnOnce(&mut State, u64, ConsentOutcome) + Send + 'static,
    ) -> u64 {
        self.consent_dialogs.next_ticket += 1;
        let ticket = self.consent_dialogs.next_ticket;
        let Some(executable) = consent_dialog_executable() else {
            tracing::warn!(target: AUDIT, "the consent dialog is not installed");
            self.common
                .tasks
                .deliver(move |state| on_answer(state, ticket, ConsentOutcome::Unavailable));
            return ticket;
        };
        let spawned = self
            .common
            .tasks
            .spawn_process(&mut dialog_command(&question, &executable));
        let child = match spawned {
            Ok(child) => child,
            Err(error) => {
                tracing::warn!(target: AUDIT, %error, "cannot start the consent dialog");
                self.common
                    .tasks
                    .deliver(move |state| on_answer(state, ticket, ConsentOutcome::Unavailable));
                return ticket;
            }
        };
        let Some(pid) = child.id().and_then(|pid| i32::try_from(pid).ok()) else {
            self.common
                .tasks
                .deliver(move |state| on_answer(state, ticket, ConsentOutcome::Unavailable));
            return ticket;
        };
        let (withdraw, withdrawn) = oneshot::channel();
        self.consent_dialogs.open.insert(
            ticket,
            OpenDialog {
                pid,
                shown: false,
                withdraw: Some(withdraw),
            },
        );
        self.common
            .tasks
            .run(wait_for_dialog(child, withdrawn), move |state, exit| {
                let answer = state.consent_answer(ticket, &question, exit);
                on_answer(state, ticket, answer);
            });
        ticket
    }

    /// Closes the dialog of `ticket`; its answer is [`ConsentOutcome::Withdrawn`].
    pub fn withdraw_consent(&mut self, ticket: u64) {
        if let Some(withdraw) = self
            .consent_dialogs
            .open
            .get_mut(&ticket)
            .and_then(|open| open.withdraw.take())
        {
            let _ = withdraw.send(());
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
        for open in self.consent_dialogs.open.values_mut() {
            if open.pid == process {
                open.shown = true;
            }
        }
    }

    fn consent_answer(
        &mut self,
        ticket: u64,
        question: &ConsentQuestion,
        exit: DialogExit,
    ) -> ConsentOutcome {
        let shown = self
            .consent_dialogs
            .open
            .remove(&ticket)
            .is_some_and(|open| open.shown);
        decide(question, exit, shown)
    }
}

/// The answer from what the dialog printed and whether it showed itself.
fn decide(question: &ConsentQuestion, exit: DialogExit, shown: bool) -> ConsentOutcome {
    match exit {
        DialogExit::Exited(status) => match question.answer(status) {
            Ok(answer) if answer.granted && !shown => {
                tracing::warn!(target: AUDIT, "a dialog allowed without showing itself");
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
        let denied = decide(&question(), DialogExit::Exited(Some(17)), false);
        assert!(matches!(denied, ConsentOutcome::Answered(ref answer) if !answer.granted));
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
}
