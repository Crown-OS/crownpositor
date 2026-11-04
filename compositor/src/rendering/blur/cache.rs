//! What a blur pipeline keeps between frames.
//!
//! The scene and its pyramid belong to the *output*: a backdrop refreshes them
//! only inside the rectangles the damage tracker hands it, so everywhere else
//! they hold the composite from the frames that did own those pixels, which is
//! the only correct thing to blur there.

use std::rc::Rc;

use smithay::{
    backend::renderer::{
        element::Id,
        gles::{GlesError, GlesRenderer},
    },
    utils::{Buffer as BufferCoords, Physical, Rectangle, Size, Transform},
};

use crate::rendering::blur::{
    BlurConfig, content::ContentDamage, scene::BlurScene, stack::GlassStack,
};

/// One output's blur state, across frames.
#[derive(Debug, Default)]
pub struct BlurCache {
    scene: Option<Rc<BlurScene>>,
    /// Which glass stands in front of which, rebuilt as this frame's elements
    /// are. Emptied here rather than at the end of a frame, because a frame
    /// that renders nothing never reaches an end.
    stack: GlassStack,
    content: ContentDamage,
}

impl BlurCache {
    pub fn begin_frame(&mut self) {
        self.stack.clear();
        self.content.begin_frame();
        if let Some(scene) = &self.scene {
            scene.begin_frame();
        }
    }

    /// The handle a backend observes this frame's elements through, taken
    /// before the elements borrow the cache.
    pub fn content(&self) -> ContentDamage {
        self.content.clone()
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

    /// Marks `id` as glass this frame, so the content tracker leaves it out.
    pub(super) fn register(&mut self, id: &Id) -> ContentDamage {
        self.content.add_glass(id);
        self.content.clone()
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
