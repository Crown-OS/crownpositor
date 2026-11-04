//! Scenes for capturing one window or one workspace on its own, built from
//! the same pieces as an output's scene.

use smithay::{
    backend::renderer::{ImportAll, ImportMem, Renderer},
    utils::{Logical, Point, Scale},
    wayland::shell::wlr_layer::Layer,
};

use crate::{
    rendering::{
        Elements, FrameStyle, Stacking, cursor::Cursor, decorate::TileDecorator, layer_elements,
        popup, tile_elements, workspace_window_elements,
    },
    shell::{Shell, monitor::Monitor, tile::Tile, workspace::Workspace},
};

/// The pointer, for a source the user is looking at right now.
pub struct CursorOverlay<'a> {
    pub cursor: &'a mut Cursor,
    pub pointer: Point<f64, Logical>,
    /// Where the image's top-left corner is, in global coordinates.
    pub origin: Point<i32, Logical>,
}

impl CursorOverlay<'_> {
    fn render<R, D>(
        self,
        elements: &mut Elements<R, D>,
        renderer: &mut R,
        decorator: &mut D,
        scale: Scale<f64>,
    ) where
        R: Renderer + ImportAll + ImportMem,
        R::TextureId: Send + Clone + 'static,
        D: TileDecorator<R>,
    {
        self.cursor.render(
            elements,
            renderer,
            decorator,
            self.origin,
            self.pointer,
            scale,
        );
    }
}

/// One window with its popups and frame, its frame's top-left corner at the
/// image's.
pub fn window_elements<R, D>(
    shell: &Shell,
    tile: &Tile,
    renderer: &mut R,
    decorator: &mut D,
    cursor: Option<CursorOverlay<'_>>,
    scale: Scale<f64>,
    style: &mut FrameStyle<'_>,
) -> Elements<R, D>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    let mut elements = Elements::<R, D>::new();
    if let Some(cursor) = cursor {
        cursor.render(&mut elements, renderer, decorator, scale);
    }
    let offset = Point::default() - tile.render_rect().loc;
    popup::tile_popup_elements(&mut elements, tile, renderer, decorator, scale, offset);
    tile_elements(
        &mut elements,
        shell,
        tile,
        renderer,
        decorator,
        scale,
        offset,
        style.radius,
        style,
    );
    elements
}

/// One workspace as its output shows it when it is the active one: its
/// windows over the wallpaper, without the panels and notifications above.
#[allow(clippy::too_many_arguments)]
pub fn workspace_elements<R, D>(
    shell: &Shell,
    monitor: &Monitor,
    workspace: &Workspace,
    renderer: &mut R,
    decorator: &mut D,
    cursor: Option<CursorOverlay<'_>>,
    scale: Scale<f64>,
    style: &mut FrameStyle<'_>,
) -> Elements<R, D>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    let mut elements = Elements::<R, D>::new();
    if let Some(cursor) = cursor {
        cursor.render(&mut elements, renderer, decorator, scale);
    }
    popup::workspace_popup_elements(
        &mut elements,
        workspace,
        renderer,
        decorator,
        scale,
        Point::default(),
    );
    let covered = workspace_window_elements(
        &mut elements,
        shell,
        workspace,
        renderer,
        decorator,
        scale,
        Point::default(),
        style,
        Stacking::All,
    );
    if !covered {
        layer_elements(
            &mut elements,
            monitor,
            renderer,
            decorator,
            scale,
            &[Layer::Bottom, Layer::Background],
        );
    }
    elements
}
