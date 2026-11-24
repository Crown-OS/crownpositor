//! Shortcuts: what the user pressed, and what the compositor does about it.

pub mod action;
pub mod app_keybinds;
pub mod bindings;
pub mod chord;
pub mod custom;
pub mod defaults;
pub mod gestures;

#[allow(unused_imports)]
pub use action::{Action, Direction, WorkspaceRef};
pub use app_keybinds::AppKeybinds;
pub use bindings::Bindings;
#[allow(unused_imports)]
pub use chord::{Chord, ModMask};
pub use gestures::GestureBindings;
