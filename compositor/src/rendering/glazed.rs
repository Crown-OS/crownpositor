//! A surface laid on its own glass and cut with it once (`composited_blur`).
//!
//! Drawn the old way, the glass is cut by the shape's edge and the client's
//! antialiased body is drawn over it, so an edge pixel is covered once by each
//! and mixes body, glass and desktop in the wrong proportions. A client that
//! speaks `composited_blur` paints its body square instead, and this draws its
//! pixels over the glass and cuts the two together, so the edge is antialiased
//! exactly once.
//!
//! The glass is lifted by the surface's [`BlurBackdrop`], which keeps its own
//! place further back in the list — under the shadow the surface casts — and
//! hands its blur here instead of drawing it. Everything that marks the glass
//! as changed is that backdrop's damage, so this element damages as the
//! surface it wraps does. With nothing handed over this frame, the surface is
//! cut to the shape with nothing under it, which is what its client drew it
//! for — and so it is where opaque glass in front took its glass over, as the
//! backdrop gave those pixels up.
//!
//! [`BlurBackdrop`]: crate::rendering::blur::BlurBackdrop

use std::borrow::Cow;

use smithay::{
    backend::renderer::{
        element::{Element, Id, Kind, RenderElement, UnderlyingStorage},
        gles::{GlesError, GlesFrame, GlesTexProgram, Uniform},
        utils::{CommitCounter, DamageSet, OpaqueRegions},
    },
    utils::{
        Buffer as BufferCoords, Physical, Point, Rectangle, Scale, Transform,
        user_data::UserDataMap,
    },
};

use crate::{
    rendering::{
        blur::{GlassHandoff, LiftedGlass, bind_scene},
        decorate::Outline,
        framebuffer::FramebufferSpace,
        gles::GlesBacked,
        opaque,
    },
    shaders::{glazed::GlazedShader, rounded_corner::RoundedCornerShader},
    utils::region,
};

/// The window a surface is clipped to: what a
/// [`Rounded`](crate::rendering::rounded::Rounded) would cut it to.
#[derive(Debug, Clone, Copy)]
pub struct Clip {
    /// Output-local.
    pub shape: Rectangle<i32, Physical>,
    /// One per corner, (+x, +y), (+x, -y), (-x, +y), (-x, -y).
    pub radius: [f32; 4],
    /// The clip changed shape since the last frame: the whole element is
    /// repainted.
    pub reshaped: bool,
}

/// Some of a draw's damage, and the glass under it if any.
type Part<'a> = (Cow<'a, [Rectangle<i32, Physical>]>, Option<&'a LiftedGlass>);

#[derive(Debug)]
pub struct Glazed<E> {
    inner: E,
    program: GlesTexProgram,
    clip: Clip,
    /// What the surface and its glass are cut by, output-local.
    outline: Outline,
    handoff: GlassHandoff,
    /// Where opaque glass in front took this glass over, output-local.
    occluders: Vec<Rectangle<i32, Physical>>,
}

impl<E> Glazed<E> {
    pub fn new(
        inner: E,
        program: GlesTexProgram,
        clip: Clip,
        outline: Outline,
        handoff: GlassHandoff,
        occluders: Vec<Rectangle<i32, Physical>>,
    ) -> Self {
        Self {
            inner,
            program,
            clip,
            outline,
            handoff,
            occluders,
        }
    }

    /// `damage`, element-local, split by what lies under it: the glass the
    /// backdrop handed over, or none where it did not blur — nowhere this
    /// frame, or under opaque glass in front.
    fn parts<'a>(
        &self,
        dst: Rectangle<i32, Physical>,
        damage: &'a [Rectangle<i32, Physical>],
        lifted: Option<&'a LiftedGlass>,
    ) -> [Part<'a>; 2] {
        if lifted.is_none() || self.occluders.is_empty() {
            return [(Cow::Borrowed(damage), lifted), (Cow::Borrowed(&[]), None)];
        }
        let occluded: Vec<_> = self
            .occluders
            .iter()
            .map(|rect| Rectangle::new(rect.loc - dst.loc, rect.size))
            .collect();
        [
            (
                Cow::Owned(region::subtract(
                    damage.iter().copied(),
                    occluded.iter().copied(),
                )),
                lifted,
            ),
            (Cow::Owned(region::intersect(damage, occluded)), None),
        ]
    }

    /// Binds the program, and `lifted` when there is glass to lay the surface
    /// on, for drawing `src` into `dst`. Returns whether the glass needs
    /// unbinding; with no framebuffer to measure in, nothing is bound and the
    /// surface is drawn as it is.
    fn bind(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        transform: Transform,
        lifted: Option<&LiftedGlass>,
    ) -> Result<bool, GlesError> {
        let Ok(space) = FramebufferSpace::current(frame) else {
            return Ok(false);
        };
        let mut uniforms: Vec<Uniform<'static>> = RoundedCornerShader::uniform_values(
            space.rect(self.clip.shape),
            space.corners(self.clip.radius),
            RoundedCornerShader::resample(src, dst, transform),
        )
        .into();
        uniforms.extend(GlazedShader::scene_values(lifted.is_some()));
        match lifted {
            Some(lifted) => {
                uniforms.extend(lifted.uniforms.iter().cloned());
                frame.with_context(|gl| unsafe { bind_scene(gl, lifted.texture) })?;
            }
            None => uniforms.extend(GlazedShader::cut_values(
                space.rect(self.outline.rect),
                self.outline.radius,
            )),
        }
        frame.override_default_tex_program(self.program.clone(), uniforms);
        Ok(lifted.is_some())
    }

    fn unbind(frame: &mut GlesFrame<'_, '_>, scene: bool) -> Result<(), GlesError> {
        frame.clear_tex_program_override();
        if scene {
            frame.with_context(|gl| unsafe { bind_scene(gl, 0) })?;
        }
        Ok(())
    }
}

impl<E: Element> Element for Glazed<E> {
    fn id(&self) -> &Id {
        self.inner.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
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

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        if self.clip.reshaped {
            return DamageSet::from_slice(&[Rectangle::from_size(self.geometry(scale).size)]);
        }
        self.inner.damage_since(scale, commit)
    }

    /// What the client declared opaque, less the corners the clip cuts from
    /// the surface and the corners the outline cuts from it on its glass.
    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        let location = self.geometry(scale).loc;
        let clipped = opaque::within(
            &self.inner.opaque_regions(scale),
            self.clip.shape,
            self.clip.radius,
            location,
        );
        let cut = opaque::corners(self.outline.rect, self.outline.radius)
            .into_iter()
            .map(|corner| Rectangle::new(corner.loc - location, corner.size));
        region::subtract(clipped.iter().copied(), cut)
            .into_iter()
            .collect()
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }

    fn kind(&self) -> Kind {
        self.inner.kind()
    }
}

impl<R: GlesBacked, E: RenderElement<R>> RenderElement<R> for Glazed<E> {
    fn draw(
        &self,
        frame: &mut R::Frame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), R::Error> {
        // The override and the scene live on the GLES frame; the surface still
        // draws through the renderer's own frame, so a buffer from another GPU
        // is copied across as usual.
        let lifted = self.handoff.take();
        for (damage, lifted) in self.parts(dst, damage, lifted.as_ref()) {
            if damage.is_empty() {
                continue;
            }
            let scene = self
                .bind(
                    R::gles_frame(frame),
                    src,
                    dst,
                    self.inner.transform(),
                    lifted,
                )
                .map_err(R::lift)?;
            let result = self
                .inner
                .draw(frame, src, dst, &damage, opaque_regions, cache);
            Self::unbind(R::gles_frame(frame), scene).map_err(R::lift)?;
            result?;
        }
        Ok(())
    }

    fn underlying_storage(&self, _renderer: &mut R) -> Option<UnderlyingStorage<'_>> {
        // Scanning the buffer out directly would skip the glass and the cut.
        None
    }
}
