//! Live config reload.
//!
//! The schema lives in `crownos-config` and the loading in the `config` crate;
//! this is only the plumbing that cannot, because it needs the event loop and
//! `&mut State`.

use anyhow::anyhow;
use calloop::{channel, LoopHandle};
use config::{Config, Update, Watch};

use crate::state::State;

pub struct ConfigState {
    pub current: Config,
    /// Dropping this unregisters the watches, so it has to be owned here.
    _watch: Watch,
}

impl ConfigState {
    /// Watcher callbacks run on a background thread and are `Send + Sync`, so
    /// they can never touch `State`. They post through a calloop channel and the
    /// loop applies the change where `&mut State` is actually available.
    pub fn init(loop_handle: &LoopHandle<'static, State>) -> anyhow::Result<Self> {
        let (sender, receiver) = channel::channel::<Update>();

        loop_handle
            .insert_source(receiver, |event, _, state| {
                if let channel::Event::Msg(update) = event {
                    state.apply_update(update);
                }
            })
            .map_err(|err| anyhow!("Failed to insert the config source: {err}"))?;

        // Subscribing first costs at most one redundant apply; loading first
        // would silently drop an edit that lands before the watch is live.
        let watch = Watch::spawn(move |update| {
            let _ = sender.send(update);
        });

        Ok(Self {
            current: Config::load(),
            _watch: watch,
        })
    }
}
