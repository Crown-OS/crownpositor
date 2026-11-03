//! A drag icon swinging from the pointer: its surface tree painted into a
//! canvas, then drawn turned about the point it hangs from.

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, Offscreen, Texture,
            damage::OutputDamageTracker,
            element::{Id, Kind},
            gles::{GlesRenderer, GlesTexture},
            utils::CommitCounter,
        },
    },
    desktop::utils::bbox_from_surface_tree,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Buffer as BufferCoords, Logical, Physical, Point, Rectangle, Scale, Size, Transform},
};

use crate::{
    rendering::{
        surface_tree::surface_elements,
        warp::{WarpCanvases, WarpElement, canvas::Painted},
    },
    shaders::warp::WarpShaders,
};

/// Transparent border round the icon, physical pixels, so its turned edges
/// are filtered rather than jagged.
const PAD: i32 = 2;
const TRANSPARENT: [f32; 4] = [0.0; 4];

/// A drag icon to draw turned.
pub struct SwingRequest<'a> {
    pub surface: &'a WlSurface,
    /// The unturned icon's origin, output-local.
    pub origin: Point<f64, Logical>,
    /// What it turns about, output-local.
    pub pivot: Point<f64, Logical>,
    /// Clockwise on screen, radians.
    pub angle: f64,
    pub scale: Scale<f64>,
}

#[derive(Debug)]
pub(super) struct SwingCanvas {
    texture: GlesTexture,
    id: Id,
    commit: CommitCounter,
    used: bool,
}

impl SwingCanvas {
    pub(super) fn used_last_frame(&mut self) -> bool {
        std::mem::take(&mut self.used)
    }
}

/// `request`'s icon, painted and turned. `None` when the programs are missing,
/// the icon shows nothing, or the GPU refused the canvas.
pub fn swing(
    gles: &mut GlesRenderer,
    canvases: &mut WarpCanvases,
    request: SwingRequest<'_>,
) -> Option<WarpElement> {
    let shaders = WarpShaders::get(gles)?;
    let canvas = paint(gles, &mut canvases.swing, &request)?;
    let pivot = request.pivot.to_physical(request.scale);
    Some(WarpElement::swing(shaders, canvas, pivot, request.angle))
}

/// Repaints the icon every frame, so an animated one keeps playing.
fn paint(
    gles: &mut GlesRenderer,
    slot: &mut Option<SwingCanvas>,
    request: &SwingRequest<'_>,
) -> Option<Painted> {
    let scale = request.scale;
    let bounds = bbox_from_surface_tree(request.surface, (0, 0));
    if bounds.is_empty() {
        return None;
    }
    let size = bounds.size.to_physical_precise_round(scale) + Size::from((2 * PAD, 2 * PAD));
    let buffer = Size::<i32, BufferCoords>::from((size.w, size.h));
    if slot
        .as_ref()
        .is_none_or(|canvas| canvas.texture.size() != buffer)
    {
        let texture = gles.create_buffer(Fourcc::Abgr8888, buffer).ok()?;
        let id = slot.take().map_or_else(Id::new, |canvas| canvas.id);
        *slot = Some(SwingCanvas {
            texture,
            id,
            commit: CommitCounter::default(),
            used: false,
        });
    }
    let canvas = slot.as_mut()?;

    let pad = Point::from((PAD, PAD));
    let origin = pad - bounds.loc.to_physical_precise_round(scale);
    let elements = surface_elements(gles, request.surface, origin, scale, 1.0, Kind::Unspecified);
    let mut target = gles.bind(&mut canvas.texture).ok()?;
    OutputDamageTracker::new(size, scale, Transform::Normal)
        .render_output(gles, &mut target, 0, &elements, TRANSPARENT)
        .ok()?;
    drop(target);
    canvas.used = true;
    canvas.commit.increment();

    let at = (request.origin + bounds.loc.to_f64()).to_physical(scale) - pad.to_f64();
    Some(Painted {
        texture: canvas.texture.clone(),
        id: canvas.id.clone(),
        commit: canvas.commit,
        rect: Rectangle::new(at, size.to_f64()),
    })
}

/// The corners of `rect` turned `angle` about `pivot`, boxed.
pub(super) fn bounds(
    rect: Rectangle<f64, Physical>,
    pivot: Point<f64, Physical>,
    angle: f64,
) -> Rectangle<i32, Physical> {
    let (sin, cos) = angle.sin_cos();
    let corners = [
        rect.loc,
        rect.loc + Point::from((rect.size.w, 0.0)),
        rect.loc + Point::from((0.0, rect.size.h)),
        rect.loc + rect.size.to_point(),
    ]
    .map(|corner| {
        let arm = corner - pivot;
        pivot + Point::from((arm.x * cos - arm.y * sin, arm.x * sin + arm.y * cos))
    });
    let (min, max) = corners
        .iter()
        .fold((corners[0], corners[0]), |(min, max), corner| {
            (
                Point::from((min.x.min(corner.x), min.y.min(corner.y))),
                Point::from((max.x.max(corner.x), max.y.max(corner.y))),
            )
        });
    Rectangle::from_extremities(min.to_i32_floor::<i32>(), max.to_i32_ceil::<i32>())
}

#[cfg(test)]
mod tests {
    use std::f64::consts::FRAC_PI_2;

    use super::*;

    #[test]
    fn unturned_bounds_are_the_rect() {
        let rect = Rectangle::new(Point::from((10.0, 20.0)), (30.0, 40.0).into());
        assert_eq!(
            bounds(rect, Point::from((10.0, 20.0)), 0.0),
            Rectangle::new((10, 20).into(), (30, 40).into())
        );
    }

    #[test]
    fn a_quarter_turn_swings_the_rect_round_its_pivot() {
        let rect = Rectangle::new(Point::from((0.0, 0.0)), (30.0, 40.0).into());
        let turned = bounds(rect, Point::default(), FRAC_PI_2);
        assert_eq!(turned.loc, Point::from((-40, 0)));
        assert!((40..=41).contains(&turned.size.w), "{turned:?}");
        assert!((30..=31).contains(&turned.size.h), "{turned:?}");
    }
}
