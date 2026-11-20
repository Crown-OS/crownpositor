//! Live config, one subscription per component.
//!
//! Each entry watches only the key or section that one part of the compositor
//! owns, so editing the blur radius does not rebuild the keybinding table and
//! editing a keybind does not recompile the window-rule regexes.

use std::sync::Arc;

use crownos_config::{
    Appearance, Display, Key, Subscription,
    schema::{
        Agents, Binding, GamingOptions, Glass, Lock, OutputLayout, OutputSetting, Theme,
        VisualContext, WorkspaceMode, WorkspaceScope, compositor, keybinds,
    },
    subscribe_key, subscribe_typed,
};
use serde::de::DeserializeOwned;

use crate::rules::WindowRules;

type Sink = dyn Fn(Update) + Send + Sync;

/// A config change, narrowed to the component that has to act on it.
#[derive(Debug)]
pub enum Update {
    DefaultMode(WorkspaceMode),
    FocusFollowsMouse(bool),
    PrimaryOutput(Option<String>),
    WorkspaceScope(WorkspaceScope),
    Outputs(Vec<OutputSetting>),
    OutputLayouts(Vec<OutputLayout>),
    WindowRules(WindowRules),
    Gaming(GamingOptions),
    CustomKeybinds(Vec<Binding>),
    Appearance(Appearance),
    Display(Display),
    Glass(Glass),
    Lock(Box<Lock>),
    Theme(Box<Theme>),
    Agents(Box<Agents>),
    VisualContext(Box<VisualContext>),
}

/// The live subscriptions. Dropping it stops every one of them.
#[must_use = "dropping the watch unregisters every subscription"]
pub struct Watch {
    _subscriptions: Vec<Subscription>,
}

impl Watch {
    /// `sink` runs on the watcher thread, so it must do nothing but hand the
    /// update to whoever owns the state.
    pub fn spawn(sink: impl Fn(Update) + Send + Sync + 'static) -> Self {
        let sink: Arc<Sink> = Arc::new(sink);

        Self {
            _subscriptions: vec![
                key(&sink, compositor::Layout, Update::DefaultMode),
                key(
                    &sink,
                    compositor::FocusFollowsMouse,
                    Update::FocusFollowsMouse,
                ),
                key(&sink, compositor::PrimaryOutput, Update::PrimaryOutput),
                key(&sink, compositor::Scope, Update::WorkspaceScope),
                key(&sink, compositor::Outputs, Update::Outputs),
                key(&sink, compositor::OutputLayouts, Update::OutputLayouts),
                key(&sink, compositor::WindowRules, |rules| {
                    Update::WindowRules(WindowRules::compile(&rules))
                }),
                key(&sink, compositor::Gaming, Update::Gaming),
                key(&sink, keybinds::CustomKeybinds, Update::CustomKeybinds),
                section(&sink, Appearance::SECTION, Update::Appearance),
                section(&sink, Display::SECTION, Update::Display),
                section(&sink, Glass::SECTION, Update::Glass),
                section(&sink, Lock::SECTION, |lock| Update::Lock(Box::new(lock))),
                section(&sink, Theme::SECTION, |theme| {
                    Update::Theme(Box::new(theme))
                }),
                section(&sink, Agents::SECTION, |agents| {
                    Update::Agents(Box::new(agents))
                }),
                section(&sink, VisualContext::SECTION, |rules| {
                    Update::VisualContext(Box::new(rules))
                }),
            ],
        }
    }
}

fn key<K: Key>(sink: &Arc<Sink>, key: K, wrap: fn(K::Value) -> Update) -> Subscription {
    let sink = Arc::clone(sink);
    subscribe_key(key, move |value| sink(wrap(value)))
}

fn section<T: DeserializeOwned + Send + 'static>(
    sink: &Arc<Sink>,
    section: &str,
    wrap: fn(T) -> Update,
) -> Subscription {
    let sink = Arc::clone(sink);
    subscribe_typed::<T, _>(section, move |value| sink(wrap(value)))
}
