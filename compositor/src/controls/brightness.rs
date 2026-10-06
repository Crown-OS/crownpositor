//! Backlights through `brightnessctl`.

use std::process::Command;

use super::{
    Step,
    osd::{Gauge, Level},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Display,
    Keyboard,
}

impl Panel {
    fn selector(self) -> &'static str {
        match self {
            Self::Display => "--class=backlight",
            Self::Keyboard => "--device=*::kbd_backlight",
        }
    }

    fn gauge(self) -> Gauge {
        match self {
            Self::Display => Gauge::Display,
            Self::Keyboard => Gauge::Keyboard,
        }
    }
}

/// `--min-value` keeps the display from stepping into a fully dark backlight,
/// which on most panels is indistinguishable from the screen being off.
pub fn step(panel: Panel, step: Step) -> Option<Level> {
    let mut command = Command::new("brightnessctl");
    command.args([panel.selector(), "--machine-readable"]);
    if panel == Panel::Display {
        command.arg("--min-value=1");
    }
    let output = command
        .args(["set", &step.to_string()])
        .output()
        .inspect_err(|error| tracing::warn!(%error, "cannot run brightnessctl"))
        .ok()?;
    if !output.status.success() {
        tracing::debug!(?panel, "no backlight brightnessctl can drive");
        return None;
    }
    let percent = parse_percent(&String::from_utf8_lossy(&output.stdout))?;
    Some(Level {
        gauge: panel.gauge(),
        percent,
        muted: false,
    })
}

/// `--machine-readable` prints `device,class,current,percent%,max`.
fn parse_percent(text: &str) -> Option<u8> {
    let field = text.lines().last()?.split(',').nth(3)?;
    let percent: u16 = field.trim().trim_end_matches('%').parse().ok()?;
    Some(percent.min(100) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_percent_column_is_read() {
        assert_eq!(
            parse_percent("amdgpu_bl1,backlight,128,50%,255\n"),
            Some(50)
        );
        assert_eq!(parse_percent("garbage"), None);
    }
}
