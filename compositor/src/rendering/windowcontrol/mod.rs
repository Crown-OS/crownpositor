//! Drawing one output's Alt+Tab strip.
//!
//! Front to back, in one place, because every piece of glass blurs the same
//! copy of the scene and nothing that is not glass may sit between two pieces
//! that overlap:
//!
//! 1. The window zooming forward out of the strip.
//! 2. The strip: titles, the selection ring and the thumbnails, then its
//!    shadow and its glass.
//! 3. The workspace, stepped back behind it.

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

/// Appends the strip and the workspace behind it. The wallpaper and the
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
    painter.zooming(monitor);
    painter.strip(monitor);
    painter.stage(monitor);
}
