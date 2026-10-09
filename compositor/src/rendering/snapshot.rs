//! A window's last frame, kept on the GPU once its client lets go of it.
//!
//! A destroyed surface takes its textures with it, so a window that should
//! shrink away after it closed is rendered into a texture of its own while its
//! buffers are still attached. Drawing that texture afterwards is a plain
//! textured quad: it goes through the same rounded-corner program as a live
//! window, at any size, with no readback to the CPU.

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, Frame, Offscreen, Texture,
            damage::OutputDamageTracker,
            element::{
                AsRenderElements, Element, Id, Kind, RenderElement, UnderlyingStorage,
                surface::WaylandSurfaceRenderElement,
            },
            gles::{GlesError, GlesFrame, GlesRenderer, GlesTexture},
            multigpu::{Error as MultiError, MultiFrame},
            utils::{CommitCounter, OpaqueRegions},
        },
    },
    desktop::Window,
    utils::{
        Buffer as BufferCoords, Physical, Point, Rectangle, Scale, Size, Transform,
        user_data::UserDataMap,
    },
};

use crate::backend::render::{GbmGlesApi, KmsRenderer};

const TRANSPARENT: [f32; 4] = [0.0; 4];

/// The content of a window, cropped to its window geometry.
#[derive(Debug, Clone)]
pub struct Snapshot {
    texture: GlesTexture,
    /// Stable for the snapshot's life, so the damage tracker sees one element
    /// moving rather than a new one every frame.
    id: Id,
}

impl Snapshot {
    /// Renders `window` as it is now, at `scale`. `None` when it has nothing
    /// to show or the GPU refused.
    pub fn capture(
        renderer: &mut GlesRenderer,
        window: &Window,
        scale: Scale<f64>,
    ) -> Option<Self> {
        let geometry = window.geometry();
        if geometry.is_empty() {
            return None;
        }
        let size: Size<i32, Physical> = geometry.size.to_physical_precise_round(scale);
        let buffer = Size::<i32, BufferCoords>::from((size.w, size.h));
        let mut texture: GlesTexture = renderer.create_buffer(Fourcc::Abgr8888, buffer).ok()?;

        let origin = Point::default() - geometry.loc.to_physical_precise_round(scale);
        let elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
            window.render_elements(renderer, origin, scale, 1.0);
        if elements.is_empty() {
            return None;
        }
        let mut target = renderer.bind(&mut texture).ok()?;
        OutputDamageTracker::new(size, scale, Transform::Normal)
            .render_output(renderer, &mut target, 0, &elements, TRANSPARENT)
            .ok()?;
        drop(target);

        Some(Self {
            texture,
            id: Id::new(),
        })
    }
}

/// A snapshot drawn into `geometry`, cut to what of it is visible.
#[derive(Debug)]
pub struct SnapshotElement {
    snapshot: Snapshot,
    src: Rectangle<f64, BufferCoords>,
    geometry: Rectangle<i32, Physical>,
    alpha: f32,
}

impl SnapshotElement {
    /// The snapshot stretched over `content`, keeping only the part inside
    /// `clip`. `None` when none of it is.
    pub fn new(
        snapshot: &Snapshot,
        content: Rectangle<f64, Physical>,
        clip: Rectangle<i32, Physical>,
        alpha: f32,
    ) -> Option<Self> {
        let visible = content.intersection(clip.to_f64())?;
        let texture = snapshot.texture.size().to_f64();
        let (sx, sy) = (texture.w / content.size.w, texture.h / content.size.h);
        let src = Rectangle::new(
            (
                (visible.loc.x - content.loc.x) * sx,
                (visible.loc.y - content.loc.y) * sy,
            )
                .into(),
            (visible.size.w * sx, visible.size.h * sy).into(),
        );
        let geometry = visible.to_i32_round();
        (!geometry.is_empty()).then(|| Self {
            snapshot: snapshot.clone(),
            src,
            geometry,
            alpha,
        })
    }

    fn draw_gles(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        Frame::render_texture_from_to(
            frame,
            &self.snapshot.texture,
            src,
            dst,
            damage,
            opaque_regions,
            Transform::Normal,
            self.alpha,
        )
    }
}

impl Element for SnapshotElement {
    fn id(&self) -> &Id {
        &self.snapshot.id
    }

    fn current_commit(&self) -> CommitCounter {
        CommitCounter::default()
    }

    fn src(&self) -> Rectangle<f64, BufferCoords> {
        self.src
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.geometry
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        self.alpha
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }
}

impl RenderElement<GlesRenderer> for SnapshotElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        self.draw_gles(frame, src, dst, damage, opaque_regions)
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

impl<'render> RenderElement<KmsRenderer<'render>> for SnapshotElement {
    fn draw(
        &self,
        frame: &mut MultiFrame<'render, 'render, '_, '_, GbmGlesApi, GbmGlesApi>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&UserDataMap>,
    ) -> Result<(), MultiError<GbmGlesApi, GbmGlesApi>> {
        // Captured on the render device, which is the GLES context under the
        // multi frame: the texture never crosses GPUs.
        self.draw_gles(frame.as_mut(), src, dst, damage, opaque_regions)
            .map_err(MultiError::Render)
    }

    fn underlying_storage(
        &self,
        _renderer: &mut KmsRenderer<'render>,
    ) -> Option<UnderlyingStorage<'_>> {
        None
    }
}
