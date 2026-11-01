//! A liquid morph's shape, measured in the canvas's physical pixels.

use smithay::utils::{Logical, Physical, Point, Rectangle, Scale};

use crate::{shaders::warp::LiquidUniforms, shell::morph::Morph};

/// Seconds of motion an edge is smeared over: about a quarter of a frame at
/// 60 Hz, enough to soften a fast edge without blurring a slow one.
const SMEAR_SECONDS: f64 = 1.0 / 240.0;
/// The most an edge is smeared, physical pixels.
const MAX_SOFTNESS: f64 = 6.0;

/// `morph` this frame, for a window at rest in `window` (output-local) with
/// corners of `radius`, drawn from a canvas covering `canvas`.
pub fn uniforms(
    morph: &Morph,
    window: Rectangle<f64, Logical>,
    radius: f64,
    canvas: Rectangle<f64, Physical>,
    scale: Scale<f64>,
) -> LiquidUniforms {
    let shape = morph.shape(window, radius);
    let content = window.to_physical(scale);
    LiquidUniforms {
        corners: shape.corners.map(|corner| physical(corner, scale)),
        content: [
            ((content.loc.x - canvas.loc.x) / canvas.size.w) as f32,
            ((content.loc.y - canvas.loc.y) / canvas.size.h) as f32,
            (content.size.w / canvas.size.w) as f32,
            (content.size.h / canvas.size.h) as f32,
        ],
        radius: (shape.radius * scale.x) as f32,
        softness: (morph.speed(window) * SMEAR_SECONDS * scale.x).min(MAX_SOFTNESS) as f32,
        reveal: morph.reveal(),
        icon_alpha: morph.icon_alpha(),
    }
}

/// Everywhere the shape can draw this frame, softened edge included.
pub fn bounds(uniforms: &LiquidUniforms) -> Rectangle<i32, Physical> {
    let reach = f64::from(uniforms.softness) + 1.0;
    let [first, rest @ ..] = uniforms.corners;
    let (mut min, mut max) = (first, first);
    for [x, y] in rest {
        min = [min[0].min(x), min[1].min(y)];
        max = [max[0].max(x), max[1].max(y)];
    }
    Rectangle::from_extremities(
        (
            (f64::from(min[0]) - reach).floor() as i32,
            (f64::from(min[1]) - reach).floor() as i32,
        ),
        (
            (f64::from(max[0]) + reach).ceil() as i32,
            (f64::from(max[1]) + reach).ceil() as i32,
        ),
    )
}

fn physical(point: Point<f64, Logical>, scale: Scale<f64>) -> [f32; 2] {
    let point = point.to_physical(scale);
    [point.x as f32, point.y as f32]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::morph::{Choreography, Edge, IconOrigin, MorphEnd};

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Rectangle<f64, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    fn morph_on(end: MorphEnd) -> Morph {
        let icon = IconOrigin {
            rect: rect(500.0, 1000.0, 64.0, 64.0),
            radius: 14.0,
            edge: Some(Edge::Bottom),
            carrier: None,
        };
        let mut morph = Morph::new(icon, end);
        morph.step(0.0, &Choreography::LIQUID);
        morph
    }

    #[test]
    fn at_rest_on_the_window_the_shape_covers_exactly_the_window_in_the_canvas() {
        let window = rect(100.0, 50.0, 800.0, 600.0);
        let scale = Scale::from(1.5);
        let canvas = Rectangle::new((148.0, 73.0).into(), (1204.0, 904.0).into());
        let uniforms = uniforms(&morph_on(MorphEnd::Window), window, 8.0, canvas, scale);
        assert_eq!(uniforms.corners[0], [150.0, 75.0]);
        assert_eq!(uniforms.corners[3], [1350.0, 975.0]);
        assert!((uniforms.radius - 12.0).abs() < 1e-4);
        assert_eq!(uniforms.softness, 0.0);
        assert_eq!(uniforms.reveal, 1.0);
        let [u, v, w, h] = uniforms.content;
        assert!((u - 2.0 / 1204.0).abs() < 1e-6 && (v - 2.0 / 904.0).abs() < 1e-6);
        assert!((w - 1200.0 / 1204.0).abs() < 1e-6 && (h - 900.0 / 904.0).abs() < 1e-6);
    }

    #[test]
    fn the_bounds_hold_every_corner_and_the_softened_edge() {
        let uniforms = LiquidUniforms {
            corners: [[10.5, 20.0], [90.0, 18.0], [30.0, 200.0], [70.0, 200.0]],
            content: [0.0, 0.0, 1.0, 1.0],
            radius: 4.0,
            softness: 2.0,
            reveal: 0.0,
            icon_alpha: 0.0,
        };
        assert_eq!(
            bounds(&uniforms),
            Rectangle::from_extremities((7, 15), (93, 203))
        );
    }
}
