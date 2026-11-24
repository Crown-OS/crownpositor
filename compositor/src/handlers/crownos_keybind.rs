//! Delegation glue for `crownos_keybind_v1`. The protocol lives in
//! [`protocols::crownos_keybind`], the assignment rules in
//! [`crate::input::shortcuts::app_keybinds`].

use std::collections::HashMap;

use config::{AppAction, AppActions};
use crownos_config::Keybind;
use crownos_protocols::keybind::v1::server::{
    crownos_keybind_manager_v1::CrownosKeybindManagerV1, crownos_keybind_v1::CrownosKeybindV1,
};
use protocols::crownos_keybind::{KeybindHandler, KeybindState};
use wayland_server::{delegate_dispatch, delegate_global_dispatch};

use crate::{
    input::shortcuts::{
        Chord,
        app_keybinds::{ActionRequest, assign_chords, record_registered_actions},
    },
    state::State,
};

impl KeybindHandler for State {
    fn keybind_state(&mut self) -> &mut KeybindState {
        &mut self.wayland.keybind_state
    }

    fn keybinds_changed(&mut self) {
        self.assign_app_keybinds();
    }
}

impl State {
    /// Assigns the chords again after a registration or a settings change,
    /// tells each application whose chord changed, and points each chord at
    /// the newest registration of its action.
    pub fn assign_app_keybinds(&mut self) {
        let registrations = self.wayland.keybind_state.registrations_mut();
        let requests: Vec<ActionRequest<'_>> = registrations
            .iter()
            .map(|registration| ActionRequest {
                app_id: registration.app_id(),
                name: registration.name(),
                preferred_keys: registration.preferred_keys(),
                assigned_keys: registration.assigned_keys(),
            })
            .collect();
        let bindings = &self.input.bindings;
        let config = &self.config.current;
        let chords_grabbed_elsewhere = [
            config.keybinds.launcher,
            config.keybinds.dictation,
            config.input.dictation_hotkey,
        ]
        .map(Chord::from);
        let chords = assign_chords(&requests, &config.keybinds.app_keybinds, |chord| {
            bindings.is_bound(chord) || chords_grabbed_elsewhere.contains(&chord)
        });

        let mut receiver_of_chord: HashMap<Chord, CrownosKeybindV1> = HashMap::new();
        let mut registered_actions = Vec::with_capacity(registrations.len());
        for (registration, chord) in registrations.iter_mut().zip(chords) {
            let assigned_keys = chord.unwrap_or(Keybind::NONE);
            let keys = assigned_keys.to_string();
            if registration.assign(&keys) {
                tracing::info!(
                    app_id = registration.app_id(),
                    name = registration.name(),
                    description = registration.description(),
                    preferred = registration.preferred_keys(),
                    assigned = keys,
                    "application keybind assigned"
                );
            }
            if let Some(chord) = chord {
                receiver_of_chord.insert(chord.into(), registration.resource().clone());
            }
            let preferred_keys = registration
                .preferred_keys()
                .parse()
                .unwrap_or(Keybind::NONE);
            let wanted_keys = config
                .keybinds
                .app_keybinds
                .iter()
                .find(|user_chord| {
                    user_chord.app_id == registration.app_id()
                        && user_chord.name == registration.name()
                })
                .map_or(preferred_keys, |user_chord| user_chord.keys);
            registered_actions.push(AppAction {
                app_id: registration.app_id().to_owned(),
                name: registration.name().to_owned(),
                description: registration.description().to_owned(),
                preferred_keys,
                wanted_keys,
                assigned_keys,
                running: true,
            });
        }
        self.input.app_keybinds.replace(receiver_of_chord);
        publish_app_actions(registered_actions);
    }
}

/// Writes the registered actions to `app_actions.ron` for the settings panel.
/// Nothing is written when the list did not change.
fn publish_app_actions(registered_actions: Vec<AppAction>) {
    let mut app_actions: AppActions = crownos_config::load(AppActions::SECTION);
    let last_written = app_actions.actions.clone();
    record_registered_actions(&mut app_actions.actions, registered_actions);
    if app_actions.actions == last_written {
        return;
    }
    if let Err(error) = crownos_config::save(AppActions::SECTION, &app_actions) {
        tracing::warn!(%error, "cannot write the application actions");
    }
}

delegate_global_dispatch!(State: [CrownosKeybindManagerV1: ()] => KeybindState);
delegate_dispatch!(State: [CrownosKeybindManagerV1: ()] => KeybindState);
delegate_dispatch!(State: [CrownosKeybindV1: ()] => KeybindState);
