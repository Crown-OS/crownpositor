//! Windows on their way between the desktop and the grid, and the one the
//! pointer carries.

use smithay::{
    backend::renderer::{ImportAll, ImportMem, Renderer},
    utils::{Point, Rectangle},
};

use spacecontrol::scene;

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
    /// The window under the pointer, above everything it can be dropped on.
    pub(super) fn carried(&mut self, monitor: &Monitor) {
        let Some((slot, rect)) = monitor.spacecontrol().carrying() else {
            return;
        };
        let Some(tile) = monitor.active().tiles().get(slot) else {
            return;
        };
        self.thumbnail(Thumbnail {
            window: tile.window(),
            rect,
            clip: None,
            alpha: tile.render_alpha(),
            backing: Backing::Glass,
        });
    }

    /// Every workspace with part of its grid on screen, at the offset the
    /// viewport spring has it at — a swipe across the overview slides pages
    /// exactly the way the desktop does.
    ///
    /// Stacking order, topmost first, so windows keep their depth on the way
    /// into the grid.
    pub(super) fn grid(&mut self, monitor: &Monitor) {
        let space = monitor.spacecontrol();
        let progress = space.overview().progress();
        let carried = space.carrying().map(|(slot, _)| slot);
        let stride = monitor.page_stride();

        for (index, page) in monitor.switch().visible(monitor.workspaces().len()) {
            let Some(workspace) = monitor.workspaces().get(index) else {
                continue;
            };
            let offset = Point::from((page * stride, 0.0));
            let active = index == monitor.active_index();

            for tile in workspace.stacking_order() {
                let Some(slot) = workspace.tiles().iter().position(|it| it.id() == tile.id())
                else {
                    continue;
                };
                if active && carried == Some(slot) {
                    continue;
                }
                let Some(cell) = space.grid_rect(index, slot) else {
                    continue;
                };
                // Scaled by progress, so a window under the pointer on the way
                // in grows with everything else instead of jumping.
                let lift = space.metrics().hover * space.window_lift(index, slot) * progress;
                let rect = scene::between(tile.render_rect(), scene::lift(cell, lift), progress);
                self.thumbnail(Thumbnail {
                    window: tile.window(),
                    rect: Rectangle::new(rect.loc + offset, rect.size),
                    clip: None,
                    alpha: tile.render_alpha(),
                    backing: Backing::Glass,
                });
            }
        }
    }
}
