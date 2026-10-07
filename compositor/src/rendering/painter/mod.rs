//! What every compositor-drawn scene draws with: SpaceControl and
//! WindowControl both place windows, glass and text through one [`Painter`].

mod thumbnail;

pub(in crate::rendering) use thumbnail::{Backing, Thumbnail};

use smithay::{
    backend::renderer::{
        ImportAll, ImportMem, Renderer,
        element::{Id, Kind, Wrap, memory::MemoryRenderBufferRenderElement},
        utils::CommitCounter,
    },
    utils::{Logical, Physical, Point, Rectangle, Scale},
};

use crate::rendering::{
    Elements, FrameStyle,
    blur::GlassKind,
    decorate::{Backdrop, TileDecorator},
    decoration::TextRenderer,
    element::CrownElement,
    logical,
};

pub(in crate::rendering) struct Painter<'a, R, D>
where
    R: Renderer + ImportAll + ImportMem,
    D: TileDecorator<R>,
{
    pub elements: &'a mut Elements<R, D>,
    pub renderer: &'a mut R,
    pub decorator: &'a mut D,
    pub text: &'a mut TextRenderer,
    pub scale: Scale<f64>,
    /// A window's corner radius at its own size, in physical pixels.
    pub radius: f32,
}

impl<'a, R, D> Painter<'a, R, D>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    D: TileDecorator<R>,
{
    pub fn new(
        elements: &'a mut Elements<R, D>,
        renderer: &'a mut R,
        decorator: &'a mut D,
        scale: Scale<f64>,
        style: &'a mut FrameStyle<'_>,
    ) -> Self {
        Self {
            elements,
            renderer,
            decorator,
            text: &mut *style.text,
            scale,
            radius: style.radius,
        }
    }

    pub fn push(&mut self, element: D::Element) {
        self.elements.push(CrownElement::Tile(Wrap::from(element)));
    }

    pub fn physical(&self, rect: Rectangle<f64, Logical>) -> Rectangle<i32, Physical> {
        Rectangle::new(
            rect.loc.to_physical_precise_round(self.scale),
            rect.size.to_physical_precise_round(self.scale),
        )
    }

    /// The commit compositor-drawn glass is drawn under: it changes exactly
    /// when the blur settings do. Everything else that moves its pixels — its
    /// rect, its alpha, what is under it — the damage tracker already sees.
    pub fn glass_commit(&self) -> CommitCounter {
        CommitCounter::from(self.decorator.blur_fingerprint().unwrap_or_default() as usize)
    }

    pub(in crate::rendering) fn glass(
        &mut self,
        id: Id,
        kind: GlassKind,
        geometry: Rectangle<i32, Physical>,
        radius: f32,
        shrink: f64,
        alpha: f32,
    ) {
        if geometry.is_empty() || alpha <= 0.0 {
            return;
        }
        let backdrop = Backdrop {
            id,
            commit: self.glass_commit(),
            geometry,
            mask: geometry,
            radius,
            glass: self.decorator.glass(self.scale.x * shrink, kind),
            alpha,
            strength: self.decorator.blur_strength_for(shrink),
        };
        if let Some(glass) = self.decorator.backdrop(self.renderer, backdrop) {
            self.push(glass);
        }
    }

    /// Text rasterised by the compositor, centred on `centre`.
    pub(in crate::rendering) fn label(
        &mut self,
        text: &str,
        centre: Point<f64, Logical>,
        colour: [f32; 4],
        alpha: f32,
    ) {
        let scale = self.scale;
        let Some(label) = self.text.label(text, scale.x, colour, true) else {
            return;
        };
        let size = logical(label.size);
        let origin = Point::<f64, Physical>::from((
            ((centre.x - f64::from(size.w) / 2.0) * scale.x).round(),
            ((centre.y - f64::from(size.h) / 2.0) * scale.y).round(),
        ));
        if let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
            self.renderer,
            origin,
            &label.buffer,
            Some(alpha),
            Some(Rectangle::from_size(size.to_f64())),
            Some(size),
            Kind::Unspecified,
        ) {
            self.elements.push(CrownElement::Memory(element));
        }
    }
}
