//! Live config reload.
//!
//! The schema lives in `crownos-config` and the loading in the `config` crate;
//! this is only the plumbing that cannot, because it needs the event loop and
//! `&mut State`.

use anyhow::anyhow;
use calloop::LoopHandle;
use config::Config;
use crownos_config::Subscription;

use crate::state::State;

/// Which layer changed, so a reload only re-reads what it has to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// `compositor.ron` — tiling, gaps, keybindings, window rules.
    Compositor,
    /// A shared CrownOS section the compositor reads.
    System,
}

pub struct ConfigState {
    pub current: Config,
    /// Dropping these unregisters the watches, so they have to be owned here.
    _subscriptions: Vec<Subscription>,
}

impl ConfigState {
    /// Watcher callbacks run on a background thread and are `Send + Sync`, so
    /// they can never touch `State`. They post through a calloop channel and the
    /// loop applies the change where `&mut State` is actually available.
    pub fn init(loop_handle: &LoopHandle<'static, State>) -> anyhow::Result<Self> {
        let (tx, rx) = calloop::channel::channel::<Section>();

        loop_handle
            .insert_source(rx, |event, _, state| {
                if let calloop::channel::Event::Msg(section) = event {
                    let next = match section {
                        Section::Compositor => state.config.current.reload_compositor(),
                        Section::System => state.config.current.reload_system(),
                    };
                    tracing::info!(?section, "reloading config");
                    state.apply_config(next);
                }
            })
            .map_err(|err| anyhow!("Failed to insert the config source: {err}"))?;

        let mut subscriptions = Vec::new();
        for section in Config::sections() {
            let which = if section == config::Compositor::SECTION {
                Section::Compositor
            } else {
                Section::System
            };
            let tx = tx.clone();
            subscriptions.push(crownos_config::subscribe(section, move |_| {
                let _ = tx.send(which);
            }));
        }

        Ok(Self {
            current: Config::load(),
            _subscriptions: subscriptions,
        })
    }
}
