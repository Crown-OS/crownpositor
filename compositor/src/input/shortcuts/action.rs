//! The single action vocabulary.
//!
//! Keyboard chords, touchpad gestures and later IPC all resolve to an [`Action`]
//! and go down one dispatch path, `State::handle_action`. No second enum for
//! gestures — that is how "swipe left" and "Super+Tab" drift apart.

use std::str::FromStr;

use thiserror::Error;

/// Which way a focus or move operation goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl FromStr for Direction {
    type Err = ParseActionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "left" => Ok(Self::Left),
            "right" => Ok(Self::Right),
            "up" => Ok(Self::Up),
            "down" => Ok(Self::Down),
            other => Err(ParseActionError::bad_argument("direction", other)),
        }
    }
}

pub use crate::shell::workspace::WorkspaceRef;

impl FromStr for WorkspaceRef {
    type Err = ParseActionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == "prev" || s == "previous" {
            return Ok(Self::Previous);
        }
        // A leading sign means relative, a bare number absolute.
        if let Some(rest) = s.strip_prefix(['+', '-']) {
            let magnitude: i32 = rest
                .parse()
                .map_err(|_| ParseActionError::bad_argument("workspace", s))?;
            let signed = if s.starts_with('-') {
                -magnitude
            } else {
                magnitude
            };
            return Ok(Self::Relative(signed));
        }
        s.parse::<usize>()
            .map(Self::Index)
            .map_err(|_| ParseActionError::bad_argument("workspace", s))
    }
}

pub use crate::layout::{SnapZone, WorkspaceMode};

/// Parses the `set-mode` argument. The spellings are the config's own, in
/// kebab case.
pub fn parse_mode(s: &str) -> Result<WorkspaceMode, ParseActionError> {
    match s {
        "tiling" | "tiled" => Ok(WorkspaceMode::Tiling),
        "floating" => Ok(WorkspaceMode::Floating),
        other => Err(ParseActionError::bad_argument("mode", other)),
    }
}

/// Parses the `snap` argument.
pub fn parse_zone(s: &str) -> Result<SnapZone, ParseActionError> {
    match s {
        "left" => Ok(SnapZone::LeftHalf),
        "right" => Ok(SnapZone::RightHalf),
        "top-left" => Ok(SnapZone::TopLeft),
        "top-right" => Ok(SnapZone::TopRight),
        "bottom-left" => Ok(SnapZone::BottomLeft),
        "bottom-right" => Ok(SnapZone::BottomRight),
        "maximize" | "top" => Ok(SnapZone::Maximize),
        other => Err(ParseActionError::bad_argument("zone", other)),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    None,

    Quit,
    ReloadConfig,
    Spawn(Vec<String>),

    SwitchVt(i32),

    CloseWindow,
    Focus(Direction),
    MoveWindow(Direction),
    FocusOutput(Direction),
    MoveWindowToOutput(Direction),

    Workspace(WorkspaceRef),
    MoveWindowToWorkspace {
        target: WorkspaceRef,
        follow: bool,
    },
    MoveWorkspaceToOutput(Direction),

    ToggleFloating,
    ToggleFullscreen,
    ToggleMaximize,

    /// Flips the focused workspace between tiling and floating.
    ToggleWorkspaceMode,
    SetWorkspaceMode(WorkspaceMode),
    /// Parks the focused window on an edge, as a drag to that edge would.
    SnapWindow(SnapZone),
    /// Dismisses an open application menu.
    CloseMenu,

    OpenWorkspaceView,
    CloseWorkspaceView,
    /// One chord for both, the way a dedicated mission-control key behaves.
    ToggleWorkspaceView,

    /// Grow or shrink the master column by a fraction of the area.
    ResizeSplit(f64),
    /// Into or out of the master column.
    PromoteDemote,
    ResetSize,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ParseActionError {
    #[error("empty action")]
    Empty,
    #[error("unknown action `{0}`")]
    UnknownAction(String),
    #[error("invalid {what} `{got}`")]
    BadArgument { what: &'static str, got: String },
    #[error("`{action}` needs a {what}")]
    MissingArgument { action: String, what: &'static str },
}

impl ParseActionError {
    fn bad_argument(what: &'static str, got: &str) -> Self {
        Self::BadArgument {
            what,
            got: got.to_owned(),
        }
    }
}

impl FromStr for Action {
    type Err = ParseActionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split_whitespace();
        let name = parts.next().ok_or(ParseActionError::Empty)?;

        let arg = |what: &'static str, mut rest: std::str::SplitWhitespace<'_>| {
            rest.next()
                .map(str::to_owned)
                .ok_or_else(|| ParseActionError::MissingArgument {
                    action: name.to_owned(),
                    what,
                })
        };

        match name {
            "none" => Ok(Self::None),
            "quit" | "exit" => Ok(Self::Quit),
            "reload-config" => Ok(Self::ReloadConfig),

            "spawn" => {
                let argv: Vec<String> = parts.map(str::to_owned).collect();
                if argv.is_empty() {
                    Err(ParseActionError::MissingArgument {
                        action: name.to_owned(),
                        what: "program",
                    })
                } else {
                    Ok(Self::Spawn(argv))
                }
            }

            // Ctrl+Alt+F<n> is wired up unconditionally in the keyboard filter;
            // this spelling exists so a config can put a VT on another chord.
            "switch-vt" => {
                let raw = arg("vt", parts)?;
                let vt: i32 = raw
                    .parse()
                    .map_err(|_| ParseActionError::bad_argument("vt", &raw))?;
                Ok(Self::SwitchVt(vt))
            }

            "close-window" | "close" => Ok(Self::CloseWindow),
            "focus" => Ok(Self::Focus(arg("direction", parts)?.parse()?)),
            "move" | "move-window" => Ok(Self::MoveWindow(arg("direction", parts)?.parse()?)),
            "focus-output" => Ok(Self::FocusOutput(arg("direction", parts)?.parse()?)),
            "move-to-output" => Ok(Self::MoveWindowToOutput(arg("direction", parts)?.parse()?)),
            "move-workspace-to-output" => Ok(Self::MoveWorkspaceToOutput(
                arg("direction", parts)?.parse()?,
            )),

            "workspace" => Ok(Self::Workspace(arg("workspace", parts)?.parse()?)),
            "move-to-workspace" => {
                let target: WorkspaceRef = arg("workspace", parts.clone())?.parse()?;
                // Opt-in: being yanked along with the window is surprising.
                let follow = parts.skip(1).any(|token| token == "follow");
                Ok(Self::MoveWindowToWorkspace { target, follow })
            }

            "toggle-float" | "toggle-floating" => Ok(Self::ToggleFloating),
            "toggle-fullscreen" => Ok(Self::ToggleFullscreen),
            "toggle-maximize" => Ok(Self::ToggleMaximize),

            "toggle-mode" | "toggle-workspace-mode" => Ok(Self::ToggleWorkspaceMode),
            "set-mode" => Ok(Self::SetWorkspaceMode(parse_mode(&arg("mode", parts)?)?)),
            "snap" => Ok(Self::SnapWindow(parse_zone(&arg("zone", parts)?)?)),
            "close-menu" => Ok(Self::CloseMenu),

            "open-workspace-view" => Ok(Self::OpenWorkspaceView),
            "close-workspace-view" => Ok(Self::CloseWorkspaceView),
            "toggle-workspace-view" => Ok(Self::ToggleWorkspaceView),

            "resize-split" => {
                let raw = arg("fraction", parts)?;
                let fraction: f64 = raw
                    .parse()
                    .map_err(|_| ParseActionError::bad_argument("fraction", &raw))?;
                Ok(Self::ResizeSplit(fraction))
            }
            "promote" | "demote" => Ok(Self::PromoteDemote),
            "reset-size" => Ok(Self::ResetSize),

            other => Err(ParseActionError::UnknownAction(other.to_owned())),
        }
    }
}

impl From<Direction> for crate::layout::Direction {
    fn from(dir: Direction) -> Self {
        match dir {
            Direction::Left => Self::Left,
            Direction::Right => Self::Right,
            Direction::Up => Self::Up,
            Direction::Down => Self::Down,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Action {
        s.parse()
            .unwrap_or_else(|err| panic!("`{s}` failed to parse: {err}"))
    }

    #[test]
    fn parses_bare_actions() {
        assert_eq!(parse("quit"), Action::Quit);
        assert_eq!(parse("close-window"), Action::CloseWindow);
        assert_eq!(parse("toggle-fullscreen"), Action::ToggleFullscreen);
    }

    #[test]
    fn spawn_collects_argv() {
        assert_eq!(
            parse("spawn foot -e nvim"),
            Action::Spawn(vec!["foot".into(), "-e".into(), "nvim".into()])
        );
    }

    #[test]
    fn spawn_without_a_program_is_an_error() {
        assert!("spawn".parse::<Action>().is_err());
    }

    #[test]
    fn switch_vt_takes_a_number() {
        assert_eq!(parse("switch-vt 3"), Action::SwitchVt(3));
        assert!("switch-vt".parse::<Action>().is_err());
        assert!("switch-vt tty3".parse::<Action>().is_err());
    }

    #[test]
    fn absolute_and_relative_workspaces_differ() {
        assert_eq!(
            parse("workspace 3"),
            Action::Workspace(WorkspaceRef::Index(3))
        );
        assert_eq!(
            parse("workspace +1"),
            Action::Workspace(WorkspaceRef::Relative(1))
        );
        assert_eq!(
            parse("workspace -2"),
            Action::Workspace(WorkspaceRef::Relative(-2))
        );
        assert_eq!(
            parse("workspace prev"),
            Action::Workspace(WorkspaceRef::Previous)
        );
    }

    #[test]
    fn follow_is_opt_in() {
        assert_eq!(
            parse("move-to-workspace 0"),
            Action::MoveWindowToWorkspace {
                target: WorkspaceRef::Index(0),
                follow: false
            }
        );
        assert_eq!(
            parse("move-to-workspace 0 follow"),
            Action::MoveWindowToWorkspace {
                target: WorkspaceRef::Index(0),
                follow: true
            }
        );
    }

    #[test]
    fn errors_name_the_offending_token() {
        let err = "focus sideways".parse::<Action>().unwrap_err();
        assert_eq!(err.to_string(), "invalid direction `sideways`");

        let err = "frobnicate".parse::<Action>().unwrap_err();
        assert_eq!(err.to_string(), "unknown action `frobnicate`");

        let err = "focus".parse::<Action>().unwrap_err();
        assert_eq!(err.to_string(), "`focus` needs a direction");
    }
}
