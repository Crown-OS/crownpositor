//! The decorated window, painted into a texture a warp can bend.

use std::{collections::HashMap, mem};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, Offscreen, Texture,
            damage::OutputDamageTracker,
            element::{Id, Kind, RenderElement},
            gles::{GlesRenderer, GlesTexture, ffi},
            utils::CommitCounter,
        },
    },
    utils::{Buffer as BufferCoords, Logical, Physical, Point, Rectangle, Scale, Size, Transform},
};

use crate::{
    rendering::{
        corners::CornerMemory,
        morph::{Content, morph_elements},
        rounded::GlesDecorator,
        surface_tree::transformed_surface_elements,
        warp::{WarpRequest, carried::CarriedIcons},
    },
    shell::{morph::Morph, tile::Tile},
    utils::id::WindowId,
};

/// Transparent border round the window, physical pixels: the edges a warp
/// bends are then filtered against it instead of cut off hard.
const PAD: i32 = 2;
const TRANSPARENT: [f32; 4] = [0.0; 4];

/// One output's canvases, kept only while a warp draws from them, and the
/// launcher icons its morphs carry.
#[derive(Debug, Default)]
pub struct WarpCanvases {
    canvases: HashMap<WindowId, Canvas>,
    pub(super) carried: CarriedIcons,
}

#[derive(Debug)]
struct Canvas {
    texture: GlesTexture,
    id: Id,
    commit: CommitCounter,
    painted: bool,
    used: bool,
}

/// A canvas ready to draw from this frame.
#[derive(Debug, Clone)]
pub struct Painted {
    pub texture: GlesTexture,
    pub id: Id,
    pub commit: CommitCounter,
    /// Where the canvas covers the window at rest, padding included:
    /// output-local physical pixels.
    pub rect: Rectangle<f64, Physical>,
}

impl WarpCanvases {
    /// Lets go of the canvases no warp drew from last frame.
    pub fn begin_frame(&mut self) {
        self.canvases
            .retain(|_, canvas| mem::take(&mut canvas.used));
        self.carried.begin_frame();
    }

    /// The window `request` names, painted as it stands. A live window is
    /// repainted every frame so a playing video keeps playing; a closed one's
    /// last frame is painted once.
    pub(super) fn paint(
        &mut self,
        gles: &mut GlesRenderer,
        request: &mut WarpRequest<'_, '_>,
    ) -> Option<Painted> {
        let scale = request.scale;
        let animated = request.tile.render_rect();
        let window = Rectangle::new(animated.loc + request.offset, animated.size);
        let size = window.size.to_physical_precise_round(scale) + Size::from((2 * PAD, 2 * PAD));
        let buffer = Size::<i32, BufferCoords>::from((size.w, size.h));

        let canvas = self.sized(gles, request.tile.id(), buffer)?;
        if matches!(request.content, Content::Live) || !canvas.painted {
            render_window(gles, &mut canvas.texture, request, size, animated.loc)?;
            canvas.painted = true;
        }

        let origin = window.loc.to_physical_precise_round(scale) - Point::from((PAD, PAD));
        Some(canvas.drawn(Rectangle::new(origin, size).to_f64()))
    }

    /// `tile`'s client content alone, at the size the client last drew it,
    /// to be drawn over `rect`.
    pub(super) fn paint_content(
        &mut self,
        gles: &mut GlesRenderer,
        tile: &Tile,
        scale: Scale<f64>,
        rect: Rectangle<i32, Physical>,
    ) -> Option<Painted> {
        let size = tile
            .window()
            .geometry()
            .size
            .to_physical_precise_round(scale);
        if size.is_empty() {
            return None;
        }
        let canvas = self.sized(gles, tile.id(), (size.w, size.h).into())?;
        render_content(gles, &mut canvas.texture, tile, size, scale)?;
        Some(canvas.drawn(rect.to_f64()))
    }

    /// `window`'s canvas, reallocated when it is not `size` texels big.
    fn sized(
        &mut self,
        gles: &mut GlesRenderer,
        window: WindowId,
        size: Size<i32, BufferCoords>,
    ) -> Option<&mut Canvas> {
        if self
            .canvases
            .get(&window)
            .is_none_or(|canvas| canvas.texture.size() != size)
        {
            let texture = gles.create_buffer(Fourcc::Abgr8888, size).ok()?;
            let id = self
                .canvases
                .get(&window)
                .map_or_else(Id::new, |canvas| canvas.id.clone());
            self.canvases.insert(
                window,
                Canvas {
                    texture,
                    id,
                    commit: CommitCounter::default(),
                    painted: false,
                    used: false,
                },
            );
        }
        self.canvases.get_mut(&window)
    }
}

impl Canvas {
    fn drawn(&mut self, rect: Rectangle<f64, Physical>) -> Painted {
        self.used = true;
        self.commit.increment();
        Painted {
            texture: self.texture.clone(),
            id: self.id.clone(),
            commit: self.commit,
            rect,
        }
    }
}

/// Draws the window at rest, frame and all, with its top left `PAD` into
/// `texture`. Glass is left out: there is no scene behind a canvas to blur.
fn render_window(
    gles: &mut GlesRenderer,
    texture: &mut GlesTexture,
    request: &mut WarpRequest<'_, '_>,
    size: Size<i32, Physical>,
    at: Point<f64, Logical>,
) -> Option<()> {
    let scale = request.scale;
    let pad = f64::from(PAD) / scale.x;
    let offset = Point::<f64, Logical>::from((pad, pad)) - at;

    let mut corners = CornerMemory::default();
    let mut canvases = WarpCanvases::default();
    let mut decorator = GlesDecorator::new(None, &mut corners, &mut canvases);
    let mut elements = Vec::new();
    morph_elements(
        &mut elements,
        request.shell,
        request.tile,
        &Morph::resting(),
        request.content,
        gles,
        &mut decorator,
        scale,
        offset,
        request.radius,
        request.style,
    );

    fill(gles, texture, &elements, size, scale)
}

/// Draws the client's surfaces with its window geometry's corner at the
/// texture's.
fn render_content(
    gles: &mut GlesRenderer,
    texture: &mut GlesTexture,
    tile: &Tile,
    size: Size<i32, Physical>,
    scale: Scale<f64>,
) -> Option<()> {
    let origin = Point::default()
        - tile
            .window()
            .geometry()
            .loc
            .to_physical_precise_round(scale);
    let elements =
        transformed_surface_elements(gles, tile.surface(), origin, scale, 1.0, Kind::Unspecified);
    fill(gles, texture, &elements, size, scale)
}

/// Replaces `texture` with `elements`, then builds the mipmaps a shrinking
/// or blurring draw samples.
fn fill<E: RenderElement<GlesRenderer>>(
    gles: &mut GlesRenderer,
    texture: &mut GlesTexture,
    elements: &[E],
    size: Size<i32, Physical>,
    scale: Scale<f64>,
) -> Option<()> {
    let mut target = gles.bind(texture).ok()?;
    OutputDamageTracker::new(size, scale, Transform::Normal)
        .render_output(gles, &mut target, 0, elements, TRANSPARENT)
        .ok()?;
    drop(target);

    let texture_id = texture.tex_id();
    gles.with_context(|gl| unsafe {
        gl.BindTexture(ffi::TEXTURE_2D, texture_id);
        gl.GenerateMipmap(ffi::TEXTURE_2D);
        gl.BindTexture(ffi::TEXTURE_2D, 0);
    })
    .ok()
}
