//! The keys every laptop has: volume, brightness and media transport.
//!
//! Each control runs a system tool (`wpctl`, `brightnessctl`, `playerctl`) on
//! the async pool and reports the new level through an on-screen display. One
//! worker applies them in order, so a held key queues steps rather than racing
//! a dozen tool invocations against each other.

mod audio;
mod brightness;
mod media;
mod osd;

use std::{fmt, str::FromStr};

use thiserror::Error;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use crate::utils::runtime::TaskSender;

pub use media::MediaCommand;
use osd::Osd;

/// A signed change, in percent of the full range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step(pub i8);

impl fmt::Display for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.0 < 0 { '-' } else { '+' };
        write!(f, "{}%{sign}", self.0.unsigned_abs())
    }
}

impl FromStr for Step {
    type Err = ParseControlError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.trim_end_matches('%')
            .parse::<i8>()
            .ok()
            .filter(|step| *step != 0 && step.unsigned_abs() <= 100)
            .map(Step)
            .ok_or_else(|| ParseControlError(s.to_owned()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Volume(Step),
    ToggleMute,
    ToggleMicMute,
    Brightness(Step),
    KeyboardBrightness(Step),
    Media(MediaCommand),
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid control `{0}`")]
pub struct ParseControlError(String);

impl Control {
    /// Whether holding the key keeps stepping, the way a held volume key does.
    pub fn repeats(self) -> bool {
        matches!(
            self,
            Self::Volume(_) | Self::Brightness(_) | Self::KeyboardBrightness(_)
        )
    }

    /// Parses the argument of a `volume`, `brightness`, `kbd-brightness` or
    /// `media` action.
    pub fn parse(name: &str, argument: &str) -> Result<Self, ParseControlError> {
        match (name, argument) {
            ("volume", "mute") => Ok(Self::ToggleMute),
            ("volume", step) => step.parse().map(Self::Volume),
            ("mic", "mute") => Ok(Self::ToggleMicMute),
            ("brightness", step) => step.parse().map(Self::Brightness),
            ("kbd-brightness", step) => step.parse().map(Self::KeyboardBrightness),
            ("media", command) => command.parse().map(Self::Media),
            _ => Err(ParseControlError(format!("{name} {argument}"))),
        }
    }

    /// Runs the tool and reads back the level to show, if this control has one.
    fn apply(self) -> Option<osd::Level> {
        match self {
            Self::Volume(step) => audio::step_volume(step),
            Self::ToggleMute => audio::toggle_mute(),
            Self::ToggleMicMute => audio::toggle_mic_mute(),
            Self::Brightness(step) => brightness::step(brightness::Panel::Display, step),
            Self::KeyboardBrightness(step) => brightness::step(brightness::Panel::Keyboard, step),
            Self::Media(command) => {
                media::send(command);
                None
            }
        }
    }
}

/// The compositor's handle on the control worker.
pub struct Controls {
    requests: UnboundedSender<Control>,
}

impl Controls {
    pub fn start(tasks: &TaskSender) -> Self {
        let (requests, queue) = mpsc::unbounded_channel();
        tasks.spawn(work(queue));
        Self { requests }
    }

    pub fn apply(&self, control: Control) {
        if self.requests.send(control).is_err() {
            tracing::warn!(?control, "the control worker is gone");
        }
    }
}

async fn work(mut queue: UnboundedReceiver<Control>) {
    let mut osd = Osd::default();
    while let Some(control) = queue.recv().await {
        let level = tokio::task::spawn_blocking(move || control.apply())
            .await
            .ok()
            .flatten();
        if let Some(level) = level {
            osd.show(level).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_read_with_or_without_a_percent_sign() {
        assert_eq!("+5".parse(), Ok(Step(5)));
        assert_eq!("-10%".parse(), Ok(Step(-10)));
        assert!("0".parse::<Step>().is_err());
        assert!("+101".parse::<Step>().is_err());
        assert!("loud".parse::<Step>().is_err());
    }

    #[test]
    fn steps_print_the_way_the_tools_take_them() {
        assert_eq!(Step(5).to_string(), "5%+");
        assert_eq!(Step(-5).to_string(), "5%-");
    }

    #[test]
    fn controls_parse_from_their_action_arguments() {
        assert_eq!(Control::parse("volume", "+5"), Ok(Control::Volume(Step(5))));
        assert_eq!(Control::parse("volume", "mute"), Ok(Control::ToggleMute));
        assert_eq!(Control::parse("mic", "mute"), Ok(Control::ToggleMicMute));
        assert_eq!(
            Control::parse("kbd-brightness", "-33"),
            Ok(Control::KeyboardBrightness(Step(-33)))
        );
        assert_eq!(
            Control::parse("media", "play-pause"),
            Ok(Control::Media(MediaCommand::PlayPause))
        );
        assert!(Control::parse("mic", "+5").is_err());
    }

    #[test]
    fn only_stepping_controls_repeat() {
        assert!(Control::Volume(Step(5)).repeats());
        assert!(!Control::ToggleMute.repeats());
        assert!(!Control::Media(MediaCommand::Next).repeats());
    }
}
