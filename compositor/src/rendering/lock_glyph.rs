//! Where a lock-screen clock glyph lands once the clock's warp bends it, and
//! the map its shader undoes the bend with. Physical output pixels throughout.

use lockscreen::{clock::Homography, layout::Rect};
use smithay::utils::{Physical, Rectangle};

/// The clock's box pulled by `warp`: each corner moved by its share of the
/// box's size. `None` while nothing is pulled, or when the pull folds the box.
pub fn clock_warp(clock: Rect, warp: [[f32; 2]; 4]) -> Option<Homography> {
    if warp.iter().flatten().all(|pull| *pull == 0.0) {
        return None;
    }
    let corners = [
        [clock.x, clock.y],
        [clock.x + clock.width, clock.y],
        [clock.x + clock.width, clock.y + clock.height],
        [clock.x, clock.y + clock.height],
    ];
    let quad: [[f32; 2]; 4] = std::array::from_fn(|index| {
        let ([x, y], [dx, dy]) = (corners[index], warp[index]);
        [x + dx * clock.width, y + dy * clock.height]
    });
    Homography::box_to_quad([clock.x, clock.y], [clock.width, clock.height], quad)
}

/// The whole-pixel box around `sprite` once `warp` bends it.
pub fn warped_bounds(sprite: Rect, warp: &Homography) -> Option<Rectangle<i32, Physical>> {
    let corners = [
        [sprite.x, sprite.y],
        [sprite.x + sprite.width, sprite.y],
        [sprite.x + sprite.width, sprite.y + sprite.height],
        [sprite.x, sprite.y + sprite.height],
    ];
    let mut bounds = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for corner in corners {
        let [x, y] = warp.apply(corner)?;
        bounds = [
            bounds[0].min(x),
            bounds[1].min(y),
            bounds[2].max(x),
            bounds[3].max(y),
        ];
    }
    let [left, top] = [bounds[0].floor(), bounds[1].floor()];
    let [right, bottom] = [bounds[2].ceil(), bounds[3].ceil()];
    Some(Rectangle::new(
        (left as i32, top as i32).into(),
        ((right - left) as i32, (bottom - top) as i32).into(),
    ))
}

/// Column-major: UV across `element` to UV across `sprite`, through the
/// inverse of `warp`.
pub fn unwarp(
    element: Rectangle<i32, Physical>,
    sprite: Rect,
    warp: &Homography,
) -> Option<[f32; 9]> {
    let across_element = Homography::affine(
        [element.size.w as f32, element.size.h as f32],
        [element.loc.x as f32, element.loc.y as f32],
    );
    let into_sprite = Homography::affine(
        [1.0 / sprite.width, 1.0 / sprite.height],
        [-sprite.x / sprite.width, -sprite.y / sprite.height],
    );
    Some(
        into_sprite
            .after(warp.inverse()?)
            .after(across_element)
            .columns(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLOCK: Rect = Rect {
        x: 100.0,
        y: 50.0,
        width: 400.0,
        height: 200.0,
    };

    fn sprite_uv(matrix: &[f32; 9], [u, v]: [f32; 2]) -> [f32; 2] {
        let x = matrix[0] * u + matrix[3] * v + matrix[6];
        let y = matrix[1] * u + matrix[4] * v + matrix[7];
        let w = matrix[2] * u + matrix[5] * v + matrix[8];
        [x / w, y / w]
    }

    #[test]
    fn an_unpulled_clock_needs_no_warp() {
        assert_eq!(clock_warp(CLOCK, [[0.0; 2]; 4]), None);
    }

    #[test]
    fn a_pulled_corner_moves_by_its_share_of_the_box() {
        let warp = clock_warp(CLOCK, [[0.0; 2], [0.1, -0.25], [0.0; 2], [0.0; 2]]).expect("warp");
        let corner = warp.apply([500.0, 50.0]).expect("finite");
        assert!((corner[0] - 540.0).abs() < 1e-2 && (corner[1] - 0.0).abs() < 1e-2);
    }

    #[test]
    fn the_unwarp_lands_the_element_on_the_sprite() {
        let warp =
            clock_warp(CLOCK, [[0.05, 0.0], [0.0, 0.1], [-0.05, 0.0], [0.0, -0.1]]).expect("warp");
        let sprite = Rect {
            x: 120.0,
            y: 60.0,
            width: 90.0,
            height: 180.0,
        };
        let element = warped_bounds(sprite, &warp).expect("bounds");
        let matrix = unwarp(element, sprite, &warp).expect("invertible");
        let corner = warp.apply([sprite.x, sprite.y]).expect("finite");
        let uv = [
            (corner[0] - element.loc.x as f32) / element.size.w as f32,
            (corner[1] - element.loc.y as f32) / element.size.h as f32,
        ];
        let back = sprite_uv(&matrix, uv);
        assert!(back[0].abs() < 1e-3 && back[1].abs() < 1e-3, "{back:?}");
    }
}
