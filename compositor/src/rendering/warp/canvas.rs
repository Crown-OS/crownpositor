//! The decorated window, painted into a texture a warp can bend.

use std::{collections::HashMap, mem};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, Offscreen, Texture,
            damage::OutputDamageTracker,
            element::Id,
            gles::{GlesRenderer, GlesTexture, ffi},
            utils::CommitCounter,
        },
    },
    utils::{Buffer as BufferCoords, Logical, Physical, Point, Rectangle, Size, Transform},
};

use crate::{
    rendering::{
        corners::CornerMemory,
        morph::{Content, morph_elements},
        rounded::GlesDecorator,
        warp::WarpRequest,
    },
    shell::morph::Morph,
    utils::id::WindowId,
};

/// Transparent border round the window, physical pixels: the edges a warp
/// bends are then filtered against it instead of cut off hard.
const PAD: i32 = 2;
const TRANSPARENT: [f32; 4] = [0.0; 4];

/// One output's canvases, kept only while a warp draws from them.
#[derive(Debug, Default)]
pub struct WarpCanvases(HashMap<WindowId, Canvas>);

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
        self.0.retain(|_, canvas| mem::take(&mut canvas.used));
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

        let id = request.tile.id();
        if self
            .0
            .get(&id)
            .is_none_or(|canvas| canvas.texture.size() != buffer)
        {
            let texture = gles.create_buffer(Fourcc::Abgr8888, buffer).ok()?;
            let element_id = self
                .0
                .get(&id)
                .map_or_else(Id::new, |canvas| canvas.id.clone());
            self.0.insert(
                id,
                Canvas {
                    texture,
                    id: element_id,
                    commit: CommitCounter::default(),
                    painted: false,
                    used: false,
                },
            );
        }
        let canvas = self.0.get_mut(&id)?;

        if matches!(request.content, Content::Live) || !canvas.painted {
            render_window(gles, &mut canvas.texture, request, size, animated.loc)?;
            canvas.painted = true;
        }
        canvas.used = true;
        canvas.commit.increment();

        let origin = window.loc.to_physical_precise_round(scale) - Point::from((PAD, PAD));
        Some(Painted {
            texture: canvas.texture.clone(),
            id: canvas.id.clone(),
            commit: canvas.commit,
            rect: Rectangle::new(origin, size).to_f64(),
        })
    }
}

/// Draws the window at rest, frame and all, with its top left `PAD` into
/// `texture`, then builds the mipmaps a warp shrinking it to an icon samples.
/// Glass is left out: there is no scene behind a canvas to blur.
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

    let mut target = gles.bind(texture).ok()?;
    OutputDamageTracker::new(size, scale, Transform::Normal)
        .render_output(gles, &mut target, 0, &elements, TRANSPARENT)
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
