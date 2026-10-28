//! Drawing one output's Alt+Tab panel.
//!
//! Front to back, in one place, because every piece of glass blurs the same
//! copy of the scene and nothing that is not glass may sit between two pieces
//! that overlap:
//!
//! 1. The panel: the selection's title, the cards, then its shadow and its
//!    glass.
//! 2. The workspace, the selected window come forward and the rest stepped
//!    back behind it.

mod stage;
mod strip;

use smithay::{
    backend::renderer::{ImportAll, ImportMem, Renderer},
    utils::Scale,
};

use crate::{
    rendering::{Elements, FrameStyle, decorate::TileDecorator, painter::Painter},
    shell::monitor::Monitor,
};

/// Appends the panel and the workspace behind it. The wallpaper and the
/// panels are the caller's, drawn as they always are.
pub fn window_control_elements<R, D>(
    elements: &mut Elements<R, D>,
    monitor: &Monitor,
    renderer: &mut R,
    decorator: &mut D,
    scale: Scale<f64>,
    style: &mut FrameStyle<'_>,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    let mut painter = Painter::new(elements, renderer, decorator, scale, style);
    painter.strip(monitor);
    painter.stage(monitor);
}
