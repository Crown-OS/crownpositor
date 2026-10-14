use anyhow::anyhow;
use calloop::{
    ping::make_ping,
    timer::{TimeoutAction, Timer},
};

use crate::state::State;

impl State {
    /// Redraws the pointer each time an animated cursor shape is due its next
    /// frame. The cursor pings when it draws an animated frame; the timer for
    /// the frame after is armed here, where the event loop is.
    pub(super) fn animate_cursor(&mut self) -> anyhow::Result<()> {
        let (wake, source) =
            make_ping().map_err(|err| anyhow!("Failed to create the cursor wake-up: {err}"))?;
        self.common
            .event_loop_handle
            .insert_source(source, |(), (), state| {
                let Some(due) = state.input.cursor.animation_due() else {
                    return;
                };
                let timer = state.common.event_loop_handle.insert_source(
                    Timer::from_deadline(due),
                    |_, (), state| {
                        state.input.cursor.animation_frame_reached();
                        state.queue_pointer_redraw();
                        TimeoutAction::Drop
                    },
                );
                if let Err(err) = timer {
                    tracing::warn!(%err, "failed to schedule the next cursor frame");
                    state.input.cursor.animation_frame_reached();
                }
            })
            .map_err(|err| anyhow!("Failed to insert the cursor wake-up: {err}"))?;
        self.input.cursor.set_animation_wake(wake);
        Ok(())
    }
}
