//! Server-side `crownos_context_v1`: what the user is doing, without content,
//! for the visual context engine and the apps the user lets see it.
//!
//! Many clients can bind. Each gets a scope from the compositor, through
//! [`ContextHandler::context_bind_decision`] or later through
//! [`ContextState::set_scope`], and only the events its scope allows. The
//! compositor syncs window records every loop iteration, and pushes session
//! changes, input activity buckets and selection metadata as they happen;
//! the clients' own requests come back through [`ContextHandler`].

mod bound;
mod dispatch;
mod window_events;

use std::{collections::HashMap, time::Instant};

use crownos_protocols::context::v1::server::{
    crownos_context_manager_v1::CrownosContextManagerV1,
    crownos_context_window_v1::CrownosContextWindowV1,
};
use wayland_server::{
    Client, DisplayHandle, Resource,
    backend::{ClientId, GlobalId},
};

use bound::{Bound, Grant};
use window_events::{announce, send_changes};

pub use crate::crownos_window_control::{Placement, WindowKey};
pub use crownos_protocols::context::v1::server::{
    crownos_context_manager_v1::{
        ActivityFlags, PauseOrigin, RevokeReason, Scope, SelectionFlags, SelectionKind,
        SessionState,
    },
    crownos_context_window_v1::{
        ContentType, FlagReason, Sensitivity, State as ContextWindowFlags,
    },
};
pub use dispatch::ContextDispatch;

/// The protocol version this module speaks.
pub const VERSION: u32 = 2;

/// The `wp_security_context_v1` a client came from; all empty when unsandboxed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Sandbox {
    pub engine: String,
    pub app_id: String,
    pub instance_id: String,
}

/// One mapped window as the engine sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextRecord {
    pub key: WindowKey,
    pub identifier: String,
    pub app_id: String,
    pub title: String,
    pub pid: i32,
    pub sandbox: Sandbox,
    pub placement: Placement,
    pub flags: ContextWindowFlags,
    pub content_type: ContentType,
    pub sensitivity: Sensitivity,
}

/// What the user did in the focused window over one interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputActivity {
    pub interval_ms: u32,
    pub text: u32,
    pub edit: u32,
    pub nav: u32,
    pub chords: u32,
    pub clicks: u32,
    pub scroll_ticks: u32,
    pub pointer_travel: u32,
    pub flags: ActivityFlags,
}

impl Default for InputActivity {
    fn default() -> Self {
        Self {
            interval_ms: 0,
            text: 0,
            edit: 0,
            nav: 0,
            chords: 0,
            clicks: 0,
            scroll_ticks: 0,
            pointer_travel: 0,
            flags: ActivityFlags::empty(),
        }
    }
}

impl InputActivity {
    pub fn is_empty(&self) -> bool {
        self.text == 0
            && self.edit == 0
            && self.nav == 0
            && self.chords == 0
            && self.clicks == 0
            && self.scroll_ticks == 0
            && self.pointer_travel == 0
    }
}

/// A selection changed: its kind, the window whose client set it, and what
/// it offers. Never the content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionChange {
    pub kind: SelectionKind,
    pub window: Option<WindowKey>,
    pub mime_types: String,
    pub size_hint: u32,
    pub flags: SelectionFlags,
}

/// What the compositor decides when a client binds the manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindDecision {
    Granted(Scope),
    /// The compositor asks the user and answers through [`ContextState::set_scope`].
    Pending,
    Refused,
}

/// What the compositor provides for this protocol to be delegated to
/// [`ContextState`].
pub trait ContextHandler {
    fn context_state(&mut self) -> &mut ContextState;
    /// The scope of a client that just bound the manager.
    fn context_bind_decision(&mut self, client: &Client) -> BindDecision;
    /// A client with the engine or pause scope paused or resumed observation
    /// for every client.
    fn set_context_paused(&mut self, paused: bool, origin: PauseOrigin);
    /// A client saw something in a window that must not be captured.
    fn flag_window_sensitive(&mut self, key: WindowKey, reason: FlagReason, duration_ms: u32);
    /// A client destroyed its manager.
    fn context_client_gone(&mut self, client: Option<ClientId>);
}

/// Global data: which clients may see the manager.
pub struct ContextGlobalData {
    filter: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

/// Delegate type for the `crownos_context_manager_v1` global.
pub struct ContextState {
    global: GlobalId,
    bound: Vec<Bound>,
    known: HashMap<WindowKey, ContextRecord>,
    focus: Option<WindowKey>,
    session: SessionState,
    paused: bool,
}

impl ContextState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: ContextDispatch,
        F: Fn(&Client) -> bool + Send + Sync + 'static,
    {
        let global = display.create_global::<D, CrownosContextManagerV1, _>(
            VERSION,
            ContextGlobalData {
                filter: Box::new(filter),
            },
        );
        Self {
            global,
            bound: Vec::new(),
            known: HashMap::new(),
            focus: None,
            session: SessionState::Active,
            paused: false,
        }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }

    /// Whether any client sees windows, so the directory must be kept.
    pub fn is_bound(&self) -> bool {
        self.bound.iter().any(Bound::sees_metadata)
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn knows(&self, key: WindowKey) -> bool {
        self.known.contains_key(&key)
    }

    /// The clients with a manager that is not revoked, to decide again when
    /// the grants change.
    pub fn clients(&self) -> Vec<Client> {
        let mut clients: Vec<Client> = Vec::new();
        for client in self
            .bound
            .iter()
            .filter(|bound| bound.grant != Grant::Revoked)
            .filter_map(|bound| bound.manager.client())
        {
            if !clients.iter().any(|known| known.id() == client.id()) {
                clients.push(client);
            }
        }
        clients
    }

    /// Whether `client` waits for the user: one of its managers is pending
    /// and none has a scope. A revoked manager does not count.
    pub fn is_waiting(&self, client: &ClientId) -> bool {
        let of_client = || {
            self.bound.iter().filter(move |bound| {
                bound.client.as_ref() == Some(client) && bound.manager.is_alive()
            })
        };
        of_client().any(|bound| bound.grant == Grant::Pending)
            && !of_client().any(|bound| matches!(bound.grant, Grant::Scoped(_)))
    }

    /// The scope of `client`, or `None` while it waits or after a revoke.
    pub fn scope_of(&self, client: &ClientId) -> Option<Scope> {
        self.bound
            .iter()
            .filter(|bound| bound.client.as_ref() == Some(client))
            .find_map(|bound| match bound.grant {
                Grant::Scoped(scope) => Some(scope),
                Grant::Pending | Grant::Revoked => None,
            })
    }

    /// Brings every client up to date. While paused, nothing is sent and the
    /// records are not remembered, so resuming announces the current state.
    pub fn sync<D: ContextDispatch>(
        &mut self,
        display: &DisplayHandle,
        records: &[ContextRecord],
        focus: Option<WindowKey>,
        time_ms: u32,
    ) {
        self.bound.retain(|bound| bound.manager.is_alive());
        if self.paused {
            return;
        }
        let gone: Vec<WindowKey> = self
            .known
            .keys()
            .filter(|key| !records.iter().any(|record| record.key == **key))
            .copied()
            .collect();
        for key in gone {
            self.known.remove(&key);
            for bound in &mut self.bound {
                if let Some(window) = bound.windows.remove(&key) {
                    window.closed();
                }
            }
        }
        for record in records {
            let known = self.known.get(&record.key);
            if known == Some(record) {
                continue;
            }
            for bound in &mut self.bound {
                if !bound.sees_windows(false) {
                    continue;
                }
                let with_text = bound.sees_text(false);
                match (known, bound.windows.get(&record.key)) {
                    (Some(known), Some(window)) => {
                        send_changes(window, known, record, with_text);
                        window.done();
                    }
                    _ => announce::<D>(display, bound, record, with_text),
                }
            }
            self.known.insert(record.key, record.clone());
        }
        if self.focus != focus {
            self.focus = focus;
            for bound in self.bound.iter().filter(|bound| bound.sees_windows(false)) {
                bound
                    .manager
                    .focus(focus.and_then(|key| bound.windows.get(&key)), time_ms);
            }
        }
    }

    pub fn set_session(&mut self, session: SessionState) {
        if self.session == session {
            return;
        }
        self.session = session;
        for bound in self.bound.iter().filter(|bound| bound.sees_metadata()) {
            bound.manager.session(session);
        }
    }

    pub fn input_activity(&self, activity: InputActivity) {
        for bound in self
            .bound
            .iter()
            .filter(|bound| bound.sees_windows(self.paused))
        {
            let window = self.focus.and_then(|key| bound.windows.get(&key));
            bound.manager.input_activity(
                window,
                activity.interval_ms,
                activity.text,
                activity.edit,
                activity.nav,
                activity.chords,
                activity.clicks,
                activity.scroll_ticks,
                activity.pointer_travel,
                activity.flags,
            );
        }
    }

    pub fn selection(&self, change: &SelectionChange) {
        for bound in self
            .bound
            .iter()
            .filter(|bound| bound.sees_text(self.paused))
        {
            let window = change.window.and_then(|key| bound.windows.get(&key));
            bound.manager.selection(
                change.kind,
                window,
                change.mime_types.clone(),
                change.size_hint,
                change.flags,
            );
        }
    }

    /// Pauses or resumes observation for every client. Pausing forgets the
    /// announced windows and closes them; resuming re-announces them on the
    /// next sync.
    pub fn set_paused(&mut self, paused: bool, origin: PauseOrigin) {
        if self.paused == paused {
            return;
        }
        self.paused = paused;
        if paused {
            self.known.clear();
            self.focus = None;
        }
        for bound in self.bound.iter_mut().filter(|bound| bound.is_live()) {
            if paused {
                bound.close_windows();
            }
            bound.send_paused(paused, origin);
        }
    }

    /// Gives every manager of `client` a new scope, or revokes it with
    /// `reason` when `scope` is `None`. A version 1 manager keeps working
    /// only with the engine scope. Returns whether anything changed.
    pub fn set_scope<D: ContextDispatch>(
        &mut self,
        display: &DisplayHandle,
        client: &ClientId,
        scope: Option<Scope>,
        reason: RevokeReason,
    ) -> bool {
        let mut changed = false;
        for index in 0..self.bound.len() {
            if self.bound[index].client.as_ref() != Some(client) {
                continue;
            }
            changed |= self.rescope::<D>(display, index, scope, reason);
        }
        changed
    }

    fn rescope<D: ContextDispatch>(
        &mut self,
        display: &DisplayHandle,
        index: usize,
        scope: Option<Scope>,
        reason: RevokeReason,
    ) -> bool {
        let bound = &self.bound[index];
        let scope =
            scope.filter(|scope| bound.is_scoped_version() || scope.contains(Scope::Engine));
        match (bound.grant, scope) {
            (Grant::Revoked, _) => false,
            (Grant::Pending, Some(scope)) => {
                self.activate::<D>(display, index, scope);
                true
            }
            (Grant::Scoped(old), Some(new)) if old == new => false,
            (Grant::Scoped(old), Some(new)) => {
                self.change_scope::<D>(display, index, old, new);
                true
            }
            (Grant::Pending | Grant::Scoped(_), None) => {
                revoke(&mut self.bound[index], reason);
                true
            }
        }
    }

    /// The first scope of a manager: the scope event, then the description.
    fn activate<D: ContextDispatch>(
        &mut self,
        display: &DisplayHandle,
        index: usize,
        scope: Scope,
    ) {
        let paused = self.paused;
        let bound = &mut self.bound[index];
        bound.grant = Grant::Scoped(scope);
        if bound.is_scoped_version() {
            bound.manager.scope(scope);
        }
        if bound.sees_metadata() {
            self.describe::<D>(display, index);
            let bound = &self.bound[index];
            bound.manager.session(self.session);
        }
        let bound = &self.bound[index];
        if paused {
            bound.send_paused(true, PauseOrigin::Engine);
        }
        bound.manager.done();
    }

    /// Announces every known window, and the focus, to one manager.
    fn describe<D: ContextDispatch>(&mut self, display: &DisplayHandle, index: usize) {
        let paused = self.paused;
        let bound = &mut self.bound[index];
        if !bound.sees_windows(paused) {
            return;
        }
        let with_text = bound.sees_text(paused);
        for record in self.known.values() {
            announce::<D>(display, bound, record, with_text);
        }
        bound
            .manager
            .focus(self.focus.and_then(|key| bound.windows.get(&key)), 0);
    }

    fn change_scope<D: ContextDispatch>(
        &mut self,
        display: &DisplayHandle,
        index: usize,
        old: Scope,
        new: Scope,
    ) {
        let paused = self.paused;
        let bound = &mut self.bound[index];
        bound.grant = Grant::Scoped(new);
        if bound.is_scoped_version() {
            bound.manager.scope(new);
        }
        if !new.contains(Scope::Metadata) {
            bound.close_windows();
            return;
        }
        if !old.contains(Scope::Metadata) {
            self.describe::<D>(display, index);
            let bound = &self.bound[index];
            bound.manager.session(self.session);
            bound.manager.done();
            return;
        }
        let gained_text = new.contains(Scope::Text) && !old.contains(Scope::Text);
        if gained_text && bound.sees_windows(paused) {
            for (key, window) in &bound.windows {
                if let Some(record) = self.known.get(key) {
                    window.title(record.title.clone());
                    window.done();
                }
            }
        }
    }

    /// A client without the engine or pause scope paused or resumed its own
    /// stream.
    fn set_client_paused<D: ContextDispatch>(
        &mut self,
        display: &DisplayHandle,
        manager: &CrownosContextManagerV1,
        paused: bool,
    ) {
        let Some(index) = self.index_of(manager) else {
            return;
        };
        let bound = &mut self.bound[index];
        if bound.self_paused == paused {
            return;
        }
        bound.self_paused = paused;
        if paused {
            bound.close_windows();
        }
        bound.send_paused(paused, PauseOrigin::Client);
        if !paused {
            self.describe::<D>(display, index);
            self.bound[index].manager.done();
        }
    }

    fn index_of(&self, manager: &CrownosContextManagerV1) -> Option<usize> {
        self.bound
            .iter()
            .position(|bound| bound.manager.id() == manager.id())
    }

    fn bound_of(&self, manager: &CrownosContextManagerV1) -> Option<&Bound> {
        self.index_of(manager).map(|index| &self.bound[index])
    }

    /// Whether the client that owns `window` may flag `key` now.
    fn may_flag(&mut self, window: &CrownosContextWindowV1, key: WindowKey) -> bool {
        let now = Instant::now();
        self.bound
            .iter_mut()
            .find(|bound| bound.is_live() && bound.owns_window(window))
            .is_some_and(|bound| bound.take_flag(key, now))
    }

    fn bind<D: ContextDispatch>(
        &mut self,
        display: &DisplayHandle,
        manager: CrownosContextManagerV1,
        decision: BindDecision,
    ) {
        self.bound.push(Bound::new(manager));
        let index = self.bound.len() - 1;
        match decision {
            BindDecision::Granted(scope) => {
                self.rescope::<D>(display, index, Some(scope), RevokeReason::User);
            }
            BindDecision::Pending if self.bound[index].is_scoped_version() => {}
            BindDecision::Pending | BindDecision::Refused => {
                revoke(&mut self.bound[index], RevokeReason::User);
            }
        }
    }

    fn unbind(&mut self, manager: &CrownosContextManagerV1) -> Option<Option<ClientId>> {
        let index = self.index_of(manager)?;
        Some(self.bound.remove(index).client)
    }
}

/// Makes a manager inert: a version 2 client gets revoked, a version 1
/// client sees its windows close and the description end.
fn revoke(bound: &mut Bound, reason: RevokeReason) {
    let was_pending = bound.grant == Grant::Pending;
    bound.grant = Grant::Revoked;
    bound.close_windows();
    if bound.is_scoped_version() {
        bound.manager.revoked(reason);
    } else if was_pending {
        bound.manager.done();
    }
}
