//! The lock's background flooding out over the desktop: a piece of it drawn
//! through the flood program under an id of its own. The program measures
//! the water against the output from `gl_FragCoord`, so it is bound per draw
//! like [`Rounded`](super::rounded::Rounded)'s, once the framebuffer is known.

use smithay::{
    backend::renderer::{
        element::{Element, Id, Kind, RenderElement, UnderlyingStorage},
        gles::GlesTexProgram,
        utils::{CommitCounter, DamageSet, OpaqueRegions},
    },
    utils::{
        Buffer as BufferCoords, Physical, Point, Rectangle, Scale, Transform,
        user_data::UserDataMap,
    },
};

use crate::{
    rendering::{decorate::Flood, framebuffer::FramebufferSpace, gles::GlesBacked},
    shaders::flood::FloodShader,
};

#[derive(Debug)]
pub struct Flooded<E> {
    inner: E,
    program: GlesTexProgram,
    flood: Flood,
}

impl<E> Flooded<E> {
    pub fn new(inner: E, program: GlesTexProgram, flood: Flood) -> Self {
        Self {
            inner,
            program,
            flood,
        }
    }
}

impl<E: Element> Element for Flooded<E> {
    fn id(&self) -> &Id {
        &self.flood.id
    }

    fn current_commit(&self) -> CommitCounter {
        let [_, down] = self.flood.source;
        CommitCounter::from((self.flood.level.to_bits() ^ down.to_bits().rotate_left(16)) as usize)
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

    /// Whole whenever the water moved, which is every frame it floods.
    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        if commit == Some(self.current_commit()) {
            return DamageSet::default();
        }
        DamageSet::from_slice(&[Rectangle::from_size(self.geometry(scale).size)])
    }

    /// None: the desktop shows past the water's front.
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

impl<R: GlesBacked, E: RenderElement<R>> RenderElement<R> for Flooded<E> {
    fn draw(
        &self,
        frame: &mut R::Frame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), R::Error> {
        let gles = R::gles_frame(frame);
        let bound = FramebufferSpace::current(gles).map(|space| {
            let uniforms = FloodShader::uniform_values(&space, &self.flood);
            gles.override_default_tex_program(self.program.clone(), uniforms.to_vec());
        });
        let result = self
            .inner
            .draw(frame, src, dst, damage, opaque_regions, cache);
        if bound.is_ok() {
            R::gles_frame(frame).clear_tex_program_override();
        }
        result
    }

    /// Withheld, or a plane would scan the whole picture out and skip the
    /// water's front.
    fn underlying_storage(&self, _renderer: &mut R) -> Option<UnderlyingStorage<'_>> {
        None
    }
}
