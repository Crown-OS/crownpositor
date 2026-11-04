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

use crate::{
    rendering::{decorate::TileDecorator, element::CrownElement, painter::Painter},
    shell::monitor::Monitor,
};

impl<R, D> Painter<'_, '_, R, D>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    pub(super) fn backdrop(&mut self, monitor: &Monitor) {
        let space = monitor.spacecontrol();
        let overview = space.overview();
        let output = self.physical(monitor.geometry().to_f64());

        self.frost(
            space.ids().backdrop.clone(),
            output,
            overview.blur(),
            overview.dim(),
        );

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
                    let scaled = RescaleRenderElement::from_element(element.into(), centre, zoom);
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
