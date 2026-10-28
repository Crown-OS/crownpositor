//! A lock-screen clock glyph of frosted glass: the output's blur, cut to the
//! glyph's shape by its sprite.

use std::hash::{Hash, Hasher};

use smithay::{
    backend::renderer::{
        element::{Element, Id, Kind, RenderElement, UnderlyingStorage},
        gles::{GlesError, GlesFrame, GlesRenderer, GlesTexProgram, ffi},
        multigpu::{Error as MultiError, MultiFrame},
        utils::{CommitCounter, DamageSet, OpaqueRegions},
    },
    utils::{
        Buffer as BufferCoords, Physical, Point, Rectangle, Scale, Transform,
        user_data::UserDataMap,
    },
};

use crate::{
    backend::render::{GbmGlesApi, KmsRenderer},
    rendering::{
        blur::{BlurBackdrop, Glass, cache::BlurSession},
        decorate::{Backdrop, GlyphDraw},
    },
    shaders::lock_glyph::{LockGlyphShader, SCENE_UNIT},
};

#[derive(Debug)]
pub struct GlassGlyph<S> {
    inner: S,
    /// Lifts and blurs what lies under the glyph; draws nothing itself.
    backdrop: BlurBackdrop,
    program: GlesTexProgram,
    glyph: GlyphDraw,
    commit: CommitCounter,
}

impl<S: Element> GlassGlyph<S> {
    /// `None` when there is no blur to show through, and the glyph is better
    /// drawn some other way.
    pub fn new(
        renderer: &mut GlesRenderer,
        session: &mut BlurSession<'_>,
        inner: S,
        glyph: GlyphDraw,
    ) -> Option<Self> {
        let glass = glyph.glass.as_ref()?;
        let program = LockGlyphShader::get(renderer)?.glass;
        let geometry = glass.geometry;
        let mut hasher = std::hash::DefaultHasher::new();
        inner.id().hash(&mut hasher);
        glyph.fingerprint().hash(&mut hasher);
        let commit = CommitCounter::from(hasher.finish() as usize);
        let backdrop = BlurBackdrop::see_through(
            renderer,
            session,
            Backdrop {
                id: glass.id.clone(),
                commit,
                geometry,
                mask: geometry,
                radius: 0.0,
                glass: Glass::default(),
                alpha: inner.alpha(),
                strength: 1.0,
            },
        )?;
        Some(Self {
            inner,
            backdrop,
            program,
            glyph,
            commit,
        })
    }
}

impl<S> GlassGlyph<S> {
    /// Blurs what is under the glyph and binds it beside the sprite, for the
    /// draw that follows. `false` when there is nothing to show through.
    fn bind(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
    ) -> Result<bool, GlesError> {
        let Some((texture, frost)) = self.backdrop.prepare(frame, dst, damage)? else {
            return Ok(false);
        };
        frame.with_context(|gl| unsafe { bind_scene(gl, texture) })?;
        let mut uniforms = LockGlyphShader::uniform_values(&self.glyph);
        uniforms.extend(LockGlyphShader::glass_values(&self.glyph, frost));
        frame.override_default_tex_program(self.program.clone(), uniforms);
        Ok(true)
    }

    fn unbind(frame: &mut GlesFrame<'_, '_>) -> Result<(), GlesError> {
        frame.clear_tex_program_override();
        frame.with_context(|gl| unsafe { bind_scene(gl, 0) })
    }
}

/// Binds `texture` on the unit glass glyphs read the scene from, filtered,
/// and leaves unit 0 active for the renderer.
///
/// # Safety
///
/// `gl` must belong to the frame's context.
unsafe fn bind_scene(gl: &ffi::Gles2, texture: ffi::types::GLuint) {
    unsafe {
        gl.ActiveTexture(ffi::TEXTURE0 + SCENE_UNIT as u32);
        gl.BindTexture(ffi::TEXTURE_2D, texture);
        if texture != 0 {
            let linear = ffi::LINEAR as i32;
            gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MIN_FILTER, linear);
            gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MAG_FILTER, linear);
        }
        gl.ActiveTexture(ffi::TEXTURE0);
    }
}

impl<S: Element> Element for GlassGlyph<S> {
    /// The glass id, steady for the sprite's life, so the blur's halos and
    /// its content tracking know this glyph for glass.
    fn id(&self) -> &Id {
        self.backdrop.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn src(&self) -> Rectangle<f64, BufferCoords> {
        self.inner.src()
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.inner.geometry(scale)
    }

    fn location(&self, scale: Scale<f64>) -> Point<i32, Physical> {
        self.inner.location(scale)
    }

    fn transform(&self) -> Transform {
        self.inner.transform()
    }

    /// The whole glyph when it changed, and otherwise the band its blur
    /// still owes.
    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        self.backdrop.damage_since(scale, commit)
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }
}

impl<S: RenderElement<GlesRenderer>> RenderElement<GlesRenderer> for GlassGlyph<S> {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        if !self.bind(frame, dst, damage)? {
            return Ok(());
        }
        let result = self
            .inner
            .draw(frame, src, dst, damage, opaque_regions, cache);
        Self::unbind(frame)?;
        result
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

impl<'render, S> RenderElement<KmsRenderer<'render>> for GlassGlyph<S>
where
    S: RenderElement<KmsRenderer<'render>>,
{
    fn draw(
        &self,
        frame: &mut MultiFrame<'render, 'render, '_, '_, GbmGlesApi, GbmGlesApi>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), MultiError<GbmGlesApi, GbmGlesApi>> {
        if !self
            .bind(frame.as_mut(), dst, damage)
            .map_err(MultiError::Render)?
        {
            return Ok(());
        }
        let result = self
            .inner
            .draw(frame, src, dst, damage, opaque_regions, cache);
        Self::unbind(frame.as_mut()).map_err(MultiError::Render)?;
        result
    }

    fn underlying_storage(
        &self,
        _renderer: &mut KmsRenderer<'render>,
    ) -> Option<UnderlyingStorage<'_>> {
        None
    }
}
