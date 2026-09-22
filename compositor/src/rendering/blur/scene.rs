//! The output-wide copy of the scene every backdrop blurs out of.
//!
//! One scene and one pyramid per output, not one per backdrop. Two pieces of
//! glass that overlap then read the same unblurred pixels — the upper one no
//! longer blurs what the lower one already blurred — and two pieces that merely
//! touch blur across their shared edge instead of each clamping at it.
//!
//! What keeps that true is [`BlurScene::cover`]: once a backdrop has drawn, the
//! framebuffer under it holds glass, so nothing after it may lift those pixels
//! back into the scene. Everything else in the frame is fair game, which is why
//! the refresh below reaches a blur radius past the damage it was handed — the
//! taps at the edge of a piece of glass need real pixels to land on.

use std::cell::RefCell;

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Offscreen, Texture as _,
            gles::{GlesError, GlesRenderer, GlesTexture},
        },
    },
    utils::{Buffer as BufferCoords, Physical, Point, Rectangle, Size},
};

/// The framebuffer as it was before any glass was drawn over it, and the
/// halving kawase levels blurred out of it. Both in framebuffer orientation and
/// at framebuffer resolution, so a backdrop addresses them with `gl_FragCoord`.
#[derive(Debug)]
pub struct BlurScene {
    pub(super) scene: GlesTexture,
    pub(super) levels: Vec<GlesTexture>,
    glass: RefCell<Glass>,
}

/// Where the frame already holds glass rather than scene, in framebuffer
/// pixels.
#[derive(Debug, Default)]
struct Glass {
    /// Drawn by this frame's backdrops so far.
    drawn: Vec<Rectangle<i32, Physical>>,
    /// Last frame's, for the parts of the frame this one has not repainted:
    /// the old glass is still standing there.
    stale: Vec<Rectangle<i32, Physical>>,
}

impl BlurScene {
    pub(super) fn allocate(
        renderer: &mut GlesRenderer,
        size: Size<i32, BufferCoords>,
        passes: usize,
    ) -> Result<Self, GlesError> {
        let mut allocate =
            |size| Offscreen::<GlesTexture>::create_buffer(renderer, Fourcc::Abgr8888, size);
        let scene = allocate(size)?;
        let levels = (0..passes)
            .map(|pass| {
                let shift = pass as u32 + 1;
                allocate(Size::from((
                    (size.w >> shift).max(1),
                    (size.h >> shift).max(1),
                )))
            })
            .collect::<Result<_, _>>()?;

        Ok(Self {
            scene,
            levels,
            glass: RefCell::default(),
        })
    }

    pub(super) fn fits(&self, size: Size<i32, BufferCoords>, passes: usize) -> bool {
        self.scene.size() == size && self.levels.len() == passes
    }

    /// Retires last frame's glass. The scene itself carries over: outside this
    /// frame's damage it still holds the composite from the frames that did own
    /// those pixels, which is the only correct thing to blur there.
    pub(super) fn begin_frame(&self) {
        let mut glass = self.glass.borrow_mut();
        glass.stale = std::mem::take(&mut glass.drawn);
    }

    /// What of `damage` is worth lifting out of the frame, in framebuffer
    /// pixels. See [`Glass::refresh`].
    pub(super) fn refresh(
        &self,
        damage: &[Rectangle<i32, Physical>],
        radius: i32,
        bounds: Rectangle<i32, Physical>,
    ) -> Vec<Rectangle<i32, Physical>> {
        self.glass.borrow().refresh(damage, radius, bounds)
    }

    /// Records that `rect` now holds glass. Called whether or not the backdrop
    /// redrew anything: where it did not, the frame is still showing the glass
    /// it drew last time.
    pub(super) fn cover(&self, rect: Rectangle<i32, Physical>) {
        self.glass.borrow_mut().drawn.push(rect);
    }
}

impl Glass {
    /// The damage itself, minus the glass this frame has already drawn — that
    /// is the whole of the fix: a backdrop over another one lifts the scene the
    /// lower one blurred, not the blur it left behind.
    ///
    /// Plus a `radius`-wide skirt around the damage, minus glass of either
    /// frame. The skirt is where the outermost taps of the blur land, and it is
    /// the one part of the refresh that reads pixels this backdrop is not
    /// about to cover — so it is also the one part that has to mind the glass
    /// still standing from last frame.
    fn refresh(
        &self,
        damage: &[Rectangle<i32, Physical>],
        radius: i32,
        bounds: Rectangle<i32, Physical>,
    ) -> Vec<Rectangle<i32, Physical>> {
        let mut refresh =
            Rectangle::subtract_rects_many(damage.iter().copied(), self.drawn.iter().copied());

        let skirt = damage
            .iter()
            .filter_map(|rect| grow(*rect, radius).intersection(bounds));
        let skirt = Rectangle::subtract_rects_many(skirt, damage.iter().copied());
        refresh.extend(Rectangle::subtract_rects_many(
            skirt,
            self.drawn.iter().chain(&self.stale).copied(),
        ));

        refresh
    }
}

pub(super) fn grow(rect: Rectangle<i32, Physical>, margin: i32) -> Rectangle<i32, Physical> {
    let margin = Point::from((margin, margin));
    Rectangle::from_extremities(rect.loc - margin, rect.loc + rect.size.to_point() + margin)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Physical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    /// Every point of `rects`, which is how coverage is compared without
    /// depending on how the subtraction happened to split things up.
    fn points(rects: &[Rectangle<i32, Physical>]) -> std::collections::HashSet<(i32, i32)> {
        let mut points = std::collections::HashSet::new();
        for rect in rects {
            for y in rect.loc.y..rect.loc.y + rect.size.h {
                for x in rect.loc.x..rect.loc.x + rect.size.w {
                    assert!(points.insert((x, y)), "refresh blits {x},{y} twice");
                }
            }
        }
        points
    }

    fn bounds() -> Rectangle<i32, Physical> {
        rect(0, 0, 200, 200)
    }

    #[test]
    fn the_first_backdrop_of_a_frame_lifts_its_damage_and_a_skirt() {
        let refreshed = Glass::default().refresh(&[rect(50, 50, 20, 20)], 4, bounds());
        assert_eq!(points(&refreshed), points(&[rect(46, 46, 28, 28)]));
    }

    /// The fix, as geometry: the popup does not lift the bar's glass back out
    /// of the frame, so it blurs what the bar blurred instead of blurring the
    /// bar's own blur.
    #[test]
    fn a_backdrop_over_another_one_leaves_its_glass_alone() {
        let glass = Glass {
            drawn: vec![rect(0, 0, 200, 40)],
            stale: Vec::new(),
        };
        let refreshed = glass.refresh(&[rect(50, 20, 20, 40)], 0, bounds());
        assert_eq!(points(&refreshed), points(&[rect(50, 40, 20, 20)]));
    }

    /// The skirt reaches past the damage into pixels nothing repainted, so the
    /// glass another backdrop left standing last frame is off limits there too.
    #[test]
    fn the_skirt_stops_at_glass_from_either_frame() {
        let glass = Glass {
            drawn: vec![rect(0, 0, 200, 40)],
            stale: vec![rect(0, 160, 200, 40)],
        };
        let refreshed = glass.refresh(&[rect(50, 60, 20, 80)], 30, bounds());
        assert_eq!(points(&refreshed), points(&[rect(20, 40, 80, 120)]),);
    }

    /// Last frame's glass is only in the way of the skirt: inside the damage
    /// the frame has already repainted whatever was under it, which is exactly
    /// what a backdrop redrawing its own area every frame depends on.
    #[test]
    fn damage_is_lifted_through_last_frames_glass() {
        let glass = Glass {
            drawn: Vec::new(),
            stale: vec![rect(0, 0, 200, 40)],
        };
        let refreshed = glass.refresh(&[rect(0, 0, 200, 40)], 0, bounds());
        assert_eq!(points(&refreshed), points(&[rect(0, 0, 200, 40)]));
    }

    #[test]
    fn a_frame_retires_its_glass() {
        let mut glass = Glass::default();
        glass.drawn.push(rect(0, 0, 10, 10));

        glass.stale = std::mem::take(&mut glass.drawn);
        assert!(glass.drawn.is_empty());
        assert_eq!(glass.stale, vec![rect(0, 0, 10, 10)]);
    }
}
