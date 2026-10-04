//! The double-buffered half of the protocol: what a client asked for since
//! its last commit.

use std::time::Duration;

use motion::Spring;
use smithay::wayland::compositor::Cacheable;
use wayland_server::DisplayHandle;

use super::property::{PROPERTIES, Property, index};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PropertyChange {
    Animate {
        target: f32,
        spring: Spring,
        /// `None` starts the animation when the commit is applied.
        start: Option<Duration>,
    },
    Set(f32),
}

/// One request per property survives until the commit: a later request for
/// the same property replaces an earlier one, so a client that never commits
/// cannot grow this.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PendingMotion {
    reset: bool,
    changes: [Option<PropertyChange>; PROPERTIES.len()],
}

impl PendingMotion {
    pub fn change(&mut self, property: Property, change: PropertyChange) {
        self.changes[index(property)] = Some(change);
    }

    /// Every property back at rest, discarding whatever was requested before.
    pub fn reset(&mut self) {
        *self = Self {
            reset: true,
            ..Self::default()
        };
    }

    pub fn is_reset(&self) -> bool {
        self.reset
    }

    pub fn changes(&self) -> impl Iterator<Item = (Property, PropertyChange)> + '_ {
        PROPERTIES
            .into_iter()
            .zip(self.changes)
            .filter_map(|(property, change)| Some((property, change?)))
    }
}

impl Cacheable for PendingMotion {
    fn commit(&mut self, _dh: &DisplayHandle) -> Self {
        std::mem::take(self)
    }

    fn merge_into(self, into: &mut Self, _dh: &DisplayHandle) {
        if self.reset {
            *into = self;
            return;
        }
        for (current, change) in into.changes.iter_mut().zip(self.changes) {
            if change.is_some() {
                *current = change;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use wayland_server::{Display, backend::InitError};

    use super::*;

    #[test]
    fn a_commit_hands_over_the_requests_and_clears_pending() -> Result<(), InitError> {
        let display = Display::<()>::new()?;
        let handle = display.handle();
        let mut pending = PendingMotion::default();
        pending.change(Property::Opacity, PropertyChange::Set(0.5));

        let committed = pending.commit(&handle);

        assert_eq!(pending, PendingMotion::default());
        assert_eq!(
            committed.changes().collect::<Vec<_>>(),
            vec![(Property::Opacity, PropertyChange::Set(0.5))]
        );
        Ok(())
    }

    #[test]
    fn later_commits_override_per_property_and_keep_the_rest() -> Result<(), InitError> {
        let display = Display::<()>::new()?;
        let handle = display.handle();
        let mut current = PendingMotion::default();
        current.change(Property::Opacity, PropertyChange::Set(0.5));
        current.change(Property::Scale, PropertyChange::Set(2.0));

        let mut later = PendingMotion::default();
        later.change(Property::Scale, PropertyChange::Set(3.0));
        later.merge_into(&mut current, &handle);

        assert_eq!(
            current.changes().collect::<Vec<_>>(),
            vec![
                (Property::Scale, PropertyChange::Set(3.0)),
                (Property::Opacity, PropertyChange::Set(0.5)),
            ]
        );
        Ok(())
    }

    #[test]
    fn a_reset_discards_what_came_before_it() -> Result<(), InitError> {
        let display = Display::<()>::new()?;
        let handle = display.handle();
        let mut current = PendingMotion::default();
        current.change(Property::Opacity, PropertyChange::Set(0.5));

        let mut later = PendingMotion::default();
        later.change(Property::Scale, PropertyChange::Set(3.0));
        later.reset();
        later.change(Property::TranslateX, PropertyChange::Set(4.0));
        later.merge_into(&mut current, &handle);

        assert!(current.is_reset());
        assert_eq!(
            current.changes().collect::<Vec<_>>(),
            vec![(Property::TranslateX, PropertyChange::Set(4.0))]
        );
        Ok(())
    }
}
