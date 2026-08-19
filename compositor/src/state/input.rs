use std::collections::HashSet;

use smithay::{
    input::keyboard::Keycode,
    utils::{Logical, Point},
};

use config::Config;

use crate::{
    input::{
        mouse::gestures::GestureState,
        shortcuts::{Bindings, GestureBindings, ModMask},
    },
    rendering::cursor::Cursor,
};

pub struct InputState {
    pub bindings: Bindings,
    pub gesture_bindings: GestureBindings,
    pub gesture: GestureState,

    /// Keycodes whose press was intercepted, so the release can be swallowed
    /// too. Otherwise the client sees a release with no matching press and the
    /// app's modifier stays logically stuck down.
    pub intercepted: HashSet<Keycode>,

    /// A held modifier-only chord, plus whether an ordinary key was struck while
    /// it was held. `Super` alone fires on release, and only if nothing else
    /// happened in between.
    pub mod_chord_armed: Option<ModMask>,
    pub mod_chord_polluted: bool,

    /// Global logical coordinates, not per-output.
    pub pointer_location: Point<f64, Logical>,
    /// What the pointer looks like, and the theme it is drawn from. Owned here
    /// rather than per-output: the image cache is keyed on scale, so two
    /// monitors share every entry they have in common.
    pub cursor: Cursor,
}

impl InputState {
    pub fn new(config: &Config) -> Self {
        Self {
            bindings: Bindings::from_config(&config.compositor),
            gesture_bindings: GestureBindings::defaults(),
            gesture: GestureState::new(),
            intercepted: HashSet::new(),
            mod_chord_armed: None,
            mod_chord_polluted: false,
            pointer_location: (0.0, 0.0).into(),
            cursor: Cursor::new(),
        }
    }
}
