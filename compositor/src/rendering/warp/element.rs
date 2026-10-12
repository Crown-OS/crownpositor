//! A warped canvas in the scene graph.

use smithay::{
    backend::renderer::{
        element::{Element, Id, Kind, RenderElement, UnderlyingStorage},
        gles::{GlesError, GlesFrame, GlesRenderer},
        multigpu::{Error as MultiError, MultiFrame},
        utils::{CommitCounter, OpaqueRegions},
    },
    utils::{Buffer as BufferCoords, Physical, Rectangle, Scale, user_data::UserDataMap},
};

use crate::{
    backend::render::{GbmGlesApi, KmsRenderer},
    rendering::{
        framebuffer::FramebufferSpace,
        warp::{canvas::Painted, dust, genie},
    },
    shaders::warp::{DustUniforms, GenieUniforms, WarpDraw, WarpShaders},
    shell::{dust::Dust, morph::Edge},
};

#[derive(Debug, Clone, Copy)]
enum Bend {
    Genie(GenieUniforms),
    Dust(DustUniforms),
}

#[derive(Debug)]
pub struct WarpElement {
    shaders: WarpShaders,
    canvas: Painted,
    bend: Bend,
    /// Everywhere the bent canvas can land, output-local.
    geometry: Rectangle<i32, Physical>,
    alpha: f32,
}

impl WarpElement {
    pub(super) fn genie(
        shaders: WarpShaders,
        canvas: Painted,
        icon: Rectangle<f64, Physical>,
        edge: Option<Edge>,
        progress: f64,
        alpha: f32,
    ) -> Self {
        Self {
            shaders,
            bend: Bend::Genie(genie::uniforms(canvas.rect, icon, edge, progress)),
            geometry: genie::bounds(canvas.rect, icon),
            canvas,
            alpha,
        }
    }

    pub(super) fn dust(shaders: WarpShaders, canvas: Painted, dust: &Dust, scale: f64) -> Self {
        Self {
            shaders,
            bend: Bend::Dust(dust::uniforms(canvas.rect, dust, scale)),
            geometry: dust::bounds(canvas.rect, scale),
            canvas,
            alpha: 1.0,
        }
    }

    fn draw_gles(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        let space = FramebufferSpace::current(frame)?;
        let scissors: Vec<_> = damage
            .iter()
            .map(|rect| space.rect(Rectangle::new(rect.loc + dst.loc, rect.size)))
            .collect();
        let draw = WarpDraw {
            texture: &self.canvas.texture,
            projection: *frame.projection(),
            window: [
                self.canvas.rect.loc.x as f32,
                self.canvas.rect.loc.y as f32,
                self.canvas.rect.size.w as f32,
                self.canvas.rect.size.h as f32,
            ],
            alpha: self.alpha,
            scissors: &scissors,
        };
        frame.with_context(|gl| unsafe {
            match &self.bend {
                Bend::Genie(uniforms) => self.shaders.genie.draw(gl, &draw, uniforms),
                Bend::Dust(uniforms) => self.shaders.dust.draw(gl, &draw, uniforms),
            }
        })
    }
}

impl Element for WarpElement {
    fn id(&self) -> &Id {
        &self.canvas.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.canvas.commit
    }

    fn src(&self) -> Rectangle<f64, BufferCoords> {
        Rectangle::from_size((self.geometry.size.w, self.geometry.size.h).into()).to_f64()
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.geometry
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        self.alpha
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }
}

impl RenderElement<GlesRenderer> for WarpElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        _src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        self.draw_gles(frame, dst, damage)
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

impl<'render> RenderElement<KmsRenderer<'render>> for WarpElement {
    fn draw(
        &self,
        frame: &mut MultiFrame<'render, 'render, '_, '_, GbmGlesApi, GbmGlesApi>,
        _src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&UserDataMap>,
    ) -> Result<(), MultiError<GbmGlesApi, GbmGlesApi>> {
        // Painted on the render device, the GLES context under the multi
        // frame, so the canvas never crosses GPUs.
        self.draw_gles(frame.as_mut(), dst, damage)
            .map_err(MultiError::Render)
    }

    fn underlying_storage(
        &self,
        _renderer: &mut KmsRenderer<'render>,
    ) -> Option<UnderlyingStorage<'_>> {
        None
    }
}
