//! The single action vocabulary.
//!
//! Keyboard chords, touchpad gestures and later IPC all resolve to an [`Action`]
//! and go down one dispatch path, `State::handle_action`. No second enum for
//! gestures — that is how "swipe left" and "Super+Tab" drift apart.

use std::{fmt, str::FromStr};

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
            let signed = if s.starts_with('-') { -magnitude } else { magnitude };
            return Ok(Self::Relative(signed));
        }
        s.parse::<usize>()
            .map(Self::Index)
            .map_err(|_| ParseActionError::bad_argument("workspace", s))
    }
}

/// Which layout a `set-layout` action names. Mirrors the config vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutSelection {
    MasterStack,
    ScrollingColumns,
    Floating,
}

impl FromStr for LayoutSelection {
    type Err = ParseActionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "master-stack" | "master" => Ok(Self::MasterStack),
            "scrolling-columns" | "scrolling" => Ok(Self::ScrollingColumns),
            "floating" => Ok(Self::Floating),
            other => Err(ParseActionError::bad_argument("layout", other)),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// The swallowed release of an intercepted chord. Dispatch does nothing for
    /// it; it exists so the release never reaches the client unpaired.
    None,

    Quit,
    ReloadConfig,
    /// argv, never a shell string.
    Spawn(Vec<String>),

    /// Hand the seat to another virtual terminal. Only the KMS backend can do
    /// this; nested backends log and ignore it.
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

    /// Flips the compositor-wide default; explicit per-workspace overrides stay.
    ToggleLayoutMode,
    /// Cycles this workspace's override: none -> master -> scrolling -> none.
    CycleLayout,
    SetLayout(LayoutSelection),

    OpenWorkspaceView,
    CloseWorkspaceView,

    /// Grow or shrink the layout's primary split by a fraction of the area.
    ResizeSplit(f64),
    /// Into or out of the master area; full width in a scrolling layout.
    PromoteDemote,
    /// Cycle the focused window through the layout's preset sizes.
    CycleSize,
    ResetSize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseActionError {
    Empty,
    UnknownAction(String),
    BadArgument { what: &'static str, got: String },
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

impl fmt::Display for ParseActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "empty action"),
            Self::UnknownAction(name) => write!(f, "unknown action `{name}`"),
            Self::BadArgument { what, got } => write!(f, "invalid {what} `{got}`"),
            Self::MissingArgument { action, what } => {
                write!(f, "`{action}` needs a {what}")
            }
        }
    }
}

impl std::error::Error for ParseActionError {}

impl FromStr for Action {
    type Err = ParseActionError;

    /// Parses the on-disk form: `"quit"`, `"spawn foot -e nvim"`,
    /// `"focus left"`, `"workspace +1"`, `"move-to-workspace 0 follow"`.
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

            // Everything after `spawn` is argv. Arguments containing spaces are
            // not expressible; that is the trade for never invoking a shell.
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
            "move-workspace-to-output" => {
                Ok(Self::MoveWorkspaceToOutput(arg("direction", parts)?.parse()?))
            }

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

            "toggle-layout-mode" => Ok(Self::ToggleLayoutMode),
            "cycle-layout" => Ok(Self::CycleLayout),
            "set-layout" => Ok(Self::SetLayout(arg("layout", parts)?.parse()?)),

            "open-workspace-view" => Ok(Self::OpenWorkspaceView),
            "close-workspace-view" => Ok(Self::CloseWorkspaceView),

            "resize-split" => {
                let raw = arg("fraction", parts)?;
                let fraction: f64 = raw
                    .parse()
                    .map_err(|_| ParseActionError::bad_argument("fraction", &raw))?;
                Ok(Self::ResizeSplit(fraction))
            }
            "promote" | "demote" => Ok(Self::PromoteDemote),
            "cycle-size" => Ok(Self::CycleSize),
            "reset-size" => Ok(Self::ResetSize),

            other => Err(ParseActionError::UnknownAction(other.to_owned())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Action {
        s.parse().unwrap_or_else(|err| panic!("`{s}` failed to parse: {err}"))
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
        assert_eq!(parse("workspace 3"), Action::Workspace(WorkspaceRef::Index(3)));
        assert_eq!(parse("workspace +1"), Action::Workspace(WorkspaceRef::Relative(1)));
        assert_eq!(parse("workspace -2"), Action::Workspace(WorkspaceRef::Relative(-2)));
        assert_eq!(parse("workspace prev"), Action::Workspace(WorkspaceRef::Previous));
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

impl From<LayoutSelection> for crate::layout::LayoutKind {
    fn from(selection: LayoutSelection) -> Self {
        match selection {
            LayoutSelection::MasterStack => Self::MasterStack,
            LayoutSelection::ScrollingColumns => Self::ScrollingColumns,
            LayoutSelection::Floating => Self::Floating,
        }
    }
}
