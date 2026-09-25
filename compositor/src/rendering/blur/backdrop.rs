//! One rectangle of blurred glass, blurred out of the live framebuffer.

use std::{cell::RefCell, mem, rc::Rc};

use smithay::{
    backend::renderer::{
        Texture as _,
        element::{Element, Id, Kind, RenderElement, UnderlyingStorage},
        gles::{GlesError, GlesFrame, GlesRenderer, GlesTexture, Uniform, ffi},
        multigpu::{Error as MultiError, MultiFrame},
        utils::{CommitCounter, DamageSet, OpaqueRegions},
    },
    utils::{
        Buffer as BufferCoords, Physical, Point, Rectangle, Scale, Size, Transform,
        user_data::UserDataMap,
    },
};

use crate::{
    backend::render::{GbmGlesApi, KmsRenderer},
    rendering::{
        blur::{
            BlurConfig, Glass,
            cache::{BlurSession, Halo},
            scene::{BlurScene, grow},
        },
        decorate::Backdrop,
    },
    shaders::blur::{BlurShaders, KawaseProgram},
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
    halo: Rc<RefCell<Halo>>,
    shaders: BlurShaders,
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
        let occluders = session.cache.stack().occlude(&params, geometry);
        let halo = session.cache.halo(&params.id);

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
            halo,
            shaders,
        })
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

        self.owe_halo(dst, damage);

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

    /// `damage` without the parts an opaque piece of glass in front of this
    /// one has taken over, element-local. `None` when nothing is in front,
    /// which is every backdrop on a frame with no overlapping glass.
    fn visible(
        &self,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
    ) -> Option<Vec<Rectangle<i32, Physical>>> {
        (!self.occluders.is_empty()).then(|| {
            Rectangle::subtract_rects_many(
                damage.iter().copied(),
                self.occluders.iter().map(|rect| local(*rect, dst)),
            )
        })
    }

    /// Records the band this draw leaves stale: a changed pixel spreads
    /// `radius` beyond the rectangle it arrived in, and the tracker has already
    /// closed this frame's damage, so the ring is offered at the start of the
    /// next one. Rectangles the tracker is repainting anyway are not owed
    /// twice.
    fn owe_halo(&self, dst: Rectangle<i32, Physical>, damage: &[Rectangle<i32, Physical>]) {
        let mut halo = self.halo.borrow_mut();
        let fresh =
            Rectangle::subtract_rects_many(damage.iter().copied(), mem::take(&mut halo.reported));

        let bounds = Rectangle::from_size(dst.size);
        let radius = self.config.radius();
        halo.pending = Rectangle::subtract_rects_many(
            fresh
                .iter()
                .filter_map(|rect| grow(*rect, radius).intersection(bounds)),
            damage.iter().copied(),
        );
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
    ) -> Option<[Uniform<'static>; 11]> {
        let (mut viewport, mut previous) = ([0; 4], 0);
        unsafe {
            gl.GetIntegerv(ffi::VIEWPORT, viewport.as_mut_ptr());
            gl.GetIntegerv(ffi::DRAW_FRAMEBUFFER_BINDING, &mut previous);
        }

        let space = FramebufferSpace {
            projection,
            viewport: Size::from((viewport[2], viewport[3])),
        };
        let size = self.scene.scene.size();
        let frame = Rectangle::from_size(Size::<i32, Physical>::from((size.w, size.h)));
        if frame.size != space.viewport {
            return None;
        }
        // Only the levels this backdrop's strength reaches: a blur on its way
        // in runs a shallower pyramid than a settled one, out of the same
        // textures.
        let levels = self.scene.levels.get(..self.config.passes())?;
        let top = levels.first()?;
        let radius = self.config.radius();

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
        let refresh = self.scene.refresh(&dirty, radius, frame);
        // From here on these rectangles are glass, whether or not the frame
        // repaints any of them, so nothing drawn after may blur these pixels
        // back out of the framebuffer. Not the ones given up to the glass in
        // front, though: the piece above is about to blur exactly those.
        self.scene.cover(
            Rectangle::subtract_rects_many([dst], self.occluders.iter().copied())
                .into_iter()
                .filter_map(|rect| space.rect(rect).intersection(frame)),
        );

        let footprint = dirty.into_iter().reduce(Rectangle::merge)?;
        let footprint = grow(footprint, radius).intersection(frame)?;
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
                pass(gl, down, source, level, footprint, index, offset);
                source = level;
            }
            for index in (0..levels.len().saturating_sub(1)).rev() {
                let source = &levels[index + 1];
                let level = &levels[index];
                pass(gl, up, source, level, footprint, index, offset);
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
            // Where the screen's upper left lies, in the framebuffer's own
            // axes. The material lights itself from there, and this is the one
            // place a rotated or flipped output has to be accounted for.
            space.direction((-1.0, -1.0)),
        ))
    }
}

/// Maps output-local physical coordinates to framebuffer pixels.
///
/// The frame's projection already carries the output transform, so this is the
/// single place a rotated or flipped output is dealt with: every rotation and
/// flip keeps rectangles axis-aligned, which is all the blit and the scissor
/// ask for.
struct FramebufferSpace {
    projection: [f32; 9],
    viewport: Size<i32, Physical>,
}

impl FramebufferSpace {
    fn point(&self, point: Point<i32, Physical>) -> Point<i32, Physical> {
        let matrix = &self.projection;
        let (x, y) = (point.x as f32, point.y as f32);
        let ndc = (
            matrix[0] * x + matrix[3] * y + matrix[6],
            matrix[1] * x + matrix[4] * y + matrix[7],
        );
        Point::from((
            ((ndc.0 + 1.0) * 0.5 * self.viewport.w as f32).round() as i32,
            ((ndc.1 + 1.0) * 0.5 * self.viewport.h as f32).round() as i32,
        ))
    }

    /// A direction in output-local coordinates, as a unit vector in
    /// framebuffer pixels. Only the projection's linear part is involved: a
    /// direction has no origin to translate.
    fn direction(&self, delta: (f32, f32)) -> (f32, f32) {
        let matrix = &self.projection;
        let x = (matrix[0] * delta.0 + matrix[3] * delta.1) * self.viewport.w as f32;
        let y = (matrix[1] * delta.0 + matrix[4] * delta.1) * self.viewport.h as f32;
        let length = x.hypot(y);
        if length > 0.0 {
            (x / length, y / length)
        } else {
            (0.0, 0.0)
        }
    }

    fn rect(&self, rect: Rectangle<i32, Physical>) -> Rectangle<i32, Physical> {
        let start = self.point(rect.loc);
        let end = self.point(rect.loc + rect.size.to_point());
        Rectangle::from_extremities(
            (start.x.min(end.x), start.y.min(end.y)),
            (start.x.max(end.x), start.y.max(end.y)),
        )
    }
}

/// One pyramid pass: attach `destination`, clip to the footprint at its level,
/// draw. `index` is the level `destination` sits at.
///
/// # Safety
///
/// The scratch framebuffer must be bound and the scissor test enabled.
unsafe fn pass(
    gl: &ffi::Gles2,
    program: &KawaseProgram,
    source: &GlesTexture,
    destination: &GlesTexture,
    footprint: Rectangle<i32, Physical>,
    index: usize,
    offset: f32,
) {
    let size = destination.size();
    let area = level_area(footprint, index as u32 + 1, size);
    unsafe {
        gl.FramebufferTexture2D(
            ffi::FRAMEBUFFER,
            ffi::COLOR_ATTACHMENT0,
            ffi::TEXTURE_2D,
            destination.tex_id(),
            0,
        );
        gl.Viewport(0, 0, size.w, size.h);
        gl.Scissor(area.loc.x, area.loc.y, area.size.w, area.size.h);
        program.run(
            gl,
            source,
            (size.w as f32, size.h as f32),
            half_pixel(source.size(), size),
            offset,
        );
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

    /// Almost nothing of its own: a backdrop's pixels follow whatever is drawn
    /// under it, which already damages this area, and the commit only moves
    /// when the blur settings or the client's region do. What it does claim is
    /// the band the last draw left stale around that damage.
    fn damage_since(
        &self,
        _scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        let mut halo = self.halo.borrow_mut();
        if commit != Some(self.commit) {
            halo.reported.clear();
            return DamageSet::from_slice(&[Rectangle::from_size(self.geometry.size)]);
        }

        DamageSet::from_slice(&halo.reported)
    }

    /// Empty even though the blur itself is opaque: the corners are cut away,
    /// and the backdrop fades with its window during animations.
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

impl RenderElement<GlesRenderer> for BlurBackdrop {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        self.draw_gles(frame, src, dst, damage, opaque_regions)
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        // Never a plane candidate: the pixels only exist through the shader.
        None
    }
}

impl<'render> RenderElement<KmsRenderer<'render>> for BlurBackdrop {
    fn draw(
        &self,
        frame: &mut MultiFrame<'render, 'render, '_, '_, GbmGlesApi, GbmGlesApi>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&UserDataMap>,
    ) -> Result<(), MultiError<GbmGlesApi, GbmGlesApi>> {
        // The pyramid lives on the render device's GLES context — the same one
        // `frame.as_mut()` exposes — so this never crosses GPUs.
        self.draw_gles(frame.as_mut(), src, dst, damage, opaque_regions)
            .map_err(MultiError::Render)
    }

    fn underlying_storage(
        &self,
        _renderer: &mut KmsRenderer<'render>,
    ) -> Option<UnderlyingStorage<'_>> {
        None
    }
}
