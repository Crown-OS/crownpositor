//! The workspace behind the panel: the selected window comes forward, full
//! size and opaque, while every other one steps back and fades.

use smithay::backend::renderer::{ImportAll, ImportMem, Renderer};

use windowcontrol::{
    layout,
    motion::{FADE_BACK, SCALE_BACK},
};

use crate::{
    rendering::{
        decorate::TileDecorator,
        painter::{Backing, Painter, Thumbnail},
    },
    shell::{monitor::Monitor, tile::Tile},
};

impl<R, D> Painter<'_, R, D>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    /// While the panel is open, windows on their way forward are drawn over
    /// the rest, the most forward first; once it closes the stacking order is
    /// the desktop's again. A fullscreen window hides whatever is not forward.
    pub(super) fn stage(&mut self, monitor: &Monitor) {
        let control = monitor.window_control();
        let reveal = control.switcher().eased();
        let point = control.vanishing_point();
        let workspace = monitor.active();
        let fullscreen = workspace.fullscreen();
        let open = control.switcher().is_open();
        let focus = |tile: &Tile| control.focus_of(tile.id());
        let forward = |tile: &Tile| open && focus(tile) > 0.0;

        let mut front: Vec<&Tile> = workspace
            .stacking_order()
            .filter(|tile| forward(tile))
            .collect();
        front.sort_by(|a, b| focus(b).total_cmp(&focus(a)));
        let behind = workspace.stacking_order().filter(|tile| {
            !forward(tile) && fullscreen.is_none_or(|id| id == tile.id() || focus(tile) > 0.0)
        });

        for tile in front.into_iter().chain(behind) {
            let back = reveal * (1.0 - focus(tile));
            self.thumbnail(Thumbnail {
                window: tile.window(),
                rect: layout::recede(tile.render_rect(), point, 1.0 - (1.0 - SCALE_BACK) * back),
                clip: None,
                alpha: tile.render_alpha() * (1.0 - (1.0 - FADE_BACK) * back) as f32,
                backing: Backing::Glass,
                radius: None,
            });
        }
    }
}
