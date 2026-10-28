use resvg::usvg;
use tiny_skia::{Pixmap, Transform};

use crate::palette::Rgba;

/// A Lucide icon body stroked in `color`, `size` pixels square at `origin`.
pub(super) fn draw(pixmap: &mut Pixmap, body: &str, color: Rgba, origin: f32, size: f32) {
    let [r, g, b, a] = color.map(|channel| channel.clamp(0.0, 1.0));
    let hex = format!(
        "#{:02x}{:02x}{:02x}",
        (r * 255.0).round() as u8,
        (g * 255.0).round() as u8,
        (b * 255.0).round() as u8
    );
    let svg = crownplugin_view::to_svg(body, 24.0)
        .replacen("currentColor", &hex, 1)
        .replacen("<svg ", &format!("<svg stroke-opacity=\"{a}\" "), 1);
    let Ok(tree) = usvg::Tree::from_str(&svg, &usvg::Options::default()) else {
        return;
    };
    let scale = size / 24.0;
    resvg::render(
        &tree,
        Transform::from_row(scale, 0.0, 0.0, scale, origin, origin),
        &mut pixmap.as_mut(),
    );
}
