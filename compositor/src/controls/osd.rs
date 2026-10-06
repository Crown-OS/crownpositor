//! The level readout, posted as a freedesktop notification.
//!
//! crownotify draws a notification carrying the `value` hint as a progress
//! card, keeps a `transient` one out of its history, and replaces a
//! notification in place when it is posted again under the same id — which is
//! everything an on-screen display needs, through the standard interface any
//! notification daemon speaks.

use std::collections::HashMap;

use zbus::{Connection, zvariant::Value};

/// What a level measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gauge {
    Volume,
    Microphone,
    Display,
    Keyboard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Level {
    pub gauge: Gauge,
    pub percent: u8,
    pub muted: bool,
}

impl Level {
    fn summary(self) -> &'static str {
        match self.gauge {
            Gauge::Volume => "Volume",
            Gauge::Microphone => "Microphone",
            Gauge::Display => "Brightness",
            Gauge::Keyboard => "Keyboard Brightness",
        }
    }

    fn body(self) -> &'static str {
        if self.muted { "Muted" } else { "" }
    }

    fn icon(self) -> &'static str {
        match (self.gauge, self.muted, self.percent) {
            (Gauge::Volume, true, _) | (Gauge::Volume, _, 0) => "audio-volume-muted",
            (Gauge::Volume, _, 1..=33) => "audio-volume-low",
            (Gauge::Volume, _, 34..=66) => "audio-volume-medium",
            (Gauge::Volume, _, _) => "audio-volume-high",
            (Gauge::Microphone, true, _) => "microphone-sensitivity-muted",
            (Gauge::Microphone, false, _) => "microphone-sensitivity-high",
            (Gauge::Display, _, _) => "display-brightness",
            (Gauge::Keyboard, _, _) => "keyboard-brightness",
        }
    }
}

const APP_NAME: &str = "CrownOS";
const TIMEOUT_MS: i32 = 1500;

/// One bubble for every gauge: stepping the volume and then the brightness
/// changes what the display reads rather than stacking a second one.
#[derive(Default)]
pub struct Osd {
    bus: Option<Connection>,
    id: u32,
}

impl Osd {
    pub async fn show(&mut self, level: Level) {
        let Some(bus) = self.bus().await else {
            return;
        };
        let hints: HashMap<&str, Value<'_>> = HashMap::from([
            ("value", Value::from(i32::from(level.percent))),
            ("transient", Value::from(true)),
            (
                "x-canonical-private-synchronous",
                Value::from("crownos-osd"),
            ),
        ]);
        let reply = bus
            .call_method(
                Some("org.freedesktop.Notifications"),
                "/org/freedesktop/Notifications",
                Some("org.freedesktop.Notifications"),
                "Notify",
                &(
                    APP_NAME,
                    self.id,
                    level.icon(),
                    level.summary(),
                    level.body(),
                    Vec::<&str>::new(),
                    hints,
                    TIMEOUT_MS,
                ),
            )
            .await;
        match reply.and_then(|message| message.body().deserialize::<u32>()) {
            Ok(id) => self.id = id,
            Err(error) => tracing::debug!(%error, "no notification daemon took the OSD"),
        }
    }

    async fn bus(&mut self) -> Option<Connection> {
        if self.bus.is_none() {
            self.bus = Connection::session()
                .await
                .inspect_err(|error| tracing::warn!(%error, "no session bus for the OSD"))
                .ok();
        }
        self.bus.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_muted_volume_shows_the_muted_icon_whatever_its_level() {
        let level = Level {
            gauge: Gauge::Volume,
            percent: 80,
            muted: true,
        };
        assert_eq!(level.icon(), "audio-volume-muted");
        assert_eq!(level.body(), "Muted");
    }
}
