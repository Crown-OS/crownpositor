use smithay::wayland::{
    foreign_toplevel_list::ForeignToplevelHandle,
    image_capture_source::{
        ImageCaptureSource, ImageCaptureSourceHandler, OutputCaptureSourceHandler,
        OutputCaptureSourceState, ToplevelCaptureSourceHandler, ToplevelCaptureSourceState,
    },
    image_copy_capture::{
        BufferConstraints, CaptureFailureReason, CursorSession, CursorSessionRef, Frame, FrameRef,
        ImageCopyCaptureHandler, ImageCopyCaptureState, Session, SessionRef,
    },
};
use smithay::{
    output::Output,
    reexports::wayland_server::protocol::wl_pointer::WlPointer,
    utils::{Buffer as BufferCoords, Size},
};

use crate::{
    backend::capture::{
        copy::{self, CopySession},
        copy_cursor::{CopyCursor, cursor_constraints, shape_image},
        source::CaptureSource,
    },
    state::State,
};

impl ImageCaptureSourceHandler for State {}

impl OutputCaptureSourceHandler for State {
    fn output_capture_source_state(&mut self) -> &mut OutputCaptureSourceState {
        &mut self.wayland.output_capture_source_state
    }

    fn output_source_created(&mut self, source: ImageCaptureSource, output: &Output) {
        source
            .user_data()
            .insert_if_missing(|| CaptureSource::Output(output.clone()));
    }
}

impl ToplevelCaptureSourceHandler for State {
    fn toplevel_capture_source_state(&mut self) -> &mut ToplevelCaptureSourceState {
        &mut self.wayland.toplevel_capture_source_state
    }

    fn toplevel_source_created(
        &mut self,
        source: ImageCaptureSource,
        toplevel: ForeignToplevelHandle,
    ) {
        if let Some(window) = self.wayland.foreign_toplevels.window_of(&toplevel) {
            source
                .user_data()
                .insert_if_missing(|| CaptureSource::Window(window));
        }
    }
}

fn capture_source(source: &ImageCaptureSource) -> Option<CaptureSource> {
    source.user_data().get::<CaptureSource>().cloned()
}

impl ImageCopyCaptureHandler for State {
    fn image_copy_capture_state(&mut self) -> &mut ImageCopyCaptureState {
        &mut self.wayland.image_copy_capture_state
    }

    fn capture_constraints(&mut self, source: &ImageCaptureSource) -> Option<BufferConstraints> {
        let source = capture_source(source)?;
        let size = copy::image_size(&source.place(&self.shell)?);
        let dma = self.capture.dmabuf_constraints(&mut self.backend);
        Some(copy::constraints((size.w, size.h).into(), dma))
    }

    fn cursor_capture_constraints(
        &mut self,
        source: &ImageCaptureSource,
        _pointer: &WlPointer,
    ) -> Option<BufferConstraints> {
        let source = capture_source(source)?;
        let scale = source.place(&self.shell)?.scale();
        let cursor = &self.input.cursor;
        let size = shape_image(cursor, &cursor.status, scale)
            .map_or((1, 1), |image| (image.width, image.height));
        Some(cursor_constraints(Size::<i32, BufferCoords>::from(size)))
    }

    fn new_session(&mut self, session: Session) {
        if let Some(source) = capture_source(&session.source()) {
            self.capture.copies.push(CopySession::new(session, source));
        }
    }

    fn new_cursor_session(&mut self, session: CursorSession) {
        if let Some(source) = capture_source(&session.source()) {
            self.capture
                .copy_cursors
                .push(CopyCursor::new(session, source));
        }
    }

    fn frame(&mut self, session: &SessionRef, frame: Frame) {
        match self
            .capture
            .copies
            .iter_mut()
            .find(|copy| copy.session == *session)
        {
            Some(copy) => copy.pending.push(frame),
            None => frame.fail(CaptureFailureReason::Stopped),
        }
    }

    fn cursor_frame(&mut self, session: &CursorSessionRef, frame: Frame) {
        match self
            .capture
            .copy_cursors
            .iter_mut()
            .find(|cursor| cursor.session == *session)
        {
            Some(cursor) => cursor.pending.push(frame),
            None => frame.fail(CaptureFailureReason::Stopped),
        }
    }

    fn frame_aborted(&mut self, frame: FrameRef) {
        let capture = &mut self.capture;
        for copy in &mut capture.copies {
            copy.pending.retain(|pending| *pending != frame);
        }
        for cursor in &mut capture.copy_cursors {
            cursor.pending.retain(|pending| *pending != frame);
        }
    }

    fn session_destroyed(&mut self, session: SessionRef) {
        self.capture.copies.retain(|copy| copy.session != session);
    }

    fn cursor_session_destroyed(&mut self, session: CursorSessionRef) {
        self.capture
            .copy_cursors
            .retain(|cursor| cursor.session != session);
    }
}
