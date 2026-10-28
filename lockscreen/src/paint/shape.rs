use tiny_skia::{
    Color, FillRule, GradientStop, LinearGradient, Paint, Path, PathBuilder, Pixmap, Point,
    SpreadMode, Stroke, Transform,
};

use crate::palette::Rgba;

/// Control-point distance that makes a cubic Bézier hug a quarter circle.
const KAPPA: f32 = 0.552_284_8;

pub(super) fn rounded_rect(x: f32, y: f32, width: f32, height: f32, radius: f32) -> Option<Path> {
    let r = radius.clamp(0.0, width.min(height) / 2.0);
    let k = r * (1.0 - KAPPA);
    let (right, bottom) = (x + width, y + height);
    let mut path = PathBuilder::new();
    path.move_to(x + r, y);
    path.line_to(right - r, y);
    path.cubic_to(right - k, y, right, y + k, right, y + r);
    path.line_to(right, bottom - r);
    path.cubic_to(right, bottom - k, right - k, bottom, right - r, bottom);
    path.line_to(x + r, bottom);
    path.cubic_to(x + k, bottom, x, bottom - k, x, bottom - r);
    path.line_to(x, y + r);
    path.cubic_to(x, y + k, x + k, y, x + r, y);
    path.close();
    path.finish()
}

pub(super) fn paint(color: Rgba) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(colour(color));
    paint.anti_alias = true;
    paint
}

pub(super) fn colour([r, g, b, a]: Rgba) -> Color {
    Color::from_rgba(r, g, b, a).unwrap_or(Color::TRANSPARENT)
}

pub(super) fn fill(pixmap: &mut Pixmap, path: &Path, color: Rgba) {
    pixmap.fill_path(
        path,
        &paint(color),
        FillRule::Winding,
        Transform::identity(),
        None,
    );
}

pub(super) fn stroke(pixmap: &mut Pixmap, path: &Path, width: f32, color: Rgba) {
    let stroke = Stroke {
        width,
        ..Stroke::default()
    };
    pixmap.stroke_path(path, &paint(color), &stroke, Transform::identity(), None);
}

pub(super) fn gradient_fill(
    pixmap: &mut Pixmap,
    path: &Path,
    from: Point,
    to: Point,
    stops: [Rgba; 2],
) {
    let Some(shader) = LinearGradient::new(
        from,
        to,
        vec![
            GradientStop::new(0.0, colour(stops[0])),
            GradientStop::new(1.0, colour(stops[1])),
        ],
        SpreadMode::Pad,
        Transform::identity(),
    ) else {
        return;
    };
    let paint = Paint {
        shader,
        anti_alias: true,
        ..Paint::default()
    };
    pixmap.fill_path(path, &paint, FillRule::Winding, Transform::identity(), None);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pill_is_round_at_the_ends() {
        let mut pixmap = Pixmap::new(40, 20).expect("size");
        let path = rounded_rect(0.0, 0.0, 40.0, 20.0, 99.0).expect("path");
        fill(&mut pixmap, &path, [1.0; 4]);
        assert_eq!(pixmap.pixel(0, 0).map(|p| p.alpha()), Some(0));
        assert_eq!(pixmap.pixel(20, 10).map(|p| p.alpha()), Some(255));
    }
}
