//! When to actually switch adaptive sync, as opposed to when it is wanted.
//!
//! A toggle is a blocking modeset-class commit, so an on-demand change has to
//! hold for a while first: glancing at the overview over a game must not
//! flip the panel twice.

use std::time::Duration;

/// How long an on-demand change must hold before the hardware follows.
const HOLD: Duration = Duration::from_millis(750);

#[derive(Debug, Default)]
pub struct VrrGate {
    /// The state wanted since `since`, when it differs from the hardware's.
    pending: Option<(bool, Duration)>,
}

impl VrrGate {
    /// The state to switch the hardware to now, if any. `immediate` skips the
    /// hold, for policies the user set explicitly.
    pub fn decide(
        &mut self,
        wanted: bool,
        current: bool,
        immediate: bool,
        now: Duration,
    ) -> Option<bool> {
        if wanted == current {
            self.pending = None;
            return None;
        }
        if immediate {
            self.pending = None;
            return Some(wanted);
        }
        match self.pending {
            Some((pending, since)) if pending == wanted => {
                if now.saturating_sub(since) < HOLD {
                    return None;
                }
                self.pending = None;
                Some(wanted)
            }
            _ => {
                self.pending = Some((wanted, now));
                None
            }
        }
    }

    /// Whether a change is waiting out its hold, which needs frames to keep
    /// coming or it would never be re-checked.
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    #[test]
    fn nothing_happens_when_the_hardware_already_agrees() {
        let mut gate = VrrGate::default();
        assert_eq!(gate.decide(true, true, false, at(0)), None);
        assert!(!gate.is_pending());
    }

    #[test]
    fn an_explicit_policy_switches_at_once() {
        let mut gate = VrrGate::default();
        assert_eq!(gate.decide(true, false, true, at(0)), Some(true));
    }

    #[test]
    fn an_on_demand_change_waits_out_the_hold() {
        let mut gate = VrrGate::default();
        assert_eq!(gate.decide(true, false, false, at(0)), None);
        assert!(gate.is_pending());
        assert_eq!(gate.decide(true, false, false, at(500)), None);
        assert_eq!(gate.decide(true, false, false, at(800)), Some(true));
        assert!(!gate.is_pending());
    }

    #[test]
    fn a_change_that_reverts_inside_the_hold_never_lands() {
        let mut gate = VrrGate::default();
        assert_eq!(gate.decide(false, true, false, at(0)), None);
        assert_eq!(
            gate.decide(true, true, false, at(300)),
            None,
            "back to what it was"
        );
        assert!(!gate.is_pending());
        assert_eq!(
            gate.decide(false, true, false, at(900)),
            None,
            "the hold restarts"
        );
    }
}
