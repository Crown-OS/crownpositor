//! Drawing one output's overview.
//!
//! Everything is pushed here, front to back, in one order — and for the glass
//! that order is the design. Every piece of glass on an output blurs the same
//! copy of the scene, so nothing that is not glass may sit between two pieces:
//! a card above a separate dimming quad would lift the dimmed pixels into the
//! shared scene, and the wallpaper's own blur would read them back out around
//! the card on the next partial frame — a dark halo that comes and goes with
//! every hover. That is why the dim is the backdrop's tint rather than a quad.
//!
//! 1. The window the pointer carries.
//! 2. The workspace bar: each preview with its '×', windows, card and label,
//!    the dragged one first; then the '+' tile.
//! 3. The grid of windows flying to or from the desktop.
//! 4. The backdrop: the wallpaper blurred and dimmed, then the zoomed wallpaper.

mod backdrop;
mod bar;
mod thumbnail;

use smithay::{
    backend::renderer::{ImportAll, ImportMem, Renderer},
    utils::Scale,
};

use crate::{
    rendering::{Elements, FrameStyle, decorate::TileDecorator, painter::Painter},
    shell::monitor::Monitor,
};

/// Appends everything the overview draws on this output.
pub fn overview_elements<R, D>(
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
    painter.carried(monitor);
    painter.bar(monitor);
    painter.grid(monitor);
    painter.backdrop(monitor);
}
