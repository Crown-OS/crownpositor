//! Per-backdrop pyramid storage, kept across frames.
//!
//! A backdrop refreshes its scene copy only inside the rectangles the damage
//! tracker hands it, so the texture has to be the very one it filled last
//! frame: everywhere else it holds the composite from the frames that did own
//! those pixels, which is the only correct thing to blur there. Hence a cache
//! keyed by element [`Id`] rather than a scratch buffer per frame.

use std::{cell::RefCell, collections::HashMap, mem, rc::Rc};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Offscreen, Texture as _,
            element::Id,
            gles::{GlesError, GlesRenderer, GlesTexture},
        },
    },
    utils::{Buffer as BufferCoords, Physical, Rectangle, Size, Transform},
};

use crate::rendering::blur::BlurConfig;

/// One backdrop's textures: a full-resolution copy of what sits under it, in
/// framebuffer orientation, and the halving kawase levels above that.
#[derive(Debug)]
pub struct Pyramid {
    pub(super) scene: GlesTexture,
    pub(super) levels: Vec<GlesTexture>,
    pub(super) halo: RefCell<Halo>,
}

/// The band a blur still owes, element-local.
///
/// A pass spreads a changed pixel `radius` further than the rectangle it
/// arrived in, so the ring around each damage rect is stale once the draw is
/// done. It is carried into the next frame rather than widening this one's
/// damage, which the tracker has already closed.
#[derive(Debug, Default)]
pub struct Halo {
    /// Owed by this frame's draw, offered at the start of the next one.
    pub(super) pending: Vec<Rectangle<i32, Physical>>,
    /// This frame's offer, so the draw can tell a repaint it asked for from
    /// genuinely new content. Offered whether or not the element is still
    /// visible, so a band nobody claims expires with the frame instead of
    /// keeping the output awake.
    pub(super) reported: Vec<Rectangle<i32, Physical>>,
}

impl Pyramid {
    fn allocate(
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
            halo: RefCell::default(),
        })
    }
}

/// Every backdrop's pyramid on one output.
#[derive(Debug, Default)]
pub struct BlurCache {
    current: HashMap<Id, Rc<Pyramid>>,
    previous: HashMap<Id, Rc<Pyramid>>,
}

impl BlurCache {
    /// Retires last frame's map. Whatever is not asked for again is dropped
    /// with it, which is how a closed window's textures are freed. Each
    /// surviving pyramid's owed band becomes this frame's offer.
    pub fn begin_frame(&mut self) {
        self.previous = mem::take(&mut self.current);
        for pyramid in self.previous.values() {
            let mut halo = pyramid.halo.borrow_mut();
            halo.reported = mem::take(&mut halo.pending);
        }
    }

    /// This frame's pyramid for one backdrop, reusing last frame's whenever it
    /// still has the right shape.
    pub fn pyramid(
        &mut self,
        renderer: &mut GlesRenderer,
        id: &Id,
        size: Size<i32, BufferCoords>,
        passes: usize,
    ) -> Result<Rc<Pyramid>, GlesError> {
        let reusable = self
            .previous
            .remove(id)
            .filter(|pyramid| pyramid.scene.size() == size && pyramid.levels.len() == passes);
        let pyramid = match reusable {
            Some(pyramid) => pyramid,
            None => Rc::new(Pyramid::allocate(renderer, size, passes)?),
        };

        self.current.insert(id.clone(), Rc::clone(&pyramid));
        Ok(pyramid)
    }

    /// Whether any backdrop this frame drew owes a band, and the output
    /// therefore needs one more frame to settle.
    pub fn wants_redraw(&self) -> bool {
        self.current
            .values()
            .any(|pyramid| !pyramid.halo.borrow().pending.is_empty())
    }
}

/// What a decorator needs to build backdrops for one output's frame.
#[derive(Debug)]
pub struct BlurSession<'a> {
    pub cache: &'a mut BlurCache,
    pub config: BlurConfig,
    /// The transform the frame renders with: a pyramid is sized in the
    /// framebuffer's orientation, not the output's.
    pub transform: Transform,
    /// Output-local physical bounds, the clip a backdrop's geometry survives.
    pub output: Rectangle<i32, Physical>,
}
