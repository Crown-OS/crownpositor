//! Who may use the screen context stream, from `context_access.ron`: the
//! scope of each bound client, the first-use question, and the grants the
//! compositor writes as their one owner.

use std::time::{Duration, Instant, SystemTime};

use calloop::timer::{TimeoutAction, Timer};
use crownos_config::schema::{
    Agents, AppKey, ContextAccess, ContextDecision, Expiry, Scope as GrantScope, ScopeSet,
};
use protocols::crownos_context::{BindDecision, RevokeReason, Scope};
use wayland_server::{Client, backend::ClientId};

use protocols::crownos_agent_cursor::RevokeReason as AgentRevokeReason;

use crate::{
    state::{ClientState, State},
    utils::consent_questions::{ContextAnswer, context_access_question, context_answer},
    utils::privilege::may_drive_agents,
};

pub const AUDIT: &str = "crownpositor::audit::context";
/// How often time-limited grants are checked while their clients stay bound.
const EXPIRY_CHECK: Duration = Duration::from_secs(30);
/// After the user refuses without remembering, the app is not asked again
/// for this long, so it cannot ask over and over by binding again.
const ASK_AGAIN_AFTER: Duration = Duration::from_secs(10 * 60);

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

/// What an answer leaves behind besides the scope it sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AnswerEffects {
    /// The grant lives only in the clients bound now: the user did not ask to
    /// remember it, or it could not be written.
    holds_for_bound_clients_only: bool,
    /// The app is not asked again for a while.
    cool_down: bool,
}

impl AnswerEffects {
    const fn of(granted: bool, remembered: bool) -> Self {
        Self {
            holds_for_bound_clients_only: granted && !remembered,
            cool_down: !granted && !remembered,
        }
    }
}

/// What a new decision does to a bound client, or `None` to leave it as it
/// is. Only a client that waits for the user, or that the user allowed
/// without remembering, keeps its state when the file says ask: any other
/// client lost the grant its scope came from.
fn next_scope(
    decision: Option<ContextDecision>,
    waiting: bool,
    answered_without_remembering: bool,
) -> Option<Option<Scope>> {
    match decision {
        Some(ContextDecision::Granted(scopes)) => Some(Some(protocol_scope(scopes))),
        Some(ContextDecision::Ask) if waiting || answered_without_remembering => None,
        Some(ContextDecision::Ask | ContextDecision::Denied) | None => Some(None),
    }
}

fn app_key_of(client: &Client) -> Option<AppKey> {
    client
        .get_data::<ClientState>()
        .and_then(ClientState::current_app_key)
        .cloned()
}

impl State {
    fn context_access(&self) -> &ContextAccess {
        &self.config.current.context_access
    }

    /// Replaces the grants everywhere the compositor keeps them.
    pub fn set_context_access(&mut self, access: ContextAccess) {
        if let Ok(mut shared) = self.wayland.context_grants.write() {
            *shared = access.clone();
        }
        self.config.current.context_access = access;
    }

    /// The decision for `client` now. A client that loads other code is
    /// refused when its grant would run the engine, and an app the user just
    /// refused is not asked again yet.
    fn decide_for_client(&mut self, client: &Client, key: &AppKey) -> ContextDecision {
        let decision = self.context_access().decide(key, SystemTime::now());
        let preloaded = client
            .get_data::<ClientState>()
            .is_some_and(ClientState::is_preloaded);
        match decision {
            ContextDecision::Granted(scopes)
                if preloaded && scopes.contains(GrantScope::Engine) =>
            {
                tracing::warn!(target: AUDIT, actor = %key, "refused the engine to a preloaded process");
                ContextDecision::Denied
            }
            ContextDecision::Ask if self.is_cooling_down(key) => ContextDecision::Denied,
            other => other,
        }
    }

    fn is_cooling_down(&mut self, key: &AppKey) -> bool {
        let now = Instant::now();
        self.context_cooldown.retain(|_, until| *until > now);
        self.context_cooldown.contains_key(key)
    }

    /// The scope of a client that binds the context manager. An app without
    /// a grant waits while the user is asked.
    pub fn context_bind_decision_for(&mut self, client: &Client) -> BindDecision {
        let Some(key) = app_key_of(client) else {
            tracing::info!(target: AUDIT, "refused a client that cannot be identified");
            return BindDecision::Refused;
        };
        match self.decide_for_client(client, &key) {
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
        let question = context_access_question(&key.display_name(), &key.to_string());
        tracing::info!(target: AUDIT, actor = %key, "asking the user");
        let ticket = self.context_prompts.park(key);
        self.ask_consent(question, move |state, answer| {
            state.answer_context_access(ticket, context_answer(&answer));
        });
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
        let remembered =
            answer.remember && self.store_context_answer(&key, answer.granted.then_some(scopes));
        let effects = AnswerEffects::of(answer.granted, remembered);
        if effects.cool_down {
            self.context_cooldown
                .insert(key.clone(), Instant::now() + ASK_AGAIN_AFTER);
        }
        let scope = answer.granted.then(|| protocol_scope(scopes));
        for client in self.clients_with_key(&key) {
            if effects.holds_for_bound_clients_only {
                self.context_answered.insert(client.clone());
            }
            let display = self.common.display_handle.clone();
            self.wayland.context_state.set_scope::<State>(
                &display,
                &client,
                scope,
                RevokeReason::User,
            );
        }
    }

    /// Writes a remembered answer, and returns whether it was written. A
    /// file that does not parse is not replaced; the answer then holds for
    /// the clients bound now, as if it was not remembered.
    fn store_context_answer(&mut self, key: &AppKey, scopes: Option<ScopeSet>) -> bool {
        let mut access: ContextAccess = match crownos_config::load_strict(ContextAccess::SECTION) {
            Ok(access) => access,
            Err(error) => {
                tracing::warn!(target: AUDIT, %error, "cannot remember the answer");
                return false;
            }
        };
        match scopes {
            Some(scopes) => access.grant(key, scopes, Expiry::Never),
            None => access.deny(key),
        }
        if let Err(error) = crownos_config::save(ContextAccess::SECTION, &access) {
            tracing::warn!(target: AUDIT, %error, "cannot remember the answer");
            return false;
        }
        self.set_context_access(access);
        true
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
        self.revoke_lost_agent_scopes();
        let display = self.common.display_handle.clone();
        let clients = self.wayland.context_state.clients();
        self.context_answered
            .retain(|answered| clients.iter().any(|client| client.id() == *answered));
        for client in clients {
            let id = client.id();
            let key = app_key_of(&client);
            let decision = key.as_ref().map(|key| self.decide_for_client(&client, key));
            let waiting = self.wayland.context_state.is_waiting(&id);
            let answered = self.context_answered.contains(&id);
            let Some(scope) = next_scope(decision, waiting, answered) else {
                continue;
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

    /// Cuts the window control and agent cursor clients that lost their
    /// scope since they bound.
    fn revoke_lost_agent_scopes(&mut self) {
        let grants = &self.wayland.context_grants;
        let trusted = &self.wayland.trusted_executables;
        let lost = |scope: GrantScope| {
            move |client: &Client| !may_drive_agents(client, grants, trusted, scope)
        };
        let control_lost = lost(GrantScope::WaylandWindowControl);
        let losing_control: Vec<Client> = self
            .wayland
            .window_control_state
            .clients()
            .into_iter()
            .filter(|client| control_lost(client))
            .collect();
        let cursor_lost = lost(GrantScope::WaylandAgentCursor);
        let losing_cursors: Vec<Client> = self
            .wayland
            .agent_cursor_state
            .clients()
            .into_iter()
            .filter(|client| cursor_lost(client))
            .collect();
        if !losing_control.is_empty() {
            let cut = self
                .wayland
                .window_control_state
                .revoke_where(|client| losing_control.iter().any(|lost| lost.id() == client.id()));
            tracing::info!(target: AUDIT, clients = cut, "window control withdrawn");
        }
        if !losing_cursors.is_empty() {
            self.revoke_agents_of(
                |client| losing_cursors.iter().any(|lost| lost.id() == client.id()),
                AgentRevokeReason::User,
            );
            tracing::info!(target: AUDIT, clients = losing_cursors.len(), "agent cursors withdrawn");
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
    fn an_answer_that_could_not_be_written_holds_for_the_bound_clients() {
        assert_eq!(
            AnswerEffects::of(true, false),
            AnswerEffects {
                holds_for_bound_clients_only: true,
                cool_down: false
            }
        );
        assert_eq!(
            AnswerEffects::of(false, false),
            AnswerEffects {
                holds_for_bound_clients_only: false,
                cool_down: true
            }
        );
        assert_eq!(
            AnswerEffects::of(true, true),
            AnswerEffects {
                holds_for_bound_clients_only: false,
                cool_down: false
            }
        );
    }

    #[test]
    fn a_grant_that_ends_revokes_unless_the_client_waits_or_was_answered() {
        let key = AppKey::Spawned {
            component: "agent".to_owned(),
        };
        let mut access = ContextAccess::default();
        access.grant(
            &key,
            ScopeSet::EMPTY.with(GrantScope::Metadata),
            Expiry::At(100),
        );
        let at = |seconds| std::time::UNIX_EPOCH + Duration::from_secs(seconds);
        let before = Some(access.decide(&key, at(99)));
        let after = Some(access.decide(&key, at(100)));
        assert_eq!(after, Some(ContextDecision::Ask));

        assert_eq!(
            next_scope(before, false, false),
            Some(Some(Scope::Metadata))
        );
        assert_eq!(next_scope(after, false, false), Some(None));
        assert_eq!(next_scope(after, true, false), None);
        assert_eq!(next_scope(after, false, true), None);
        assert_eq!(
            next_scope(Some(ContextDecision::Denied), true, true),
            Some(None)
        );
        assert_eq!(next_scope(None, false, false), Some(None));
    }
}
