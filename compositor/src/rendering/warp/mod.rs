//! Windows bent out of shape on the GPU: poured through a neck into their icon,
//! crumbled into dust that falls away, stretched into a drop of liquid glass
//! that carries their icon, or frosted over while they change size.
//!
//! A mesh can only bend one texture, so the decorated window is first painted
//! into a canvas of its own, then drawn through a raw GL program that builds
//! its geometry from `gl_VertexID`. A renderer without the programs gets
//! `None` and the caller draws the flat morph instead.

mod canvas;
mod carried;
mod dust;
mod element;
mod genie;
mod liquid;

pub use canvas::WarpCanvases;
pub use element::WarpElement;

use smithay::{
    backend::renderer::gles::GlesRenderer,
    utils::{Logical, Physical, Point, Rectangle, Scale},
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
    Liquid(&'a Morph),
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

/// A window's content to draw frosted over its resize.
pub struct ResizeBlurRequest<'a> {
    pub tile: &'a Tile,
    /// Where the content lands, output-local.
    pub rect: Rectangle<i32, Physical>,
    /// Physical pixels.
    pub radius: f32,
    pub alpha: f32,
    pub scale: Scale<f64>,
}

/// `request`'s content, painted at the size the client drew it and frosted
/// over the rect it is resizing through. `None` when the programs are missing
/// or the GPU refused the canvas.
pub fn resize_blur(
    gles: &mut GlesRenderer,
    canvases: &mut WarpCanvases,
    request: ResizeBlurRequest<'_>,
) -> Option<WarpElement> {
    let shaders = WarpShaders::get(gles)?;
    let canvas = canvases.paint_content(gles, request.tile, request.scale, request.rect)?;
    Some(WarpElement::resize_blur(
        shaders,
        canvas,
        request.tile.resize_blur().strength(),
        request.radius,
        request.alpha,
    ))
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
        Motion::Liquid(morph) => {
            let scale = request.scale;
            let rest = request.tile.render_rect();
            let window = Rectangle::new(rest.loc + request.offset, rest.size);
            let radius = f64::from(request.radius) / scale.x;
            let uniforms = liquid::uniforms(morph, window, radius, canvas.rect, scale);
            let icon = morph
                .origin()
                .carrier
                .as_ref()
                .filter(|_| morph.shows_icon())
                .and_then(|icon| canvases.carried.copy_of(gles, icon, scale));
            WarpElement::liquid(shaders, canvas, icon, uniforms)
        }
    })
}
