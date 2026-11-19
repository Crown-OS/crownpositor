//! The workspace under the frost, as it was on the desktop when the stack came
//! up, less the windows that have left it as cards: the top card fades in over
//! its window as it lifts off, and only then does the window leave its place.

use smithay::backend::renderer::{ImportAll, ImportMem, Renderer};

use crate::{
    rendering::{
        decorate::TileDecorator,
        painter::{Backing, Painter, Thumbnail},
    },
    shell::monitor::Monitor,
};

impl<R, D> Painter<'_, '_, R, D>
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
        let shown = control
            .stacking()
            .iter()
            .filter_map(|&id| workspace.tile(id))
            .filter(|tile| tile.is_shown() && fullscreen.is_none_or(|id| id == tile.id()));
        let vacated = control.switcher().vacated();
        for tile in shown {
            let alpha = tile.render_alpha() * (1.0 - control.focus_of(tile.id()) * vacated) as f32;
            if alpha > 0.0 {
                self.thumbnail(Thumbnail {
                    tile,
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
