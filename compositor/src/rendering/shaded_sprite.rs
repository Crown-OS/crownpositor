//! A compositor-drawn sprite drawn through a program of its own: blurred as
//! a lock-screen layer comes and goes, or bent with the clock's warp. Like
//! [`Rounded`](super::rounded::Rounded), the program is a per-draw
//! texture-program override bracketing the inner draw.

use smithay::{
    backend::renderer::{
        element::{Element, Id, Kind, RenderElement, UnderlyingStorage},
        gles::{GlesError, GlesFrame, GlesRenderer, GlesTexProgram, Uniform},
        multigpu::{Error as MultiError, MultiFrame},
        utils::{CommitCounter, DamageSet, OpaqueRegions},
    },
    utils::{
        Buffer as BufferCoords, Physical, Point, Rectangle, Scale, Transform,
        user_data::UserDataMap,
    },
};

use crate::backend::render::{GbmGlesApi, KmsRenderer};

#[derive(Debug)]
pub struct ShadedSprite<E> {
    inner: E,
    program: GlesTexProgram,
    uniforms: Vec<Uniform<'static>>,
    /// Changes whenever the uniforms do: the sprite's own pixels change only
    /// with a new buffer, and so a new id.
    commit: CommitCounter,
}

impl<E: Element> ShadedSprite<E> {
    pub fn new(
        inner: E,
        program: GlesTexProgram,
        uniforms: Vec<Uniform<'static>>,
        fingerprint: u64,
    ) -> Self {
        Self {
            inner,
            program,
            uniforms,
            commit: CommitCounter::from(fingerprint as usize),
        }
    }

    fn bind(&self, frame: &mut GlesFrame<'_, '_>) {
        frame.override_default_tex_program(self.program.clone(), self.uniforms.clone());
    }
}

impl<E: Element> Element for ShadedSprite<E> {
    fn id(&self) -> &Id {
        self.inner.id()
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

    /// Whole whenever the uniforms changed: a blur's radius or a warp moves
    /// every pixel at once.
    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        if commit == Some(self.commit) {
            return DamageSet::default();
        }
        DamageSet::from_slice(&[Rectangle::from_size(self.geometry(scale).size)])
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }

    fn kind(&self) -> Kind {
        self.inner.kind()
    }
}

impl<E: RenderElement<GlesRenderer>> RenderElement<GlesRenderer> for ShadedSprite<E> {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        self.bind(frame);
        let result = self
            .inner
            .draw(frame, src, dst, damage, opaque_regions, cache);
        frame.clear_tex_program_override();
        result
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

impl<'render, E> RenderElement<KmsRenderer<'render>> for ShadedSprite<E>
where
    E: RenderElement<KmsRenderer<'render>>,
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
        self.bind(frame.as_mut());
        let result = self
            .inner
            .draw(frame, src, dst, damage, opaque_regions, cache);
        frame.as_mut().clear_tex_program_override();
        result
    }

    fn underlying_storage(
        &self,
        _renderer: &mut KmsRenderer<'render>,
    ) -> Option<UnderlyingStorage<'_>> {
        None
    }
}
