//! A warped canvas in the scene graph.

use glam::{Mat3, Vec2};
use smithay::{
    backend::renderer::{
        Texture,
        element::{Element, Id, Kind, RenderElement, UnderlyingStorage},
        gles::{GlesError, GlesFrame, GlesTexture, ffi},
        utils::{CommitCounter, OpaqueRegions},
    },
    utils::{Buffer as BufferCoords, Physical, Point, Rectangle, Scale, user_data::UserDataMap},
};

use crate::{
    rendering::{
        framebuffer::FramebufferSpace,
        gles::GlesBacked,
        warp::{canvas::Painted, dust, genie, liquid, swing},
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
    /// Turned clockwise by `angle` radians about `pivot`, output-local.
    Swing {
        pivot: Point<f64, Physical>,
        angle: f64,
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

    pub(super) fn swing(
        shaders: WarpShaders,
        canvas: Painted,
        pivot: Point<f64, Physical>,
        angle: f64,
    ) -> Self {
        Self {
            shaders,
            geometry: swing::bounds(canvas.rect, pivot, angle),
            bend: Bend::Swing { pivot, angle },
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
        if let Bend::Swing { pivot, angle } = self.bend {
            let shift = (dst.loc - self.geometry.loc).to_f64();
            return self.draw_turned(frame, pivot + shift, angle, shift, &scissors);
        }
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
                Bend::Swing { .. } => {}
            }
        })
    }

    /// The canvas as a quad turned about `pivot`, through smithay's own
    /// texture program, clipped to each of `scissors` in turn.
    fn draw_turned(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        pivot: Point<f64, Physical>,
        angle: f64,
        shift: Point<f64, Physical>,
        scissors: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        let vector = |point: Point<f64, Physical>| Vec2::new(point.x as f32, point.y as f32);
        let pivot = vector(pivot);
        let matrix = Mat3::from_translation(pivot)
            * Mat3::from_angle(angle as f32)
            * Mat3::from_translation(vector(self.canvas.rect.loc + shift) - pivot);
        let texture = &self.canvas.texture;
        let size = texture.size();
        let mut tex_matrix = Mat3::from_scale(Vec2::new(1.0 / size.w as f32, 1.0 / size.h as f32));
        if texture.is_y_inverted() {
            tex_matrix = Mat3::from_scale(Vec2::new(1.0, -1.0)) * tex_matrix;
        }
        let quad = [0.0, 0.0, size.w as f32, size.h as f32];

        let mut viewport = [0; 4];
        let scissored = frame.with_context(|gl| unsafe {
            gl.GetIntegerv(ffi::VIEWPORT, viewport.as_mut_ptr());
            gl.IsEnabled(ffi::SCISSOR_TEST) == ffi::TRUE
        })?;
        let drawn = scissors.iter().try_for_each(|rect| {
            frame.with_context(|gl| unsafe {
                gl.Enable(ffi::SCISSOR_TEST);
                gl.Scissor(rect.loc.x, rect.loc.y, rect.size.w, rect.size.h);
            })?;
            frame.render_texture(
                texture,
                tex_matrix,
                matrix,
                Some(quad),
                self.alpha,
                None,
                &[],
            )
        });
        frame.with_context(|gl| unsafe {
            gl.Scissor(viewport[0], viewport[1], viewport[2], viewport[3]);
            if !scissored {
                gl.Disable(ffi::SCISSOR_TEST);
            }
        })?;
        drawn
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
