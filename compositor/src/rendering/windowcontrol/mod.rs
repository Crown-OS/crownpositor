//! Drawing one output's Alt+Tab stack.
//!
//! Front to back, in one place, because every piece of glass blurs the same
//! copy of the scene and nothing that is not glass may sit between two pieces
//! that overlap:
//!
//! 1. The cards, top card first: each with its icon and the top card's title
//!    over it, then its window, its base and its shadow.
//! 2. The frost: everything below, blurred and dimmed as the stack comes up.
//! 3. The workspace as it is, less the windows the cards stand in for.

mod cards;
mod stage;

use smithay::{
    backend::renderer::{ImportAll, ImportMem, Renderer},
    utils::Scale,
};

use crate::{
    rendering::{Elements, FrameStyle, decorate::TileDecorator, painter::Painter},
    shell::monitor::Monitor,
};

/// How dark the frost behind the stack gets.
const DIM: f64 = 0.3;

/// Appends the stack and the workspace behind it. The wallpaper and the
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
    painter.cards(monitor);

    let control = monitor.window_control();
    let shown = control.switcher().eased();
    let output = painter.physical(monitor.geometry().to_f64());
    painter.frost(
        control.frost_id().clone(),
        output,
        shown as f32,
        (shown * DIM) as f32,
    );

    painter.stage(monitor);
}
