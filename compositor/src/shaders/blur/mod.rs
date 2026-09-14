//! The blur pipeline's shader programs.
//!
//! The dual-kawase pyramid runs as raw GL — see [`KawaseProgram`] — because its
//! passes fill offscreen levels rather than place a texture in an output. Only
//! the last upsample lands in the frame, and that one goes through smithay's
//! custom-texture-shader machinery so it inherits its vertex stage, damage
//! instancing and variant handling (`EXTERNAL`/`NO_ALPHA`/`DEBUG_FLAGS`).
//!
//! Like [`RoundedCornerShader`], the compiled programs live in the EGL
//! context's user data: per GPU, reachable from anywhere the renderer is. The
//! scratch framebuffer the pyramid renders through lives with them, because it
//! has exactly the same lifetime.
//!
//! [`RoundedCornerShader`]: crate::shaders::rounded_corner::RoundedCornerShader

mod kawase;

pub use kawase::KawaseProgram;

use smithay::{
    backend::renderer::gles::{
        Capability, GlesError, GlesRenderer, GlesTexProgram, Uniform, UniformName, UniformType, ffi,
    },
    utils::{Physical, Rectangle},
};

static DOWN_SHADER: &str = include_str!("./blur_down.frag");
static UP_SHADER: &str = include_str!("./blur_up.frag");
static FINISH_SHADER: &str = include_str!("./blur_finish.frag");

/// The compiled blur programs for one GL context.
#[derive(Debug, Clone)]
pub struct BlurShaders {
    pub down: KawaseProgram,
    pub up: KawaseProgram,
    pub finish: GlesTexProgram,
    /// Scratch FBO the pyramid levels are attached to one at a time.
    pub framebuffer: ffi::types::GLuint,
}

impl BlurShaders {
    fn finish_names() -> [UniformName<'static>; 8] {
        [
            UniformName::new("backdrop_origin", UniformType::_2f),
            UniformName::new("backdrop_size", UniformType::_2f),
            UniformName::new("mask_origin", UniformType::_2f),
            UniformName::new("mask_size", UniformType::_2f),
            UniformName::new("half_pixel", UniformType::_2f),
            UniformName::new("offset", UniformType::_1f),
            UniformName::new("corner_radius", UniformType::_1f),
            UniformName::new("noise", UniformType::_1f),
        ]
    }

    /// Compiles the programs into this context, once.
    ///
    /// Without [`Capability::Blit`] there is no way to lift the frame into the
    /// pyramid, so nothing is installed and blur degrades to no backdrops.
    pub fn init(renderer: &mut GlesRenderer) -> Result<(), GlesError> {
        if renderer
            .egl_context()
            .user_data()
            .get::<BlurShaders>()
            .is_some()
        {
            return Ok(());
        }
        if !renderer.capabilities().contains(&Capability::Blit) {
            tracing::info!("no framebuffer blit on this GPU; blur is unavailable");
            return Ok(());
        }

        let finish =
            renderer.compile_custom_texture_shader(FINISH_SHADER, &Self::finish_names())?;
        let (down, up, framebuffer) = renderer.with_context(|gl| unsafe {
            let down = KawaseProgram::compile(gl, DOWN_SHADER)?;
            let up = KawaseProgram::compile(gl, UP_SHADER)?;
            let mut framebuffer = 0;
            gl.GenFramebuffers(1, &mut framebuffer);
            Ok::<_, GlesError>((down, up, framebuffer))
        })??;

        renderer
            .egl_context()
            .user_data()
            .insert_if_missing(|| BlurShaders {
                down,
                up,
                finish,
                framebuffer,
            });

        Ok(())
    }

    /// `None` when [`init`](Self::init) never ran, failed, or found no blit
    /// support: blur then degrades to windows without backdrops rather than
    /// taking the compositor down.
    pub fn get(renderer: &GlesRenderer) -> Option<BlurShaders> {
        renderer
            .egl_context()
            .user_data()
            .get::<BlurShaders>()
            .cloned()
    }

    /// Uniforms for one backdrop rectangle.
    ///
    /// `backdrop` and `mask` are both in framebuffer pixels, which is the space
    /// the shader reads `gl_FragCoord` in; `half_pixel` is half a pixel of the
    /// pyramid's top level, the source of the upsample this pass performs.
    pub fn finish_uniforms(
        backdrop: Rectangle<i32, Physical>,
        mask: Rectangle<i32, Physical>,
        half_pixel: (f32, f32),
        offset: f32,
        corner_radius: f32,
        noise: f32,
    ) -> [Uniform<'static>; 8] {
        [
            Uniform::new(
                "backdrop_origin",
                (backdrop.loc.x as f32, backdrop.loc.y as f32),
            ),
            Uniform::new(
                "backdrop_size",
                (backdrop.size.w as f32, backdrop.size.h as f32),
            ),
            Uniform::new("mask_origin", (mask.loc.x as f32, mask.loc.y as f32)),
            Uniform::new("mask_size", (mask.size.w as f32, mask.size.h as f32)),
            Uniform::new("half_pixel", half_pixel),
            Uniform::new("offset", offset),
            Uniform::new("corner_radius", corner_radius),
            Uniform::new("noise", noise),
        ]
    }
}
