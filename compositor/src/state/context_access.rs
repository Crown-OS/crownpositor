//! Who may use the screen context stream, from `context_access.ron`: the
//! scope of each bound client, the first-use question, and the grants the
//! compositor writes as their one owner.

use std::time::{Duration, SystemTime};

use calloop::timer::{TimeoutAction, Timer};
use crownos_config::schema::{
    Agents, AppKey, ContextAccess, ContextDecision, Expiry, Scope as GrantScope, ScopeSet,
};
use protocols::crownos_context::{BindDecision, RevokeReason, Scope};
use wayland_server::{Client, backend::ClientId};

use crate::{
    state::{ClientState, State},
    utils::portal::{ContextAnswer, ContextPrompt, ask_context_access},
};

pub const AUDIT: &str = "crownpositor::audit::context";
/// How often time-limited grants are checked while their clients stay bound.
const EXPIRY_CHECK: Duration = Duration::from_secs(30);

/// Copies the trusted executables of `agents.ron` one time, and ends the
/// session grants of an earlier login. Runs before the context global
/// exists. A grant file that does not parse is left alone.
pub fn prepare_on_startup() {
    let mut access: ContextAccess = match crownos_config::load_strict(ContextAccess::SECTION) {
        Ok(access) => access,
        Err(error) => {
            tracing::warn!(target: AUDIT, %error, "context_access.ron does not parse; not migrating");
            return;
        }
    };
    let agents: Agents = crownos_config::load(Agents::SECTION);
    let forgot = access.forget_session_grants();
    let migrated = access.migrate_agents(&agents);
    if !forgot && !migrated {
        return;
    }
    tracing::info!(target: AUDIT, forgot, migrated, "context grants prepared");
    if let Err(error) = crownos_config::save(ContextAccess::SECTION, &access) {
        tracing::warn!(target: AUDIT, %error, "cannot save the context grants");
    }
}

/// The protocol scope of the grant scopes the context stream knows.
pub fn protocol_scope(scopes: ScopeSet) -> Scope {
    [
        (GrantScope::Metadata, Scope::Metadata),
        (GrantScope::Text, Scope::Text),
        (GrantScope::Raw, Scope::Raw),
        (GrantScope::Engine, Scope::Engine),
        (GrantScope::Pause, Scope::Pause),
    ]
    .into_iter()
    .filter(|(grant, _)| scopes.contains(*grant))
    .fold(Scope::empty(), |scope, (_, bit)| scope | bit)
}

fn app_key_of(client: &Client) -> Option<AppKey> {
    client
        .get_data::<ClientState>()
        .and_then(ClientState::current_app_key)
        .cloned()
}

/// The name the user knows an app by, for the question.
fn display_name(key: &AppKey) -> String {
    let file_name = |path: &std::path::Path| {
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| key.to_string())
    };
    match key {
        AppKey::Sandboxed { app_id, .. } => app_id.clone(),
        AppKey::Spawned { component } => component.clone(),
        AppKey::HostExecutable { path } => file_name(path),
        AppKey::Interpreted { script, .. } => file_name(script),
    }
}

impl State {
    fn context_access(&self) -> &ContextAccess {
        &self.config.current.context_access
    }

    /// The scope of a client that binds the context manager. An app without
    /// a grant waits while the user is asked.
    pub fn context_bind_decision_for(&mut self, client: &Client) -> BindDecision {
        let Some(key) = app_key_of(client) else {
            tracing::info!(target: AUDIT, "refused a client that cannot be identified");
            return BindDecision::Refused;
        };
        match self.context_access().decide(&key, SystemTime::now()) {
            ContextDecision::Granted(scopes) => {
                tracing::info!(target: AUDIT, actor = %key, scope = ?scopes, "bound");
                BindDecision::Granted(protocol_scope(scopes))
            }
            ContextDecision::Denied => {
                tracing::info!(target: AUDIT, actor = %key, "refused");
                BindDecision::Refused
            }
            ContextDecision::Ask => {
                self.prompt_context_access(key);
                BindDecision::Pending
            }
        }
    }

    fn prompt_context_access(&mut self, key: AppKey) {
        if self.context_prompts.is_asking(|asked| *asked == key) {
            return;
        }
        let prompt = ContextPrompt {
            ticket: 0,
            app_name: display_name(&key),
            app_key: key.to_string(),
        };
        tracing::info!(target: AUDIT, actor = %key, "asking the user");
        let ticket = self.context_prompts.park(key);
        self.common.tasks.run(
            ask_context_access(ContextPrompt { ticket, ..prompt }),
            move |state, answer| state.answer_context_access(ticket, answer),
        );
    }

    fn answer_context_access(&mut self, ticket: u64, answer: Result<ContextAnswer, String>) {
        let Some(key) = self.context_prompts.take(ticket) else {
            return;
        };
        let answer = answer.unwrap_or_else(|error| {
            tracing::warn!(target: AUDIT, %error, "nobody could be asked");
            ContextAnswer {
                granted: false,
                remember: false,
                titles: false,
            }
        });
        let scopes = if answer.titles {
            ScopeSet::EMPTY
                .with(GrantScope::Metadata)
                .with(GrantScope::Text)
        } else {
            ScopeSet::EMPTY.with(GrantScope::Metadata)
        };
        tracing::info!(
            target: AUDIT,
            actor = %key,
            granted = answer.granted,
            remember = answer.remember,
            titles = answer.titles,
            "the user answered"
        );
        if answer.remember {
            self.store_context_answer(&key, answer.granted.then_some(scopes));
        }
        let scope = answer.granted.then(|| protocol_scope(scopes));
        for client in self.clients_with_key(&key) {
            let display = self.common.display_handle.clone();
            self.wayland.context_state.set_scope::<State>(
                &display,
                &client,
                scope,
                RevokeReason::User,
            );
        }
    }

    /// Writes a remembered answer. A file that does not parse is not
    /// replaced; the answer then holds for the clients bound now only.
    fn store_context_answer(&mut self, key: &AppKey, scopes: Option<ScopeSet>) {
        let mut access: ContextAccess = match crownos_config::load_strict(ContextAccess::SECTION) {
            Ok(access) => access,
            Err(error) => {
                tracing::warn!(target: AUDIT, %error, "cannot remember the answer");
                return;
            }
        };
        match scopes {
            Some(scopes) => access.grant(key, scopes, Expiry::Never),
            None => access.deny(key),
        }
        if let Err(error) = crownos_config::save(ContextAccess::SECTION, &access) {
            tracing::warn!(target: AUDIT, %error, "cannot remember the answer");
            return;
        }
        self.config.current.context_access = access;
    }

    fn clients_with_key(&self, key: &AppKey) -> Vec<ClientId> {
        self.wayland
            .context_state
            .clients()
            .into_iter()
            .filter(|client| app_key_of(client).as_ref() == Some(key))
            .map(|client| client.id())
            .collect()
    }

    /// Decides again for every bound client, after the grants changed or
    /// time passed: a new scope is sent, a withdrawn grant revokes, and a
    /// client that waits for the user keeps waiting.
    pub fn rescope_context_clients(&mut self, reason: RevokeReason) {
        let display = self.common.display_handle.clone();
        let now = SystemTime::now();
        for client in self.wayland.context_state.clients() {
            let id = client.id();
            let key = app_key_of(&client);
            let scope = match key
                .as_ref()
                .map(|key| self.context_access().decide(key, now))
            {
                Some(ContextDecision::Granted(scopes)) => Some(protocol_scope(scopes)),
                Some(ContextDecision::Ask) => continue,
                Some(ContextDecision::Denied) | None => None,
            };
            let changed = self
                .wayland
                .context_state
                .set_scope::<State>(&display, &id, scope, reason);
            if changed {
                let actor = key.map(|key| key.to_string()).unwrap_or_default();
                tracing::info!(target: AUDIT, %actor, ?scope, "scope changed");
            }
        }
    }

    pub fn arm_context_expiry(&mut self) {
        let armed = self.common.event_loop_handle.insert_source(
            Timer::from_duration(EXPIRY_CHECK),
            |_, _, state| {
                state.rescope_context_clients(RevokeReason::Expired);
                TimeoutAction::ToDuration(EXPIRY_CHECK)
            },
        );
        if let Err(error) = armed {
            tracing::warn!(%error, "time-limited context grants will end only on a config change");
        }
    }

    pub fn context_client_gone(&self, client: Option<ClientId>) {
        tracing::info!(target: AUDIT, ?client, "unbound");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grant_scopes_map_to_protocol_bits() {
        let scopes = ScopeSet::EMPTY
            .with(GrantScope::Metadata)
            .with(GrantScope::Pause)
            .with(GrantScope::MemoryWrite);
        assert_eq!(protocol_scope(scopes), Scope::Metadata | Scope::Pause);
        assert_eq!(
            protocol_scope(ScopeSet::EMPTY.with(GrantScope::Engine).with_included()),
            Scope::Metadata | Scope::Text | Scope::Raw | Scope::Engine | Scope::Pause
        );
    }

    #[test]
    fn apps_are_named_by_what_the_user_knows() {
        assert_eq!(
            display_name(&AppKey::HostExecutable {
                path: "/usr/bin/agent".into()
            }),
            "agent"
        );
        assert_eq!(
            display_name(&AppKey::Interpreted {
                interpreter: "/usr/bin/python3".into(),
                script: "/srv/bot.py".into()
            }),
            "bot.py"
        );
    }
}
