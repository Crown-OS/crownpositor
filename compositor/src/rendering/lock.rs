//! What a locked output shows.

use smithay::{
    backend::renderer::{
        ImportAll, ImportMem, Renderer,
        element::{
            Kind,
            solid::SolidColorRenderElement,
            surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
        },
    },
    utils::{Point, Rectangle, Scale},
};

use crate::{
    rendering::{Elements, decorate::TileDecorator},
    shell::{monitor::Monitor, session_lock::SessionLock},
};

/// Opaque, so nothing of the desktop shows through before the lock client
/// has drawn, or if it never does.
const LOCKED_BACKDROP: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

pub fn lock_elements<R, D>(
    elements: &mut Elements<R, D>,
    lock: &SessionLock,
    monitor: &Monitor,
    renderer: &mut R,
    scale: Scale<f64>,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    if let Some(surface) = lock.surface_on(monitor.output()) {
        let surfaces: Vec<WaylandSurfaceRenderElement<R>> = render_elements_from_surface_tree(
            renderer,
            surface.wl_surface(),
            Point::default(),
            scale,
            1.0,
            Kind::Unspecified,
        );
        elements.extend(surfaces.into_iter().map(Into::into));
    }

    let (id, commit) = lock.backdrop();
    let size = monitor.geometry().size.to_physical_precise_round(scale);
    elements.push(
        SolidColorRenderElement::new(
            id.clone(),
            Rectangle::from_size(size),
            commit,
            LOCKED_BACKDROP,
            Kind::Unspecified,
        )
        .into(),
    );
}
