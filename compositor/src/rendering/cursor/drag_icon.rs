//! The picture a client's drag carries under the pointer, drawn by the
//! compositor so it follows the pointer over every surface. The grabbing hand
//! holds it by the middle of its top edge and it sways a little as the
//! pointer moves.

use std::time::Instant;

use config::AnimationProfile;
use smithay::{
    backend::renderer::{ImportAll, ImportMem, Renderer, element::Kind},
    desktop::utils::bbox_from_surface_tree,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Scale},
};

use crate::rendering::{
    Elements,
    cursor::{Cursor, pendulum::Pendulum},
    decorate::TileDecorator,
    element::CrownElement,
    surface_tree::surface_elements,
    warp::SwingRequest,
};

/// Swings smaller than this are drawn upright, straight from the client's
/// buffers.
const UPRIGHT: f64 = 0.002;
/// How far below its top edge the hand grips the icon.
const GRIP_DEPTH: f64 = 6.0;

/// A drag's icon surface. Where the client asked it to sit is ignored: it
/// always hangs from the hand.
pub struct DragIcon {
    pub surface: WlSurface,
    /// `None` while animations are off: the icon then never swings.
    pendulum: Option<Pendulum>,
    swung: Instant,
}

impl DragIcon {
    pub fn new(
        surface: WlSurface,
        pointer: Point<f64, Logical>,
        animations: AnimationProfile,
    ) -> Self {
        Self {
            surface,
            pendulum: (animations != AnimationProfile::None)
                .then(|| Pendulum::new(pointer, Point::default())),
            swung: Instant::now(),
        }
    }

    pub fn is_swinging(&self) -> bool {
        self.pendulum
            .as_ref()
            .is_some_and(|pendulum| !pendulum.at_rest())
    }

    /// Follows the pointer to `pointer` and says how far the icon has swung,
    /// clockwise in radians.
    fn swing(&mut self, pointer: Point<f64, Logical>, middle: Point<f64, Logical>) -> f64 {
        let now = Instant::now();
        let dt = now.duration_since(std::mem::replace(&mut self.swung, now));
        self.pendulum.as_mut().map_or(0.0, |pendulum| {
            pendulum.swing(pointer, middle, dt.as_secs_f64())
        })
    }

    /// Where the icon's origin and its middle sit from the pointer's hot
    /// point, held by the middle of its top edge.
    fn hold(&self) -> (Point<f64, Logical>, Point<f64, Logical>) {
        let bounds = bbox_from_surface_tree(&self.surface, (0, 0)).to_f64();
        let depth = GRIP_DEPTH.min(bounds.size.h / 2.0);
        let grip = bounds.loc + Point::from((bounds.size.w / 2.0, depth));
        let middle = Point::from((0.0, bounds.size.h / 2.0 - depth));
        (Point::default() - grip, middle)
    }
}

impl Cursor {
    /// Pushes the drag icon, held under the hand at `local` and turned as far
    /// as it has swung.
    pub(super) fn render_drag_icon<R, D>(
        &mut self,
        elements: &mut Elements<R, D>,
        renderer: &mut R,
        decorator: &mut D,
        local: Point<f64, Logical>,
        pointer: Point<f64, Logical>,
        scale: Scale<f64>,
    ) where
        R: Renderer + ImportAll + ImportMem,
        R::TextureId: Send + Clone + 'static,
        D: TileDecorator<R>,
    {
        let Some(icon) = &mut self.drag_icon else {
            return;
        };
        let (origin, middle) = icon.hold();
        let angle = icon.swing(pointer, middle);
        if icon.is_swinging() {
            self.animation.request(Instant::now());
        }
        let origin = local + origin;
        if angle.abs() >= UPRIGHT {
            let request = SwingRequest {
                surface: &icon.surface,
                origin,
                pivot: local,
                angle,
                scale,
            };
            if let Some(element) = decorator.swing(renderer, request) {
                elements.push(CrownElement::Tile(element.into()));
                return;
            }
        }
        let location = origin.to_physical_precise_round(scale);
        let surfaces = surface_elements(
            renderer,
            &icon.surface,
            location,
            scale,
            1.0,
            Kind::Unspecified,
        );
        elements.extend(surfaces.into_iter().map(CrownElement::from));
    }
}
