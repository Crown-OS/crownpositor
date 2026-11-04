//! The workspace under the frost, as it is on the desktop, less the windows
//! that have left it as cards: the top card stands in for its window from the
//! moment it lifts off until it lands back on it.

use smithay::backend::renderer::{ImportAll, ImportMem, Renderer};

use crate::{
    rendering::{
        decorate::TileDecorator,
        painter::{Backing, Painter, Thumbnail},
    },
    shell::monitor::Monitor,
};

impl<R, D> Painter<'_, R, D>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    /// A fullscreen window hides everything else, as on the desktop.
    pub(super) fn stage(&mut self, monitor: &Monitor) {
        let control = monitor.window_control();
        let workspace = monitor.active();
        let fullscreen = workspace.fullscreen();
        let shown = workspace
            .stacking_order()
            .filter(|tile| fullscreen.is_none_or(|id| id == tile.id()));
        for tile in shown {
            let alpha = tile.render_alpha() * (1.0 - control.focus_of(tile.id())) as f32;
            if alpha > 0.0 {
                self.thumbnail(Thumbnail {
                    window: tile.window(),
                    rect: tile.render_rect(),
                    clip: None,
                    alpha,
                    backing: Backing::Glass,
                    radius: None,
                });
            }
        }
    }
}
