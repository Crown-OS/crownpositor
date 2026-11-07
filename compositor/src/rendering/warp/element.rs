//! A warped canvas in the scene graph.

use smithay::{
    backend::renderer::{
        element::{Element, Id, Kind, RenderElement, UnderlyingStorage},
        gles::{GlesError, GlesFrame, GlesTexture},
        utils::{CommitCounter, OpaqueRegions},
    },
    utils::{Buffer as BufferCoords, Physical, Rectangle, Scale, user_data::UserDataMap},
};

use crate::{
    rendering::{
        framebuffer::FramebufferSpace,
        gles::GlesBacked,
        warp::{canvas::Painted, dust, genie, liquid},
    },
    shaders::warp::{DustUniforms, GenieUniforms, LiquidUniforms, WarpDraw, WarpShaders},
    shell::{dust::Dust, morph::Edge},
};

#[derive(Debug, Clone)]
enum Bend {
    Genie(GenieUniforms),
    Dust(DustUniforms),
    Liquid {
        uniforms: LiquidUniforms,
        icon: Option<GlesTexture>,
    },
    ResizeBlur {
        strength: f32,
        radius: f32,
    },
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

    pub(super) fn liquid(
        shaders: WarpShaders,
        canvas: Painted,
        icon: Option<GlesTexture>,
        uniforms: LiquidUniforms,
    ) -> Self {
        Self {
            shaders,
            geometry: liquid::bounds(&uniforms),
            bend: Bend::Liquid { uniforms, icon },
            canvas,
            alpha: 1.0,
        }
    }

    pub(super) fn resize_blur(
        shaders: WarpShaders,
        canvas: Painted,
        strength: f32,
        radius: f32,
        alpha: f32,
    ) -> Self {
        Self {
            shaders,
            geometry: canvas.rect.to_i32_round(),
            bend: Bend::ResizeBlur { strength, radius },
            canvas,
            alpha,
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
                Bend::Liquid { uniforms, icon } => {
                    self.shaders.liquid.draw(gl, &draw, icon.as_ref(), uniforms)
                }
                Bend::ResizeBlur { strength, radius } => {
                    self.shaders.resize_blur.draw(gl, &draw, *strength, *radius)
                }
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

impl<R: GlesBacked> RenderElement<R> for WarpElement {
    fn draw(
        &self,
        frame: &mut R::Frame<'_, '_>,
        _src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&UserDataMap>,
    ) -> Result<(), R::Error> {
        // Painted on the GLES context every renderer here draws through, so
        // the canvas never crosses GPUs.
        self.draw_gles(R::gles_frame(frame), dst, damage)
            .map_err(R::lift)
    }

    fn underlying_storage(&self, _renderer: &mut R) -> Option<UnderlyingStorage<'_>> {
        None
    }
}
