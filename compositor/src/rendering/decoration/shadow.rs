//! The drop shadow a surface casts.
//!
//! One element per primitive of the shape a client set through
//! `crownos_background_effects.set_shadow`, sized to the silhouette plus the
//! room its gaussian needs to fall off in.
//!
//! Like [`WindowDecoration`] this is a pixel shader: a shadow has no buffer to
//! sample, it generates every pixel it draws. And like it, the element cannot
//! be generic over the renderer — a generic `R::Frame` offers no way to bind a
//! GLES program — so the two impls below name their renderers and the
//! multi-GPU one forwards to the GLES frame underneath it.
//!
//! [`WindowDecoration`]: crate::rendering::decoration::window::WindowDecoration

use smithay::{
    backend::renderer::{
        element::{Element, Id, Kind, RenderElement, UnderlyingStorage},
        gles::{GlesError, GlesFrame, GlesPixelProgram, GlesRenderer},
        multigpu::{Error as MultiError, MultiFrame},
        utils::{CommitCounter, DamageSet, OpaqueRegions},
    },
    utils::{Buffer as BufferCoords, Physical, Rectangle, Scale, Size, Transform},
};

use crate::{
    backend::render::{GbmGlesApi, KmsRenderer},
    rendering::{blur::ShadowPiece, decorate::Shadow},
    shaders::shadow::ShadowShader,
};

/// One blurred silhouette under a surface.
#[derive(Debug, Clone)]
pub struct GlassShadow {
    id: Id,
    commit: CommitCounter,
    /// `None` means the shader never compiled. The element then draws nothing
    /// rather than being dropped, so the damage tracker does not rescan the
    /// area for no gain.
    program: Option<GlesPixelProgram>,
    piece: ShadowPiece,
    alpha: f32,
}

impl GlassShadow {
    /// `None` when there would be nothing to see: no silhouette, or a fully
    /// transparent colour. Both are ordinary states — a client turns its shadow
    /// off by setting either to zero — so they cost no element rather than an
    /// invisible one.
    pub fn new(renderer: &GlesRenderer, shadow: Shadow) -> Option<Self> {
        if shadow.alpha <= 0.0 || shadow.piece.color[3] <= 0.0 || shadow.piece.geometry.is_empty() {
            return None;
        }

        Some(Self {
            id: shadow.id,
            commit: shadow.commit,
            program: ShadowShader::get(renderer),
            piece: shadow.piece,
            alpha: shadow.alpha,
        })
    }

    /// The element's own area, which is what `v_coords` has to span: the shader
    /// reads it as a pixel position, and a `src` smaller than the element would
    /// silently rescale the whole distance field.
    fn source_rect(&self) -> Rectangle<f64, BufferCoords> {
        Rectangle::from_size(Size::from((
            self.piece.geometry.size.w as f64,
            self.piece.geometry.size.h as f64,
        )))
    }

    fn draw_gles(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        let Some(program) = self.program.as_ref() else {
            return Ok(());
        };

        let geometry = self.piece.geometry;
        let size: Size<i32, BufferCoords> = Size::from((geometry.size.w, geometry.size.h));
        // The silhouette in the element's own space, which is where the shader
        // measures it from.
        let shape = Rectangle::new(self.piece.shape.loc - geometry.loc, self.piece.shape.size);

        frame.render_pixel_shader_to(
            program,
            src,
            dst,
            size,
            Some(damage),
            self.alpha,
            &ShadowShader::values(self.piece.color, shape, self.piece.radius, self.piece.sigma),
        )
    }
}

impl Element for GlassShadow {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn src(&self) -> Rectangle<f64, BufferCoords> {
        self.source_rect()
    }

    /// Already physical: the rect was built against the same scale the caller
    /// placed the surface at, so there is nothing left to convert.
    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.piece.geometry
    }

    fn transform(&self) -> Transform {
        Transform::Normal
    }

    fn damage_since(
        &self,
        _scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        // Generated wholesale from its uniforms, so any change to them repaints
        // all of it; moving and resizing are tracked separately.
        if commit == Some(self.commit) {
            DamageSet::default()
        } else {
            DamageSet::from_slice(&[Rectangle::from_size(self.piece.geometry.size)])
        }
    }

    /// Empty. A shadow is translucent everywhere, by definition.
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

impl RenderElement<GlesRenderer> for GlassShadow {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        self.draw_gles(frame, src, dst, damage)
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        // Never a plane candidate: the pixels only exist through the shader.
        None
    }
}

impl<'render> RenderElement<KmsRenderer<'render>> for GlassShadow {
    fn draw(
        &self,
        frame: &mut MultiFrame<'render, 'render, '_, '_, GbmGlesApi, GbmGlesApi>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), MultiError<GbmGlesApi, GbmGlesApi>> {
        // The program lives on the render device's GLES context — the same one
        // `frame.as_mut()` exposes — so this never crosses GPUs.
        self.draw_gles(frame.as_mut(), src, dst, damage)
            .map_err(MultiError::Render)
    }

    fn underlying_storage(
        &self,
        _renderer: &mut KmsRenderer<'render>,
    ) -> Option<UnderlyingStorage<'_>> {
        None
    }
}
