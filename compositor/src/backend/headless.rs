//! The output behind `WORKSPACE=headless`: a compositor-owned virtual
//! output nobody can see or reach, ticking its own frame clock so the windows
//! on it keep drawing at full rate. Created on first use, and taken down once
//! it has stayed empty for a while.

use std::time::{Duration, Instant};

use calloop::timer::{TimeoutAction, Timer};
use smithay::output::Output;

use protocols::crownos_virtual_output::VirtualMode;

use crate::{
    backend::virtual_output::{Owner, refresh_interval},
    shell::{Location, monitor::Presence, workspace::Workspace},
    state::State,
};

/// Outside the `VIRTUAL-` namespace, so no client can take the name first.
const NAME: &str = "HEADLESS-1";
const WIDTH: i32 = 3840;
const HEIGHT: i32 = 2160;
const REFRESH_MHZ: u32 = 60_000;
const SCALE_120: u32 = 120;
/// Long enough to survive an app restarting, so its layer clients are not
/// torn down and rebuilt in between.
const LINGER: Duration = Duration::from_secs(10);

impl State {
    /// Where a headless window maps, bringing the output up if it is not.
    pub fn headless_location(&mut self) -> Option<Location> {
        let output = match self.virtual_outputs.compositor_owned() {
            Some(output) => output.clone(),
            None => self.create_headless_output()?,
        };
        let monitor = self.shell.monitor(&output)?;
        Some(Location {
            output: monitor.id(),
            workspace: monitor.active().id(),
        })
    }

    fn create_headless_output(&mut self) -> Option<Output> {
        let mode = match VirtualMode::new(WIDTH, HEIGHT, REFRESH_MHZ, SCALE_120) {
            Ok(mode) => mode,
            Err(error) => {
                tracing::error!(%error, "invalid headless mode");
                return None;
            }
        };
        let output = self.add_virtual_head(Owner::Compositor, NAME, mode, Presence::Hidden)?;
        self.pace_headless(refresh_interval(mode));
        Some(output)
    }

    fn pace_headless(&mut self, interval: Duration) {
        let mut empty_since: Option<Instant> = None;
        let paced = self.common.event_loop_handle.insert_source(
            Timer::from_duration(interval),
            move |deadline, _, state| {
                if !state.headless_is_empty() {
                    empty_since = None;
                } else if deadline.duration_since(*empty_since.get_or_insert(deadline)) >= LINGER {
                    state.remove_compositor_owned();
                    return TimeoutAction::Drop;
                }
                state.virtual_outputs.request_compositor_frame();
                TimeoutAction::ToDuration(interval)
            },
        );
        if let Err(error) = paced {
            tracing::warn!(%error, "the headless output has no frame clock");
        }
    }

    fn headless_is_empty(&self) -> bool {
        self.virtual_outputs
            .compositor_owned()
            .and_then(|output| self.shell.monitor(output))
            .is_none_or(|monitor| monitor.workspaces().iter().all(Workspace::is_empty))
    }
}
