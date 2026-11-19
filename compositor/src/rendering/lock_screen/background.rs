//! The lock's background — the live wallpaper, a picture or a colour —
//! zoomed about the output's centre, flooding out over the desktop as the
//! lock arrives, and the blur over it as it arrives and behind the prompt.

use lockscreen::{image::DESKTOP_WALLPAPER_ZOOM, transition::LayerStyle};
use smithay::{
    backend::renderer::{
        ImportAll, ImportMem, Renderer,
        element::{
            AsRenderElements, Kind, Wrap,
            memory::MemoryRenderBufferRenderElement,
            surface::WaylandSurfaceRenderElement,
            utils::{CropRenderElement, RescaleRenderElement},
        },
        utils::CommitCounter,
    },
    desktop::layer_map_for_output,
    utils::{Logical, Physical, Point, Rectangle, Scale, Size},
    wayland::shell::wlr_layer::Layer,
};

use super::{VISIBLE_BLUR, quantise, solid};
use crate::{
    rendering::{
        Elements,
        blur::{Glass, Shading},
        decorate::{Backdrop as GlassBackdrop, Flood, Flooding, TileDecorator},
        element::CrownElement,
        unstretched::Unstretched,
    },
    shell::{
        lock_screen::{ActiveLock, Backdrop, OutputPicture, OutputVisuals},
        monitor::Monitor,
    },
};

pub(super) struct Background<'a> {
    pub lock: &'a ActiveLock,
    pub monitor: &'a Monitor,
    pub visuals: Option<&'a OutputVisuals>,
    pub scale: Scale<f64>,
    pub style: LayerStyle,
    /// How far it has flooded out, while it still is.
    pub flood: Option<f32>,
    /// Where it floods out from, as a fraction of the output across and down.
    pub source: [f32; 2],
}

impl Background<'_> {
    pub fn output(&self) -> Rectangle<i32, Physical> {
        Rectangle::from_size(
            self.monitor
                .geometry()
                .size
                .to_physical_precise_round(self.scale),
        )
    }

    pub fn logical_size(&self) -> Size<i32, Logical> {
        self.monitor.geometry().size
    }

    /// Never below 1, so the background always covers the output.
    fn zoom(&self) -> f32 {
        self.style.scale.max(1.0)
    }

    /// What the subject, cut from the background's picture, is zoomed by to
    /// land on the background pixel for pixel.
    pub fn subject_zoom(&self) -> f32 {
        match self.lock.backdrop {
            Backdrop::Wallpaper => self.zoom() * DESKTOP_WALLPAPER_ZOOM,
            Backdrop::Picture | Backdrop::Color(_) => self.zoom(),
        }
    }

    /// The blur over the background. It stays opaque as the background
    /// fades, its reach shrinking instead: glass fading over the desktop's
    /// own glass would blur that glass's blur. A colour has nothing to blur.
    pub fn blur<R, D>(&self, renderer: &mut R, decorator: &mut D) -> Option<D::Element>
    where
        R: Renderer + ImportAll,
        D: TileDecorator<R>,
    {
        if matches!(self.lock.backdrop, Backdrop::Color(_)) {
            return None;
        }
        let sigma = self.style.blur * self.style.opacity * self.scale.x as f32;
        if sigma < VISIBLE_BLUR {
            return None;
        }
        let fingerprint = decorator.blur_fingerprint()?;
        let strength = decorator.blur_strength_for_sigma(sigma);
        let output = self.output();
        decorator.backdrop(
            renderer,
            GlassBackdrop {
                id: self.lock.ids.blur.clone(),
                commit: CommitCounter::from(fingerprint as usize ^ quantise(strength)),
                geometry: output,
                mask: output,
                radius: 0.0,
                glass: clear_glass(),
                alpha: 1.0,
                strength,
            },
        )
    }

    /// Returns whether the desktop shows around it, as it floods in.
    pub fn draw<R, D>(
        &self,
        elements: &mut Elements<R, D>,
        renderer: &mut R,
        decorator: &mut D,
    ) -> bool
    where
        R: Renderer + ImportAll + ImportMem,
        R::TextureId: Send + Clone + 'static,
        D: TileDecorator<R>,
    {
        let alpha = self.style.opacity;
        match self.lock.backdrop {
            Backdrop::Wallpaper if self.lock.flow.is_unlocking() => false,
            Backdrop::Wallpaper => self.wallpaper(elements, renderer, decorator),
            Backdrop::Picture => self
                .visuals
                .and_then(|visuals| visuals.picture.as_ref())
                .and_then(|picture| {
                    let size = self.logical_size();
                    covering(
                        renderer,
                        picture,
                        Point::default(),
                        size,
                        self.zoom(),
                        alpha,
                    )
                })
                .map_or(self.flood.is_some(), |picture| {
                    self.pour(elements, renderer, decorator, Flooding::Picture(picture), 0)
                }),
            Backdrop::Color([red, green, blue, opacity]) => {
                let alpha = opacity * alpha;
                elements.push(solid(
                    &self.lock.ids.color,
                    self.output(),
                    [red * alpha, green * alpha, blue * alpha, alpha],
                ));
                false
            }
        }
    }

    /// The desktop's own wallpaper surfaces, zoomed about the output's centre.
    /// Not while unlocking: the desktop beneath draws the same surfaces, and
    /// an element twice in one frame confuses the damage tracker — which is
    /// why the flood draws them under ids of its own.
    fn wallpaper<R, D>(
        &self,
        elements: &mut Elements<R, D>,
        renderer: &mut R,
        decorator: &mut D,
    ) -> bool
    where
        R: Renderer + ImportAll + ImportMem,
        R::TextureId: Send + Clone + 'static,
        D: TileDecorator<R>,
    {
        let output = self.output();
        let centre = output.loc + Point::from((output.size.w / 2, output.size.h / 2));
        let zoom = f64::from(self.zoom());
        // A second guard for the same output deadlocks, so keep the scope tight.
        let map = layer_map_for_output(self.monitor.output());
        let mut pieces = 0;
        let mut floods = self.flood.is_some();
        for surface in map.layers_on(Layer::Background).rev() {
            let Some(geometry) = map.layer_geometry(surface) else {
                continue;
            };
            let location: Point<i32, Physical> = geometry.loc.to_physical_precise_round(self.scale);
            let surfaces: Vec<WaylandSurfaceRenderElement<R>> =
                surface.render_elements(renderer, location, self.scale, self.style.opacity);
            for element in surfaces {
                let zoomed = RescaleRenderElement::from_element(element.into(), centre, zoom);
                if let Some(cropped) = CropRenderElement::from_element(zoomed, self.scale, output) {
                    floods &= self.pour(
                        elements,
                        renderer,
                        decorator,
                        Flooding::Surface(cropped),
                        pieces,
                    );
                    pieces += 1;
                }
            }
        }
        floods
    }

    /// `piece` of the background, the `index`th, as far as it has flooded.
    /// Returns whether it left the desktop showing: drawn whole, it covers it.
    fn pour<R, D>(
        &self,
        elements: &mut Elements<R, D>,
        renderer: &mut R,
        decorator: &mut D,
        piece: Flooding<R>,
        index: usize,
    ) -> bool
    where
        R: Renderer + ImportAll + ImportMem,
        R::TextureId: Send + Clone + 'static,
        D: TileDecorator<R>,
    {
        let flood = self.flood.zip(self.lock.ids.flood.get(index));
        let piece = match flood {
            Some((level, id)) => {
                let flood = Flood {
                    id: id.clone(),
                    output: self.output(),
                    source: self.source,
                    level,
                };
                match decorator.flood(renderer, piece, flood) {
                    Ok(flooded) => {
                        elements.push(CrownElement::Tile(Wrap::from(flooded)));
                        return true;
                    }
                    Err(piece) => piece,
                }
            }
            None => piece,
        };
        elements.push(match piece {
            Flooding::Surface(surface) => CrownElement::Scaled(surface),
            Flooding::Picture(picture) => Unstretched::from(picture).into(),
        });
        false
    }
}

/// `picture` over the whole `size` output from `origin`, zoomed `zoom` about
/// its centre by cropping what it samples, as crownpaper's viewport crops the
/// wallpaper: at 1 it draws pixel for pixel.
pub(super) fn covering<R>(
    renderer: &mut R,
    picture: &OutputPicture,
    origin: Point<f64, Physical>,
    size: Size<i32, Logical>,
    zoom: f32,
    alpha: f32,
) -> Option<MemoryRenderBufferRenderElement<R>>
where
    R: Renderer + ImportMem,
    R::TextureId: Send + Clone + 'static,
{
    MemoryRenderBufferRenderElement::from_buffer(
        renderer,
        origin,
        &picture.buffer,
        Some(alpha),
        Some(centre_crop(picture.pixels, zoom)),
        Some(size),
        Kind::Unspecified,
    )
    .ok()
}

/// The middle of a `pixels` picture that fills it at `zoom`, never below 1.
fn centre_crop(pixels: [u32; 2], zoom: f32) -> Rectangle<f64, Logical> {
    let zoom = f64::from(zoom.max(1.0));
    let [width, height] = pixels.map(f64::from);
    let (shown_width, shown_height) = (width / zoom, height / zoom);
    Rectangle::new(
        ((width - shown_width) / 2.0, (height - shown_height) / 2.0).into(),
        (shown_width, shown_height).into(),
    )
}

/// Blur and nothing else: no tint, no rim, the background's own colours.
fn clear_glass() -> Glass {
    Glass {
        tint: [0.0; 4],
        tint_inset: 0.0,
        saturation: 1.0,
        rim: 0.0,
        shading: Shading::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unzoomed_picture_is_sampled_whole() {
        assert_eq!(
            centre_crop([1920, 1080], 1.0),
            Rectangle::from_size((1920.0, 1080.0).into())
        );
        assert_eq!(
            centre_crop([1920, 1080], 0.9),
            centre_crop([1920, 1080], 1.0)
        );
    }

    /// crownpaper crops `1 / zoom` of the covered picture out of its middle.
    #[test]
    fn a_zoomed_picture_is_cropped_about_its_centre() {
        let crop = centre_crop([2000, 1000], 2.0);
        assert_eq!(
            crop,
            Rectangle::new((500.0, 250.0).into(), (1000.0, 500.0).into())
        );
    }
}
