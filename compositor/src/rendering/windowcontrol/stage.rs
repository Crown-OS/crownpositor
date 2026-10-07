//! The workspace behind the strip, and the window leaving the strip for it.

use smithay::backend::renderer::{ImportAll, ImportMem, Renderer};

use spacecontrol::scene;
use windowcontrol::{layout, motion::SCALE_BACK};

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
    /// The chosen window growing from its thumbnail back to its own frame as
    /// the strip sinks, in front of everything.
    pub(super) fn zooming(&mut self, monitor: &Monitor) {
        let control = monitor.window_control();
        let Some(tile) = control
            .switcher()
            .committed()
            .and_then(|id| monitor.active().tile(id))
        else {
            return;
        };
        let home = tile.render_rect();
        let rect = control.slot_of(tile.id()).map_or(home, |slot| {
            scene::between(home, slot.rect, control.switcher().eased())
        });
        self.thumbnail(Thumbnail {
            window: tile.window(),
            rect,
            clip: None,
            alpha: tile.render_alpha(),
            backing: Backing::Glass,
        });
    }

    /// The active workspace, stepped back by however far the strip has risen.
    /// A fullscreen window is all of its workspace, as on the desktop.
    pub(super) fn stage(&mut self, monitor: &Monitor) {
        let control = monitor.window_control();
        let factor = 1.0 - (1.0 - SCALE_BACK) * control.switcher().eased();
        let point = control.vanishing_point();
        let committed = control.switcher().committed();
        let workspace = monitor.active();
        let fullscreen = workspace.fullscreen();

        for tile in workspace.stacking_order() {
            if Some(tile.id()) == committed || fullscreen.is_some_and(|id| id != tile.id()) {
                continue;
            }
            self.thumbnail(Thumbnail {
                window: tile.window(),
                rect: layout::recede(tile.render_rect(), point, factor),
                clip: None,
                alpha: tile.render_alpha(),
                backing: Backing::Glass,
            });
        }
    }
}
