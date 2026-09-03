//! The border drawn around a window.
//!
//! A ring, not a filled rectangle: the window is drawn over the hole, so the
//! two never overlap and the border costs one shader pass over its own area
//! rather than the whole tile.
//!
//! The element takes the *window's* rect and grows it outward itself, because
//! the ring's geometry and the shader's distance field have to agree to the
//! pixel — the outer radius is the window's radius plus the thickness, and
//! working that out in two places is how you get a seam at the corners. Growing
//! outward rather than insetting also keeps the layout out of it: a border
//! eats into the gap between tiles, and no window is resized to make room.
//!
//! Like [`BlurBackdrop`] this cannot be generic over the renderer. A generic
//! `R::Frame` only offers what [`Frame`] declares — clears, solid fills and
//! texture blits — and there is no way to bind a GLES program through it, so
//! the two impls below name their renderers and the multi-GPU one forwards to
//! the GLES frame underneath it.
//!
//! [`BlurBackdrop`]: crate::rendering::blur::BlurBackdrop
//! [`Frame`]: smithay::backend::renderer::Frame

use smithay::{
    backend::renderer::{
        element::{Element, Id, Kind, RenderElement, UnderlyingStorage},
        gles::{GlesError, GlesFrame, GlesPixelProgram, GlesRenderer},
        multigpu::{Error as MultiError, MultiFrame},
        utils::{CommitCounter, DamageSet, OpaqueRegions},
    },
    utils::{Buffer as BufferCoords, Physical, Point, Rectangle, Scale, Size, Transform},
};

use crate::{
    backend::render::{GbmGlesApi, KmsRenderer},
    shaders::border::BorderShader,
};

/// What a decorator needs to build the border for one window.
#[derive(Debug, Clone)]
pub struct Border {
    /// Stable across frames for the same window, or the damage tracker treats
    /// every frame's border as a brand new element and repaints around the
    /// window continuously.
    pub id: Id,
    /// Changes exactly when the ring's pixels do — a focus change swapping the
    /// colour, or the width or radius being reconfigured. Geometry changes are
    /// the damage tracker's own job and need no bump.
    pub commit: CommitCounter,
    /// The window's rect in output-local physical coordinates. The ring goes
    /// *around* this, not inside it.
    pub window: Rectangle<i32, Physical>,
    /// Ring width in physical pixels.
    pub thickness: f32,
    /// The window's own corner radius, in physical pixels — the same value the
    /// rounded-corner shader is masking it with, so the ring traces its edge.
    pub radius: f32,
    /// Straight (non-premultiplied) RGBA.
    pub color: [f32; 4],
    /// The window's animation alpha, so a border fades in with its window.
    pub alpha: f32,
}

/// One window's border ring.
#[derive(Debug, Clone)]
pub struct WindowDecoration {
    id: Id,
    commit: CommitCounter,
    /// `None` means the shader never compiled. The element then draws nothing
    /// at all — a missing border is a cosmetic loss, and dropping the element
    /// instead would make the damage tracker rescan the area for no gain.
    program: Option<GlesPixelProgram>,
    /// The ring's own rect: the window grown by `thickness` on every side.
    geometry: Rectangle<i32, Physical>,
    /// Integral, and the same value the geometry was grown by.
    thickness: f32,
    /// The *outer* radius — the window's radius plus `thickness`.
    radius: f32,
    /// Premultiplied, as the shader expects.
    color: [f32; 4],
    alpha: f32,
}

impl WindowDecoration {
    /// `None` when there would be nothing to see: no width to draw, a fully
    /// transparent colour, or no window to draw around. All three are ordinary
    /// states — a zero `border_width` is how the border is turned off, and a
    /// window mid-animation can round to an empty rect — so they cost no
    /// element rather than an invisible one.
    pub fn new(renderer: &GlesRenderer, border: Border) -> Option<Self> {
        if border.alpha <= 0.0 || border.color[3] <= 0.0 {
            return None;
        }

        let (geometry, thickness, radius) =
            Self::ring(border.window, border.thickness, border.radius)?;
        let [red, green, blue, alpha] = border.color;

        Some(Self {
            id: border.id,
            commit: border.commit,
            program: BorderShader::get(renderer),
            geometry,
            thickness,
            radius,
            color: [red * alpha, green * alpha, blue * alpha, alpha],
            alpha: border.alpha,
        })
    }

    /// The ring's rect, its integral thickness and its *outer* radius.
    ///
    /// Kept separate from [`new`] and free of the renderer so the arithmetic
    /// the shader depends on can be tested without a GL context — this is the
    /// half that decides whether the ring meets the window or leaves a seam.
    ///
    /// [`new`]: Self::new
    fn ring(
        window: Rectangle<i32, Physical>,
        thickness: f32,
        radius: f32,
    ) -> Option<(Rectangle<i32, Physical>, f32, f32)> {
        // Rounded once, and used for both the geometry and the distance field.
        // A fractional thickness would put the ring's outer edge half a pixel
        // off the element boundary, where it is either clipped or leaves a gap.
        let thickness = thickness.round();

        // An empty window collapses the hole to a point and the ring fills
        // solid — a block of accent colour where no window is.
        if thickness < 1.0 || window.is_empty() {
            return None;
        }

        let grown = thickness as i32;
        let geometry = Rectangle::new(
            window.loc - Point::from((grown, grown)),
            window.size + Size::from((grown * 2, grown * 2)),
        );

        Some((geometry, thickness, radius.max(0.0) + thickness))
    }

    /// The ring's own area, which is what `v_coords` has to span: the shader
    /// reads it as a pixel position, and a `src` smaller than the element
    /// would silently rescale the whole distance field.
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
            &BorderShader::values(self.color, self.thickness, self.radius),
        )
    }
}

impl Element for WindowDecoration {
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
        // The ring is generated wholesale from three uniforms, so any change to
        // them repaints all of it; moving and resizing are tracked separately.
        if commit == Some(self.commit) {
            DamageSet::default()
        } else {
            DamageSet::from_slice(&[Rectangle::from_size(self.geometry.size)])
        }
    }

    /// Empty. The ring is antialiased at both edges and covers only a frame's
    /// worth of pixels, so there is no rectangle inside it that is fully
    /// opaque — and it fades with its window during animations.
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

impl RenderElement<GlesRenderer> for WindowDecoration {
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

impl<'render> RenderElement<KmsRenderer<'render>> for WindowDecoration {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn window(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Physical> {
        Rectangle::new(Point::from((x, y)), Size::from((w, h)))
    }

    #[test]
    fn the_ring_surrounds_the_window_evenly() {
        let (ring, _, _) = WindowDecoration::ring(window(40, 20, 200, 100), 2.0, 8.0).unwrap();

        assert_eq!(ring, window(38, 18, 204, 104));
    }

    /// The whole point of deriving both edges from one thickness: the ring's
    /// inner edge has to land exactly on the window's outer edge, or a seam
    /// shows at the corners where the two curves disagree.
    #[test]
    fn the_inner_edge_traces_the_window() {
        let window = window(40, 20, 200, 100);
        let (ring, thickness, radius) = WindowDecoration::ring(window, 2.0, 8.0).unwrap();

        let inset = 2 * thickness as i32;
        assert_eq!(ring.size.w - inset, window.size.w);
        assert_eq!(ring.size.h - inset, window.size.h);
        // What the shader subtracts to get its hole's radius.
        assert_eq!(radius - thickness, 8.0);
    }

    /// Rounding on each axis separately would grow the rect by 1px on one side
    /// and 2px on the other, and the ring would sit off-centre.
    #[test]
    fn a_fractional_thickness_is_rounded_once() {
        let (ring, thickness, _) =
            WindowDecoration::ring(window(0, 0, 100, 100), 1.6, 8.0).unwrap();

        assert_eq!(thickness, 2.0);
        assert_eq!(ring, window(-2, -2, 104, 104));
    }

    #[test]
    fn a_hairline_border_is_dropped_rather_than_drawn() {
        assert!(WindowDecoration::ring(window(0, 0, 100, 100), 0.4, 8.0).is_none());
        assert!(WindowDecoration::ring(window(0, 0, 100, 100), 0.0, 8.0).is_none());
    }

    /// A window mid-animation can round to an empty rect. Drawing a ring around
    /// it fills solid, because the hole collapses to a point.
    #[test]
    fn an_empty_window_has_no_border() {
        assert!(WindowDecoration::ring(window(40, 20, 0, 0), 2.0, 8.0).is_none());
        assert!(WindowDecoration::ring(window(40, 20, 200, 0), 2.0, 8.0).is_none());
    }

    /// Square windows get a square ring, not one rounded by the thickness.
    #[test]
    fn a_square_window_keeps_square_corners() {
        let (_, thickness, radius) =
            WindowDecoration::ring(window(0, 0, 100, 100), 3.0, 0.0).unwrap();

        assert_eq!(radius - thickness, 0.0);
    }
}
