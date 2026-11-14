//! Server-side `crownos_context_v1`: what the user is doing, without content,
//! for the visual context engine.
//!
//! One client binds at a time. The compositor syncs window records every
//! loop iteration, and pushes session changes, input activity buckets and
//! selection metadata as they happen; the engine's own requests come back
//! through [`ContextHandler`].

mod dispatch;

use std::collections::HashMap;

use crownos_protocols::context::v1::server::{
    crownos_context_manager_v1::CrownosContextManagerV1,
    crownos_context_window_v1::CrownosContextWindowV1,
};
use wayland_server::{Client, DisplayHandle, Resource, backend::GlobalId};

pub use crate::crownos_window_control::{Placement, WindowKey};
pub use crownos_protocols::context::v1::server::{
    crownos_context_manager_v1::{
        ActivityFlags, PauseOrigin, SelectionFlags, SelectionKind, SessionState,
    },
    crownos_context_window_v1::{
        ContentType, FlagReason, Sensitivity, State as ContextWindowFlags,
    },
};
pub use dispatch::ContextDispatch;

/// The protocol version this module speaks.
pub const VERSION: u32 = 1;

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

/// What the compositor provides for this protocol to be delegated to
/// [`ContextState`].
pub trait ContextHandler {
    fn context_state(&mut self) -> &mut ContextState;
    /// The engine paused or resumed observation.
    fn set_context_paused(&mut self, paused: bool);
    /// The engine saw something in a window that must not be captured.
    fn flag_window_sensitive(&mut self, key: WindowKey, reason: FlagReason, duration_ms: u32);
    /// The engine disconnected.
    fn context_engine_gone(&mut self);
}

/// Global data: which clients may see the manager.
pub struct ContextGlobalData {
    filter: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

struct Bound {
    manager: CrownosContextManagerV1,
    windows: HashMap<WindowKey, CrownosContextWindowV1>,
}

/// Delegate type for the `crownos_context_manager_v1` global.
pub struct ContextState {
    global: GlobalId,
    bound: Option<Bound>,
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
            bound: None,
            known: HashMap::new(),
            focus: None,
            session: SessionState::Active,
            paused: false,
        }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }

    pub fn is_bound(&self) -> bool {
        self.bound
            .as_ref()
            .is_some_and(|bound| bound.manager.is_alive())
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn knows(&self, key: WindowKey) -> bool {
        self.known.contains_key(&key)
    }

    /// Brings the engine up to date. While paused, nothing is sent and the
    /// records are not remembered, so resuming announces the current state.
    pub fn sync<D: ContextDispatch>(
        &mut self,
        display: &DisplayHandle,
        records: &[ContextRecord],
        focus: Option<WindowKey>,
        time_ms: u32,
    ) {
        if self.paused {
            return;
        }
        if !self.is_bound() {
            self.bound = None;
        }
        let gone: Vec<WindowKey> = self
            .known
            .keys()
            .filter(|key| !records.iter().any(|record| record.key == **key))
            .copied()
            .collect();
        for key in gone {
            self.known.remove(&key);
            if let Some(window) = self
                .bound
                .as_mut()
                .and_then(|bound| bound.windows.remove(&key))
            {
                window.closed();
            }
        }
        for record in records {
            match self.known.get(&record.key) {
                Some(known) if known == record => continue,
                Some(known) => {
                    if let Some(window) = self
                        .bound
                        .as_ref()
                        .and_then(|bound| bound.windows.get(&record.key))
                    {
                        send_changes(window, known, record);
                        window.done();
                    }
                }
                None => {
                    if let Some(bound) = &mut self.bound {
                        announce::<D>(display, bound, record);
                    }
                }
            }
            self.known.insert(record.key, record.clone());
        }
        if self.focus != focus {
            self.focus = focus;
            if let Some(bound) = &self.bound {
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
        if let Some(bound) = &self.bound {
            bound.manager.session(session);
        }
    }

    pub fn input_activity(&self, activity: InputActivity) {
        let Some(bound) = self.bound.as_ref().filter(|_| !self.paused) else {
            return;
        };
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

    pub fn selection(&self, change: &SelectionChange) {
        let Some(bound) = self.bound.as_ref().filter(|_| !self.paused) else {
            return;
        };
        let window = change.window.and_then(|key| bound.windows.get(&key));
        bound.manager.selection(
            change.kind,
            window,
            change.mime_types.clone(),
            change.size_hint,
            change.flags,
        );
    }

    /// Pauses or resumes. Pausing forgets the announced windows and closes
    /// them for the engine; resuming re-announces them on the next sync.
    pub fn set_paused(&mut self, paused: bool, origin: PauseOrigin) {
        if self.paused == paused {
            return;
        }
        self.paused = paused;
        if let Some(bound) = &mut self.bound {
            if paused {
                for window in bound.windows.drain() {
                    window.1.closed();
                }
                self.known.clear();
                self.focus = None;
            }
            bound.manager.paused(u32::from(paused), origin);
        }
    }

    fn bind<D: ContextDispatch>(
        &mut self,
        display: &DisplayHandle,
        manager: CrownosContextManagerV1,
    ) -> bool {
        if self.is_bound() {
            manager.done();
            return false;
        }
        let mut bound = Bound {
            manager,
            windows: HashMap::new(),
        };
        if !self.paused {
            for record in self.known.values() {
                announce::<D>(display, &mut bound, record);
            }
            bound
                .manager
                .focus(self.focus.and_then(|key| bound.windows.get(&key)), 0);
        }
        bound.manager.session(self.session);
        if self.paused {
            bound.manager.paused(1, PauseOrigin::Engine);
        }
        bound.manager.done();
        self.bound = Some(bound);
        true
    }

    fn unbind(&mut self, manager: &CrownosContextManagerV1) -> bool {
        if self
            .bound
            .as_ref()
            .is_some_and(|bound| bound.manager.id() == manager.id())
        {
            self.bound = None;
            return true;
        }
        false
    }
}

fn announce<D: ContextDispatch>(
    display: &DisplayHandle,
    bound: &mut Bound,
    record: &ContextRecord,
) {
    let Some(client) = bound.manager.client() else {
        return;
    };
    let Ok(window) = client.create_resource::<CrownosContextWindowV1, WindowKey, D>(
        display,
        bound.manager.version(),
        record.key,
    ) else {
        return;
    };
    bound.manager.window(&window, record.identifier.clone());
    window.app_id(record.app_id.clone());
    window.title(record.title.clone());
    window.process(record.pid);
    window.sandbox(
        record.sandbox.engine.clone(),
        record.sandbox.app_id.clone(),
        record.sandbox.instance_id.clone(),
    );
    send_placement(&window, &record.placement);
    window.state(record.flags);
    window.content_type(record.content_type);
    window.sensitivity(record.sensitivity);
    window.done();
    bound.windows.insert(record.key, window);
}

fn send_changes(window: &CrownosContextWindowV1, known: &ContextRecord, record: &ContextRecord) {
    if known.app_id != record.app_id {
        window.app_id(record.app_id.clone());
    }
    if known.title != record.title {
        window.title(record.title.clone());
    }
    if known.pid != record.pid {
        window.process(record.pid);
    }
    if known.sandbox != record.sandbox {
        window.sandbox(
            record.sandbox.engine.clone(),
            record.sandbox.app_id.clone(),
            record.sandbox.instance_id.clone(),
        );
    }
    if known.placement != record.placement {
        send_placement(window, &record.placement);
    }
    if known.flags != record.flags {
        window.state(record.flags);
    }
    if known.content_type != record.content_type {
        window.content_type(record.content_type);
    }
    if known.sensitivity != record.sensitivity {
        window.sensitivity(record.sensitivity);
    }
}

fn send_placement(window: &CrownosContextWindowV1, placement: &Placement) {
    window.placement(
        placement.x,
        placement.y,
        placement.width,
        placement.height,
        placement.output.clone(),
        placement.workspace.clone(),
    );
}
