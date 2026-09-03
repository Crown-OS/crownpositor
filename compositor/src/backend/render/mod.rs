//! The graphics-API abstraction the KMS backend renders through.
//!
//! Two seams live here, deliberately small:
//!
//! * [`CrownRenderer`] binds a smithay renderer to the compositor's effect
//!   stack — which [`TileDecorator`] it drives and how its shaders are
//!   compiled. `rendering::output_elements` stays generic over any renderer;
//!   this trait is what a backend uses to get a matching decorator without
//!   naming a concrete type.
//! * [`CrownAllocator`] is where the EGL/Vulkan switch actually happens on
//!   smithay 0.7. Smithay ships a full GLES3 *renderer* but only a Vulkan
//!   *allocator* ([`VulkanAllocator`]) — there is no Vulkan `Renderer` yet. So
//!   "the Vulkan backend" means: scanout buffers allocated through Vulkan,
//!   exported as dmabufs, and drawn into by GLES3 via EGL import. Everything
//!   downstream (`DrmCompositor`, damage tracking, the scene graph) only ever
//!   sees `Allocator<Buffer = Dmabuf>`, so the day smithay grows a Vulkan
//!   renderer the swap happens behind these two seams and nowhere else.

use smithay::backend::{
    allocator::{
        Allocator, Fourcc, Modifier,
        dmabuf::{AnyError, Dmabuf, DmabufAllocator},
        gbm::{GbmAllocator, GbmDevice},
        vulkan::VulkanAllocator,
    },
    drm::{
        DrmDeviceFd, DrmNode,
        exporter::{ExportBuffer, ExportFramebuffer},
        gbm::{
            Error as GbmFramebufferError, GbmFramebuffer, framebuffer_from_dmabuf,
            framebuffer_from_wayland_buffer,
        },
    },
    renderer::{
        ImportAll, ImportDma, ImportMem, Renderer,
        gles::GlesRenderer,
        multigpu::{GpuManager, MultiRenderer, gbm::GbmGlesBackend},
    },
};

use crate::{
    rendering::{
        blur::BackdropSource,
        decorate::TileDecorator,
        rounded::{GlesDecorator, MultiDecorator},
    },
    shaders::{blur::BlurShaders, border::BorderShader, rounded_corner::RoundedCornerShader},
};

/// The GLES-over-GBM graphics stack every GPU gets on the KMS backend.
pub type GbmGlesApi = GbmGlesBackend<GlesRenderer, DrmDeviceFd>;
/// The renderer the KMS backend hands to the generic element builder: it
/// composites on one GPU and can copy client buffers across from any other.
pub type KmsRenderer<'render> = MultiRenderer<'render, 'render, GbmGlesApi, GbmGlesApi>;
/// The manager that owns every GPU's GLES context.
pub type KmsGpuManager = GpuManager<GbmGlesApi>;

/// Which graphics API allocates the frames we scan out.
///
/// Selected once at startup from `CROWN_RENDER_API`; there is no live
/// switching, because re-allocating every surface's swapchain mid-session
/// buys nothing over a restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GraphicsApi {
    /// GBM-allocated buffers, GLES3 rendering through EGL. The default: it is
    /// the path every driver stack has burned in.
    #[default]
    EglGles3,
    /// Vulkan-allocated buffers (exported as dmabufs), GLES3 rendering. Useful
    /// where GBM allocation is the buggy half of a driver, and the stepping
    /// stone to a full Vulkan renderer.
    Vulkan,
}

impl GraphicsApi {
    /// Reads `CROWN_RENDER_API` (`egl` / `gles` or `vulkan`). Anything else —
    /// including unset — picks the default and says so, because silently
    /// ignoring a typo in a tuning knob wastes an afternoon.
    pub fn detect() -> Self {
        match std::env::var("CROWN_RENDER_API") {
            Ok(value) => match value.to_ascii_lowercase().as_str() {
                "egl" | "gles" | "gles3" => Self::EglGles3,
                "vulkan" | "vk" => Self::Vulkan,
                other => {
                    tracing::warn!(
                        requested = other,
                        "unknown CROWN_RENDER_API, falling back to EGL/GLES3"
                    );
                    Self::EglGles3
                }
            },
            Err(_) => Self::EglGles3,
        }
    }
}

/// Errors from bringing a rendering context up. `thiserror` because callers
/// (backend init, device hotplug) match on these to decide between "fall back"
/// and "fail the whole backend".
#[derive(Debug, thiserror::Error)]
pub enum RenderInitError {
    #[error("failed to create the EGL/GLES3 context: {0}")]
    EglContext(String),
    #[error("no Vulkan physical device matches DRM node {node}")]
    NoVulkanDevice { node: String },
    #[error("failed to create the Vulkan instance: {0}")]
    VulkanInstance(String),
    #[error("failed to create the Vulkan allocator: {0}")]
    VulkanAllocator(String),
    #[error("shader compilation failed: {0}")]
    Shader(String),
}

/// A renderer the compositor knows how to decorate windows with.
///
/// The supertraits are exactly what `rendering::output_elements` and the
/// dmabuf/shm import paths demand — nothing KMS- or winit-specific, so both
/// backends (and tests) can be generic over this.
/// `ImportMem` is here for the cursor: a themed shape is rasterised on the CPU,
/// so the renderer has to be able to take pixels from main memory, and the
/// element that draws it wants a `Send` texture handle.
pub trait CrownRenderer: Renderer + ImportAll + ImportDma + ImportMem + Sized
where
    Self::TextureId: Send + Clone + 'static,
{
    /// The effect stack this renderer supports. [`PassThrough`] for a
    /// renderer with no custom shaders.
    ///
    /// [`PassThrough`]: crate::rendering::decorate::PassThrough
    type Decorator: TileDecorator<Self>;

    /// Compiles this renderer's shader programs. Failure is *reported*, not
    /// fatal: effects degrade (square corners, no blur), windows still draw.
    fn compile_shaders(&mut self) -> Result<(), RenderInitError>;

    /// A fresh decorator for one output's render pass. `backdrop` is the
    /// output's blurred scene for this frame, if the backend produced one.
    fn decorator(&mut self, backdrop: Option<BackdropSource>) -> Self::Decorator;
}

impl CrownRenderer for GlesRenderer {
    type Decorator = GlesDecorator;

    fn compile_shaders(&mut self) -> Result<(), RenderInitError> {
        RoundedCornerShader::init(self).map_err(|err| RenderInitError::Shader(err.to_string()))?;
        BlurShaders::init(self).map_err(|err| RenderInitError::Shader(err.to_string()))?;
        BorderShader::init(self).map_err(|err| RenderInitError::Shader(err.to_string()))
    }

    fn decorator(&mut self, backdrop: Option<BackdropSource>) -> Self::Decorator {
        GlesDecorator::new(backdrop)
    }
}

impl<'render> CrownRenderer for KmsRenderer<'render> {
    type Decorator = MultiDecorator;

    fn compile_shaders(&mut self) -> Result<(), RenderInitError> {
        // The programs land in the GLES renderer's EGL user data, so they
        // persist per GPU, not per `MultiRenderer` instance.
        RoundedCornerShader::init(self.as_mut())
            .map_err(|err| RenderInitError::Shader(err.to_string()))?;
        BlurShaders::init(self.as_mut()).map_err(|err| RenderInitError::Shader(err.to_string()))?;
        BorderShader::init(self.as_mut()).map_err(|err| RenderInitError::Shader(err.to_string()))
    }

    fn decorator(&mut self, backdrop: Option<BackdropSource>) -> Self::Decorator {
        MultiDecorator::new(backdrop)
    }
}

/// The scanout-buffer allocator, unified to `Buffer = Dmabuf`.
///
/// A closed enum rather than `Box<dyn Allocator>` for the same reason
/// [`BackendState`] is one: there are exactly two, the render path wants a
/// concrete type, and a trait object would only launder that through `dyn`.
///
/// [`BackendState`]: crate::state::BackendState
#[derive(Debug)]
pub enum CrownAllocator {
    Gbm(DmabufAllocator<GbmAllocator<DrmDeviceFd>>),
    Vulkan(DmabufAllocator<VulkanAllocator>),
}

impl CrownAllocator {
    pub fn api(&self) -> GraphicsApi {
        match self {
            Self::Gbm(_) => GraphicsApi::EglGles3,
            Self::Vulkan(_) => GraphicsApi::Vulkan,
        }
    }
}

impl Allocator for CrownAllocator {
    type Buffer = Dmabuf;
    type Error = AnyError;

    fn create_buffer(
        &mut self,
        width: u32,
        height: u32,
        fourcc: Fourcc,
        modifiers: &[Modifier],
    ) -> Result<Self::Buffer, Self::Error> {
        match self {
            Self::Gbm(gbm) => gbm.create_buffer(width, height, fourcc, modifiers),
            Self::Vulkan(vulkan) => vulkan.create_buffer(width, height, fourcc, modifiers),
        }
    }
}

/// Turns the dmabufs [`CrownAllocator`] produces into DRM framebuffers.
///
/// Smithay 0.7 only ships a `GbmBuffer` exporter, but unifying the two
/// allocation APIs means the swapchain speaks `Dmabuf` — so this is the
/// missing `ExportFramebuffer<Dmabuf>`: swapchain buffers are re-imported
/// through GBM and attached with `ADDFB2`, and client buffers offered for
/// direct scanout are accepted only when they already live on this GPU.
#[derive(Debug)]
pub struct DmabufExporter {
    gbm: GbmDevice<DrmDeviceFd>,
    /// The render node clients allocate on. A dmabuf from any other device
    /// cannot be scanned out here without a copy, so it is refused and takes
    /// the composition path instead.
    import_node: Option<DrmNode>,
}

impl DmabufExporter {
    pub fn new(gbm: GbmDevice<DrmDeviceFd>, import_node: Option<DrmNode>) -> Self {
        Self { gbm, import_node }
    }
}

impl ExportFramebuffer<Dmabuf> for DmabufExporter {
    type Framebuffer = GbmFramebuffer;
    type Error = GbmFramebufferError;

    fn add_framebuffer(
        &self,
        drm: &DrmDeviceFd,
        buffer: ExportBuffer<'_, Dmabuf>,
        use_opaque: bool,
    ) -> Result<Option<Self::Framebuffer>, Self::Error> {
        match buffer {
            ExportBuffer::Wayland(wl_buffer) => {
                framebuffer_from_wayland_buffer(drm, &self.gbm, wl_buffer, use_opaque)
            }
            ExportBuffer::Allocator(dmabuf) => {
                framebuffer_from_dmabuf(drm, &self.gbm, dmabuf, use_opaque, true).map(Some)
            }
        }
    }

    fn can_add_framebuffer(&self, buffer: &ExportBuffer<'_, Dmabuf>) -> bool {
        match buffer {
            ExportBuffer::Wayland(wl_buffer) => {
                // Direct scanout of a client buffer: only when it is a dmabuf
                // from the GPU this exporter scans out from.
                smithay::wayland::dmabuf::get_dmabuf(wl_buffer)
                    .ok()
                    .and_then(|dmabuf| dmabuf.node())
                    .is_some_and(|node| Some(node) == self.import_node)
            }
            // Swapchain buffers were allocated for exactly this.
            ExportBuffer::Allocator(_) => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::decorate::PassThrough;

    /// Compile-time proof the seam holds: any `CrownRenderer` can drive the
    /// generic element builder with its own decorator.
    fn assert_renderable<R>()
    where
        R: CrownRenderer,
        R::TextureId: Send + Clone + 'static,
    {
        fn takes_decorator<R, D>()
        where
            R: Renderer + ImportAll,
            R::TextureId: Clone + 'static,
            D: TileDecorator<R>,
        {
        }
        takes_decorator::<R, R::Decorator>();
        // And the no-effect fallback always fits.
        takes_decorator::<R, PassThrough>();
    }

    #[test]
    fn gles_satisfies_the_renderer_seam() {
        assert_renderable::<GlesRenderer>();
    }

    #[test]
    fn api_detection_defaults_to_egl() {
        // Not touching the real environment (tests run in parallel); the
        // default path is the one every CI machine exercises.
        assert_eq!(GraphicsApi::default(), GraphicsApi::EglGles3);
    }
}
