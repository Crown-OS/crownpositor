//! RGB → NV12 on the GPU.
//!
//! A frame is first composited into an RGB texture, then drawn twice through
//! the conversion shader: once into the client buffer's luma plane, imported
//! on its own as an R8 image, and once at half size into its chroma plane,
//! imported as GR88. Nothing leaves the GPU, and the encoder receives exactly
//! the layout it consumes.

use std::os::fd::OwnedFd;

use smithay::{
    backend::{
        allocator::{
            Buffer, Fourcc,
            dmabuf::{Dmabuf, DmabufFlags},
        },
        renderer::{
            Bind, Frame, Offscreen, Renderer,
            gles::{GlesError, GlesRenderer, GlesTexture},
            sync::SyncPoint,
        },
    },
    utils::{Buffer as BufferCoords, Physical, Rectangle, Size, Transform},
};

use protocols::crownos_screencast::MAX_SLOTS;

use crate::{
    backend::capture::formats::{NV12_CHROMA_PLANE, NV12_LUMA_PLANE},
    shaders::nv12::{CHROMA_BLUE, CHROMA_RED, LUMA, Nv12Shader},
};

#[derive(Debug, thiserror::Error)]
pub enum Nv12Error {
    #[error("the conversion shader is not available")]
    NoShader,
    #[error("the NV12 buffer does not have exactly two planes")]
    PlaneLayout,
    #[error("failed to duplicate a plane descriptor: {0}")]
    Descriptor(#[from] std::io::Error),
    #[error(transparent)]
    Gles(#[from] GlesError),
}

/// A client NV12 buffer seen as the two images it is rendered through.
#[derive(Debug)]
struct PlaneImages {
    source: Dmabuf,
    luma: Dmabuf,
    chroma: Dmabuf,
}

impl PlaneImages {
    fn split(nv12: &Dmabuf) -> Result<Self, Nv12Error> {
        let planes: Vec<(OwnedFd, u32, u32)> = nv12
            .handles()
            .zip(nv12.offsets())
            .zip(nv12.strides())
            .map(|((fd, offset), stride)| Ok((fd.try_clone_to_owned()?, offset, stride)))
            .collect::<Result<_, std::io::Error>>()?;
        let [luma, chroma]: [(OwnedFd, u32, u32); 2] =
            planes.try_into().map_err(|_| Nv12Error::PlaneLayout)?;

        let size = nv12.size();
        let modifier = nv12.format().modifier;
        let plane = |(fd, offset, stride): (OwnedFd, u32, u32), code, size| {
            let mut builder = Dmabuf::builder(size, code, modifier, DmabufFlags::empty());
            builder.add_plane(fd, offset, stride);
            builder.build().ok_or(Nv12Error::PlaneLayout)
        };

        Ok(Self {
            source: nv12.clone(),
            luma: plane(luma, NV12_LUMA_PLANE, size)?,
            chroma: plane(chroma, NV12_CHROMA_PLANE, chroma_size(size))?,
        })
    }
}

/// Half the luma size, rounded up, as NV12 subsamples it.
pub fn chroma_size(size: Size<i32, BufferCoords>) -> Size<i32, BufferCoords> {
    Size::from(((size.w + 1) / 2, (size.h + 1) / 2))
}

/// Everything an NV12 session keeps between frames.
#[derive(Debug, Default)]
pub struct Nv12Target {
    intermediate: Option<GlesTexture>,
    planes: [Option<PlaneImages>; MAX_SLOTS],
}

impl Nv12Target {
    /// The RGB texture frames are composited into, and whether it still holds
    /// the previous frame (so damage tracking against it is valid).
    pub fn intermediate(
        &mut self,
        gles: &mut GlesRenderer,
        size: Size<i32, BufferCoords>,
    ) -> Result<(GlesTexture, bool), Nv12Error> {
        if let Some(texture) = self
            .intermediate
            .as_ref()
            .filter(|texture| smithay::backend::renderer::Texture::size(*texture) == size)
        {
            return Ok((texture.clone(), true));
        }
        let texture: GlesTexture =
            Offscreen::<GlesTexture>::create_buffer(gles, Fourcc::Abgr8888, size)?;
        self.intermediate = Some(texture.clone());
        Ok((texture, false))
    }

    /// Converts the composited frame into slot `index`'s buffer.
    pub fn convert(
        &mut self,
        gles: &mut GlesRenderer,
        source: &GlesTexture,
        index: usize,
        nv12: &Dmabuf,
    ) -> Result<SyncPoint, Nv12Error> {
        let program = Nv12Shader::get(gles).ok_or(Nv12Error::NoShader)?;
        let slot = self.planes.get_mut(index).ok_or(Nv12Error::PlaneLayout)?;
        if slot.as_ref().is_none_or(|planes| planes.source != *nv12) {
            *slot = Some(PlaneImages::split(nv12)?);
        }
        let planes = slot.as_mut().ok_or(Nv12Error::PlaneLayout)?;

        let source_size = smithay::backend::renderer::Texture::size(source);
        draw(
            gles,
            source,
            source_size,
            &mut planes.luma,
            &program,
            (LUMA, LUMA),
        )
        .map(drop)?;
        draw(
            gles,
            source,
            source_size,
            &mut planes.chroma,
            &program,
            (CHROMA_BLUE, CHROMA_RED),
        )
    }

    pub fn forget(&mut self) {
        *self = Self::default();
    }
}

fn draw(
    gles: &mut GlesRenderer,
    source: &GlesTexture,
    source_size: Size<i32, BufferCoords>,
    target: &mut Dmabuf,
    program: &smithay::backend::renderer::gles::GlesTexProgram,
    (first, second): (
        crate::shaders::nv12::MatrixRow,
        crate::shaders::nv12::MatrixRow,
    ),
) -> Result<SyncPoint, Nv12Error> {
    let target_size = target.size();
    let physical: Size<i32, Physical> = Size::from((target_size.w, target_size.h));
    let destination = Rectangle::from_size(physical);

    let mut framebuffer = gles.bind(target)?;
    let mut frame = gles.render(&mut framebuffer, physical, Transform::Normal)?;
    frame.render_texture_from_to(
        source,
        Rectangle::from_size(source_size.to_f64()),
        destination,
        &[destination],
        &[destination],
        Transform::Normal,
        1.0,
        Some(program),
        &Nv12Shader::uniform_values(first, second),
    )?;
    Ok(frame.finish()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn odd_sizes_round_the_chroma_plane_up() {
        assert_eq!(chroma_size((1920, 1080).into()), Size::from((960, 540)));
        assert_eq!(chroma_size((1366, 767).into()), Size::from((683, 384)));
    }
}
