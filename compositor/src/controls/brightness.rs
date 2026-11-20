//! The keyboard backlight through `brightnessctl`.

use std::process::Command;

use super::{
    Step,
    osd::{Gauge, Level},
};

pub fn step_keyboard(step: Step) -> Option<Level> {
    let output = Command::new("brightnessctl")
        .args(["--device=*::kbd_backlight", "--machine-readable"])
        .args(["set", &step.to_string()])
        .output()
        .inspect_err(|error| tracing::warn!(%error, "cannot run brightnessctl"))
        .ok()?;
    if !output.status.success() {
        tracing::debug!("no keyboard backlight brightnessctl can drive");
        return None;
    }
    let percent = parse_percent(&String::from_utf8_lossy(&output.stdout))?;
    Some(Level {
        gauge: Gauge::Keyboard,
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
