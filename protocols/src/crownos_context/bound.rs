//! One bound manager and what its client may see.

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use crownos_protocols::context::v1::server::{
    crownos_context_manager_v1::{CrownosContextManagerV1, PauseOrigin, Scope},
    crownos_context_window_v1::CrownosContextWindowV1,
};
use wayland_server::{Resource, backend::ClientId};

use super::WindowKey;

/// The first version with the scope and revoked events.
pub(super) const SCOPED_VERSION: u32 = 2;
/// A client without the engine scope may flag one window this often.
const FLAG_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Grant {
    /// The compositor asks the user; the client gets nothing meanwhile.
    Pending,
    Scoped(Scope),
    /// The object is inert until the client destroys it.
    Revoked,
}

pub(super) struct Bound {
    pub manager: CrownosContextManagerV1,
    pub client: Option<ClientId>,
    pub grant: Grant,
    /// The client paused its own stream.
    pub self_paused: bool,
    pub windows: HashMap<WindowKey, CrownosContextWindowV1>,
    last_flag: HashMap<WindowKey, Instant>,
}

impl Bound {
    pub fn new(manager: CrownosContextManagerV1) -> Self {
        Self {
            client: manager.client().map(|client| client.id()),
            manager,
            grant: Grant::Pending,
            self_paused: false,
            windows: HashMap::new(),
            last_flag: HashMap::new(),
        }
    }

    pub fn scope(&self) -> Scope {
        match self.grant {
            Grant::Scoped(scope) => scope,
            Grant::Pending | Grant::Revoked => Scope::empty(),
        }
    }

    pub fn is_live(&self) -> bool {
        matches!(self.grant, Grant::Scoped(_)) && self.manager.is_alive()
    }

    pub fn is_scoped_version(&self) -> bool {
        self.manager.version() >= SCOPED_VERSION
    }

    /// Window objects, focus, session and activity.
    pub fn sees_metadata(&self) -> bool {
        self.is_live() && self.scope().contains(Scope::Metadata)
    }

    pub fn sees_windows(&self, globally_paused: bool) -> bool {
        self.sees_metadata() && !self.self_paused && !globally_paused
    }

    /// Titles and selections, which need metadata as well.
    pub fn sees_text(&self, globally_paused: bool) -> bool {
        self.sees_windows(globally_paused) && self.scope().contains(Scope::Text)
    }

    /// Pauses every stream, not only its own.
    pub fn pauses_everyone(&self) -> bool {
        self.scope().intersects(Scope::Engine | Scope::Pause)
    }

    pub fn is_rate_limited(&self) -> bool {
        !self.scope().contains(Scope::Engine)
    }

    /// Whether a flag for `key` is allowed now, and remembers it if so.
    pub fn take_flag(&mut self, key: WindowKey, now: Instant) -> bool {
        if self.is_rate_limited()
            && self
                .last_flag
                .get(&key)
                .is_some_and(|last| now.duration_since(*last) < FLAG_INTERVAL)
        {
            return false;
        }
        self.last_flag.insert(key, now);
        true
    }

    pub fn close_windows(&mut self) {
        for (_, window) in self.windows.drain() {
            window.closed();
        }
    }

    pub fn send_paused(&self, paused: bool, origin: PauseOrigin) {
        let origin = if self.is_scoped_version() || origin != PauseOrigin::Client {
            origin
        } else {
            PauseOrigin::Engine
        };
        self.manager.paused(u32::from(paused), origin);
    }

    pub fn owns_window(&self, window: &CrownosContextWindowV1) -> bool {
        self.windows.values().any(|known| known.id() == window.id())
    }
}
