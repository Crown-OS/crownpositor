//! The wallpaper behind the overview: blurred, dimmed, and creeping towards
//! the viewer, all following the overview's progress — a swipe that stops
//! halfway leaves it half blurred and half zoomed.

use smithay::{
    backend::renderer::{
        ImportAll, ImportMem, Renderer,
        element::{
            AsRenderElements,
            surface::WaylandSurfaceRenderElement,
            utils::{CropRenderElement, RescaleRenderElement},
        },
    },
    desktop::layer_map_for_output,
    utils::{Physical, Point},
    wayland::shell::wlr_layer::Layer,
};

use super::Painter;
use crate::{
    rendering::{
        blur::{Glass, GlassKind},
        decorate::{Backdrop, TileDecorator},
        element::CrownElement,
    },
    shell::monitor::Monitor,
};

impl<R, D> Painter<'_, R, D>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    pub(super) fn backdrop(&mut self, monitor: &Monitor) {
        let space = monitor.spacecontrol();
        let overview = space.overview();
        let output = self.physical(monitor.geometry().to_f64());

        // The blur grows with the swipe rather than a finished blur being
        // faded over a sharp wallpaper, which would read as a double image.
        let blur = overview.blur();
        if blur > 0.0 && self.decorator.blur_fingerprint().is_some() {
            let backdrop = Backdrop {
                id: space.ids().backdrop.clone(),
                commit: self.glass_commit(),
                geometry: output,
                mask: output,
                radius: 0.0,
                glass: dimmed(
                    self.decorator.glass(self.scale.x, GlassKind::Window),
                    overview.dim(),
                ),
                alpha: blur,
                strength: blur,
            };
            if let Some(pane) = self.decorator.backdrop(self.renderer, backdrop) {
                self.push(pane);
            }
        }

        // Scaled about the middle of the output, so the zoom pulls evenly
        // towards the viewer.
        let centre = output.loc + Point::from((output.size.w / 2, output.size.h / 2));
        let zoom = overview.background_scale();
        // A second guard for the same output deadlocks, so keep the scope tight.
        let map = layer_map_for_output(monitor.output());
        for layer in [Layer::Bottom, Layer::Background] {
            for surface in map.layers_on(layer).rev() {
                let Some(geometry) = map.layer_geometry(surface) else {
                    continue;
                };
                let location: Point<i32, Physical> =
                    geometry.loc.to_physical_precise_round(self.scale);
                let surfaces: Vec<WaylandSurfaceRenderElement<R>> =
                    surface.render_elements(self.renderer, location, self.scale, 1.0);
                for element in surfaces {
                    let scaled = RescaleRenderElement::from_element(element, centre, zoom);
                    // The zoom pushes the wallpaper past the screen's edges;
                    // the crop keeps it off a neighbouring output.
                    if let Some(cropped) =
                        CropRenderElement::from_element(scaled, self.scale, output)
                    {
                        self.elements.push(CrownElement::Scaled(cropped));
                    }
                }
            }
        }
    }
}

/// The wallpaper's glass: `glass` with a black wash of `dim` folded into its
/// tint — the same pixels as a separate quad laid over it, without anything
/// between the backdrop and the glass in front of it — and no rim, because
/// its edges are the screen's.
fn dimmed(glass: Glass, dim: f32) -> Glass {
    let [red, green, blue, alpha] = glass.tint;
    let washed = 1.0 - (1.0 - alpha) * (1.0 - dim);
    let kept = if washed > 0.0 {
        alpha * (1.0 - dim) / washed
    } else {
        0.0
    };
    Glass {
        tint: [red * kept, green * kept, blue * kept, washed],
        rim: 0.0,
        ..glass
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `mix(colour, tint.rgb, tint.a)`, as the finish shader applies it.
    fn tinted(colour: f32, tint: [f32; 4]) -> f32 {
        colour * (1.0 - tint[3]) + tint[0] * tint[3]
    }

    #[test]
    fn the_folded_dim_matches_a_wash_laid_over_the_glass() {
        let glass = Glass::default();
        for dim in [0.0, 0.2, 0.4] {
            for colour in [0.0, 0.3, 1.0] {
                let washed = tinted(colour, glass.tint) * (1.0 - dim);
                let folded = tinted(colour, dimmed(glass, dim).tint);
                assert!(
                    (washed - folded).abs() < 1e-6,
                    "{dim} {colour}: {washed} {folded}"
                );
            }
        }
    }

    #[test]
    fn the_wallpaper_glass_has_no_rim() {
        assert_eq!(
            dimmed(
                Glass {
                    rim: 3.0,
                    ..Glass::default()
                },
                0.4
            )
            .rim,
            0.0
        );
    }
}
