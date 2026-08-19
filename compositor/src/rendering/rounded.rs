//! Rounded corners.
//!
//! The shader is a per-*draw* override on the GLES frame, not a property of an
//! element, so this wraps an element and brackets its draw call: bind the
//! program, draw, unbind. Without the unbind every element after it in the list
//! would be drawn with rounded corners too.

use smithay::{
    backend::renderer::{
        element::{Element, Id, Kind, RenderElement, UnderlyingStorage},
        gles::{GlesError, GlesFrame, GlesRenderer, GlesTexProgram},
        utils::{CommitCounter, DamageSet, OpaqueRegions},
    },
    utils::{Buffer as BufferCoords, Physical, Point, Rectangle, Scale, Transform},
};

use crate::{
    rendering::decorate::{Cropped, TileDecorator},
    shaders::rounded_corner::RoundedCornerShader,
};

#[derive(Debug)]
pub struct Rounded<E> {
    inner: E,
    /// Resolved up front: `GlesFrame` does not expose its renderer, so the
    /// program cannot be looked up from inside `draw`.
    program: GlesTexProgram,
    /// The window's size in physical pixels; the shader needs it to know where
    /// the corners are.
    size: (f32, f32),
    radius: f32,
}

impl<E> Rounded<E> {
    pub fn new(inner: E, program: GlesTexProgram, size: (f32, f32), radius: f32) -> Self {
        Self {
            inner,
            program,
            size,
            radius,
        }
    }
}

impl<E: Element> Element for Rounded<E> {
    fn id(&self) -> &Id {
        self.inner.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }

    fn src(&self) -> Rectangle<f64, BufferCoords> {
        self.inner.src()
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.inner.geometry(scale)
    }

    fn location(&self, scale: Scale<f64>) -> Point<i32, Physical> {
        self.inner.location(scale)
    }

    fn transform(&self) -> Transform {
        self.inner.transform()
    }

    fn damage_since(&self, scale: Scale<f64>, commit: Option<CommitCounter>) -> DamageSet<i32, Physical> {
        self.inner.damage_since(scale, commit)
    }

    /// Deliberately empty: the corners are cut away, so the element is no longer
    /// fully opaque and anything behind it has to be drawn.
    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }

    fn kind(&self) -> Kind {
        self.inner.kind()
    }
}

impl<E: RenderElement<GlesRenderer>> RenderElement<GlesRenderer> for Rounded<E> {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        frame.override_default_tex_program(
            self.program.clone(),
            RoundedCornerShader::uniform_values(self.size, self.radius).to_vec(),
        );
        let result = self.inner.draw(frame, src, dst, damage, opaque_regions);
        frame.clear_tex_program_override();
        result
    }

    fn underlying_storage(&self, renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        // Withheld on purpose: handing this to a DRM plane would scan the buffer
        // out directly and skip the shader, so the corners would come back.
        let _ = renderer;
        None
    }
}

/// Rounds corners with the GLES texture-program override.
///
/// The program is resolved once, at construction, because `GlesFrame` does not
/// expose its renderer — so it cannot be looked up from inside `draw`.
#[derive(Debug, Default, Clone)]
pub struct GlesDecorator;

impl TileDecorator<GlesRenderer> for GlesDecorator {
    type Element = Rounded<Cropped<GlesRenderer>>;

    fn decorate(
        &mut self,
        renderer: &mut GlesRenderer,
        element: Cropped<GlesRenderer>,
        size: (f32, f32),
        radius: f32,
    ) -> Option<Self::Element> {
        // `None` means the shader never compiled. Squaring the corners is the
        // right degradation; dropping the window is not.
        let program = RoundedCornerShader::get(renderer)?;
        Some(Rounded::new(element, program, size, radius))
    }
}
