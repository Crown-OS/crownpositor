//! What an Android task window adds to its tile, from its
//! `crownos_android_toplevel_v1`.

use protocols::crownos_android_shell::{AndroidDetails, SizeLock, TitleColor};

use crate::{shell::Shell, utils::id::WindowId};

/// The extras of one Android task window: how it may be resized and the
/// colour of its title bar. A window with these also gets a Back button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AndroidWindow {
    pub size_lock: SizeLock,
    pub title_color: Option<TitleColor>,
}

impl From<&AndroidDetails> for AndroidWindow {
    fn from(details: &AndroidDetails) -> Self {
        Self {
            size_lock: details.size_lock(),
            title_color: details.title_color,
        }
    }
}

impl Shell {
    /// Gives window `id` its Android extras, or takes them away. Returns
    /// whether anything changed, which the frame has to be redrawn for.
    pub fn set_android_window(&mut self, id: WindowId, android: Option<AndroidWindow>) -> bool {
        let Some(area) = self
            .location(id)
            .and_then(|location| self.workspace(location))
            .map(|workspace| workspace.area())
        else {
            return false;
        };
        let changed = self
            .tile_mut(id)
            .is_some_and(|tile| tile.set_android(android, area));
        if changed {
            self.mark_dirty(id);
        }
        changed
    }
}
