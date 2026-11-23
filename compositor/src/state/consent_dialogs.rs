//! The consent dialogs the compositor started, and whether each one showed
//! its surface. A choice counts only from the process the compositor started,
//! after that same process mapped a verified consent layer.

use std::collections::HashMap;

use smithay::{desktop::LayerSurface, reexports::wayland_server::Resource};
use tokio::sync::oneshot;

use crate::{
    state::{ClientState, State},
    utils::{
        consent_dialog::{ConsentAnswer, ConsentQuestion, DialogExit, wait_for_dialog},
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
    /// Starts the dialog and calls `on_answer` with the answer on the
    /// compositor thread. Returns the ticket, for [`Self::withdraw_consent`].
    pub fn ask_consent(
        &mut self,
        question: ConsentQuestion,
        on_answer: impl FnOnce(&mut State, ConsentAnswer) + Send + 'static,
    ) -> u64 {
        self.consent_dialogs.next_ticket += 1;
        let ticket = self.consent_dialogs.next_ticket;
        let Some(executable) = consent_dialog_executable() else {
            tracing::warn!(target: AUDIT, "the consent dialog is not installed");
            self.common
                .tasks
                .deliver(move |state| on_answer(state, ConsentAnswer::Unavailable));
            return ticket;
        };
        let spawned = self
            .common
            .tasks
            .spawn_process(&mut question.command(&executable));
        let child = match spawned {
            Ok(child) => child,
            Err(error) => {
                tracing::warn!(target: AUDIT, %error, "cannot start the consent dialog");
                self.common
                    .tasks
                    .deliver(move |state| on_answer(state, ConsentAnswer::Unavailable));
                return ticket;
            }
        };
        let Some(pid) = child.id().and_then(|pid| i32::try_from(pid).ok()) else {
            self.common
                .tasks
                .deliver(move |state| on_answer(state, ConsentAnswer::Unavailable));
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
                on_answer(state, answer);
            });
        ticket
    }

    /// Closes the dialog of `ticket`; its answer is [`ConsentAnswer::Withdrawn`].
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
                .filter(|process| process.is_alive())
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
    ) -> ConsentAnswer {
        let shown = self
            .consent_dialogs
            .open
            .remove(&ticket)
            .is_some_and(|open| open.shown);
        match exit {
            DialogExit::Printed(line) => match question.choice(&line) {
                Some(choice) if shown => ConsentAnswer::Chosen(choice),
                Some(_) => {
                    tracing::warn!(target: AUDIT, ticket, "a dialog answered without showing itself");
                    ConsentAnswer::Unavailable
                }
                None => ConsentAnswer::Declined,
            },
            DialogExit::Declined => ConsentAnswer::Declined,
            DialogExit::TimedOut => ConsentAnswer::TimedOut,
            DialogExit::Withdrawn => ConsentAnswer::Withdrawn,
        }
    }
}
