//! Touchpad gestures, resolving to the same [`Action`]s the keyboard produces.

use std::collections::HashMap;

use crate::input::{
    shortcuts::action::{Action, WorkspaceRef},
    trackpad::gestures::{Fingers, SwipeGesture},
};

#[derive(Debug, Default)]
pub struct GestureBindings {
    map: HashMap<SwipeGesture, Action>,
}

impl GestureBindings {
    pub fn defaults() -> Self {
        Self {
            map: [
                (
                    SwipeGesture::LeftToRight(Fingers::Three),
                    Action::Workspace(WorkspaceRef::Relative(-1)),
                ),
                (
                    SwipeGesture::RightToLeft(Fingers::Three),
                    Action::Workspace(WorkspaceRef::Relative(1)),
                ),
                (
                    SwipeGesture::BottomToTop(Fingers::Four),
                    Action::OpenWorkspaceView,
                ),
                (
                    SwipeGesture::TopToBottom(Fingers::Four),
                    Action::CloseWorkspaceView,
                ),
            ]
            .into(),
        }
    }

    pub fn lookup(&self, gesture: SwipeGesture) -> Option<Action> {
        self.map.get(&gesture).cloned()
    }
}
