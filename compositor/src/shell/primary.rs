//! The primary monitor: the one the dock and notifications go to by default,
//! and where the pointer starts.

use smithay::utils::{Logical, Point};

use crate::{
    shell::{Shell, monitor::Monitor, output_kind::OutputKind},
    utils::{id::OutputId, panel},
};

#[derive(Debug, Clone, Copy)]
pub struct PrimaryCandidate<'a> {
    pub connector: &'a str,
    pub identity: &'a str,
    pub kind: OutputKind,
    pub position: Point<i32, Logical>,
}

/// Which candidate is primary: the configured one, by connector or identity,
/// then a built-in panel, then the top-left monitor (one that is not virtual
/// first).
pub fn choose_primary(candidates: &[PrimaryCandidate], configured: Option<&str>) -> Option<usize> {
    let configured = configured.and_then(|name| {
        candidates
            .iter()
            .position(|candidate| candidate.connector == name || candidate.identity == name)
    });
    let internal_panel = || {
        candidates
            .iter()
            .position(|candidate| panel::is_internal(candidate.connector))
    };
    let top_left = || {
        candidates
            .iter()
            .enumerate()
            .min_by_key(|(_, candidate)| {
                (
                    candidate.kind == OutputKind::Virtual,
                    candidate.position.y,
                    candidate.position.x,
                )
            })
            .map(|(index, _)| index)
    };
    configured.or_else(internal_panel).or_else(top_left)
}

impl Shell {
    /// The primary among the shown monitors, decided afresh on every call.
    pub fn primary(&self) -> Option<&Monitor> {
        self.primary_among(self.shown_monitors())
    }

    /// The primary the shown monitors would have without `excluded`.
    pub(super) fn primary_besides(&self, excluded: OutputId) -> Option<&Monitor> {
        self.primary_among(
            self.shown_monitors()
                .filter(move |monitor| monitor.id() != excluded),
        )
    }

    fn primary_among<'a>(
        &'a self,
        monitors: impl Iterator<Item = &'a Monitor>,
    ) -> Option<&'a Monitor> {
        let shown: Vec<&Monitor> = monitors.collect();
        let candidates: Vec<PrimaryCandidate> = shown
            .iter()
            .map(|monitor| PrimaryCandidate {
                connector: &monitor.config().name,
                identity: monitor.config().connector.as_str(),
                kind: monitor.config().kind,
                position: monitor.config().position,
            })
            .collect();
        let index = choose_primary(&candidates, self.primary_output.as_deref())?;
        shown.get(index).copied()
    }

    pub fn set_primary_output(&mut self, primary_output: Option<String>) {
        self.primary_output = primary_output;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(connector: &str, x: i32, y: i32) -> PrimaryCandidate<'_> {
        PrimaryCandidate {
            connector,
            identity: connector,
            kind: OutputKind::Physical,
            position: (x, y).into(),
        }
    }

    #[test]
    fn the_top_left_monitor_wins_when_nothing_else_decides() {
        let candidates = [
            candidate("DP-2", 1920, 0),
            candidate("DP-1", 0, 300),
            candidate("DP-3", 0, 0),
        ];
        assert_eq!(choose_primary(&candidates, None), Some(2));
    }

    #[test]
    fn a_virtual_monitor_is_primary_only_when_nothing_else_is_there() {
        let mut tablet = candidate("VIRTUAL-1", 0, 0);
        tablet.kind = OutputKind::Virtual;
        let candidates = [tablet, candidate("DP-1", 1920, 0)];
        assert_eq!(choose_primary(&candidates, None), Some(1));
        assert_eq!(choose_primary(&candidates[..1], None), Some(0));
    }

    #[test]
    fn nothing_to_choose_from_has_no_primary() {
        assert_eq!(choose_primary(&[], Some("DP-1")), None);
    }
}
