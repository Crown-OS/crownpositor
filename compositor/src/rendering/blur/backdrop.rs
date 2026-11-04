//! One rectangle of blurred glass, blurred out of the live framebuffer.

use std::rc::Rc;

use smithay::{
    backend::renderer::{
        Texture as _,
        element::{Element, Id, Kind, RenderElement, UnderlyingStorage},
        gles::{GlesError, GlesFrame, GlesRenderer, GlesTexture, Uniform, ffi},
        utils::{CommitCounter, DamageSet, OpaqueRegions},
    },
    utils::{
        Buffer as BufferCoords, Physical, Rectangle, Scale, Size, Transform, user_data::UserDataMap,
    },
};

use crate::{
    rendering::{
        blur::{
            BlurConfig, Glass,
            cache::BlurSession,
            content::ContentDamage,
            lifted::{GlassHandoff, LiftedGlass},
            scene::{BlurScene, grow},
            stack::GlassStack,
        },
        decorate::Backdrop,
        framebuffer::FramebufferSpace,
        gles::GlesBacked,
    },
    shaders::{
        blur::{BlurShaders, KawaseProgram},
        lock_glyph::Frost,
    },
    utils::region,
};

/// One rectangle of the blurred glass behind a surface.
///
/// Holds no pixels of its own. The blur happens in `draw`, out of the output's
/// shared scene — everything below this rectangle in the element list, and no
/// glass at all, so pieces that overlap read one layer of blur between them
/// rather than each blurring the one under it.
#[derive(Debug)]
pub struct BlurBackdrop {
    id: Id,
    commit: CommitCounter,
    geometry: Rectangle<i32, Physical>,
    mask: Rectangle<i32, Physical>,
    radius: f32,
    glass: Glass,
    alpha: f32,
    config: BlurConfig,
    /// Where opaque glass already stands in front of this one, output-local.
    /// Painting there would leave a blur for that glass to blur again, so this
    /// backdrop gives those pixels up; what the piece above then lifts out of
    /// the frame is the surface's own contents, unblurred.
    occluders: Vec<Rectangle<i32, Physical>>,
    scene: Rc<BlurScene>,
    content: ContentDamage,
    shaders: BlurShaders,
    /// The surface laid on this glass, when it is drawn there rather than
    /// here — see [`lifting_for`](Self::lifting_for).
    handoff: Option<GlassHandoff>,
}

impl BlurBackdrop {
    /// `None` when nothing of this backdrop is on screen, or when the pipeline
    /// is unavailable on this GPU.
    ///
    /// The *mask* is left whole, so the corners still round against the window
    /// rather than against the piece of it that survived the output's edge.
    pub fn new(
        renderer: &mut GlesRenderer,
        session: &mut BlurSession<'_>,
        params: Backdrop,
    ) -> Option<Self> {
        Self::stacked(renderer, session, params, GlassStack::occlude)
    }

    /// Glass that never hides what is behind it from the glass built after
    /// it: a shape cut finer than its rectangle, like a glyph, covers too
    /// little of it to claim any.
    pub(super) fn see_through(
        renderer: &mut GlesRenderer,
        session: &mut BlurSession<'_>,
        params: Backdrop,
    ) -> Option<Self> {
        Self::stacked(renderer, session, params, |_, _, _| Vec::new())
    }

    fn stacked(
        renderer: &mut GlesRenderer,
        session: &mut BlurSession<'_>,
        params: Backdrop,
        occlude: impl FnOnce(
            &mut GlassStack,
            &Backdrop,
            Rectangle<i32, Physical>,
        ) -> Vec<Rectangle<i32, Physical>>,
    ) -> Option<Self> {
        let shaders = BlurShaders::get(renderer)?;
        let geometry = params.geometry.intersection(session.output)?;

        // The pyramid is the output's, so it is always allocated to the full
        // configured depth; this backdrop simply uses the first few levels of
        // it. Nothing left to blur means nothing to draw.
        let (passes, offset) = session.config.taper(params.strength);
        if passes == 0 {
            return None;
        }
        let config = BlurConfig {
            passes: passes as u8,
            offset,
            ..session.config
        };

        let size = session.transform.transform_size(session.output.size);
        let scene = session
            .cache
            .scene(
                renderer,
                Size::<i32, BufferCoords>::from((size.w, size.h)),
                session.config.passes(),
            )
            .inspect_err(|err| tracing::warn!(%err, "failed to allocate a blur pyramid"))
            .ok()?;
        scene.stand(geometry);
        let occluders = occlude(session.cache.stack(), &params, geometry);
        let content = session.cache.register(&params.id);

        Some(Self {
            id: params.id,
            commit: params.commit,
            geometry,
            mask: params.mask,
            radius: params.radius,
            glass: params.glass,
            alpha: params.alpha,
            config,
            occluders,
            scene,
            content,
            shaders,
            handoff: None,
        })
    }

    /// The backdrop of a surface laid on its own glass: it lifts and blurs
    /// what is under it as it always does, in the same place in the frame and
    /// owing the same damage, and hands the result to that surface instead of
    /// drawing it.
    pub fn lifting_for(self, handoff: GlassHandoff) -> Self {
        Self {
            handoff: Some(handoff),
            ..self
        }
    }

    fn draw_gles(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        let Some(top) = self.scene.levels.first() else {
            return Ok(());
        };
        let visible = self.visible(dst, damage);
        let damage = visible.as_deref().unwrap_or(damage);
        let projection = *frame.projection();
        let Some(uniforms) =
            frame.with_context(|gl| unsafe { self.blur(gl, projection, dst, damage) })?
        else {
            return Ok(());
        };

        if let Some(handoff) = &self.handoff {
            handoff.hand_over(LiftedGlass {
                texture: top.tex_id(),
                uniforms,
            });
            return Ok(());
        }

        frame.render_texture_from_to(
            top,
            src,
            dst,
            damage,
            opaque_regions,
            Transform::Normal,
            self.alpha,
            Some(&self.shaders.finish),
            &uniforms,
        )
    }

    /// Where opaque glass in front stands over this one, output-local. This
    /// backdrop gives those pixels up, and so does a surface laid on it.
    pub fn occluders(&self) -> &[Rectangle<i32, Physical>] {
        &self.occluders
    }

    /// Lifts and blurs what lies under `dst` without drawing it, for glass of
    /// another shape that samples the pyramid's top level itself. `None` when
    /// there is nothing to sample.
    pub(super) fn prepare(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
    ) -> Result<Option<(ffi::types::GLuint, Frost)>, GlesError> {
        let Some(top) = self.scene.levels.first() else {
            return Ok(None);
        };
        let projection = *frame.projection();
        let blurred = frame
            .with_context(|gl| unsafe { self.blur(gl, projection, dst, damage) })?
            .is_some();
        let scene = self.scene.scene.size();
        Ok(blurred.then(|| {
            let frost = Frost {
                scene: [scene.w as f32, scene.h as f32],
                half_pixel: half_pixel(top.size(), scene),
                offset: self.config.offset.max(0.0),
            };
            (top.tex_id(), frost)
        }))
    }

    /// `damage` without the parts an opaque piece of glass in front of this
    /// one has taken over, element-local. `None` when nothing is in front,
    /// which is every backdrop on a frame with no overlapping glass.
    fn visible(
        &self,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
    ) -> Option<Vec<Rectangle<i32, Physical>>> {
        (!self.occluders.is_empty()).then(|| {
            region::subtract(
                damage.iter().copied(),
                self.occluders.iter().map(|rect| local(*rect, dst)),
            )
        })
    }

    /// How far a changed pixel of the scene reaches into this glass: the whole
    /// pyramid's spread, plus the refraction at the rim, which moves the taps
    /// that far inwards.
    fn reach(&self) -> i32 {
        self.config.radius() + self.glass.rim.max(0.0).ceil() as i32
    }

    /// Lifts the part of the frame this backdrop is about to cover into the
    /// output's scene copy and runs the pyramid over the dirty footprint,
    /// leaving the last upsample to the draw that follows. `None` skips the
    /// backdrop entirely.
    ///
    /// # Safety
    ///
    /// `gl` must belong to the frame's context. Every state this changes is
    /// restored before it returns.
    unsafe fn blur(
        &self,
        gl: &ffi::Gles2,
        projection: [f32; 9],
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
    ) -> Option<[Uniform<'static>; 15]> {
        let (mut viewport, mut previous) = ([0; 4], 0);
        unsafe {
            gl.GetIntegerv(ffi::VIEWPORT, viewport.as_mut_ptr());
            gl.GetIntegerv(ffi::DRAW_FRAMEBUFFER_BINDING, &mut previous);
        }

        let space = FramebufferSpace::new(projection, Size::from((viewport[2], viewport[3])));
        let size = self.scene.scene.size();
        let frame = Rectangle::from_size(Size::<i32, Physical>::from((size.w, size.h)));
        if frame.size != space.viewport() {
            return None;
        }
        // Only the levels this backdrop's strength reaches: a blur on its way
        // in runs a shallower pyramid than a settled one, out of the same
        // textures.
        let levels = self.scene.levels.get(..self.config.passes())?;
        let top = levels.first()?;
        let radius = self.reach();

        // The scene spans the whole framebuffer, so damage maps into it with no
        // per-backdrop origin to subtract: one rectangle serves as both the
        // blit's source and its destination.
        let dirty: Vec<_> = damage
            .iter()
            .filter_map(|rect| {
                space
                    .rect(Rectangle::new(rect.loc + dst.loc, rect.size))
                    .intersection(frame)
            })
            .collect();
        // Each damaged rectangle and the reach of the taps around it, kept
        // apart where they are apart: a bounding box would fill in the hole a
        // piece of glass above leaves in this one, and blur it twice.
        let footprint = region::coalesce(
            dirty
                .iter()
                .filter_map(|rect| grow(*rect, radius).intersection(frame)),
        );
        let refresh = self
            .scene
            .refresh(&dirty, &footprint, |rect| space.rect(rect));
        // From here on these rectangles are glass, whether or not the frame
        // repaints any of them, so nothing drawn after may blur these pixels
        // back out of the framebuffer. Not the ones given up to the glass in
        // front, though: the piece above is about to blur exactly those.
        self.scene.cover(
            region::subtract([dst], self.occluders.iter().copied())
                .into_iter()
                .filter_map(|rect| space.rect(rect).intersection(frame)),
        );

        if footprint.is_empty() {
            return None;
        }
        let offset = self.config.offset.max(0.0);

        unsafe {
            gl.BindFramebuffer(ffi::READ_FRAMEBUFFER, previous as ffi::types::GLuint);
            gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, self.shaders.framebuffer);
            gl.FramebufferTexture2D(
                ffi::DRAW_FRAMEBUFFER,
                ffi::COLOR_ATTACHMENT0,
                ffi::TEXTURE_2D,
                self.scene.scene.tex_id(),
                0,
            );
            gl.Disable(ffi::SCISSOR_TEST);
            for rect in &refresh {
                let (x, y, w, h) = (rect.loc.x, rect.loc.y, rect.size.w, rect.size.h);
                gl.BlitFramebuffer(
                    x,
                    y,
                    x + w,
                    y + h,
                    x,
                    y,
                    x + w,
                    y + h,
                    ffi::COLOR_BUFFER_BIT,
                    ffi::NEAREST,
                );
            }

            gl.BindFramebuffer(ffi::FRAMEBUFFER, self.shaders.framebuffer);
            gl.Enable(ffi::SCISSOR_TEST);
            gl.Disable(ffi::BLEND);

            let (down, up) = (&self.shaders.down, &self.shaders.up);
            let mut source = &self.scene.scene;
            for (index, level) in levels.iter().enumerate() {
                pass(gl, down, source, level, &footprint, index, offset);
                source = level;
            }
            for index in (0..levels.len().saturating_sub(1)).rev() {
                let source = &levels[index + 1];
                let level = &levels[index];
                pass(gl, up, source, level, &footprint, index, offset);
            }

            gl.BindFramebuffer(ffi::FRAMEBUFFER, previous as ffi::types::GLuint);
            gl.Viewport(viewport[0], viewport[1], viewport[2], viewport[3]);
            gl.Scissor(viewport[0], viewport[1], viewport[2], viewport[3]);
            gl.Enable(ffi::BLEND);
            gl.BindTexture(ffi::TEXTURE_2D, 0);
        }

        // This draw is the pyramid's last upsample, so it reads the tap
        // spacing of the level it samples exactly as the passes above do.
        Some(BlurShaders::finish_uniforms(
            size,
            space.rect(self.mask),
            half_pixel(top.size(), size),
            offset,
            self.radius,
            self.config.noise,
            self.glass,
            // Where the light lies, in the framebuffer's own axes: the one
            // place a rotated or flipped output has to be accounted for.
            space.direction(self.glass.shading.light),
        ))
    }
}

/// One pyramid pass: attach `destination`, then draw each piece of the
/// footprint at its level. `index` is the level `destination` sits at.
///
/// # Safety
///
/// The scratch framebuffer must be bound and the scissor test enabled.
unsafe fn pass(
    gl: &ffi::Gles2,
    program: &KawaseProgram,
    source: &GlesTexture,
    destination: &GlesTexture,
    footprint: &[Rectangle<i32, Physical>],
    index: usize,
    offset: f32,
) {
    let size = destination.size();
    unsafe {
        gl.FramebufferTexture2D(
            ffi::FRAMEBUFFER,
            ffi::COLOR_ATTACHMENT0,
            ffi::TEXTURE_2D,
            destination.tex_id(),
            0,
        );
        gl.Viewport(0, 0, size.w, size.h);
        program.bind(
            gl,
            source,
            (size.w as f32, size.h as f32),
            half_pixel(source.size(), size),
            offset,
        );
        for piece in footprint {
            let area = level_area(*piece, index as u32 + 1, size);
            gl.Scissor(area.loc.x, area.loc.y, area.size.w, area.size.h);
            KawaseProgram::draw(gl);
        }
    }
}

/// `footprint`, expressed in the pixels of a level `shift` halvings down.
fn level_area(
    footprint: Rectangle<i32, Physical>,
    shift: u32,
    size: Size<i32, BufferCoords>,
) -> Rectangle<i32, Physical> {
    let round_up = |value: i32| (value + (1 << shift) - 1) >> shift;
    Rectangle::from_extremities(
        (
            (footprint.loc.x >> shift).min(size.w),
            (footprint.loc.y >> shift).min(size.h),
        ),
        (
            round_up(footprint.loc.x + footprint.size.w).min(size.w),
            round_up(footprint.loc.y + footprint.size.h).min(size.h),
        ),
    )
}

/// An output-local rectangle in the element-local space `draw` speaks.
fn local(
    rect: Rectangle<i32, Physical>,
    dst: Rectangle<i32, Physical>,
) -> Rectangle<i32, Physical> {
    Rectangle::new(rect.loc - dst.loc, rect.size)
}

/// The dual-filter tap spacing: half a texel of the smaller of the two
/// textures a pass involves, in UV space — the destination going down, the
/// source coming back up.
fn half_pixel(source: Size<i32, BufferCoords>, destination: Size<i32, BufferCoords>) -> (f32, f32) {
    let spacing = |source: i32, destination: i32| 0.5 / source.min(destination).max(1) as f32;
    (
        spacing(source.w, destination.w),
        spacing(source.h, destination.h),
    )
}

impl Element for BlurBackdrop {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    /// The whole shared level, because that is what the backdrop samples: the
    /// finish shader addresses it by `gl_FragCoord` rather than through the
    /// vertex stage, so what it reads is never the piece this rectangle maps
    /// to.
    fn src(&self) -> Rectangle<f64, BufferCoords> {
        self.scene
            .levels
            .first()
            .map(|level| Rectangle::from_size(level.size().to_f64()))
            .unwrap_or_default()
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.geometry
    }

    fn transform(&self) -> Transform {
        Transform::Normal
    }

    /// Whatever changed beneath this glass, grown by the blur's reach: a
    /// blurred pixel depends on every pixel within reach of it, so a change
    /// the tracker already repaints spreads a band further into the glass
    /// than the rectangle it arrived in. Claimed now, from the content tracker
    /// that ran before the frame, rather than owed to the next one. The commit
    /// only moves when the blur settings or the client's region do.
    fn damage_since(
        &self,
        _scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        let whole = || DamageSet::from_slice(&[Rectangle::from_size(self.geometry.size)]);
        if commit != Some(self.commit) {
            return whole();
        }
        match self.content.changed_near(self.geometry, self.reach()) {
            Some(bands) => DamageSet::from_slice(&bands),
            None => whole(),
        }
    }

    /// Empty, although the glass draws opaque: a backdrop *reads* the frame
    /// beneath it, so everything under it has to be drawn for it to lift. Were
    /// it opaque, the tracker would skip the wallpaper under it and the lift
    /// would find the glass's own last output there — and blur it into itself,
    /// frame after frame. The glass in front of other glass is still spared by
    /// [`GlassStack`], which cuts the overlap out of what the lower piece draws.
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

impl<R: GlesBacked> RenderElement<R> for BlurBackdrop {
    fn draw(
        &self,
        frame: &mut R::Frame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&UserDataMap>,
    ) -> Result<(), R::Error> {
        self.draw_gles(R::gles_frame(frame), src, dst, damage, opaque_regions)
            .map_err(R::lift)
    }

    fn underlying_storage(&self, _renderer: &mut R) -> Option<UnderlyingStorage<'_>> {
        // Never a plane candidate: the pixels only exist through the shader.
        None
    }
}
