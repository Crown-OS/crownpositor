//! The output-wide copy of the scene every backdrop blurs out of.
//!
//! One scene and one pyramid per output, not one per backdrop. Two pieces of
//! glass that overlap then read the same unblurred pixels — the upper one no
//! longer blurs what the lower one already blurred — and two pieces that merely
//! touch blur across their shared edge instead of each clamping at it.
//!
//! What keeps that true is [`BlurScene::cover`]: once a backdrop has drawn, the
//! framebuffer under it holds glass, so nothing after it may lift those pixels
//! back into the scene. The refresh also reaches a blur radius past the damage
//! it was handed, because the taps at the edge of a piece of glass need real
//! pixels to land on — but outside the damage the framebuffer still holds the
//! finished composite, so there it stops at every piece of glass on the frame
//! ([`BlurScene::stand`]), drawn yet or not.

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

use crate::utils::region;

/// The framebuffer as it was before any glass was drawn over it, and the
/// halving kawase levels blurred out of it. Both in framebuffer orientation and
/// at framebuffer resolution, so a backdrop addresses them with `gl_FragCoord`.
#[derive(Debug)]
pub struct BlurScene {
    pub(super) scene: GlesTexture,
    pub(super) levels: Vec<GlesTexture>,
    glass: RefCell<Glass>,
}

/// Where the frame holds glass rather than scene.
#[derive(Debug, Default)]
struct Glass {
    /// Drawn by this frame's backdrops so far, in framebuffer pixels.
    drawn: Vec<Rectangle<i32, Physical>>,
    /// Every backdrop placed on this frame, output-local. Recorded when the
    /// element is built rather than when it draws, because the damage tracker
    /// skips an element with no damage, and its glass is still standing in the
    /// framebuffer all the same.
    standing: Vec<Rectangle<i32, Physical>>,
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
        glass.drawn.clear();
        glass.standing.clear();
    }

    /// Records a backdrop placed on this frame, in output-local pixels.
    pub(super) fn stand(&self, geometry: Rectangle<i32, Physical>) {
        self.glass.borrow_mut().standing.push(geometry);
    }

    /// What of `damage` is worth lifting out of the frame, in framebuffer
    /// pixels. See [`Glass::refresh`]; `to_framebuffer` maps the output-local
    /// glass on the frame into the same space.
    pub(super) fn refresh(
        &self,
        damage: &[Rectangle<i32, Physical>],
        footprint: &[Rectangle<i32, Physical>],
        to_framebuffer: impl Fn(Rectangle<i32, Physical>) -> Rectangle<i32, Physical>,
    ) -> Vec<Rectangle<i32, Physical>> {
        let glass = self.glass.borrow();
        let standing: Vec<_> = glass
            .standing
            .iter()
            .map(|rect| to_framebuffer(*rect))
            .collect();
        glass.refresh(damage, footprint, &standing)
    }

    /// Records that `rects` now hold glass. Called whether or not the backdrop
    /// redrew anything: where it did not, the frame is still showing the glass
    /// it drew last time.
    pub(super) fn cover(&self, rects: impl IntoIterator<Item = Rectangle<i32, Physical>>) {
        self.glass.borrow_mut().drawn.extend(rects);
    }
}

impl Glass {
    /// The damage itself, minus the glass this frame has already drawn: a
    /// backdrop over another one lifts the scene the lower one blurred, not the
    /// blur it left behind.
    ///
    /// Plus the skirt: the rest of the blur's `footprint`, which reaches past
    /// the damage, minus every piece of glass `standing` on the frame. The
    /// skirt is where the outermost taps of the blur land, and it is the one
    /// part of the refresh that reads pixels nothing has repainted — the
    /// finished composite, glass and whatever sits on it included. Lifting that
    /// would blur the glass, and the text on it, back into itself.
    ///
    /// The glass goes first: in the overview one backdrop stands over the whole
    /// output, and taking it away before the damage leaves nothing to cut up.
    fn refresh(
        &self,
        damage: &[Rectangle<i32, Physical>],
        footprint: &[Rectangle<i32, Physical>],
        standing: &[Rectangle<i32, Physical>],
    ) -> Vec<Rectangle<i32, Physical>> {
        let mut refresh = region::subtract(damage.iter().copied(), self.drawn.iter().copied());
        refresh.extend(region::subtract(
            footprint.iter().copied(),
            self.drawn.iter().chain(standing).chain(damage).copied(),
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

    /// What the backdrop hands over: the damage grown by the blur's reach.
    fn footprint(
        damage: &[Rectangle<i32, Physical>],
        radius: i32,
    ) -> Vec<Rectangle<i32, Physical>> {
        region::coalesce(
            damage
                .iter()
                .filter_map(|rect| grow(*rect, radius).intersection(bounds())),
        )
    }

    fn refresh(
        glass: &Glass,
        damage: &[Rectangle<i32, Physical>],
        radius: i32,
        standing: &[Rectangle<i32, Physical>],
    ) -> Vec<Rectangle<i32, Physical>> {
        glass.refresh(damage, &footprint(damage, radius), standing)
    }

    #[test]
    fn the_first_backdrop_of_a_frame_lifts_its_damage_and_a_skirt() {
        let refreshed = refresh(&Glass::default(), &[rect(50, 50, 20, 20)], 4, &[]);
        assert_eq!(points(&refreshed), points(&[rect(46, 46, 28, 28)]));
    }

    /// The popup does not lift the bar's glass back out of the frame, so it
    /// blurs what the bar blurred instead of blurring the bar's own blur.
    #[test]
    fn a_backdrop_over_another_one_leaves_its_glass_alone() {
        let glass = Glass {
            drawn: vec![rect(0, 0, 200, 40)],
            standing: Vec::new(),
        };
        let refreshed = refresh(&glass, &[rect(50, 20, 20, 40)], 0, &[]);
        assert_eq!(points(&refreshed), points(&[rect(50, 40, 20, 20)]));
    }

    /// The skirt reaches past the damage into pixels nothing repainted, so any
    /// glass standing on the frame is off limits there — drawn yet or not.
    #[test]
    fn the_skirt_stops_at_every_piece_of_glass_on_the_frame() {
        let glass = Glass {
            drawn: vec![rect(0, 0, 200, 40)],
            standing: Vec::new(),
        };
        let standing = [rect(0, 0, 200, 40), rect(0, 160, 200, 40)];
        let refreshed = refresh(&glass, &[rect(50, 60, 20, 80)], 30, &standing);
        assert_eq!(points(&refreshed), points(&[rect(20, 40, 80, 120)]));
    }

    /// Glass that is merely standing is only in the way of the skirt: inside
    /// the damage the frame has already repainted whatever was under it, which
    /// is exactly what a backdrop redrawing its own area every frame depends on.
    #[test]
    fn damage_is_lifted_through_glass_that_has_not_drawn_yet() {
        let refreshed = refresh(
            &Glass::default(),
            &[rect(0, 0, 200, 40)],
            0,
            &[rect(0, 0, 200, 40)],
        );
        assert_eq!(points(&refreshed), points(&[rect(0, 0, 200, 40)]));
    }

    /// A backdrop the damage tracker skipped last frame still counts: its glass
    /// never left the framebuffer, so a skirt reaching into it lifts nothing.
    #[test]
    fn glass_that_did_not_draw_still_guards_the_skirt() {
        let refreshed = refresh(
            &Glass::default(),
            &[rect(50, 50, 20, 20)],
            10,
            &[rect(0, 0, 200, 200)],
        );
        assert_eq!(points(&refreshed), points(&[rect(50, 50, 20, 20)]));
    }

    /// Neighbouring damage rectangles reach into each other's skirts; each
    /// pixel is still lifted once.
    #[test]
    fn overlapping_skirts_are_lifted_once() {
        let damage = [
            rect(40, 40, 20, 20),
            rect(70, 40, 20, 20),
            rect(55, 70, 20, 20),
        ];
        let lifted = points(&refresh(&Glass::default(), &damage, 16, &[]));
        for rect in damage {
            assert!(points(&[grow(rect, 16)]).is_subset(&lifted), "{rect:?}");
        }
    }
}
