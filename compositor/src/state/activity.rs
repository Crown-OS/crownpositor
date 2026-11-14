//! How much the user does, never what: counts of key presses by kind, clicks,
//! scroll ticks and pointer travel, bucketed for `crownos_context_v1`.

use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

use protocols::crownos_context::{ActivityFlags, InputActivity};
use smithay::backend::input::{
    Axis, ButtonState, InputBackend, InputEvent, KeyState, KeyboardKeyEvent, PointerAxisEvent,
    PointerButtonEvent, PointerMotionEvent,
};

/// How often buckets are handed over.
pub const FLUSH_INTERVAL: Duration = Duration::from_secs(2);
/// Without a printable key for this long, typing counts as paused.
const TYPING_PAUSE: Duration = Duration::from_millis(1_500);
const SCROLL_STOP: Duration = Duration::from_millis(500);
/// XKB keycodes are evdev codes offset by 8.
const XKB_OFFSET: u32 = 8;
const KEY_BACKSPACE: u32 = 14;
const KEY_DELETE: u32 = 111;
const NAV_KEYS: [u32; 8] = [102, 103, 104, 105, 106, 107, 108, 109];
const MODIFIER_KEYS: [u32; 6] = [29, 97, 56, 100, 125, 126];
const V120_PER_TICK: f64 = 120.0;
const TRAVEL_BUCKET: f64 = 100.0;

#[derive(Debug)]
pub struct ActivityCounters {
    bucket: InputActivity,
    since: Instant,
    held_modifiers: HashSet<u32>,
    last_text: Option<Instant>,
    last_scroll: Option<Instant>,
    travel: f64,
}

impl Default for ActivityCounters {
    fn default() -> Self {
        Self {
            bucket: InputActivity::default(),
            since: Instant::now(),
            held_modifiers: HashSet::new(),
            last_text: None,
            last_scroll: None,
            travel: 0.0,
        }
    }
}

impl ActivityCounters {
    pub fn observe<I: InputBackend>(&mut self, event: &InputEvent<I>) {
        match event {
            InputEvent::Keyboard { event } => {
                let code = event.key_code().raw().saturating_sub(XKB_OFFSET);
                self.observe_key(code, event.state() == KeyState::Pressed);
            }
            InputEvent::PointerButton { event } if event.state() == ButtonState::Pressed => {
                self.bucket.clicks += 1;
            }
            InputEvent::PointerAxis { event } => {
                let ticks = [Axis::Vertical, Axis::Horizontal]
                    .into_iter()
                    .filter_map(|axis| event.amount_v120(axis))
                    .map(|amount| (amount / V120_PER_TICK).abs().ceil())
                    .sum::<f64>()
                    .max(1.0);
                self.bucket.scroll_ticks += ticks as u32;
                self.last_scroll = Some(Instant::now());
            }
            InputEvent::PointerMotion { event } => {
                let delta = event.delta();
                self.travel += delta.x.abs() + delta.y.abs();
            }
            _ => {}
        }
    }

    fn observe_key(&mut self, code: u32, pressed: bool) {
        if MODIFIER_KEYS.contains(&code) {
            if pressed {
                self.held_modifiers.insert(code);
            } else {
                self.held_modifiers.remove(&code);
            }
            return;
        }
        if !pressed {
            return;
        }
        if !self.held_modifiers.is_empty() {
            self.bucket.chords += 1;
        } else if code == KEY_BACKSPACE || code == KEY_DELETE {
            self.bucket.edit += 1;
        } else if NAV_KEYS.contains(&code) {
            self.bucket.nav += 1;
        } else {
            self.bucket.text += 1;
            self.last_text = Some(Instant::now());
        }
    }

    /// The bucket since the last take, if anything happened.
    pub fn take(&mut self) -> Option<InputActivity> {
        let now = Instant::now();
        let mut bucket = std::mem::take(&mut self.bucket);
        bucket.pointer_travel = ((self.travel / TRAVEL_BUCKET).round() * TRAVEL_BUCKET) as u32;
        bucket.interval_ms =
            u32::try_from(now.duration_since(self.since).as_millis()).unwrap_or(u32::MAX);
        if self
            .last_text
            .is_some_and(|last| now.duration_since(last) >= TYPING_PAUSE)
        {
            bucket.flags |= ActivityFlags::TypingPaused;
            self.last_text = None;
        }
        if self
            .last_scroll
            .is_some_and(|last| now.duration_since(last) >= SCROLL_STOP)
        {
            bucket.flags |= ActivityFlags::ScrollStopped;
            self.last_scroll = None;
        }
        self.since = now;
        self.travel = 0.0;
        (!bucket.is_empty() || !bucket.flags.is_empty()).then_some(bucket)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_counted_by_kind_never_by_identity() {
        let mut counters = ActivityCounters::default();
        for code in [30, 48, KEY_BACKSPACE, 105] {
            counters.observe_key(code, true);
            counters.observe_key(code, false);
        }
        counters.observe_key(29, true);
        counters.observe_key(46, true);
        counters.observe_key(46, false);
        counters.observe_key(29, false);
        let bucket = counters.take().unwrap_or_default();
        assert_eq!(
            (bucket.text, bucket.edit, bucket.nav, bucket.chords),
            (2, 1, 1, 1)
        );
        assert!(counters.take().is_none());
    }
}
