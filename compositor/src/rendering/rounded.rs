//! Rounded corners, and the area filter for a buffer drawn at another size.
//!
//! Every client surface the decorator sees goes through this one shader — at
//! zero radius when nothing is to be rounded — so a client that rendered for
//! the wrong scale is resampled the same way wherever it is drawn.
//!
//! The shader is a per-*draw* override on the GLES frame, not a property of an
//! element, so this wraps an element and brackets its draw call: bind the
//! program, draw, unbind. Without the unbind every element after it in the list
//! would be drawn with rounded corners too.

use smithay::{
    backend::renderer::{
        element::{
            Element, Id, Kind, RenderElement, UnderlyingStorage,
            memory::MemoryRenderBufferRenderElement,
        },
        gles::{GlesFrame, GlesTexProgram},
        utils::{CommitCounter, DamageSet, OpaqueRegions},
    },
    utils::{
        Buffer as BufferCoords, Physical, Point, Rectangle, Scale, Transform,
        user_data::UserDataMap,
    },
};

use crate::{
    color::pipeline::SurfaceTransform,
    rendering::{
        blur::{BlurBackdrop, BlurSession, Glass, GlassGlyph, GlassHandoff, GlassKind},
        color::ColorConversions,
        corners::CornerMemory,
        decorate::{Backdrop, Cropped, Glaze, Glazing, GlyphDraw, Shadow, TileDecorator},
        decoration::{Border, GlassShadow, TitleBar, TitleBarParams, WindowDecoration},
        framebuffer::FramebufferSpace,
        glazed::{Clip, Glazed},
        gles::GlesBacked,
        opaque,
        shaded_sprite::ShadedSprite,
        snapshot::SnapshotElement,
        warp::{self, WarpCanvases, WarpElement, WarpRequest},
    },
    shaders::{
        color::ColorConvertShader, glazed::GlazedShader, lock_glyph::LockGlyphShader,
        rounded_corner::RoundedCornerShader, sprite_blur::SpriteBlurShader,
    },
};

#[derive(Debug)]
pub struct Rounded<E> {
    inner: E,
    /// Resolved up front: `GlesFrame` does not expose its renderer, so the
    /// program cannot be looked up from inside `draw`. `None` means the shader
    /// never compiled — the element then draws square, because square corners
    /// beat a dropped window.
    program: Option<GlesTexProgram>,
    /// The window the corners are cut from, output-local. Not this element's
    /// own rect: a subsurface or a cropped buffer covers only part of it.
    shape: Rectangle<i32, Physical>,
    /// One radius per corner, so a decorated window can keep its top two square
    /// where the titlebar covers them.
    radius: [f32; 4],
    /// The last frame clipped this element to a different shape, so pixels the
    /// client never damaged changed: the whole element is repainted.
    reshaped: bool,
    /// Set when `program` is the colour-converting variant, which this
    /// surface's image description needs to reach the output's encoding.
    color: Option<SurfaceTransform>,
}

impl<E> Rounded<E> {
    pub fn new(
        inner: E,
        program: Option<GlesTexProgram>,
        shape: Rectangle<i32, Physical>,
        radius: [f32; 4],
        reshaped: bool,
    ) -> Self {
        Self {
            inner,
            program,
            shape,
            radius,
            reshaped,
            color: None,
        }
    }

    fn converting(mut self, program: GlesTexProgram, transform: SurfaceTransform) -> Self {
        self.program = Some(program);
        self.color = Some(transform);
        self
    }

    /// Binds the override for drawing `src` into `dst` on `frame` and returns
    /// whether it needs unbinding. The shape goes to the shader in framebuffer
    /// pixels, where it measures from `gl_FragCoord`.
    fn bind(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        transform: Transform,
    ) -> bool {
        let Some(program) = &self.program else {
            return false;
        };
        let Ok(space) = FramebufferSpace::current(frame) else {
            return false;
        };
        let clip = RoundedCornerShader::uniform_values(
            space.rect(self.shape),
            space.corners(self.radius),
            RoundedCornerShader::resample(src, dst, transform),
        );
        let uniforms = match self.color {
            Some(color) => clip
                .into_iter()
                .chain(ColorConvertShader::uniform_values(color))
                .collect(),
            None => clip.to_vec(),
        };
        frame.override_default_tex_program(program.clone(), uniforms);
        true
    }

    fn is_square(&self) -> bool {
        self.radius.iter().all(|radius| *radius <= 0.0)
    }
}

impl<E: Element> Element for Rounded<E> {
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
        if self.reshaped {
            return DamageSet::from_slice(&[Rectangle::from_size(self.geometry(scale).size)]);
        }
        self.inner.damage_since(scale, commit)
    }

    /// What the client declared opaque, less the corners the clip cuts away:
    /// an opaque window still hides everything under its middle, which is
    /// what spares the wallpaper, the glass behind its frame and the windows
    /// below from being drawn there.
    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        let regions = self.inner.opaque_regions(scale);
        if self.is_square() {
            return regions;
        }
        opaque::within(&regions, self.shape, self.radius, self.geometry(scale).loc)
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }

    fn kind(&self) -> Kind {
        self.inner.kind()
    }
}

impl<R: GlesBacked, E: RenderElement<R>> RenderElement<R> for Rounded<E> {
    fn draw(
        &self,
        frame: &mut R::Frame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), R::Error> {
        // The override lives on the GLES frame; the inner element still draws
        // through the renderer's own frame so cross-GPU copies keep working.
        let bound = self.bind(R::gles_frame(frame), src, dst, self.inner.transform());
        let result = self
            .inner
            .draw(frame, src, dst, damage, opaque_regions, cache);
        if bound {
            R::gles_frame(frame).clear_tex_program_override();
        }
        result
    }

    fn underlying_storage(&self, _renderer: &mut R) -> Option<UnderlyingStorage<'_>> {
        // Withheld on purpose: handing this to a DRM plane would scan the buffer
        // out directly and skip the shader, so the corners would come back and
        // a described surface would reach the screen unconverted.
        None
    }
}

/// A compositor-rasterised sprite as the decorator shades it.
pub type Sprite<R> = MemoryRenderBufferRenderElement<R>;

smithay::backend::renderer::element::render_elements! {
    /// What a decorated tile can be: the window itself (rounded), the blurred
    /// glass behind it, or any of the compositor's own shader-drawn pieces.
    pub Decorated<R> where R: GlesBacked;
    Window = Rounded<Cropped<R>>,
    Glazed = Glazed<Cropped<R>>,
    Snapshot = Rounded<SnapshotElement>,
    Backdrop = BlurBackdrop,
    TitleBar = TitleBar,
    Border = WindowDecoration,
    Shadow = GlassShadow,
    Warp = WarpElement,
    Sprite = ShadedSprite<Sprite<R>>,
    GlassGlyph = GlassGlyph<Sprite<R>>,
}

/// Rounds corners with the GLES texture-program override, and materialises
/// blur backdrops when the backend opened a blur session for this frame.
///
/// The programs are resolved at construction/decoration time, because
/// `GlesFrame` does not expose its renderer — so nothing can be looked up
/// from inside `draw`. One decorator serves every renderer built on GLES.
#[derive(Debug)]
pub struct GlesDecorator<'a> {
    blur: Option<BlurSession<'a>>,
    corners: &'a mut CornerMemory,
    canvases: &'a mut WarpCanvases,
    colors: ColorConversions,
}

impl<'a> GlesDecorator<'a> {
    pub fn new(
        blur: Option<BlurSession<'a>>,
        corners: &'a mut CornerMemory,
        canvases: &'a mut WarpCanvases,
    ) -> Self {
        Self {
            blur,
            corners,
            canvases,
            colors: ColorConversions::default(),
        }
    }
}

impl<R: GlesBacked> TileDecorator<R> for GlesDecorator<'_> {
    type Element = Decorated<R>;

    fn decorate(
        &mut self,
        renderer: &mut R,
        element: Cropped<R>,
        shape: Rectangle<i32, Physical>,
        radius: [f32; 4],
    ) -> Option<Self::Element> {
        // A missing program (shader never compiled) squares the corners, and
        // a missing colour program leaves the colours unconverted; neither may
        // drop the window.
        let conversion = self.colors.get(element.id());
        let description = conversion.map_or(0, |conversion| conversion.description);
        let reshaped = self
            .corners
            .reshaped(element.id(), shape.size, radius, description);
        let program = RoundedCornerShader::get(renderer.gles());
        let rounded = Rounded::new(element, program, shape, radius, reshaped);
        let rounded = match conversion.zip(ColorConvertShader::get(renderer.gles())) {
            Some((conversion, program)) => rounded.converting(program, conversion.transform),
            None => rounded,
        };
        Some(Decorated::Window(rounded))
    }

    fn glaze(
        &mut self,
        renderer: &mut R,
        element: Cropped<R>,
        shape: Rectangle<i32, Physical>,
        radius: [f32; 4],
        glaze: Glaze,
    ) -> Glazing<Self::Element> {
        // The glazed program cannot convert colour, so a described surface is
        // decorated instead and its glass drawn under it the old way.
        let glazed =
            GlazedShader::get(renderer.gles()).filter(|_| self.colors.get(element.id()).is_none());
        let Some(program) = glazed else {
            return Glazing::Decorated(self.decorate(renderer, element, shape, radius));
        };
        let clip = Clip {
            shape,
            radius,
            reshaped: self.corners.reshaped(element.id(), shape.size, radius, 0),
        };
        let handoff = GlassHandoff::default();
        let lift = glaze
            .backdrop
            .zip(self.blur.as_mut())
            .and_then(|(backdrop, session)| BlurBackdrop::new(renderer.gles(), session, backdrop))
            .map(|backdrop| backdrop.lifting_for(handoff.clone()));
        let occluders = lift
            .as_ref()
            .map_or_else(Vec::new, |lift| lift.occluders().to_vec());
        Glazing::Glazed {
            surface: Decorated::Glazed(Glazed::new(
                element,
                program,
                clip,
                glaze.outline,
                handoff,
                occluders,
            )),
            lift: lift.map(Decorated::Backdrop),
        }
    }

    fn convert_colors(&mut self, conversions: ColorConversions) {
        self.colors = conversions;
    }

    fn converts_color(&self, id: &Id) -> bool {
        self.colors.get(id).is_some()
    }

    fn blur_strength_for(&self, shrink: f64) -> f32 {
        self.blur
            .as_ref()
            .map_or(1.0, |blur| blur.config.strength_for(shrink))
    }

    fn blur_strength_for_sigma(&self, sigma: f32) -> f32 {
        self.blur
            .as_ref()
            .map_or(1.0, |blur| blur.config.strength_for_sigma(sigma))
    }

    fn blur_fingerprint(&self) -> Option<u64> {
        self.blur.as_ref().map(|blur| blur.config.fingerprint())
    }

    fn glass(&self, scale: f64, kind: GlassKind) -> Glass {
        self.blur
            .as_ref()
            .map_or_else(Glass::default, |blur| blur.config.glass(scale, kind))
    }

    fn snapshot(
        &mut self,
        renderer: &mut R,
        element: SnapshotElement,
        shape: Rectangle<i32, Physical>,
        radius: [f32; 4],
    ) -> Option<Self::Element> {
        let program = RoundedCornerShader::get(renderer.gles());
        let reshaped = self.corners.reshaped(element.id(), shape.size, radius, 0);
        Some(Decorated::Snapshot(Rounded::new(
            element, program, shape, radius, reshaped,
        )))
    }

    fn backdrop(&mut self, renderer: &mut R, backdrop: Backdrop) -> Option<Self::Element> {
        let session = self.blur.as_mut()?;
        BlurBackdrop::new(renderer.gles(), session, backdrop).map(Decorated::Backdrop)
    }

    fn shadow(&mut self, renderer: &mut R, shadow: Shadow) -> Option<Self::Element> {
        GlassShadow::new(renderer.gles(), shadow).map(Decorated::Shadow)
    }

    fn title_bar(&mut self, renderer: &mut R, params: TitleBarParams) -> Option<Self::Element> {
        TitleBar::new(renderer.gles(), params).map(Decorated::TitleBar)
    }

    fn border(&mut self, renderer: &mut R, border: Border) -> Option<Self::Element> {
        WindowDecoration::new(renderer.gles(), border).map(Decorated::Border)
    }

    fn warp(&mut self, renderer: &mut R, request: WarpRequest<'_, '_>) -> Option<Self::Element> {
        warp::warp(renderer.gles(), self.canvases, request).map(Decorated::Warp)
    }

    fn blurred_sprite(
        &mut self,
        renderer: &mut R,
        sprite: Sprite<R>,
        radius: f32,
    ) -> Option<Self::Element> {
        let program = SpriteBlurShader::get(renderer.gles())?;
        let size = sprite.src().size;
        let uniforms = SpriteBlurShader::uniform_values(radius, [size.w, size.h]).to_vec();
        Some(Decorated::Sprite(ShadedSprite::new(
            sprite,
            program,
            uniforms,
            u64::from(radius.to_bits()),
        )))
    }

    fn lock_glyph(
        &mut self,
        renderer: &mut R,
        sprite: Sprite<R>,
        glyph: GlyphDraw,
    ) -> Option<Self::Element> {
        if let Some(session) = self.blur.as_mut()
            && glyph.glass.is_some()
        {
            return GlassGlyph::new(renderer.gles(), session, sprite, glyph)
                .map(Decorated::GlassGlyph);
        }
        let program = LockGlyphShader::get(renderer.gles())?.plain;
        let uniforms = LockGlyphShader::uniform_values(&glyph);
        Some(Decorated::Sprite(ShadedSprite::new(
            sprite,
            program,
            uniforms,
            glyph.fingerprint(),
        )))
    }
}
