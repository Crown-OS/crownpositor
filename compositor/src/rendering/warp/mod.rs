//! Windows bent out of shape on the GPU: poured through a neck into their icon,
//! or crumbled into dust that falls away.
//!
//! A mesh can only bend one texture, so the decorated window is first painted
//! into a canvas of its own, then drawn through a raw GL program that builds
//! its geometry from `gl_VertexID`. A renderer without the programs gets
//! `None` and the caller draws the flat morph instead.

mod canvas;
mod dust;
mod element;
mod genie;

pub use canvas::WarpCanvases;
pub use element::WarpElement;

use smithay::{
    backend::renderer::gles::GlesRenderer,
    utils::{Logical, Point, Scale},
};

use crate::{
    rendering::{FrameStyle, morph::Content},
    shaders::warp::WarpShaders,
    shell::{Shell, dust::Dust, morph::Morph, tile::Tile},
};

/// What carries the window this frame.
#[derive(Clone, Copy)]
pub enum Motion<'a> {
    Genie(&'a Morph),
    Dust(&'a Dust),
}

/// One window to draw warped, with everything painting its canvas takes.
pub struct WarpRequest<'a, 'style> {
    pub shell: &'a Shell,
    pub tile: &'a Tile,
    pub content: Content<'a>,
    pub motion: Motion<'a>,
    pub scale: Scale<f64>,
    pub offset: Point<f64, Logical>,
    /// Corner radius of the window's frame, physical pixels.
    pub radius: f32,
    pub style: &'a mut FrameStyle<'style>,
}

/// `request`'s window, painted and bent. `None` when the programs are missing
/// or the GPU refused the canvas.
pub fn warp(
    gles: &mut GlesRenderer,
    canvases: &mut WarpCanvases,
    mut request: WarpRequest<'_, '_>,
) -> Option<WarpElement> {
    let shaders = WarpShaders::get(gles)?;
    let canvas = canvases.paint(gles, &mut request)?;
    Some(match request.motion {
        Motion::Genie(morph) => {
            let origin = morph.origin();
            let icon = origin.rect.to_physical(request.scale);
            WarpElement::genie(
                shaders,
                canvas,
                icon,
                origin.edge,
                morph.progress(),
                morph.alpha(),
            )
        }
        Motion::Dust(dust) => WarpElement::dust(shaders, canvas, dust, request.scale.x),
    })
}
