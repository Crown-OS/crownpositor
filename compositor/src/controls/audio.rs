//! Volume and mute through WirePlumber's `wpctl`.

use std::process::Command;

use super::{
    Step,
    osd::{Gauge, Level},
};

const SINK: &str = "@DEFAULT_AUDIO_SINK@";
const SOURCE: &str = "@DEFAULT_AUDIO_SOURCE@";

/// Raising never goes past 100%: amplifying past unity distorts on most sinks.
const VOLUME_LIMIT: &str = "1.0";

pub fn step_volume(step: Step) -> Option<Level> {
    // A volume key is a request to hear something, so stepping up unmutes.
    if step.0 > 0 {
        run(&["set-mute", SINK, "0"])?;
    }
    run(&["set-volume", "-l", VOLUME_LIMIT, SINK, &step.to_string()])?;
    read(SINK, Gauge::Volume)
}

pub fn toggle_mute() -> Option<Level> {
    run(&["set-mute", SINK, "toggle"])?;
    read(SINK, Gauge::Volume)
}

pub fn toggle_mic_mute() -> Option<Level> {
    run(&["set-mute", SOURCE, "toggle"])?;
    read(SOURCE, Gauge::Microphone)
}

fn read(node: &str, gauge: Gauge) -> Option<Level> {
    let output = run(&["get-volume", node])?;
    let (percent, muted) = parse_volume(&output)?;
    Some(Level {
        gauge,
        percent,
        muted,
    })
}

fn run(args: &[&str]) -> Option<String> {
    let output = Command::new("wpctl")
        .args(args)
        .output()
        .inspect_err(|error| tracing::warn!(%error, "cannot run wpctl"))
        .ok()?;
    if !output.status.success() {
        tracing::debug!(
            ?args,
            stderr = %String::from_utf8_lossy(&output.stderr),
            "wpctl failed"
        );
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

/// `Volume: 0.45` or `Volume: 0.45 [MUTED]`.
fn parse_volume(text: &str) -> Option<(u8, bool)> {
    let rest = text.trim().strip_prefix("Volume:")?.trim();
    let (number, flags) = rest.split_once(' ').unwrap_or((rest, ""));
    let fraction: f64 = number.parse().ok()?;
    let percent = (fraction * 100.0).round().clamp(0.0, 100.0) as u8;
    Some((percent, flags.contains("[MUTED]")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_reads_with_and_without_the_mute_flag() {
        assert_eq!(parse_volume("Volume: 0.45\n"), Some((45, false)));
        assert_eq!(parse_volume("Volume: 0.30 [MUTED]\n"), Some((30, true)));
        assert_eq!(parse_volume("Volume: 1.20"), Some((100, false)));
        assert_eq!(parse_volume("nonsense"), None);
    }
}
