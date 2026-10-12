//! Buffers drawn one pixel per pixel even when they miss their size by one.
//!
//! A client on a fractional output allocates `size × scale` pixels and rounds
//! that its own way — up, down or to nearest — while the compositor draws the
//! surface over `round(size × scale)`. When the two disagree by a pixel the
//! buffer is stretched across the difference, and a stretch resamples every
//! pixel of it: a whole window of text goes soft over one pixel of rounding.
//!
//! Hyprland calls this case MISALIGNEDFSV1. Here a buffer within [`MAX_MISS`]
//! pixels of its drawn size is drawn 1:1 instead: an oversized one cropped to
//! the drawn size, an undersized one drawn at its own. Repeating its last row
//! into the gap, as Hyprland does, would sample past the buffer, and smithay
//! leaves dmabuf textures to wrap there rather than clamp.

use smithay::{
    backend::renderer::{
        Renderer,
        element::{Element, Id, Kind, RenderElement, UnderlyingStorage},
        utils::{CommitCounter, DamageSet, OpaqueRegions},
    },
    utils::{
        Buffer as BufferCoords, Physical, Point, Rectangle, Scale, Size, Transform,
        user_data::UserDataMap,
    },
};

/// The most a client's rounding of `size × scale` can differ from the
/// compositor's, in pixels per axis: one for each edge.
const MAX_MISS: i32 = 2;

/// Buffers this small are solid fills — a single-pixel buffer stretched over a
/// rect — which are meant to be stretched.
const MIN_BUFFER: f64 = 2.0 * MAX_MISS as f64;

#[derive(Debug)]
pub struct Unstretched<E> {
    inner: E,
    exact: Option<Exact>,
}

/// Where the element draws and what from, when that differs from the inner
/// element's.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Exact {
    geometry: Rectangle<i32, Physical>,
    src: Rectangle<f64, BufferCoords>,
}

impl<E: Element> Unstretched<E> {
    /// `buffer_scale` other than 1 is a client rendering for an integer scale
    /// that the compositor is meant to rescale, not a rounding miss.
    pub fn new(inner: E, scale: Scale<f64>, buffer_scale: i32) -> Self {
        let exact = (buffer_scale == 1)
            .then(|| Exact::between(inner.geometry(scale), inner.src(), inner.transform()))
            .flatten();
        Self { inner, exact }
    }
}

/// Drawn exactly as the inner element is: for elements deliberately drawn at
/// another size, like a thumbnail.
impl<E> From<E> for Unstretched<E> {
    fn from(inner: E) -> Self {
        Self { inner, exact: None }
    }
}

impl Exact {
    fn between(
        geometry: Rectangle<i32, Physical>,
        src: Rectangle<f64, BufferCoords>,
        transform: Transform,
    ) -> Option<Self> {
        let drawn = transform.transform_size(geometry.size);
        let drawn = Size::<f64, BufferCoords>::from((f64::from(drawn.w), f64::from(drawn.h)));
        let max_miss = f64::from(MAX_MISS);
        if drawn == src.size
            || src.size.w <= MIN_BUFFER
            || src.size.h <= MIN_BUFFER
            || (drawn.w - src.size.w).abs() > max_miss
            || (drawn.h - src.size.h).abs() > max_miss
        {
            return None;
        }

        let size = Size::<f64, BufferCoords>::from((
            drawn.w.min(src.size.w.floor()),
            drawn.h.min(src.size.h.floor()),
        ));
        let physical =
            transform.transform_size(Size::<i32, Physical>::from((size.w as i32, size.h as i32)));
        Some(Self {
            geometry: Rectangle::new(geometry.loc, physical),
            src: Rectangle::new(src.loc, size),
        })
    }

    fn bounds(&self) -> Rectangle<i32, Physical> {
        Rectangle::from_size(self.geometry.size)
    }
}

impl<E: Element> Element for Unstretched<E> {
    fn id(&self) -> &Id {
        self.inner.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }

    fn src(&self) -> Rectangle<f64, BufferCoords> {
        self.exact
            .map_or_else(|| self.inner.src(), |exact| exact.src)
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.exact
            .map_or_else(|| self.inner.geometry(scale), |exact| exact.geometry)
    }

    fn transform(&self) -> Transform {
        self.inner.transform()
    }

    /// The inner element reports damage where its stretch would have put it,
    /// up to [`MAX_MISS`] pixels from where the 1:1 draw puts it.
    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        let damage = self.inner.damage_since(scale, commit);
        let Some(exact) = self.exact else {
            return damage;
        };
        damage
            .iter()
            .filter_map(|rect| {
                Rectangle::new(
                    rect.loc - Point::from((MAX_MISS, MAX_MISS)),
                    rect.size + Size::from((2 * MAX_MISS, 2 * MAX_MISS)),
                )
                .intersection(exact.bounds())
            })
            .collect()
    }

    /// Cut to the 1:1 draw, which may be smaller than the stretch was.
    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        let regions = self.inner.opaque_regions(scale);
        let Some(exact) = self.exact else {
            return regions;
        };
        regions
            .iter()
            .filter_map(|rect| rect.intersection(exact.bounds()))
            .collect()
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }

    fn kind(&self) -> Kind {
        self.inner.kind()
    }
}

impl<R: Renderer, E: RenderElement<R>> RenderElement<R> for Unstretched<E> {
    fn draw(
        &self,
        frame: &mut R::Frame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), R::Error> {
        self.inner
            .draw(frame, src, dst, damage, opaque_regions, cache)
    }

    fn underlying_storage(&self, renderer: &mut R) -> Option<UnderlyingStorage<'_>> {
        self.inner.underlying_storage(renderer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry(w: i32, h: i32) -> Rectangle<i32, Physical> {
        Rectangle::new((10, 20).into(), (w, h).into())
    }

    fn src(w: f64, h: f64) -> Rectangle<f64, BufferCoords> {
        Rectangle::from_size((w, h).into())
    }

    #[test]
    fn a_buffer_a_pixel_too_big_is_cropped_not_squeezed() {
        let exact = Exact::between(geometry(946, 600), src(947.0, 601.0), Transform::Normal);
        assert_eq!(
            exact,
            Some(Exact {
                geometry: geometry(946, 600),
                src: src(946.0, 600.0),
            })
        );
    }

    #[test]
    fn a_buffer_a_pixel_too_small_is_drawn_at_its_own_size() {
        let exact = Exact::between(geometry(946, 600), src(945.0, 600.0), Transform::Normal);
        assert_eq!(
            exact,
            Some(Exact {
                geometry: geometry(945, 600),
                src: src(945.0, 600.0),
            })
        );
    }

    #[test]
    fn a_buffer_that_fits_is_left_alone() {
        assert_eq!(
            Exact::between(geometry(946, 600), src(946.0, 600.0), Transform::Normal),
            None
        );
    }

    #[test]
    fn a_buffer_meant_to_be_rescaled_is_left_alone() {
        // A 2x buffer on a 1.25 output.
        assert_eq!(
            Exact::between(geometry(946, 600), src(1514.0, 960.0), Transform::Normal),
            None
        );
    }

    #[test]
    fn a_solid_fill_still_stretches() {
        assert_eq!(
            Exact::between(geometry(2, 2), src(1.0, 1.0), Transform::Normal),
            None
        );
    }

    #[test]
    fn a_fractional_crop_lands_on_whole_pixels() {
        let exact = Exact::between(geometry(946, 600), src(945.5, 600.0), Transform::Normal);
        assert_eq!(exact.map(|exact| exact.src), Some(src(945.0, 600.0)));
        assert_eq!(exact.map(|exact| exact.geometry), Some(geometry(945, 600)));
    }

    #[test]
    fn a_rotated_buffer_is_measured_along_its_own_axes() {
        // Buffer is portrait; on screen it is landscape.
        let exact = Exact::between(geometry(946, 600), src(601.0, 947.0), Transform::_90);
        assert_eq!(
            exact,
            Some(Exact {
                geometry: geometry(946, 600),
                src: src(600.0, 946.0),
            })
        );
    }
}
