//! Which rounded clips changed shape since an output's last frame.
//!
//! A clip is drawn around a client's buffer, so a new radius or size changes
//! pixels the client never damaged: a CSD window whose titlebar the compositor
//! drops rounds its top corners without committing a thing. The damage tracker
//! only sees the client's own damage, so the corners would stay as they were
//! until something else — the cursor passing over — repainted them.

use std::{collections::HashSet, mem};

use smithay::{
    backend::renderer::element::Id,
    utils::{Physical, Size},
};

/// An element and the exact shape it was clipped to. One surface can be drawn
/// twice in a frame at different sizes — a window and its thumbnail — so the
/// shape is part of the identity rather than a value stored against it.
type Shape = (Id, [u32; 2], [u32; 4]);

/// One output's clips, this frame and the last.
#[derive(Debug, Default)]
pub struct CornerMemory {
    previous: HashSet<Shape>,
    current: HashSet<Shape>,
}

impl CornerMemory {
    pub fn begin_frame(&mut self) {
        mem::swap(&mut self.previous, &mut self.current);
        self.current.clear();
    }

    /// Records this frame's clip and whether the last frame drew a different
    /// one, which is when the whole element has to be repainted.
    pub fn reshaped(&mut self, id: &Id, size: Size<i32, Physical>, radius: [f32; 4]) -> bool {
        let shape = (
            id.clone(),
            [size.w as u32, size.h as u32],
            radius.map(f32::to_bits),
        );
        let reshaped = !self.previous.contains(&shape);
        self.current.insert(shape);
        reshaped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_radius_is_a_new_shape_and_a_steady_one_is_not() {
        let id = Id::new();
        let mut memory = CornerMemory::default();
        assert!(memory.reshaped(&id, Size::from((100, 50)), [0.0, 0.0, 8.0, 8.0]));

        memory.begin_frame();
        assert!(!memory.reshaped(&id, Size::from((100, 50)), [0.0, 0.0, 8.0, 8.0]));

        memory.begin_frame();
        assert!(memory.reshaped(&id, Size::from((100, 50)), [8.0; 4]));
    }

    #[test]
    fn two_instances_of_one_surface_do_not_fight() {
        let id = Id::new();
        let mut memory = CornerMemory::default();
        memory.reshaped(&id, Size::from((100, 50)), [8.0; 4]);
        memory.reshaped(&id, Size::from((20, 10)), [2.0; 4]);

        memory.begin_frame();
        assert!(!memory.reshaped(&id, Size::from((100, 50)), [8.0; 4]));
        assert!(!memory.reshaped(&id, Size::from((20, 10)), [2.0; 4]));
    }
}
