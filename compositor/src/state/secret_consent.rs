//! crownos-secretsd's consent questions for a run or a proxy use, shown in
//! the same ticket-bound consent dialog as typing.

use crownos_protocols::secret_input::v1::server::crownos_secret_consent_v1::CrownosSecretConsentV1;
use protocols::crownos_secret_input::{ConsentAction, SecretConsentQuestion};

use crate::{
    state::{State, secret_typing::display_text},
    utils::{
        consent_dialog::ConsentOutcome,
        consent_questions::{SecretUsePrompt, secret_use_question, typing_allowed},
    },
};

const AUDIT: &str = "crownpositor::audit::secret_consent";

/// The questions whose dialog is open, by ticket.
#[derive(Default)]
pub struct SecretConsents {
    open: Vec<(u64, SecretConsentQuestion)>,
}

impl State {
    pub fn ask_secret_consent(&mut self, question: SecretConsentQuestion) {
        let prompt = SecretUsePrompt {
            agent_app_key: display_text(&question.agent_app_key),
            secret_names: display_text(&question.secret_names),
            proxy: question.action == ConsentAction::Proxy,
            detail: display_text(&question.detail),
        };
        tracing::info!(target: AUDIT, agent = prompt.agent_app_key, secrets = prompt.secret_names, "asking the user");
        let asker = question.agent_app_key.clone();
        let ticket = self.ask_consent(
            &asker,
            secret_use_question(&prompt),
            |state, ticket, answer| {
                state.answer_secret_consent(ticket, &answer);
            },
        );
        self.secret_consents.open.push((ticket, question));
    }

    pub fn withdraw_secret_consent(&mut self, resource: &CrownosSecretConsentV1) {
        let Some(index) = self
            .secret_consents
            .open
            .iter()
            .position(|(_, question)| question.resource() == resource)
        else {
            return;
        };
        let (ticket, _) = self.secret_consents.open.swap_remove(index);
        self.withdraw_consent(ticket);
    }

    fn answer_secret_consent(&mut self, ticket: u64, answer: &ConsentOutcome) {
        let Some(index) = self
            .secret_consents
            .open
            .iter()
            .position(|(asked, _)| *asked == ticket)
        else {
            return;
        };
        let (_, question) = self.secret_consents.open.swap_remove(index);
        let granted = typing_allowed(answer);
        tracing::info!(target: AUDIT, ticket, granted, "the user answered");
        question.answer(granted);
    }
}
