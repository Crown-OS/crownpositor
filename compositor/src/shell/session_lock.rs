//! `ext-session-lock-v1`: what is on screen, and who hears input, while the
//! session is locked.
//!
//! Locked means nothing else is drawn and nothing else is typeable. The
//! client is only told the lock holds once every output has drawn a locked
//! frame, so no unlocked content can still be on a screen when it hears so.

use std::collections::HashSet;

use smithay::{
    backend::renderer::{element::Id, utils::CommitCounter},
    output::Output,
    wayland::session_lock::{LockSurface, SessionLocker},
};

#[derive(Default)]
enum Phase {
    #[default]
    Unlocked,
    /// Waiting for every output to draw a locked frame before confirming.
    Locking {
        locker: SessionLocker,
        blanked: HashSet<Output>,
    },
    Locked,
}

pub struct SessionLock {
    phase: Phase,
    surfaces: Vec<(Output, LockSurface)>,
    /// Stable, so the backdrop is only damaged when the lock begins.
    backdrop: Id,
}

impl Default for SessionLock {
    fn default() -> Self {
        Self {
            phase: Phase::default(),
            surfaces: Vec::new(),
            backdrop: Id::new(),
        }
    }
}

impl SessionLock {
    /// Whether the desktop is hidden: from the lock request on, not only once
    /// it is confirmed.
    pub fn is_active(&self) -> bool {
        !matches!(self.phase, Phase::Unlocked)
    }

    pub fn begin(&mut self, locker: SessionLocker) {
        self.phase = Phase::Locking {
            locker,
            blanked: HashSet::new(),
        };
        self.backdrop = Id::new();
    }

    pub fn end(&mut self) {
        self.phase = Phase::Unlocked;
        self.surfaces.clear();
    }

    pub fn add_surface(&mut self, output: Output, surface: LockSurface) {
        self.surfaces.retain(|(held, _)| *held != output);
        self.surfaces.push((output, surface));
    }

    pub fn surface_on(&self, output: &Output) -> Option<&LockSurface> {
        self.surfaces
            .iter()
            .find(|(held, surface)| held == output && surface.alive())
            .map(|(_, surface)| surface)
    }

    pub fn any_surface(&self) -> Option<&LockSurface> {
        self.surfaces
            .iter()
            .map(|(_, surface)| surface)
            .find(|surface| surface.alive())
    }

    pub fn backdrop(&self) -> (&Id, CommitCounter) {
        (&self.backdrop, CommitCounter::default())
    }

    /// Records that `output` has drawn a locked frame.
    pub fn output_blanked(&mut self, output: &Output) {
        if let Phase::Locking { blanked, .. } = &mut self.phase {
            blanked.insert(output.clone());
        }
    }

    /// Sends `locked` once every output in `outputs` has drawn a locked frame.
    pub fn confirm_if_blanked<'a>(&mut self, mut outputs: impl Iterator<Item = &'a Output>) {
        let Phase::Locking { blanked, .. } = &self.phase else {
            return;
        };
        if !outputs.all(|output| blanked.contains(output)) {
            return;
        }
        if let Phase::Locking { locker, .. } = std::mem::replace(&mut self.phase, Phase::Locked) {
            locker.lock();
            tracing::info!("session locked");
        }
    }
}
