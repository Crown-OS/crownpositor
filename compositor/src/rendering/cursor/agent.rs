//! An agent's cursor: an arrow in the agent's colour with a flag carrying its
//! name, the way collaborative editors show who else is in the document.

use std::collections::HashMap;

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            ImportAll, ImportMem, Renderer,
            element::{
                Kind,
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
            },
        },
    },
    utils::{Logical, Physical, Point, Scale, Transform},
};
use tiny_skia::{FillRule, LineJoin, Paint, PathBuilder, Pixmap, Rect, Stroke};

use crate::rendering::{
    cursor::builtin::ARROW, decoration::label::Label, unstretched::Unstretched,
};

/// The arrow's logical size, the same as a themed cursor.
const ARROW_SIZE: f64 = 24.0;
/// Where the flag hangs off the arrow, as a fraction of its size.
const FLAG_OFFSET: (f64, f64) = (0.6, 0.9);
const FLAG_PADDING: f64 = 5.0;
const FLAG_RADIUS: f64 = 6.0;
const SCALE_QUANTUM: f64 = 1000.0;

pub struct AgentOverlay {
    pub position: Point<f64, Logical>,
    color: [u8; 3],
    /// The name, rasterised once per output scale by the compositor's text
    /// renderer, keyed by scale in thousandths.
    labels: HashMap<u32, Label>,
    arrows: HashMap<u32, MemoryRenderBuffer>,
    flags: HashMap<(u32, i32, i32), MemoryRenderBuffer>,
}

impl AgentOverlay {
    pub fn new(position: Point<f64, Logical>, color: u32, labels: Vec<(f64, Label)>) -> Self {
        Self {
            position,
            color: [(color >> 16) as u8, (color >> 8) as u8, color as u8],
            labels: labels
                .into_iter()
                .map(|(scale, label)| (scale_key(scale), label))
                .collect(),
            arrows: HashMap::new(),
            flags: HashMap::new(),
        }
    }

    pub fn render<R, E>(
        &mut self,
        elements: &mut Vec<E>,
        renderer: &mut R,
        local: Point<f64, Logical>,
        scale: Scale<f64>,
    ) where
        R: Renderer + ImportAll + ImportMem,
        R::TextureId: Send + Clone + 'static,
        E: From<Unstretched<MemoryRenderBufferRenderElement<R>>>,
    {
        let pixels = (ARROW_SIZE * scale.x).round().max(1.0) as u32;
        let color = self.color;
        let arrow = self
            .arrows
            .entry(pixels)
            .or_insert_with(|| raster_arrow(pixels, color));
        let tip = local.to_physical(scale).to_i32_round::<i32>().to_f64();
        push(elements, renderer, tip, arrow, scale);

        let Some(label) = self.labels.get(&scale_key(scale.x)) else {
            return;
        };
        let padding = (FLAG_PADDING * scale.x).round() as i32;
        let width = label.size.w + 2 * padding;
        let height = label.size.h + 2 * padding;
        let flag = self
            .flags
            .entry((pixels, width, height))
            .or_insert_with(|| raster_flag(width, height, FLAG_RADIUS * scale.x, color));
        let origin = tip
            + Point::<f64, Physical>::from((
                (ARROW_SIZE * FLAG_OFFSET.0 * scale.x).round(),
                (ARROW_SIZE * FLAG_OFFSET.1 * scale.x).round(),
            ));
        push(elements, renderer, origin, flag, scale);
        push(
            elements,
            renderer,
            origin + Point::from((f64::from(padding), f64::from(padding))),
            &label.buffer,
            scale,
        );
    }
}

fn push<R, E>(
    elements: &mut Vec<E>,
    renderer: &mut R,
    location: Point<f64, Physical>,
    buffer: &MemoryRenderBuffer,
    scale: Scale<f64>,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Send + Clone + 'static,
    E: From<Unstretched<MemoryRenderBufferRenderElement<R>>>,
{
    match MemoryRenderBufferRenderElement::from_buffer(
        renderer,
        location,
        buffer,
        None,
        None,
        None,
        Kind::Cursor,
    ) {
        Ok(element) => elements.push(E::from(Unstretched::at_own_size(element, scale))),
        Err(_) => tracing::warn!("failed to import an agent cursor"),
    }
}

fn scale_key(scale: f64) -> u32 {
    (scale * SCALE_QUANTUM).round() as u32
}

fn buffer_from(pixmap: Pixmap) -> MemoryRenderBuffer {
    let (width, height) = (pixmap.width() as i32, pixmap.height() as i32);
    MemoryRenderBuffer::from_slice(
        pixmap.data(),
        Fourcc::Abgr8888,
        (width, height),
        1,
        Transform::Normal,
        None,
    )
}

/// The built-in arrow filled with the agent's colour and outlined in white,
/// so it reads against any desktop and never passes for the user's own.
fn raster_arrow(size: u32, color: [u8; 3]) -> MemoryRenderBuffer {
    let mut pixmap = Pixmap::new(size, size).unwrap_or_else(|| Pixmap::new(1, 1).expect("1x1"));
    let width = (size as f32 / 14.0).max(1.0);
    let span = size as f32 - 2.0 * width;
    let mut builder = PathBuilder::new();
    for (index, (x, y)) in ARROW.iter().enumerate() {
        let (x, y) = (width + x * span, width + y * span);
        if index == 0 {
            builder.move_to(x, y);
        } else {
            builder.line_to(x, y);
        }
    }
    builder.close();
    if let Some(path) = builder.finish() {
        let mut fill = Paint::default();
        fill.set_color_rgba8(color[0], color[1], color[2], 255);
        fill.anti_alias = true;
        pixmap.fill_path(
            &path,
            &fill,
            FillRule::Winding,
            tiny_skia::Transform::identity(),
            None,
        );
        let mut outline = Paint::default();
        outline.set_color_rgba8(255, 255, 255, 255);
        outline.anti_alias = true;
        pixmap.stroke_path(
            &path,
            &outline,
            &Stroke {
                width,
                line_join: LineJoin::Round,
                ..Stroke::default()
            },
            tiny_skia::Transform::identity(),
            None,
        );
    }
    buffer_from(pixmap)
}

fn raster_flag(width: i32, height: i32, radius: f64, color: [u8; 3]) -> MemoryRenderBuffer {
    let mut pixmap = Pixmap::new(width.max(1) as u32, height.max(1) as u32)
        .unwrap_or_else(|| Pixmap::new(1, 1).expect("1x1"));
    let mut fill = Paint::default();
    fill.set_color_rgba8(color[0], color[1], color[2], 235);
    fill.anti_alias = true;
    if let Some(rect) = Rect::from_xywh(0.0, 0.0, width as f32, height as f32)
        && let Some(path) = rounded(rect, radius as f32)
    {
        pixmap.fill_path(
            &path,
            &fill,
            FillRule::Winding,
            tiny_skia::Transform::identity(),
            None,
        );
    }
    buffer_from(pixmap)
}

fn rounded(rect: Rect, radius: f32) -> Option<tiny_skia::Path> {
    let radius = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let (left, top, right, bottom) = (rect.left(), rect.top(), rect.right(), rect.bottom());
    let mut builder = PathBuilder::new();
    builder.move_to(left + radius, top);
    builder.line_to(right - radius, top);
    builder.quad_to(right, top, right, top + radius);
    builder.line_to(right, bottom - radius);
    builder.quad_to(right, bottom, right - radius, bottom);
    builder.line_to(left + radius, bottom);
    builder.quad_to(left, bottom, left, bottom - radius);
    builder.line_to(left, top + radius);
    builder.quad_to(left, top, left + radius, top);
    builder.close();
    builder.finish()
}
