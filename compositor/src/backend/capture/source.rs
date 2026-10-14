//! What a capture shows, and where that is on the desktop right now.

use smithay::{
    output::Output,
    utils::{Logical, Rectangle},
};

use crate::{
    shell::{Shell, monitor::Monitor, tile::Tile, workspace::Workspace},
    utils::id::{WindowId, WorkspaceId},
};

#[derive(Debug, Clone, PartialEq)]
pub enum CaptureSource {
    Output(Output),
    Window(WindowId),
    Workspace(WorkspaceId),
}

/// A source resolved against the shell for one frame.
pub struct Placement<'a> {
    pub monitor: &'a Monitor,
    pub view: View<'a>,
    /// The part of the global desktop the image covers.
    pub area: Rectangle<i32, Logical>,
    /// Whether the user sees the source right now, which is when the pointer
    /// belongs in its image.
    pub on_screen: bool,
}

pub enum View<'a> {
    Output,
    Window(&'a Tile),
    Workspace(&'a Workspace),
}

impl Placement<'_> {
    pub fn output(&self) -> &Output {
        self.monitor.output()
    }

    pub fn scale(&self) -> f64 {
        self.output().current_scale().fractional_scale()
    }
}

impl CaptureSource {
    /// `None` once the source is gone for good.
    pub fn place<'a>(&self, shell: &'a Shell) -> Option<Placement<'a>> {
        match self {
            Self::Output(output) => {
                let monitor = shell.monitor(output)?;
                Some(Placement {
                    monitor,
                    view: View::Output,
                    area: monitor.geometry(),
                    on_screen: true,
                })
            }
            Self::Window(id) => {
                let location = shell.location(*id)?;
                let monitor = shell.monitor_by_id(location.output)?;
                let tile = shell.tile(*id)?;
                let frame = tile.target();
                Some(Placement {
                    monitor,
                    view: View::Window(tile),
                    area: Rectangle::new(monitor.geometry().loc + frame.loc, frame.size),
                    on_screen: monitor.active().id() == location.workspace && tile.is_shown(),
                })
            }
            Self::Workspace(id) => {
                let monitor = shell
                    .monitors()
                    .iter()
                    .find(|monitor| monitor.workspace(*id).is_some())?;
                let workspace = monitor.workspace(*id)?;
                Some(Placement {
                    monitor,
                    view: View::Workspace(workspace),
                    area: monitor.geometry(),
                    on_screen: monitor.active().id() == *id,
                })
            }
        }
    }
}
