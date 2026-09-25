//! Rounded corners.
//!
//! The shader is a per-*draw* override on the GLES frame, not a property of an
//! element, so this wraps an element and brackets its draw call: bind the
//! program, draw, unbind. Without the unbind every element after it in the list
//! would be drawn with rounded corners too.
//!
//! Two renderers can drive the shader: the plain [`GlesRenderer`] (winit) and
//! the multi-GPU [`KmsRenderer`] (DRM). The multi renderer exposes its
//! underlying GLES renderer/frame through `AsMut`, which is all the override
//! needs — so both impls below are the same three lines around the same
//! program.

use smithay::{
    backend::renderer::{
        element::{Element, Id, Kind, RenderElement, UnderlyingStorage},
        gles::{GlesError, GlesFrame, GlesRenderer, GlesTexProgram},
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
        blur::{BlurBackdrop, BlurSession, Glass},
        decorate::{Backdrop, Cropped, Shadow, TileDecorator},
        decoration::{Border, GlassShadow, TitleBar, TitleBarParams, WindowDecoration},
    },
    shaders::rounded_corner::RoundedCornerShader,
};

#[derive(Debug)]
pub struct Rounded<E> {
    inner: E,
    /// Resolved up front: `GlesFrame` does not expose its renderer, so the
    /// program cannot be looked up from inside `draw`. `None` means the shader
    /// never compiled — the element then draws square, because square corners
    /// beat a dropped window.
    program: Option<GlesTexProgram>,
    /// The window's size in physical pixels; the shader needs it to know where
    /// the corners are.
    size: (f32, f32),
    /// One radius per corner, so a decorated window can keep its top two square
    /// where the titlebar covers them.
    radius: [f32; 4],
}

impl<E> Rounded<E> {
    pub fn new(
        inner: E,
        program: Option<GlesTexProgram>,
        size: (f32, f32),
        radius: [f32; 4],
    ) -> Self {
        Self {
            inner,
            program,
            size,
            radius,
        }
    }

    /// Binds the override on `frame` and returns whether it needs unbinding.
    fn bind(&self, frame: &mut GlesFrame<'_, '_>) -> bool {
        match &self.program {
            Some(program) => {
                frame.override_default_tex_program(
                    program.clone(),
                    RoundedCornerShader::uniform_values(self.size, self.radius).to_vec(),
                );
                true
            }
            None => false,
        }
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
        self.inner.damage_since(scale, commit)
    }

    /// Deliberately empty: the corners are cut away, so the element is no longer
    /// fully opaque and anything behind it has to be drawn.
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

impl<E: RenderElement<GlesRenderer>> RenderElement<GlesRenderer> for Rounded<E> {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        let bound = self.bind(frame);
        let result = self
            .inner
            .draw(frame, src, dst, damage, opaque_regions, cache);
        if bound {
            frame.clear_tex_program_override();
        }
        result
    }

    fn underlying_storage(&self, renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        // Withheld on purpose: handing this to a DRM plane would scan the buffer
        // out directly and skip the shader, so the corners would come back.
        let _ = renderer;
        None
    }
}

impl<'render, E> RenderElement<KmsRenderer<'render>> for Rounded<E>
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
        // The override lives on the GLES frame under the multi frame; the
        // inner element still draws through the multi frame so cross-GPU
        // copies keep working.
        let bound = self.bind(frame.as_mut());
        let result = self
            .inner
            .draw(frame, src, dst, damage, opaque_regions, cache);
        if bound {
            frame.as_mut().clear_tex_program_override();
        }
        result
    }

    fn underlying_storage(
        &self,
        renderer: &mut KmsRenderer<'render>,
    ) -> Option<UnderlyingStorage<'_>> {
        // Same as the GLES impl: no direct scanout for shaded windows.
        let _ = renderer;
        None
    }
}

/// What a decorated tile can be: the window itself (rounded), or the blurred
/// glass behind it. One enum rather than two `CrownElement` variants because
/// the `render_elements!` macro derives a `From` per variant, and two wrapped
/// generics would overlap.
#[derive(Debug)]
pub enum Decorated<E> {
    Window(Rounded<E>),
    Backdrop(BlurBackdrop),
    TitleBar(TitleBar),
    Border(WindowDecoration),
    Shadow(GlassShadow),
}

impl<E: Element> Element for Decorated<E> {
    fn id(&self) -> &Id {
        match self {
            Self::Window(window) => window.id(),
            Self::Backdrop(backdrop) => backdrop.id(),
            Self::TitleBar(bar) => bar.id(),
            Self::Border(border) => border.id(),
            Self::Shadow(shadow) => shadow.id(),
        }
    }

    fn current_commit(&self) -> CommitCounter {
        match self {
            Self::Window(window) => window.current_commit(),
            Self::Backdrop(backdrop) => backdrop.current_commit(),
            Self::TitleBar(bar) => bar.current_commit(),
            Self::Border(border) => border.current_commit(),
            Self::Shadow(shadow) => shadow.current_commit(),
        }
    }

    fn src(&self) -> Rectangle<f64, BufferCoords> {
        match self {
            Self::Window(window) => window.src(),
            Self::Backdrop(backdrop) => backdrop.src(),
            Self::TitleBar(bar) => bar.src(),
            Self::Border(border) => border.src(),
            Self::Shadow(shadow) => shadow.src(),
        }
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        match self {
            Self::Window(window) => window.geometry(scale),
            Self::Backdrop(backdrop) => backdrop.geometry(scale),
            Self::TitleBar(bar) => bar.geometry(scale),
            Self::Border(border) => border.geometry(scale),
            Self::Shadow(shadow) => shadow.geometry(scale),
        }
    }

    fn location(&self, scale: Scale<f64>) -> Point<i32, Physical> {
        match self {
            Self::Window(window) => window.location(scale),
            Self::Backdrop(backdrop) => backdrop.location(scale),
            Self::TitleBar(bar) => bar.location(scale),
            Self::Border(border) => border.location(scale),
            Self::Shadow(shadow) => shadow.location(scale),
        }
    }

    fn transform(&self) -> Transform {
        match self {
            Self::Window(window) => window.transform(),
            Self::Backdrop(backdrop) => backdrop.transform(),
            Self::TitleBar(bar) => bar.transform(),
            Self::Border(border) => border.transform(),
            Self::Shadow(shadow) => shadow.transform(),
        }
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        match self {
            Self::Window(window) => window.damage_since(scale, commit),
            Self::Backdrop(backdrop) => backdrop.damage_since(scale, commit),
            Self::TitleBar(bar) => bar.damage_since(scale, commit),
            Self::Border(border) => border.damage_since(scale, commit),
            Self::Shadow(shadow) => shadow.damage_since(scale, commit),
        }
    }

    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        match self {
            Self::Window(window) => window.opaque_regions(scale),
            Self::Backdrop(backdrop) => backdrop.opaque_regions(scale),
            Self::TitleBar(bar) => bar.opaque_regions(scale),
            Self::Border(border) => border.opaque_regions(scale),
            Self::Shadow(shadow) => shadow.opaque_regions(scale),
        }
    }

    fn alpha(&self) -> f32 {
        match self {
            Self::Window(window) => window.alpha(),
            Self::Backdrop(backdrop) => backdrop.alpha(),
            Self::TitleBar(bar) => bar.alpha(),
            Self::Border(border) => border.alpha(),
            Self::Shadow(shadow) => shadow.alpha(),
        }
    }

    fn kind(&self) -> Kind {
        match self {
            Self::Window(window) => window.kind(),
            Self::Backdrop(backdrop) => backdrop.kind(),
            Self::TitleBar(bar) => bar.kind(),
            Self::Border(border) => border.kind(),
            Self::Shadow(shadow) => shadow.kind(),
        }
    }
}

impl<E: RenderElement<GlesRenderer>> RenderElement<GlesRenderer> for Decorated<E> {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        match self {
            Self::Window(window) => window.draw(frame, src, dst, damage, opaque_regions, cache),
            Self::Backdrop(backdrop) => RenderElement::<GlesRenderer>::draw(
                backdrop,
                frame,
                src,
                dst,
                damage,
                opaque_regions,
                cache,
            ),
            Self::TitleBar(bar) => RenderElement::<GlesRenderer>::draw(
                bar,
                frame,
                src,
                dst,
                damage,
                opaque_regions,
                cache,
            ),
            Self::Border(border) => RenderElement::<GlesRenderer>::draw(
                border,
                frame,
                src,
                dst,
                damage,
                opaque_regions,
                cache,
            ),
            Self::Shadow(shadow) => RenderElement::<GlesRenderer>::draw(
                shadow,
                frame,
                src,
                dst,
                damage,
                opaque_regions,
                cache,
            ),
        }
    }

    fn underlying_storage(&self, renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        match self {
            Self::Window(window) => window.underlying_storage(renderer),
            Self::Backdrop(backdrop) => backdrop.underlying_storage(renderer),
            Self::TitleBar(bar) => RenderElement::<GlesRenderer>::underlying_storage(bar, renderer),
            Self::Border(border) => {
                RenderElement::<GlesRenderer>::underlying_storage(border, renderer)
            }
            Self::Shadow(shadow) => {
                RenderElement::<GlesRenderer>::underlying_storage(shadow, renderer)
            }
        }
    }
}

impl<'render, E> RenderElement<KmsRenderer<'render>> for Decorated<E>
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
        match self {
            Self::Window(window) => window.draw(frame, src, dst, damage, opaque_regions, cache),
            Self::Backdrop(backdrop) => RenderElement::<KmsRenderer<'render>>::draw(
                backdrop,
                frame,
                src,
                dst,
                damage,
                opaque_regions,
                cache,
            ),
            Self::TitleBar(bar) => RenderElement::<KmsRenderer<'render>>::draw(
                bar,
                frame,
                src,
                dst,
                damage,
                opaque_regions,
                cache,
            ),
            Self::Border(border) => RenderElement::<KmsRenderer<'render>>::draw(
                border,
                frame,
                src,
                dst,
                damage,
                opaque_regions,
                cache,
            ),
            Self::Shadow(shadow) => RenderElement::<KmsRenderer<'render>>::draw(
                shadow,
                frame,
                src,
                dst,
                damage,
                opaque_regions,
                cache,
            ),
        }
    }

    fn underlying_storage(
        &self,
        renderer: &mut KmsRenderer<'render>,
    ) -> Option<UnderlyingStorage<'_>> {
        match self {
            Self::Window(window) => window.underlying_storage(renderer),
            Self::Backdrop(backdrop) => backdrop.underlying_storage(renderer),
            Self::TitleBar(bar) => {
                RenderElement::<KmsRenderer<'render>>::underlying_storage(bar, renderer)
            }
            Self::Border(border) => {
                RenderElement::<KmsRenderer<'render>>::underlying_storage(border, renderer)
            }
            Self::Shadow(shadow) => {
                RenderElement::<KmsRenderer<'render>>::underlying_storage(shadow, renderer)
            }
        }
    }
}

/// Rounds corners with the GLES texture-program override, and materialises
/// blur backdrops when the backend opened a blur session for this frame.
///
/// The programs are resolved at construction/decoration time, because
/// `GlesFrame` does not expose its renderer — so nothing can be looked up
/// from inside `draw`.
#[derive(Debug)]
pub struct GlesDecorator<'a> {
    blur: Option<BlurSession<'a>>,
}

impl<'a> GlesDecorator<'a> {
    pub fn new(blur: Option<BlurSession<'a>>) -> Self {
        Self { blur }
    }
}

impl TileDecorator<GlesRenderer> for GlesDecorator<'_> {
    type Element = Decorated<Cropped<GlesRenderer>>;

    fn decorate(
        &mut self,
        renderer: &mut GlesRenderer,
        element: Cropped<GlesRenderer>,
        size: (f32, f32),
        radius: [f32; 4],
    ) -> Option<Self::Element> {
        // A missing program (shader never compiled) squares the corners; it
        // must not drop the window.
        let program = RoundedCornerShader::get(renderer);
        Some(Decorated::Window(Rounded::new(
            element, program, size, radius,
        )))
    }

    fn blur_fingerprint(&self) -> Option<u64> {
        self.blur.as_ref().map(|blur| blur.config.fingerprint())
    }

    fn glass(&self, scale: f64) -> Glass {
        self.blur
            .as_ref()
            .map_or_else(Glass::default, |blur| blur.config.glass(scale))
    }

    fn backdrop(
        &mut self,
        renderer: &mut GlesRenderer,
        backdrop: Backdrop,
    ) -> Option<Self::Element> {
        let session = self.blur.as_mut()?;
        BlurBackdrop::new(renderer, session, backdrop).map(Decorated::Backdrop)
    }

    fn shadow(&mut self, renderer: &mut GlesRenderer, shadow: Shadow) -> Option<Self::Element> {
        GlassShadow::new(renderer, shadow).map(Decorated::Shadow)
    }

    fn title_bar(
        &mut self,
        renderer: &mut GlesRenderer,
        params: TitleBarParams,
    ) -> Option<Self::Element> {
        TitleBar::new(renderer, params).map(Decorated::TitleBar)
    }

    fn border(&mut self, renderer: &mut GlesRenderer, border: Border) -> Option<Self::Element> {
        WindowDecoration::new(renderer, border).map(Decorated::Border)
    }
}

/// [`GlesDecorator`], but for the multi-GPU renderer the KMS backend uses.
#[derive(Debug)]
pub struct MultiDecorator<'a> {
    blur: Option<BlurSession<'a>>,
}

impl<'a> MultiDecorator<'a> {
    pub fn new(blur: Option<BlurSession<'a>>) -> Self {
        Self { blur }
    }
}

impl<'render> TileDecorator<KmsRenderer<'render>> for MultiDecorator<'_> {
    type Element = Decorated<Cropped<KmsRenderer<'render>>>;

    fn decorate(
        &mut self,
        renderer: &mut KmsRenderer<'render>,
        element: Cropped<KmsRenderer<'render>>,
        size: (f32, f32),
        radius: [f32; 4],
    ) -> Option<Self::Element> {
        let program = RoundedCornerShader::get(renderer.as_mut());
        Some(Decorated::Window(Rounded::new(
            element, program, size, radius,
        )))
    }

    fn blur_fingerprint(&self) -> Option<u64> {
        self.blur.as_ref().map(|blur| blur.config.fingerprint())
    }

    fn glass(&self, scale: f64) -> Glass {
        self.blur
            .as_ref()
            .map_or_else(Glass::default, |blur| blur.config.glass(scale))
    }

    fn backdrop(
        &mut self,
        renderer: &mut KmsRenderer<'render>,
        backdrop: Backdrop,
    ) -> Option<Self::Element> {
        let session = self.blur.as_mut()?;
        BlurBackdrop::new(renderer.as_mut(), session, backdrop).map(Decorated::Backdrop)
    }

    fn shadow(
        &mut self,
        renderer: &mut KmsRenderer<'render>,
        shadow: Shadow,
    ) -> Option<Self::Element> {
        GlassShadow::new(renderer.as_mut(), shadow).map(Decorated::Shadow)
    }

    fn title_bar(
        &mut self,
        renderer: &mut KmsRenderer<'render>,
        params: TitleBarParams,
    ) -> Option<Self::Element> {
        TitleBar::new(renderer.as_mut(), params).map(Decorated::TitleBar)
    }

    fn border(
        &mut self,
        renderer: &mut KmsRenderer<'render>,
        border: Border,
    ) -> Option<Self::Element> {
        WindowDecoration::new(renderer.as_mut(), border).map(Decorated::Border)
    }
}
