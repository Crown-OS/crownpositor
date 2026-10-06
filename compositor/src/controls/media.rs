//! Media transport through `playerctl`, aimed at whichever MPRIS player is
//! active.

use std::{process::Command, str::FromStr};

use super::ParseControlError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaCommand {
    PlayPause,
    Stop,
    Next,
    Previous,
}

impl MediaCommand {
    fn verb(self) -> &'static str {
        match self {
            Self::PlayPause => "play-pause",
            Self::Stop => "stop",
            Self::Next => "next",
            Self::Previous => "previous",
        }
    }
}

impl FromStr for MediaCommand {
    type Err = ParseControlError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "play-pause" => Ok(Self::PlayPause),
            "stop" => Ok(Self::Stop),
            "next" => Ok(Self::Next),
            "previous" | "prev" => Ok(Self::Previous),
            other => Err(ParseControlError(other.to_owned())),
        }
    }
}

pub fn send(command: MediaCommand) {
    match Command::new("playerctl").arg(command.verb()).status() {
        Ok(status) if !status.success() => {
            tracing::debug!(?command, "no media player answered");
        }
        Err(error) => tracing::warn!(%error, "cannot run playerctl"),
        Ok(_) => {}
    }
}
