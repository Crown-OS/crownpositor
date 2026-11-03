//! The picture a client's drag carries under the pointer, drawn by the
//! compositor so it follows the pointer over every surface. It hangs from the
//! pointer where it was grabbed and swings as the pointer moves.

use std::time::Instant;

use config::AnimationProfile;
use smithay::{
    backend::renderer::{ImportAll, ImportMem, Renderer, element::Kind},
    desktop::utils::bbox_from_surface_tree,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Scale},
    wayland::compositor::{SurfaceAttributes, with_states},
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

/// A drag's icon surface, its origin `offset` from the pointer's hot point.
pub struct DragIcon {
    pub surface: WlSurface,
    pub offset: Point<i32, Logical>,
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
            offset: Point::default(),
            pendulum: (animations != AnimationProfile::None)
                .then(|| Pendulum::new(pointer, Point::default())),
            swung: Instant::now(),
        }
    }

    /// Moves the icon by what its latest commit asked for, through
    /// `wl_surface.offset` or an attach offset.
    pub fn commit(&mut self) {
        let delta = with_states(&self.surface, |states| {
            states
                .cached_state
                .get::<SurfaceAttributes>()
                .current()
                .buffer_delta
                .take()
        });
        if let Some(delta) = delta {
            self.offset += delta;
        }
    }

    pub fn is_swinging(&self) -> bool {
        self.pendulum
            .as_ref()
            .is_some_and(|pendulum| !pendulum.at_rest())
    }

    /// Follows the pointer to `pointer` and says how far the icon has swung,
    /// clockwise in radians.
    fn swing(&mut self, pointer: Point<f64, Logical>) -> f64 {
        let now = Instant::now();
        let dt = now.duration_since(std::mem::replace(&mut self.swung, now));
        let middle = self.middle();
        self.pendulum.as_mut().map_or(0.0, |pendulum| {
            pendulum.swing(pointer, middle, dt.as_secs_f64())
        })
    }

    /// The icon's middle, from the pointer's hot point.
    fn middle(&self) -> Point<f64, Logical> {
        let bounds = bbox_from_surface_tree(&self.surface, self.offset).to_f64();
        bounds.loc + bounds.size.downscale(2.0).to_point()
    }
}

impl Cursor {
    /// Pushes the drag icon, hung from the pointer at `local` and turned as
    /// far as it has swung.
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
        let angle = icon.swing(pointer);
        if icon.is_swinging() {
            self.animation.request(Instant::now());
        }
        let origin = local + icon.offset.to_f64();
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
