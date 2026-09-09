//! The titlebar panel: tint, highlight and the three controls, in one pass.
//!
//! Everything visible here is generated from geometry by
//! [`TitleBarShader`](crate::shaders::title_bar::TitleBarShader) — there is no
//! buffer to sample. The blurred glass the panel sits on is a separate element
//! drawn underneath it, reusing the blur pipeline's own backdrop rather than
//! blurring anything a second time.
//!
//! Like the other shader elements this cannot be generic over the renderer: a
//! generic `R::Frame` offers only clears, fills and blits, with no way to bind
//! a GLES program, so the two impls below name their renderers and the
//! multi-GPU one forwards to the GLES frame underneath it.

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
    rendering::decoration::palette::FramePalette,
    shaders::title_bar::{TitleBarShader, TitleBarUniforms},
    shell::decoration::Control,
};

/// What a decorator needs to build one window's titlebar.
#[derive(Debug, Clone)]
pub struct TitleBarParams {
    /// Stable across frames for the same window, or the damage tracker treats
    /// every frame's titlebar as a brand new element and repaints it forever.
    pub id: Id,
    /// Changes exactly when the panel's pixels do — focus, hover or theme.
    /// Geometry changes are the damage tracker's own job and need no bump.
    pub commit: CommitCounter,
    /// The sheet to fill, in output-local physical coordinates: the whole
    /// window grown to the border's outer edge, so the tint reaches everything
    /// the client does not cover.
    pub geometry: Rectangle<i32, Physical>,
    /// The rect the corners are cut from, same space. Passed separately because
    /// a sheet clipped at a screen edge still has to round against the window.
    pub frame: Rectangle<i32, Physical>,
    /// The sheet's corner radius in physical pixels.
    pub radius: f32,
    /// Height of the titlebar band, down from the sheet's top edge.
    pub bar_height: f32,
    /// Centre of the leftmost control, physical, output-local.
    pub first_control: (f32, f32),
    pub control_pitch: f32,
    pub control_radius: f32,
    pub hovered: Option<Control>,
    pub palette: FramePalette,
    /// The window's animation alpha, so a frame fades in with its window.
    pub alpha: f32,
}

/// One window's titlebar.
#[derive(Debug, Clone)]
pub struct TitleBar {
    id: Id,
    commit: CommitCounter,
    /// `None` means the shader never compiled. The element then draws nothing
    /// — a missing panel is a cosmetic loss, and dropping the element instead
    /// would make the damage tracker rescan the area for no gain.
    program: Option<GlesPixelProgram>,
    geometry: Rectangle<i32, Physical>,
    uniforms: TitleBarUniforms,
    alpha: f32,
}

impl TitleBar {
    /// `None` when there would be nothing to see: a bar with no height or
    /// width, or a window faded out entirely. Both are ordinary states — a
    /// window mid-animation can round to an empty rect — so they cost no
    /// element rather than an invisible one.
    pub fn new(renderer: &GlesRenderer, params: TitleBarParams) -> Option<Self> {
        if params.alpha <= 0.0 || params.geometry.is_empty() {
            return None;
        }

        // The shader works in the element's own space, so the sheet is
        // expressed relative to the element's origin.
        let offset = params.frame.loc - params.geometry.loc;

        Some(Self {
            id: params.id,
            commit: params.commit,
            program: TitleBarShader::get(renderer),
            geometry: params.geometry,
            uniforms: TitleBarUniforms {
                window_offset: (offset.x as f32, offset.y as f32),
                window_size: (params.frame.size.w as f32, params.frame.size.h as f32),
                radius: params.radius.max(0.0),
                bar_height: params.bar_height.max(0.0),
                tint_top: params.palette.tint_top,
                tint_bottom: params.palette.tint_bottom,
                highlight: params.palette.highlight,
                control_origin: (
                    params.first_control.0 - params.geometry.loc.x as f32,
                    params.first_control.1 - params.geometry.loc.y as f32,
                ),
                control_spacing: params.control_pitch,
                control_radius: params.control_radius,
                control_fill: params.palette.control_fill,
                control_glyph: params.palette.control_glyph,
                // Outside `0..3` when nothing is hovered, which is what the
                // shader reads as "lift none of them".
                control_hover: params
                    .hovered
                    .map_or(-1.0, |control| control.index() as f32),
            },
            alpha: params.alpha,
        })
    }

    /// The bar's own area, which is what `v_coords` has to span: the shader
    /// reads it as a pixel position, and a `src` smaller than the element would
    /// silently rescale every distance field in it.
    fn source_rect(&self) -> Rectangle<f64, BufferCoords> {
        Rectangle::from_size(Size::from((
            self.geometry.size.w as f64,
            self.geometry.size.h as f64,
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

        let size: Size<i32, BufferCoords> =
            Size::from((self.geometry.size.w, self.geometry.size.h));

        frame.render_pixel_shader_to(
            program,
            src,
            dst,
            size,
            Some(damage),
            self.alpha,
            &TitleBarShader::values(&self.uniforms),
        )
    }
}

impl Element for TitleBar {
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
    /// placed the window at, so there is nothing left to convert.
    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.geometry
    }

    fn transform(&self) -> Transform {
        Transform::Normal
    }

    fn damage_since(
        &self,
        _scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        // The panel is generated wholesale from its uniforms, so any change to
        // them repaints all of it; moving and resizing are tracked separately.
        if commit == Some(self.commit) {
            DamageSet::default()
        } else {
            DamageSet::from_slice(&[Rectangle::from_size(self.geometry.size)])
        }
    }

    /// Empty. The panel is translucent glass over whatever is behind it, and
    /// it fades with its window during animations.
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

impl RenderElement<GlesRenderer> for TitleBar {
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

impl<'render> RenderElement<KmsRenderer<'render>> for TitleBar {
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
