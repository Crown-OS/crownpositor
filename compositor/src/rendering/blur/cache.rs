//! What a blur pipeline keeps between frames.
//!
//! Two things outlive a frame, with different lifetimes. The scene and its
//! pyramid belong to the *output*: a backdrop refreshes them only inside the
//! rectangles the damage tracker hands it, so everywhere else they hold the
//! composite from the frames that did own those pixels, which is the only
//! correct thing to blur there. The halo belongs to one *backdrop*, because it
//! is the damage that backdrop owes the tracker next frame.

use std::{cell::RefCell, collections::HashMap, mem, rc::Rc};

use smithay::{
    backend::renderer::{
        element::Id,
        gles::{GlesError, GlesRenderer},
    },
    utils::{Buffer as BufferCoords, Physical, Rectangle, Size, Transform},
};

use crate::rendering::blur::{BlurConfig, scene::BlurScene, stack::GlassStack};

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

/// One output's blur state, across frames.
#[derive(Debug, Default)]
pub struct BlurCache {
    scene: Option<Rc<BlurScene>>,
    /// Which glass stands in front of which, rebuilt as this frame's elements
    /// are. Emptied here rather than at the end of a frame, because a frame
    /// that renders nothing never reaches an end.
    stack: GlassStack,
    current: HashMap<Id, Rc<RefCell<Halo>>>,
    previous: HashMap<Id, Rc<RefCell<Halo>>>,
}

impl BlurCache {
    /// Retires last frame's backdrops. Whatever is not asked for again is
    /// dropped with it, which is how a closed window stops asking for frames.
    /// Each surviving backdrop's owed band becomes this frame's offer.
    pub fn begin_frame(&mut self) {
        self.previous = mem::take(&mut self.current);
        for halo in self.previous.values() {
            let mut halo = halo.borrow_mut();
            halo.reported = mem::take(&mut halo.pending);
        }
        self.stack.clear();
        if let Some(scene) = &self.scene {
            scene.begin_frame();
        }
    }

    /// This output's scene and pyramid, reallocated only when the framebuffer
    /// or the pass count changed shape.
    pub(super) fn scene(
        &mut self,
        renderer: &mut GlesRenderer,
        size: Size<i32, BufferCoords>,
        passes: usize,
    ) -> Result<Rc<BlurScene>, GlesError> {
        let scene = match self.scene.take().filter(|scene| scene.fits(size, passes)) {
            Some(scene) => scene,
            None => Rc::new(BlurScene::allocate(renderer, size, passes)?),
        };
        self.scene = Some(Rc::clone(&scene));
        Ok(scene)
    }

    pub(super) fn stack(&mut self) -> &mut GlassStack {
        &mut self.stack
    }

    pub(super) fn halo(&mut self, id: &Id) -> Rc<RefCell<Halo>> {
        let halo = self.previous.remove(id).unwrap_or_default();
        self.current.insert(id.clone(), Rc::clone(&halo));
        halo
    }

    /// Whether any backdrop this frame drew owes a band, and the output
    /// therefore needs one more frame to settle.
    pub fn wants_redraw(&self) -> bool {
        self.current
            .values()
            .any(|halo| !halo.borrow().pending.is_empty())
    }
}

/// What a decorator needs to build backdrops for one output's frame.
#[derive(Debug)]
pub struct BlurSession<'a> {
    pub cache: &'a mut BlurCache,
    pub config: BlurConfig,
    /// The transform the frame renders with: the scene is sized in the
    /// framebuffer's orientation, not the output's.
    pub transform: Transform,
    /// Output-local physical bounds, the clip a backdrop's geometry survives.
    pub output: Rectangle<i32, Physical>,
}
