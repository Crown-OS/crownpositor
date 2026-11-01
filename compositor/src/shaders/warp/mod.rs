//! The raw GL programs that draw a window's canvas bent out of shape: poured
//! into its icon, crumbled into falling dust, or stretched into a drop of
//! liquid glass.
//!
//! Like [`KawaseProgram`](crate::shaders::blur::KawaseProgram) they bind no
//! buffers: every vertex is built from `gl_VertexID`, so a mesh of tens of
//! thousands of particles costs no memory at all.

use std::ffi::CStr;

use smithay::{
    backend::renderer::gles::{GlesError, GlesRenderer, GlesTexture, ffi},
    utils::{Physical, Rectangle},
};

use crate::shaders::program;

static GENIE_SHADER: &str = include_str!("./genie.vert");
static DUST_SHADER: &str = include_str!("./dust.vert");
static WARP_SHADER: &str = include_str!("./warp.frag");
static LIQUID_VERTEX_SHADER: &str = include_str!("./liquid.vert");
static LIQUID_FRAGMENT_SHADER: &str = concat!(
    include_str!("./liquid.frag"),
    include_str!("../common/rounded_box.glsl"),
);

/// Vertices in the genie's grid: 16 columns by 64 rows of two triangles.
pub const GENIE_VERTICES: i32 = 16 * 64 * 6;
/// Vertices in the liquid shape's grid: 16 by 16 quads of two triangles.
pub const LIQUID_VERTICES: i32 = 16 * 16 * 6;

type Location = ffi::types::GLint;

/// Uniform slots both warps share.
#[derive(Debug, Clone, Copy)]
struct Common {
    program: ffi::types::GLuint,
    projection: Location,
    window: Location,
    tex: Location,
    alpha: Location,
}

impl Common {
    unsafe fn link(gl: &ffi::Gles2, vertex: &str, fragment: &str) -> Result<Self, GlesError> {
        unsafe {
            let program = program::link(gl, vertex, fragment)?;
            Ok(Self {
                program,
                projection: location(gl, program, c"projection"),
                window: location(gl, program, c"window"),
                tex: location(gl, program, c"tex"),
                alpha: location(gl, program, c"alpha"),
            })
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GenieProgram {
    common: Common,
    along: Location,
    across: Location,
    window_span: Location,
    icon_span: Location,
    progress: Location,
}

#[derive(Debug, Clone, Copy)]
pub struct DustProgram {
    common: Common,
    cells: Location,
    seed: Location,
    timing: Location,
    physics: Location,
}

#[derive(Debug, Clone, Copy)]
pub struct LiquidProgram {
    common: Common,
    icon: Location,
    corners_top: Location,
    corners_bottom: Location,
    content: Location,
    shape: Location,
}

/// Everything a liquid frame needs, in output-local physical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LiquidUniforms {
    /// Top left, top right, bottom left, bottom right.
    pub corners: [[f32; 2]; 4],
    /// The window within the canvas, in texture coordinates: u, v, width,
    /// height.
    pub content: [f32; 4],
    pub radius: f32,
    pub softness: f32,
    pub reveal: f32,
    pub icon_alpha: f32,
}

/// Everything a genie frame needs, in output-local physical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GenieUniforms {
    pub along: [f32; 2],
    pub across: [f32; 2],
    pub window_span: [f32; 4],
    pub icon_span: [f32; 4],
    pub progress: f32,
}

/// Everything a dust frame needs, in physical pixels and seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DustUniforms {
    pub cells: [i32; 2],
    pub seed: u32,
    pub timing: [f32; 4],
    pub physics: [f32; 4],
}

impl DustUniforms {
    pub fn vertices(&self) -> i32 {
        self.cells[0] * self.cells[1] * 6
    }
}

/// One warped draw of a canvas.
pub struct WarpDraw<'a> {
    pub texture: &'a GlesTexture,
    pub projection: [f32; 9],
    /// The canvas in output-local physical pixels: x, y, width, height.
    pub window: [f32; 4],
    pub alpha: f32,
    /// Framebuffer-space rectangles to fill.
    pub scissors: &'a [Rectangle<i32, Physical>],
}

/// The compiled warp programs for one GL context.
#[derive(Debug, Clone, Copy)]
pub struct WarpShaders {
    pub genie: GenieProgram,
    pub dust: DustProgram,
    pub liquid: LiquidProgram,
}

impl WarpShaders {
    /// Compiles the programs into this context, once.
    pub fn init(renderer: &mut GlesRenderer) -> Result<(), GlesError> {
        if renderer
            .egl_context()
            .user_data()
            .get::<WarpShaders>()
            .is_some()
        {
            return Ok(());
        }
        let shaders = renderer.with_context(|gl| unsafe {
            let genie = Common::link(gl, GENIE_SHADER, WARP_SHADER)?;
            let dust = Common::link(gl, DUST_SHADER, WARP_SHADER)?;
            let liquid = Common::link(gl, LIQUID_VERTEX_SHADER, LIQUID_FRAGMENT_SHADER)?;
            Ok::<_, GlesError>(WarpShaders {
                genie: GenieProgram {
                    common: genie,
                    along: location(gl, genie.program, c"along"),
                    across: location(gl, genie.program, c"across"),
                    window_span: location(gl, genie.program, c"window_span"),
                    icon_span: location(gl, genie.program, c"icon_span"),
                    progress: location(gl, genie.program, c"progress"),
                },
                dust: DustProgram {
                    common: dust,
                    cells: location(gl, dust.program, c"cells"),
                    seed: location(gl, dust.program, c"seed"),
                    timing: location(gl, dust.program, c"timing"),
                    physics: location(gl, dust.program, c"physics"),
                },
                liquid: LiquidProgram {
                    common: liquid,
                    icon: location(gl, liquid.program, c"icon"),
                    corners_top: location(gl, liquid.program, c"corners_top"),
                    corners_bottom: location(gl, liquid.program, c"corners_bottom"),
                    content: location(gl, liquid.program, c"content"),
                    shape: location(gl, liquid.program, c"shape"),
                },
            })
        })??;
        renderer
            .egl_context()
            .user_data()
            .insert_if_missing(|| shaders);
        Ok(())
    }

    /// `None` when [`init`](Self::init) never ran or failed: the windows then
    /// fall back to scaling into their icon.
    pub fn get(renderer: &GlesRenderer) -> Option<WarpShaders> {
        renderer
            .egl_context()
            .user_data()
            .get::<WarpShaders>()
            .copied()
    }
}

impl GenieProgram {
    /// # Safety
    ///
    /// `gl` must belong to the current frame's context.
    pub unsafe fn draw(&self, gl: &ffi::Gles2, draw: &WarpDraw<'_>, uniforms: &GenieUniforms) {
        unsafe {
            run(gl, &self.common, draw, GENIE_VERTICES, || {
                gl.Uniform2fv(self.along, 1, uniforms.along.as_ptr());
                gl.Uniform2fv(self.across, 1, uniforms.across.as_ptr());
                gl.Uniform4fv(self.window_span, 1, uniforms.window_span.as_ptr());
                gl.Uniform4fv(self.icon_span, 1, uniforms.icon_span.as_ptr());
                gl.Uniform1f(self.progress, uniforms.progress);
            });
        }
    }
}

impl DustProgram {
    /// # Safety
    ///
    /// `gl` must belong to the current frame's context.
    pub unsafe fn draw(&self, gl: &ffi::Gles2, draw: &WarpDraw<'_>, uniforms: &DustUniforms) {
        unsafe {
            run(gl, &self.common, draw, uniforms.vertices(), || {
                gl.Uniform2iv(self.cells, 1, uniforms.cells.as_ptr());
                gl.Uniform1ui(self.seed, uniforms.seed);
                gl.Uniform4fv(self.timing, 1, uniforms.timing.as_ptr());
                gl.Uniform4fv(self.physics, 1, uniforms.physics.as_ptr());
            });
        }
    }
}

impl LiquidProgram {
    /// Draws the canvas into the liquid shape, with `icon` — the icon the
    /// shape carries, or `None` when it carries none — fading over it.
    ///
    /// # Safety
    ///
    /// `gl` must belong to the current frame's context.
    pub unsafe fn draw(
        &self,
        gl: &ffi::Gles2,
        draw: &WarpDraw<'_>,
        icon: Option<&GlesTexture>,
        uniforms: &LiquidUniforms,
    ) {
        let [top_left, top_right, bottom_left, bottom_right] = uniforms.corners;
        let corners_top = [top_left[0], top_left[1], top_right[0], top_right[1]];
        let corners_bottom = [
            bottom_left[0],
            bottom_left[1],
            bottom_right[0],
            bottom_right[1],
        ];
        let shape = [
            uniforms.radius,
            uniforms.softness,
            uniforms.reveal,
            icon.map_or(0.0, |_| uniforms.icon_alpha),
        ];
        unsafe {
            run(gl, &self.common, draw, LIQUID_VERTICES, || {
                // Without an icon the sampler still needs a texture; the
                // canvas serves, its own filtering left alone.
                gl.ActiveTexture(ffi::TEXTURE1);
                gl.BindTexture(ffi::TEXTURE_2D, icon.unwrap_or(draw.texture).tex_id());
                if icon.is_some() {
                    for (parameter, value) in [
                        (ffi::TEXTURE_MIN_FILTER, ffi::LINEAR),
                        (ffi::TEXTURE_MAG_FILTER, ffi::LINEAR),
                        (ffi::TEXTURE_WRAP_S, ffi::CLAMP_TO_EDGE),
                        (ffi::TEXTURE_WRAP_T, ffi::CLAMP_TO_EDGE),
                    ] {
                        gl.TexParameteri(ffi::TEXTURE_2D, parameter, value as ffi::types::GLint);
                    }
                }
                gl.ActiveTexture(ffi::TEXTURE0);
                gl.Uniform1i(self.icon, 1);
                gl.Uniform4fv(self.corners_top, 1, corners_top.as_ptr());
                gl.Uniform4fv(self.corners_bottom, 1, corners_bottom.as_ptr());
                gl.Uniform4fv(self.content, 1, uniforms.content.as_ptr());
                gl.Uniform4fv(self.shape, 1, shape.as_ptr());
            });
            gl.ActiveTexture(ffi::TEXTURE1);
            gl.BindTexture(ffi::TEXTURE_2D, 0);
            gl.ActiveTexture(ffi::TEXTURE0);
        }
    }
}

/// Binds `common` and the canvas, lets `uniforms` fill in the rest, and draws
/// `vertices` once per scissor. Leaves the program, texture and scissor as
/// smithay expects to find them.
unsafe fn run(
    gl: &ffi::Gles2,
    common: &Common,
    draw: &WarpDraw<'_>,
    vertices: i32,
    uniforms: impl FnOnce(),
) {
    let mut viewport = [0; 4];
    unsafe {
        let scissored = gl.IsEnabled(ffi::SCISSOR_TEST) == ffi::TRUE;
        gl.GetIntegerv(ffi::VIEWPORT, viewport.as_mut_ptr());

        gl.UseProgram(common.program);
        gl.ActiveTexture(ffi::TEXTURE0);
        gl.BindTexture(ffi::TEXTURE_2D, draw.texture.tex_id());
        for (parameter, value) in [
            (ffi::TEXTURE_MIN_FILTER, ffi::LINEAR_MIPMAP_LINEAR),
            (ffi::TEXTURE_MAG_FILTER, ffi::LINEAR),
            (ffi::TEXTURE_WRAP_S, ffi::CLAMP_TO_EDGE),
            (ffi::TEXTURE_WRAP_T, ffi::CLAMP_TO_EDGE),
        ] {
            gl.TexParameteri(ffi::TEXTURE_2D, parameter, value as ffi::types::GLint);
        }
        gl.Uniform1i(common.tex, 0);
        gl.UniformMatrix3fv(common.projection, 1, ffi::FALSE, draw.projection.as_ptr());
        gl.Uniform4fv(common.window, 1, draw.window.as_ptr());
        gl.Uniform1f(common.alpha, draw.alpha);
        uniforms();

        gl.Enable(ffi::SCISSOR_TEST);
        for rect in draw.scissors {
            gl.Scissor(rect.loc.x, rect.loc.y, rect.size.w, rect.size.h);
            gl.DrawArrays(ffi::TRIANGLES, 0, vertices);
        }

        gl.Scissor(viewport[0], viewport[1], viewport[2], viewport[3]);
        if !scissored {
            gl.Disable(ffi::SCISSOR_TEST);
        }
        gl.BindTexture(ffi::TEXTURE_2D, 0);
        gl.UseProgram(0);
    }
}

unsafe fn location(gl: &ffi::Gles2, program: ffi::types::GLuint, name: &CStr) -> Location {
    unsafe { gl.GetUniformLocation(program, name.as_ptr()) }
}
