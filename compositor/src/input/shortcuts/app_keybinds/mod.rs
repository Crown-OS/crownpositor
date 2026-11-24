//! Chords that applications registered through `crownos_keybind_v1`, and the
//! keys of them that are held down now.
//!
//! A chord with a key starts on the press of its key and ends on the release
//! of that key. A modifier-only chord, such as `Super`, starts when a modifier
//! press makes the held modifiers exactly the chord while no other key is
//! down. It ends on the release of one of its modifiers, and it is cancelled
//! when the user presses another key, adds a modifier or presses a pointer
//! button, because then the user makes a different chord.

mod assignment;
mod published;

use std::collections::{HashMap, HashSet};

use crownos_protocols::keybind::v1::server::crownos_keybind_v1::CrownosKeybindV1;
use smithay::input::keyboard::{Keycode, KeysymHandle, ModifiersState};
use wayland_server::Resource;

pub use assignment::{ActionRequest, assign_chords};
pub use published::record_registered_actions;

use crate::input::shortcuts::chord::{Chord, ModMask, pressed_chords};

#[derive(Debug, Default)]
pub struct AppKeybinds {
    receiver_of_chord: HashMap<Chord, CrownosKeybindV1>,
    held_keys: HashMap<Keycode, HeldChord>,
    held_modifier_chord: Option<HeldChord>,
    ordinary_keys_down: HashSet<Keycode>,
}

#[derive(Debug)]
struct HeldChord {
    chord: Chord,
    receiver: CrownosKeybindV1,
}

impl HeldChord {
    fn cancel(&self) {
        if self.receiver.is_alive() {
            self.receiver.cancelled();
        }
    }
}

impl AppKeybinds {
    /// Replaces the table, and cancels each held press whose chord does not
    /// go to the same registration any more.
    pub fn replace(&mut self, receiver_of_chord: HashMap<Chord, CrownosKeybindV1>) {
        let still_receives =
            |held: &HeldChord| receiver_of_chord.get(&held.chord) == Some(&held.receiver);
        self.held_keys.retain(|_, held| {
            let keep = still_receives(held);
            if !keep {
                held.cancel();
            }
            keep
        });
        if let Some(held) = self
            .held_modifier_chord
            .take_if(|held| !still_receives(held))
        {
            held.cancel();
        }
        self.receiver_of_chord = receiver_of_chord;
    }

    /// Sends `pressed` to the action the chord is assigned to. Returns whether
    /// a chord matched, so the caller keeps the key from the focused client.
    pub fn press(
        &mut self,
        modifiers: &ModifiersState,
        handle: &KeysymHandle<'_>,
        time_in_milliseconds: u32,
    ) -> bool {
        let Some((chord, receiver)) = pressed_chords(modifiers, handle).find_map(|chord| {
            self.receiver_of_chord
                .get(&chord)
                .map(|receiver| (chord, receiver.clone()))
        }) else {
            return false;
        };
        receiver.pressed(time_in_milliseconds);
        self.held_keys
            .insert(handle.raw_code(), HeldChord { chord, receiver });
        true
    }

    pub fn release(&mut self, key: Keycode, time_in_milliseconds: u32) {
        if let Some(held) = self.held_keys.remove(&key) {
            held.receiver.released(time_in_milliseconds);
        }
    }

    /// A key that is not a modifier went down, so a held modifier-only chord
    /// becomes a different chord.
    pub fn ordinary_key_pressed(&mut self, key: Keycode) {
        self.ordinary_keys_down.insert(key);
        self.cancel_modifier_chord();
    }

    pub fn ordinary_key_released(&mut self, key: Keycode) {
        self.ordinary_keys_down.remove(&key);
    }

    /// A modifier went down and the held modifiers are now `held_modifiers`.
    /// A held modifier-only chord is cancelled, because a modifier was added.
    /// When `may_start` and no other key is down, the modifier-only chord of
    /// exactly `held_modifiers` starts.
    pub fn modifier_pressed(
        &mut self,
        held_modifiers: ModMask,
        may_start: bool,
        time_in_milliseconds: u32,
    ) {
        self.cancel_modifier_chord();
        if !may_start || !self.ordinary_keys_down.is_empty() {
            return;
        }
        let chord = Chord {
            mods: held_modifiers,
            key: None,
        };
        if let Some(receiver) = self.receiver_of_chord.get(&chord) {
            receiver.pressed(time_in_milliseconds);
            self.held_modifier_chord = Some(HeldChord {
                chord,
                receiver: receiver.clone(),
            });
        }
    }

    /// A modifier came up, so a held modifier-only chord is released.
    pub fn modifier_released(&mut self, time_in_milliseconds: u32) {
        if let Some(held) = self.held_modifier_chord.take() {
            held.receiver.released(time_in_milliseconds);
        }
    }

    /// Cancels a held modifier-only chord, for example on a pointer button
    /// press: `Super` and a click is a different gesture.
    pub fn cancel_modifier_chord(&mut self) {
        if let Some(held) = self.held_modifier_chord.take() {
            held.cancel();
        }
    }

    /// Ends every held press without a release, so no application acts on
    /// a press that the lock screen interrupted.
    pub fn cancel_all(&mut self) {
        for (_, held) in self.held_keys.drain() {
            held.cancel();
        }
        self.cancel_modifier_chord();
    }
}
