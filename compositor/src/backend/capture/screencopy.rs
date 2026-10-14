//! The render side of `wlr-screencopy`: whole outputs or regions of them,
//! one frame per request.

use smithay::{
    output::Output,
    utils::{Buffer as BufferCoords, Logical, Rectangle, Size},
};

use protocols::{
    crownos_screencast::CursorMode,
    wlr_screencopy::{ScreencopyConstraints, ScreencopyFrame},
};

use crate::backend::capture::{
    copy::{Copier, image_size},
    render::{CaptureError, CaptureRenderer, Scene},
    source::Placement,
};

pub struct PendingScreencopy {
    pub frame: ScreencopyFrame,
    pub output: Output,
    /// Whether the output changed since the request, or the request did not
    /// ask to wait for that.
    pub due: bool,
}

/// One copier per captured output, so repeated copies keep their effect
/// caches and redraw only what went stale in each buffer.
#[derive(Default)]
pub struct Screencopies {
    pub pending: Vec<PendingScreencopy>,
    copiers: Vec<(Output, Copier)>,
}

impl Screencopies {
    pub fn push(&mut self, frame: ScreencopyFrame, output: Output) {
        let due = !frame.with_damage;
        self.pending.push(PendingScreencopy { frame, output, due });
    }

    pub fn mark_output_damaged(&mut self, output: &Output) {
        for pending in self
            .pending
            .iter_mut()
            .filter(|pending| pending.output == *output)
        {
            pending.due = true;
        }
    }

    pub fn output_removed(&mut self, output: &Output) {
        self.pending.retain(|pending| pending.output != *output);
        self.copiers.retain(|(copied, _)| copied != output);
    }

    pub fn copier(&mut self, output: &Output) -> &mut Copier {
        let index = match self.copiers.iter().position(|(copied, _)| copied == output) {
            Some(index) => index,
            None => {
                self.copiers.push((output.clone(), Copier::default()));
                self.copiers.len() - 1
            }
        };
        &mut self.copiers[index].1
    }
}

/// The buffer a capture of `placement`'s output needs, whole or `region`.
/// Unlike the other capture protocols, screencopy hands out the output's own
/// buffer orientation, and clients turn it upright by the output transform.
pub fn constraints(
    placement: &Placement<'_>,
    region: Option<Rectangle<i32, Logical>>,
    dmabuf: Option<smithay::backend::allocator::Fourcc>,
) -> Option<ScreencopyConstraints> {
    let size = match region {
        None => buffer_size(placement),
        Some(region) => crop(placement, region)?.size,
    };
    Some(ScreencopyConstraints {
        size,
        dmabuf: dmabuf.filter(|_| region.is_none()),
    })
}

fn buffer_size(placement: &Placement<'_>) -> Size<i32, BufferCoords> {
    let size = placement
        .output()
        .current_transform()
        .invert()
        .transform_size(image_size(placement));
    Size::from((size.w, size.h))
}

/// `region`, output-local logical, as a rect of the output's buffer.
pub fn crop(
    placement: &Placement<'_>,
    region: Rectangle<i32, Logical>,
) -> Option<Rectangle<i32, BufferCoords>> {
    let image = image_size(placement);
    let upright = region
        .to_physical_precise_round(placement.scale())
        .intersection(Rectangle::from_size(image))?;
    let turned = placement
        .output()
        .current_transform()
        .invert()
        .transform_rect_in(upright, &image);
    Some(Rectangle::new(
        (turned.loc.x, turned.loc.y).into(),
        (turned.size.w, turned.size.h).into(),
    ))
}

/// Fills the frame's buffer and returns its damage, or `None` when nothing
/// changed and the frame keeps waiting.
pub fn copy<R>(
    renderer: &mut R,
    copier: &mut Copier,
    frame: &ScreencopyFrame,
    scene: Scene<'_>,
) -> Result<Option<Vec<Rectangle<i32, BufferCoords>>>, CaptureError>
where
    R: CaptureRenderer,
    R::TextureId: Send + Clone + 'static,
{
    let image = buffer_size(&scene.placement);
    let transform = scene.placement.output().current_transform();
    let region = match frame.region {
        Some(region) => Some(
            crop(&scene.placement, region)
                .ok_or_else(|| CaptureError::Render("the region left the output".into()))?,
        ),
        None => None,
    };
    let cursor_mode = if frame.overlay_cursor {
        CursorMode::Embedded
    } else {
        CursorMode::Hidden
    };
    let drawn = copier.draw(
        renderer,
        scene,
        &frame.buffer,
        cursor_mode,
        transform,
        region,
        !frame.with_damage,
    )?;
    // The whole buffer: damage is tracked per buffer and several clients may
    // share one output's copier, so a precise region would be right for at
    // most one of them.
    let size = region.map_or(image, |region| region.size);
    Ok(drawn.map(|_| vec![Rectangle::from_size(size)]))
}
