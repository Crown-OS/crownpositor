//! The picture a client's drag carries under the pointer, drawn by the
//! compositor so it follows the pointer over every surface. The grabbing hand
//! holds it by the middle of its top edge.

use smithay::{
    backend::renderer::{ImportAll, Renderer, element::Kind},
    desktop::utils::bbox_from_surface_tree,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Scale},
};

use crate::rendering::{
    cursor::Cursor,
    surface_tree::{SurfaceElement, surface_elements},
};

/// How far below its top edge the hand grips the icon.
const GRIP_DEPTH: f64 = 6.0;

/// A drag's icon surface. Where the client asked it to sit is ignored: it
/// always hangs from the hand.
pub struct DragIcon {
    pub surface: WlSurface,
}

impl DragIcon {
    pub fn new(surface: WlSurface) -> Self {
        Self { surface }
    }

    /// Where the icon's origin sits from the pointer's hot point, held by the
    /// middle of its top edge.
    fn origin(&self) -> Point<f64, Logical> {
        let bounds = bbox_from_surface_tree(&self.surface, (0, 0)).to_f64();
        let depth = GRIP_DEPTH.min(bounds.size.h / 2.0);
        let grip = bounds.loc + Point::from((bounds.size.w / 2.0, depth));
        Point::default() - grip
    }
}

impl Cursor {
    /// Pushes the drag icon, held under the hand at `local`.
    pub(super) fn render_drag_icon<R, E>(
        &self,
        elements: &mut Vec<E>,
        renderer: &mut R,
        local: Point<f64, Logical>,
        scale: Scale<f64>,
    ) where
        R: Renderer + ImportAll,
        R::TextureId: Send + Clone + 'static,
        E: From<SurfaceElement<R>>,
    {
        let Some(icon) = &self.drag_icon else {
            return;
        };
        let location = (local + icon.origin()).to_physical_precise_round(scale);
        let surfaces = surface_elements(
            renderer,
            &icon.surface,
            location,
            scale,
            1.0,
            Kind::Unspecified,
        );
        elements.extend(surfaces.into_iter().map(E::from));
    }
}
