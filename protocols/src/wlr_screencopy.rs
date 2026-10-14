//! Server-side `zwlr_screencopy_manager_v1`, version 3.
//!
//! The capture protocol of wlroots compositors, still what grim,
//! wf-recorder, OBS's wlrobs and older portals speak. One frame object per
//! copy: the compositor describes the buffer it wants, the client attaches
//! one, and the compositor fills it — at once with `copy`, or once the
//! output next changes with `copy_with_damage`.

use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use smithay::{
    backend::allocator::{Buffer as _, Fourcc},
    utils::{Buffer as BufferCoords, Logical, Rectangle, Size},
    wayland::{dmabuf::get_dmabuf, shm::with_buffer_contents},
};
use wayland_protocols_wlr::screencopy::v1::server::{
    zwlr_screencopy_frame_v1::{self, Flags, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::{self, ZwlrScreencopyManagerV1},
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
    backend::GlobalId,
    protocol::{wl_buffer::WlBuffer, wl_output::WlOutput, wl_shm},
};

const VERSION: u32 = 3;

/// The one shm format offered; `Xrgb8888` shares its memory layout.
pub const SHM_FORMAT: wl_shm::Format = wl_shm::Format::Argb8888;

pub trait ScreencopyHandler {
    fn screencopy_state(&mut self) -> &mut ScreencopyState;

    /// The buffer a capture of `output` needs, or of `region` of it in
    /// output-local logical coordinates. `None` refuses the capture.
    fn screencopy_constraints(
        &mut self,
        output: &WlOutput,
        region: Option<Rectangle<i32, Logical>>,
    ) -> Option<ScreencopyConstraints>;

    /// A client attached a valid buffer. The compositor completes the frame
    /// with [`ScreencopyFrame::success`] or [`ScreencopyFrame::fail`];
    /// dropping it fails it.
    fn screencopy_frame(&mut self, frame: ScreencopyFrame);
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreencopyConstraints {
    pub size: Size<i32, BufferCoords>,
    /// The dmabuf format the compositor can render into, if any.
    pub dmabuf: Option<Fourcc>,
}

pub struct ScreencopyGlobalData {
    filter: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

#[derive(Debug)]
pub struct FrameData {
    output: WlOutput,
    region: Option<Rectangle<i32, Logical>>,
    overlay_cursor: bool,
    constraints: Option<ScreencopyConstraints>,
    used: AtomicBool,
}

pub struct ScreencopyState {
    global: GlobalId,
}

impl ScreencopyState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: ScreencopyDispatch,
        F: Fn(&Client) -> bool + Send + Sync + 'static,
    {
        Self {
            global: display.create_global::<D, ZwlrScreencopyManagerV1, _>(
                VERSION,
                ScreencopyGlobalData {
                    filter: Box::new(filter),
                },
            ),
        }
    }

    pub fn global(&self) -> GlobalId {
        self.global.clone()
    }
}

/// A copy the client asked for, with its buffer attached.
#[derive(Debug)]
pub struct ScreencopyFrame {
    resource: ZwlrScreencopyFrameV1,
    pub output: WlOutput,
    /// Output-local logical; `None` is the whole output.
    pub region: Option<Rectangle<i32, Logical>>,
    pub overlay_cursor: bool,
    pub buffer: WlBuffer,
    /// Whether to wait until the output changes before copying.
    pub with_damage: bool,
    completed: bool,
}

impl ScreencopyFrame {
    pub fn success(mut self, damage: &[Rectangle<i32, BufferCoords>], presented: Duration) {
        self.completed = true;
        let resource = &self.resource;
        resource.flags(Flags::empty());
        if self.with_damage {
            for rect in damage {
                resource.damage(
                    rect.loc.x.max(0) as u32,
                    rect.loc.y.max(0) as u32,
                    rect.size.w.max(0) as u32,
                    rect.size.h.max(0) as u32,
                );
            }
        }
        let seconds = presented.as_secs();
        resource.ready(
            (seconds >> 32) as u32,
            seconds as u32,
            presented.subsec_nanos(),
        );
    }

    pub fn fail(mut self) {
        self.completed = true;
        self.resource.failed();
    }

    pub fn is_alive(&self) -> bool {
        self.resource.is_alive()
    }
}

impl Drop for ScreencopyFrame {
    fn drop(&mut self) {
        if !self.completed {
            self.resource.failed();
        }
    }
}

/// Whether `buffer` is what the frame described.
fn admits(buffer: &WlBuffer, constraints: &ScreencopyConstraints) -> bool {
    if let Ok(dmabuf) = get_dmabuf(buffer) {
        let size = dmabuf.size();
        return constraints.dmabuf == Some(dmabuf.format().code)
            && size.w == constraints.size.w
            && size.h == constraints.size.h;
    }
    with_buffer_contents(buffer, |_, _, data| {
        matches!(
            data.format,
            wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888
        ) && data.width == constraints.size.w
            && data.height == constraints.size.h
            && data.stride >= constraints.size.w * 4
    })
    .unwrap_or(false)
}

pub trait ScreencopyDispatch:
    GlobalDispatch<ZwlrScreencopyManagerV1, ScreencopyGlobalData>
    + Dispatch<ZwlrScreencopyManagerV1, ()>
    + Dispatch<ZwlrScreencopyFrameV1, FrameData>
    + ScreencopyHandler
    + 'static
{
}

impl<D> ScreencopyDispatch for D where
    D: GlobalDispatch<ZwlrScreencopyManagerV1, ScreencopyGlobalData>
        + Dispatch<ZwlrScreencopyManagerV1, ()>
        + Dispatch<ZwlrScreencopyFrameV1, FrameData>
        + ScreencopyHandler
        + 'static
{
}

impl<D: ScreencopyDispatch> GlobalDispatch<ZwlrScreencopyManagerV1, ScreencopyGlobalData, D>
    for ScreencopyState
{
    fn bind(
        _state: &mut D,
        _display: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrScreencopyManagerV1>,
        _global_data: &ScreencopyGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, ());
    }

    fn can_view(client: Client, global_data: &ScreencopyGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D: ScreencopyDispatch> Dispatch<ZwlrScreencopyManagerV1, (), D> for ScreencopyState {
    fn request(
        state: &mut D,
        _client: &Client,
        _manager: &ZwlrScreencopyManagerV1,
        request: zwlr_screencopy_manager_v1::Request,
        _data: &(),
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        let (frame, overlay_cursor, output, region) = match request {
            zwlr_screencopy_manager_v1::Request::CaptureOutput {
                frame,
                overlay_cursor,
                output,
            } => (frame, overlay_cursor, output, None),
            zwlr_screencopy_manager_v1::Request::CaptureOutputRegion {
                frame,
                overlay_cursor,
                output,
                x,
                y,
                width,
                height,
            } => (
                frame,
                overlay_cursor,
                output,
                Some(Rectangle::new((x, y).into(), (width, height).into())),
            ),
            _ => return,
        };

        let constraints = region
            .is_none_or(|region| region.size.w > 0 && region.size.h > 0)
            .then(|| state.screencopy_constraints(&output, region))
            .flatten();
        let frame = data_init.init(
            frame,
            FrameData {
                output,
                region,
                overlay_cursor: overlay_cursor != 0,
                constraints,
                used: AtomicBool::new(false),
            },
        );
        let Some(constraints) = constraints else {
            frame.failed();
            return;
        };

        let (width, height) = (constraints.size.w as u32, constraints.size.h as u32);
        frame.buffer(SHM_FORMAT, width, height, width * 4);
        if frame.version() >= 3 {
            if let Some(format) = constraints.dmabuf {
                frame.linux_dmabuf(format as u32, width, height);
            }
            frame.buffer_done();
        }
    }
}

impl<D: ScreencopyDispatch> Dispatch<ZwlrScreencopyFrameV1, FrameData, D> for ScreencopyState {
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &ZwlrScreencopyFrameV1,
        request: zwlr_screencopy_frame_v1::Request,
        data: &FrameData,
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        let (buffer, with_damage) = match request {
            zwlr_screencopy_frame_v1::Request::Copy { buffer } => (buffer, false),
            zwlr_screencopy_frame_v1::Request::CopyWithDamage { buffer } => (buffer, true),
            _ => return,
        };
        if data.used.swap(true, Ordering::Relaxed) {
            resource.post_error(
                zwlr_screencopy_frame_v1::Error::AlreadyUsed,
                "the frame was already copied",
            );
            return;
        }
        let Some(constraints) = data.constraints else {
            return;
        };
        if !admits(&buffer, &constraints) {
            resource.post_error(
                zwlr_screencopy_frame_v1::Error::InvalidBuffer,
                "the buffer does not match the advertised one",
            );
            return;
        }
        state.screencopy_frame(ScreencopyFrame {
            resource: resource.clone(),
            output: data.output.clone(),
            region: data.region,
            overlay_cursor: data.overlay_cursor,
            buffer,
            with_damage,
            completed: false,
        });
    }
}
