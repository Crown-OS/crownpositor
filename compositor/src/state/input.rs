use std::collections::HashSet;

use smithay::{
    input::keyboard::Keycode,
    utils::{Logical, Point},
};

use config::Config;

use crate::{
    input::decoration::{FramePress, LastClick},
    input::{
        shortcuts::{Bindings, GestureBindings, ModMask},
        trackpad::gestures::GestureState,
    },
    rendering::cursor::Cursor,
    shell::decoration::Control,
    utils::id::WindowId,
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

    /// The window control the pointer is over, if any. Only the renderer reads
    /// it — a hovered control lifts rather than changing what a click does.
    pub hovered_control: Option<(WindowId, Control)>,
    /// A press on a control waiting for its release. A click that goes down on
    /// one control and comes up somewhere else is cancelled, as everywhere else.
    pub frame_press: Option<FramePress>,
    /// The last click on a titlebar, for spotting the second half of a double.
    pub last_frame_click: Option<LastClick>,
}

impl InputState {
    pub fn new(config: &Config) -> Self {
        Self {
            bindings: Bindings::with_custom(&config.keybinds.custom_keybinds),
            gesture_bindings: GestureBindings::defaults(),
            gesture: GestureState::new(),
            intercepted: HashSet::new(),
            mod_chord_armed: None,
            mod_chord_polluted: false,
            pointer_location: (0.0, 0.0).into(),
            cursor: Cursor::new(),
            hovered_control: None,
            frame_press: None,
            last_frame_click: None,
        }
    }
}
